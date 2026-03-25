import json, sys

def analyze():
    # Case 29 PlanParseError
    with open('test_traces/iter33_case_29.json') as f:
        d = json.load(f)
        for step in d.get('steps', []):
            if step.get('action') == 'expert_execution':
                print("--- Case 29 Tool Execution ---")
                print("Input to tool:\n", step.get('details', {}).get('input', '')[:500])
                print("Tool output:\n", step.get('details', {}).get('output', ''))

    # Case 32 Orange Cat
    with open('test_traces/iter33_case_32.json') as f:
        d = json.load(f)
        for step in d.get('steps', []):
            if step.get('action') == 'memory_retrieval' or 'memory' in step.get('action',''):
                print("--- Case 32 Memory Retrieval ---")
                print("Query:", step.get('details', {}).get('query', ''))
                print("Result:\n", step.get('details', {}).get('result', '')[:1000])

    # Case 35 Search Returning Summary
    with open('test_traces/iter33_case_35.json') as f:
        d = json.load(f)
        for step in d.get('steps', []):
            if step.get('action') == 'expert_execution' and 'Search' in step.get('details', {}).get('expert_name', ''):
                print("--- Case 35 Search Tool ---")
                print("Input:\n", step.get('details', {}).get('input', '')[:500])
                print("Output:\n", step.get('details', {}).get('output', '')[:1000])

analyze()
