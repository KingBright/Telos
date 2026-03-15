#!/bin/bash

# Create output directory
mkdir -p test_traces

# Array of test cases including Phase 4 & 5
queries=(
    # --- Regression Baseline ---
    "今天天气怎么样？"
    "计算 25 的平方根加上 150 的 15%"
    "帮我计划一个为期3天的北京旅游行程"
    "2026年3月14日火星发生的爆炸新闻是什么？" # Blind summarization resilience test
    # --- Phase 4 & 5 ---
    "你是谁？你的名字是什么？" # Direct Reply Test
    "执行一个长文本研究任务：总结AI在2026年的前沿进展" # Triggering a background job
    "系统现在正在跑什么编程或是搜索任务吗？" # Router Omniscience Test (Should catch the above job if concurrent, but sequenced here. Actually, we might need concurrent execution for strict testing).
    "回忆一下在刚才的测试中，我问过的第一个问题是什么？" # Router Memory React Loop Test
)

for i in "${!queries[@]}"; do
    query="${queries[$i]}"
    case_num=$((i+1))
    echo "========================================"
    echo "Running Case $case_num: $query"
    echo "========================================"
    
    # Run the CLI using the globally installed binary
    cargo run --release --bin telos_cli -- run "$query" > "test_traces/iter7_case_${case_num}_cli.log" 2>&1 &
    CLI_PID=$!
    
    # If it's the long running task (Case 6), let's spawn it in background and immediately run Case 7
    if [ $case_num -eq 6 ]; then
        echo "Spawned research task in background to test Router Omniscience..."
        sleep 2 # Let it start initializing
        continue
    fi
    
    wait $CLI_PID
    
    # Fetch traces from daemon for debugging
    curl -s http://localhost:3000/api/v1/traces > "test_traces/iter7_case_${case_num}_traces.json"
    
    echo "Finished Case $case_num"
    echo ""
    sleep 2
done

# Wait for background job (Case 6)
echo "Waiting for background long-running task to complete..."
wait

echo "All ${#queries[@]} Iteration 7 evaluation cases executed."
