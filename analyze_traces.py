import json
import os
import glob
import re

report = ["# Telos Agent Test Cases Analysis Report\n"]

for i in range(1, 11):
    log_file = f"test_traces/case_{i}_cli.log"
    trace_file = f"test_traces/case_{i}_traces.json"
    
    if not os.path.exists(log_file):
        continue
        
    with open(log_file, "r") as f:
        log_content = f.read()
        
    with open(trace_file, "r") as f:
        try:
            traces_json = json.load(f).get("traces", [])
        except:
            traces_json = []

    # Check outcome from summary lines in CLI
    success = "Task completed successfully" in log_content or "✅ Task Success" in log_content
    total_time_match = re.search(r"executed in ([\d\.]+)s", log_content)
    time_taken = total_time_match.group(1) if total_time_match else "Unknown"
    
    # Check trace for tools and errors
    llm_calls = 0
    tool_calls = 0
    tools_used = set()
    errors = []
    
    for trace in traces_json:
        if trace.get("type") == "Trace":
            data = trace.get("trace", {})
            if "LlmCall" in data:
                llm_calls += 1
                res = data["LlmCall"].get("response", {})
                content = res.get("content", "")
                if "error" in content.lower() or "failed" in content.lower():
                    errors.append("LLM returned an error or failure message")
            if "ToolCall" in data:
                tool_calls += 1
                tools_used.add(data["ToolCall"].get("name", "unknown"))
                result_val = data["ToolCall"].get("result", {})
                if str(result_val).lower().find("error") != -1 or str(result_val).lower().find("failed") != -1:
                    errors.append(f"Tool {data['ToolCall'].get('name')} failed")

    report.append(f"## Case {i}")
    # Extract query from line 2
    query = log_content.splitlines()[0] if log_content else "Unknown query"
    val = [line for line in log_content.splitlines() if "Task:" in line]
    
    report.append(f"**Query**: from log...")
    report.append(f"- **Status**: {'✅ Success' if success else '❌ Failed'}")
    report.append(f"- **Execution Time**: {time_taken}s")
    report.append(f"- **LLM Calls**: {llm_calls}")
    report.append(f"- **Tool Calls**: {tool_calls} ({', '.join(tools_used)})")
    if errors:
        report.append(f"- **Noted Errors**: {len(errors)} potential issues in traces")

    # Capture the final result from the log
    # Often starts with ">>" 
    final_output = []
    for line in log_content.splitlines():
        if ">>" in line and "Router Decision" not in line and "Router QA rejected" not in line:
            final_output.append(line.replace(">> ", ""))
    
    out_text = "\n".join(final_output[-5:]) # Last 5 lines of final output
    report.append(f"\n**Final Output Snippet**:\n```\n{out_text}\n```\n")

with open("analysis_report.md", "w") as f:
    f.write("\n".join(report))

print("Report generated at analysis_report.md")
