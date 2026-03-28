import requests
import json
import time

API_URL = "http://127.0.0.1:8321/api/v1/run_sync"

payload = {
    "payload": "规划一个项目叫 borrow_demo。请在 src/main.rs 中写入一段产生 Rust compile error 的代码（例如 move 后再次使用导致 borrow error）：\n```rust\nfn main() {\n    let s = String::from(\"hello\");\n    let y = s;\n    println!(\"{}\", s);\n}\n```",
    "session_id": "borrow_test_1",
    "schema_payload": "{\"skip_approval\":\"true\"}"
}

print("Triggering DAG to create broken code...")
try:
    response = requests.post(API_URL, json=payload, stream=True, timeout=10)
    for line in response.iter_lines():
        if line:
            print(line.decode('utf-8'))
except Exception as e:
    print(f"Stream disconnected or error: {e}")
    print("Daemon will continue running the DAG in the background.")
