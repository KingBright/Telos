#!/bin/bash

# Create output directory
mkdir -p test_traces

# Specific test cases to re-run
queries=(
    "帮我计划一个为期3天的北京旅游行程"
    "写一段 python 代码实现快速排序，并保存到 /tmp/quicksort.py"
    "在 telos_dag 模块中用grep搜索所有使用 petgraph 的地方"
    "背诵一首李白的诗"
    "这首李白的诗是你刚刚背过的吗？"
    "请使用动态工具插件机制（Rhai语言）为我临时编写一个工具，发送 http_get 请求获取 https://api.ipify.org 的返回数据并告诉我"
    "回忆一下在刚才的测试中，我让你写过哪种语言的排序代码？"
)

case_numbers=(6 8 9 10 11 12 13)

for i in "${!queries[@]}"; do
    query="${queries[$i]}"
    case_num="${case_numbers[$i]}"
    echo "========================================"
    echo "Running Case $case_num: $query"
    echo "========================================"
    
    # Run the CLI using cargo to ensure latest build
    cargo run --bin telos_cli -- run "$query" > "test_traces/case_${case_num}_cli.log" 2>&1
    
    # Fetch traces from daemon
    curl -s http://localhost:3000/api/v1/traces > "test_traces/case_${case_num}_traces.json"
    
    echo "Finished Case $case_num"
    echo ""
    sleep 2
done

echo "All specified cases executed."
