import run_eval_headless
import json

if __name__ == "__main__":
    tc = next((tc for tc in run_eval_headless.test_cases if tc["id"] == 23), None)
    if tc is None:
        print("Case 23 not found")
        exit(1)
        
    print(f"Running Only Case 23: {tc['query']}")
    result = run_eval_headless.run_query(tc["query"], timeout=600)
    result["case_id"] = 23
    
    print("\n\n--- Result ---")
    print(json.dumps(result, indent=2, ensure_ascii=False))
