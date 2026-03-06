use crate::{ToolError, ToolExecutor};
use async_trait::async_trait;
use serde_json::Value;
use wasmtime::{Config, Engine, Instance, Module, Store};

#[derive(Clone)]
pub struct SandboxConfig {
    pub max_memory_bytes: usize,
    pub max_fuel: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_bytes: 50 * 1024 * 1024, // 50MB
            max_fuel: 10_000_000,
        }
    }
}

#[derive(Clone)]
pub struct WasmExecutor {
    engine: Engine,
    module: Module,
    config: SandboxConfig,
}

impl WasmExecutor {
    pub fn new(wasm_bytes: Vec<u8>, config: SandboxConfig) -> Result<Self, String> {
        let mut engine_config = Config::new();
        engine_config.consume_fuel(true);

        let engine = Engine::new(&engine_config).map_err(|e| format!("Sandbox engine init failed: {}", e))?;
        let module = Module::new(&engine, &wasm_bytes)
            .map_err(|e| format!("Failed to compile module: {}", e))?;

        Ok(Self { engine, module, config })
    }
}

#[async_trait]
impl ToolExecutor for WasmExecutor {
    async fn call(&self, params: Value) -> Result<Vec<u8>, ToolError> {
        // We clone these cheaply to move them into the blocking task.
        let engine = self.engine.clone();
        let module = self.module.clone();
        let config = self.config.clone();

        // Execute synchronously in a blocking thread to avoid starving the tokio runtime
        tokio::task::spawn_blocking(move || {
            struct Limiter {
                max_memory: usize,
            }

            impl wasmtime::ResourceLimiter for Limiter {
                fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> Result<bool, anyhow::Error> {
                    Ok(desired <= self.max_memory)
                }
                fn table_growing(&mut self, _current: u32, _desired: u32, _maximum: Option<u32>) -> Result<bool, anyhow::Error> {
                    Ok(true)
                }
            }

            let mut store = Store::new(&engine, Limiter { max_memory: config.max_memory_bytes });

            // Add fuel
            store.add_fuel(config.max_fuel)
                 .map_err(|_| ToolError::ExecutionFailed("Failed to set fuel".into()))?;

            store.limiter(|state| state as &mut dyn wasmtime::ResourceLimiter);

            let imports = [];
            let instance = Instance::new(&mut store, &module, &imports)
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to instantiate module: {}", e)))?;

            let memory = instance.get_memory(&mut store, "memory");

            let input_bytes = serde_json::to_vec(&params)
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to serialize params: {}", e)))?;

            let mut arg_ptr: i32 = 0;
            let mut arg_len: i32 = 0;

            if let Some(mem) = memory {
                let alloc = instance.get_typed_func::<i32, i32>(&mut store, "alloc");
                if let Ok(alloc_fn) = alloc {
                    let len = input_bytes.len() as i32;
                    if let Ok(ptr) = alloc_fn.call(&mut store, len) {
                        if mem.write(&mut store, ptr as usize, &input_bytes).is_ok() {
                            arg_ptr = ptr;
                            arg_len = len;
                        }
                    }
                }
            }

            let execute = instance.get_func(&mut store, "execute");

            if let Some(execute_func) = execute {
                let call_args = if arg_len > 0 {
                    vec![wasmtime::Val::I32(arg_ptr), wasmtime::Val::I32(arg_len)]
                } else {
                    vec![]
                };

                let num_results = execute_func.ty(&store).results().len();
                let mut results = vec![wasmtime::Val::I32(0); num_results];

                // If it expects no params (like our test simple wasm), pass empty args
                let actual_args = if execute_func.ty(&store).params().len() == 0 {
                    vec![]
                } else {
                    call_args
                };

                let call_result = execute_func.call(&mut store, &actual_args, &mut results);

                match call_result {
                    Ok(_) => {
                        // Attempt to read result from memory if ptr and len are returned (e.g., [i32, i32])
                        if results.len() == 2 {
                            if let (wasmtime::Val::I32(ret_ptr), wasmtime::Val::I32(ret_len)) = (&results[0], &results[1]) {
                                let len = *ret_len;
                                // Strictly validate the returned length to prevent OOM DoS attacks
                                // We cap the response size at a reasonable limit, e.g., 10MB or max_memory_bytes.
                                let max_response_size = config.max_memory_bytes.min(10 * 1024 * 1024);
                                if len > 0 && (len as usize) <= max_response_size {
                                    if let Some(mem) = memory {
                                        let mut buffer = vec![0u8; len as usize];
                                        if mem.read(&store, *ret_ptr as usize, &mut buffer).is_ok() {
                                            return Ok(buffer);
                                        }
                                    }
                                }
                            }
                        }

                        // If it doesn't return a pointer/length pair, return a basic success payload.
                        Ok(b"{\"status\":\"success\"}".to_vec())
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("out of fuel") || err_str.contains("all fuel consumed") || err_str.contains("wasm trap") || err_str.contains("error while executing at wasm backtrace") {
                            Err(ToolError::Timeout)
                        } else {
                            Err(ToolError::ExecutionFailed(format!("Wasm execution failed: {}", err_str)))
                        }
                    }
                }
            } else {
                Err(ToolError::ExecutionFailed("Export 'execute' not found".into()))
            }
        })
        .await
        .unwrap_or_else(|e| Err(ToolError::ExecutionFailed(format!("Task spawn failed: {}", e))))
    }
}