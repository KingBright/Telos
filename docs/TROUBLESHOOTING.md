# Telos 故障排查指南

本文档总结了常见问题的排查方法和调试技巧，帮助快速定位和解决问题。

## 快速诊断清单

### 1. 任务卡住/无响应

**症状**: `telos run` 命令在输出 `[DEBUG] simple_node -> Completed` 后卡住

**排查步骤**:

```bash
# 1. 检查 daemon 是否运行
pgrep -l telos_daemon

# 2. 检查端口是否监听
lsof -i :3000

# 3. 查看 daemon 日志（最新 50 行）
tail -50 ~/.telos/logs/daemon.log

# 4. 查看 daemon 错误日志
tail -50 ~/.telos/logs/daemon.err
```

**常见原因**:

| 原因 | 日志特征 | 解决方案 |
|------|---------|---------|
| UTF-8 截断 panic | `byte index X is not a char boundary` | 修复字符串截断函数使用 char 边界 |
| WebSocket 连接断开 | 无 TaskCompleted 日志 | 检查 handle_socket 错误处理 |
| 工具编译失败 | `Compilation failed` + 无后续 | 检查工具生成代码的 RiskLevel |

### 2. 工具创建失败

**症状**: 日志显示 `[ToolGenerator] ERROR: Compilation failed`

**排查步骤**:

```bash
# 查看生成的工具代码
cat ~/.telos/tools/gen_*/src/lib.rs

# 查看编译错误详情
tail -100 ~/.telos/logs/daemon.log | grep -A 20 "Compilation failed"
```

**常见原因**:

| 原因 | 错误信息 | 解决方案 |
|------|---------|---------|
| RiskLevel 无效 | `no variant named 'Low'` | 更新 prompt 模板，强调只用 Normal/HighRisk |
| 依赖缺失 | `cannot find telos_tooling` | 检查 TELOS_PATH 配置 |
| 语法错误 | 各种 Rust 编译错误 | 检查 LLM 生成的代码质量 |

### 3. Daemon 崩溃/重启

**症状**: Daemon 频繁重启或完全无响应

**排查步骤**:

```bash
# 查看 panic 日志
grep -i "panic" ~/.telos/logs/daemon.err

# 查看最近的错误
tail -100 ~/.telos/logs/daemon.err

# 检查系统资源
top -l 1 | head -10
```

### 4. WebSocket 连接问题

**症状**: CLI 提示 `Failed to connect to daemon WebSocket`

**排查步骤**:

```bash
# 检查 daemon 状态
ps aux | grep telos_daemon

# 手动测试 WebSocket 连接
curl -i -N -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Key: test" -H "Sec-WebSocket-Version: 13" http://127.0.0.1:3000/api/v1/stream

# 重启 daemon
pkill telos_daemon && ~/.cargo/bin/telos_daemon &
```

## 调试技巧

### 1. 启用详细日志

在 `~/.telos/config.toml` 中设置:
```toml
log_level = "debug"
```

或在 CLI 中使用环境变量:
```bash
RUST_LOG=debug telos run "your task"
```

### 2. 追踪特定任务

使用 trace_id 在日志中搜索:
```bash
grep "b2a2b718-bf5b-4c8b-8fbb" ~/.telos/logs/daemon.log
```

### 3. 手动测试工具编译

```bash
# 进入工具目录
cd ~/.telos/tools/gen_weather_fetcher

# 手动编译
cargo build --release --target wasm32-unknown-unknown
```

### 4. 检查 broadcast channel

在代码中添加调试输出:
```rust
// 检查订阅者数量
println!("Subscriber count: {}", feedback_tx.receiver_count());
```

## 常见修复方案

### 1. UTF-8 安全截断

```rust
// 错误的方式 - 可能 panic
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len])  // 危险！
    } else {
        s.to_string()
    }
}

// 正确的方式 - UTF-8 安全
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        let mut result = String::new();
        let mut byte_count = 0;
        for ch in s.chars() {
            if byte_count + ch.len_utf8() > max_len {
                break;
            }
            result.push(ch);
            byte_count += ch.len_utf8();
        }
        format!("{}...", result)
    } else {
        s.to_string()
    }
}
```

### 2. WebSocket 错误处理

```rust
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.broker.subscribe_feedback();

    loop {
        match rx.recv().await {
            Ok(feedback) => {
                let msg_str = serde_json::to_string(&feedback).unwrap_or_else(|_| "{}".to_string());
                if socket.send(Message::Text(msg_str)).await.is_err() {
                    eprintln!("[WebSocket] Send failed, connection closed");
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                println!("[WebSocket] Channel closed");
                break;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                eprintln!("[WebSocket] Lagged, lost {} messages", n);
                continue;  // 继续接收，不要退出
            }
        }
    }
}
```

### 3. 工具创建失败降级

```rust
// 在 ReactNode 中，工具创建失败后继续 ReAct 循环
match tool_result {
    ToolCreationResult::Failed(e) => {
        // 不要直接返回失败，让 LLM 尝试其他方法
        session_messages.push(Message {
            role: "system".to_string(),
            content: format!("Tool creation failed: {}. Please try a different approach.", e),
        });
        continue;  // 继续 ReAct 循环
    }
    // ...
}
```

## 预防措施

### 1. 代码审查检查点

- [ ] 所有字符串截断使用 UTF-8 安全方法
- [ ] WebSocket 错误处理包含 Lagged 场景
- [ ] 工具生成 prompt 明确限制 API 使用范围
- [ ] 关键路径有适当的日志输出

### 2. 测试覆盖

- 包含中文/多字节字符的测试用例
- 工具创建失败的端到端测试
- WebSocket 连接中断的恢复测试

### 3. 监控告警

建议监控:
- Daemon 进程存活
- 端口 3000 可用性
- 错误日志中的 panic 频率
- 任务完成率

## 相关文件

| 文件 | 用途 |
|------|------|
| `~/.telos/logs/daemon.log` | Daemon 正常日志 |
| `~/.telos/logs/daemon.err` | Daemon 错误日志 |
| `~/.telos/config.toml` | 配置文件 |
| `~/.telos/tools/` | 动态生成的工具目录 |
| `~/.cargo/bin/telos` | CLI 二进制 |
| `~/.cargo/bin/telos_daemon` | Daemon 二进制 |
