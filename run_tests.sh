#!/bin/bash

# Create output directory
mkdir -p test_traces

# Array of test cases
queries=(
    "今天天气怎么样？"
    "计算 25 的平方根加上 150 的 15%"
    "列出当前目录下所有以 .rs 结尾的文件"
    "总结一下 crates/telos_core/src/lib.rs 的主要功能"
    "当前的系统时间是什么？"
    "帮我计划一个为期3天的北京旅游行程"
    "搜索一下最新的 Rust 发布版本号是多少"
    "写一段 python 代码实现快速排序，并保存到 /tmp/quicksort.py"
    "在 telos_dag 模块中用grep搜索所有使用 petgraph 的地方"
    "背诵一首李白的诗"
)

for i in "${!queries[@]}"; do
    query="${queries[$i]}"
    case_num=$((i+1))
    echo "========================================"
    echo "Running Case $case_num: $query"
    echo "========================================"
    
    # Run the CLI
    /Users/jinliang/rust-target/debug/telos_cli run "$query" > "test_traces/case_${case_num}_cli.log" 2>&1
    
    # Fetch traces from daemon
    curl -s http://localhost:3000/api/v1/traces > "test_traces/case_${case_num}_traces.json"
    
    echo "Finished Case $case_num"
    echo ""
    sleep 2
done

echo "All 10 cases executed."
