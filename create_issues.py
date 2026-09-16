import re
import os
import subprocess
import time

BACKLOG_PATH = "/home/hadrien/Project/soos/AI/BACKLOG.md"

def run_gh_cmd(args):
    gh_bin = "gh"
    if not os.path.exists("/usr/bin/gh") and os.path.exists("/home/hadrien/.local/bin/gh"):
        gh_bin = "/home/hadrien/.local/bin/gh"
    
    cmd = [gh_bin] + args
    res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if res.returncode != 0:
        print(f"Error running {cmd}: {res.stderr}")
        return None
    return res.stdout.strip()

with open(BACKLOG_PATH, "r") as f:
    lines = f.readlines()

issues = []
current_issue = None

for line in lines:
    m = re.match(r'^### Issue #(\d+) — (.*)', line)
    if m:
        num = int(m.group(1))
        if num >= 17:
            current_issue = {
                "num": num,
                "title": f"[{num}] {m.group(2).strip()}",
                "body": "",
                "branch": ""
            }
            issues.append(current_issue)
            continue
    
    if current_issue:
        # If we hit another heading (like '---' or '## Issue Dependency Graph')
        if line.startswith('## Issue Dependency Graph') or line.startswith('## Recommended Execution Order'):
            current_issue = None
            continue
            
        current_issue["body"] += line
        
        b_match = re.match(r'> \*\*Branch\*\*: `(.*?)`', line)
        if b_match:
            current_issue["branch"] = b_match.group(1).strip()

print(f"Found {len(issues)} issues to create.")

mapping = {}
branch_mapping = {}

for iss in issues:
    title = iss["title"]
    body = iss["body"].strip()
    branch = iss["branch"]
    
    print(f"Creating Issue #{iss['num']}: {title}")
    
    with open("temp_body.md", "w") as f:
        f.write(body)
        
    out = run_gh_cmd(["issue", "create", "--title", title, "--body-file", "temp_body.md"])
    if out:
        # out is usually the URL: https://github.com/Mysticaly622/soos/issues/24
        gh_id = int(out.split("/")[-1])
        print(f"  -> Created as GitHub Issue #{gh_id}")
        mapping[iss["num"]] = gh_id
        if branch:
            branch_mapping[branch] = iss["num"]
    else:
        print("  -> FAILED")
        
    time.sleep(1) # prevent rate limits

print("\n--- NEW MAPPINGS ---")
for k,v in mapping.items():
    print(f"    {k}: {v},")

print("\n--- BRANCH MAPPINGS ---")
for k,v in branch_mapping.items():
    print(f'    "{k}": {v},')

