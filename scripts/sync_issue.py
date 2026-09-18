#!/usr/bin/env python3
"""
scripts/sync_issue.py — Dual Synchronization for Local AI/BACKLOG.md & GitHub Issues

Synchronizes task completion across:
  1. Local AI/BACKLOG.md (checks off - [x] **#X.Y**)
  2. GitHub Issues API (checks off - [x] **#X.Y** in the issue body & adds progress comment)

Usage:
  python3 scripts/sync_issue.py --subissue 1.1 [--comment "Optional message"]
  python3 scripts/sync_issue.py --issue 1 [--complete-all]
  python3 scripts/sync_issue.py --auto [--branch <branch_name>]
"""

import argparse
import json
import os
import re
import subprocess
import sys

REPO = "Mysticaly622/soos"
BACKLOG_PATH = "AI/BACKLOG.md"

# Backlog Issue Number to GitHub Issue Number mapping
BACKLOG_TO_GITHUB = {
    1: 8,
    2: 9,
    3: 10,
    4: 11,
    5: 12,
    6: 13,
    7: 14,
    8: 15,
    9: 16,
    10: 17,
    11: 18,
    12: 19,
    13: 20,
    14: 21,
    15: 22,
    16: 23,
    17: 56,
    18: 57,
    19: 58,
    20: 59,
    21: 60,
    22: 61,
    23: 62,
    24: 63,
    25: 64,
    26: 65,
    27: 66,
    28: 67,
    29: 68,
    30: 69,
    31: 70,
    32: 71,
    33: 72,
    34: 73,
    35: 74,
}

BRANCH_TO_ISSUE = {
    "feat/policy-crate": 1,
    "feat/daemon-skeleton": 2,
    "feat/pam-ipc-client": 3,
    "test/protocol-fuzz": 4,
    "test/protocol-fuzzing": 4,
    "feat/camera-v4l": 5,
    "feat/inference-ort": 6,
    "feat/vision-pipeline": 7,
    "feat/vision-crate": 7,
    "feat/biometric-store": 8,
    "feat/evidence-store": 9,
    "feat/enrollment-cli": 10,
    "feat/admin-cli": 11,
    "feat/daemon-pipeline": 12,
    "test/pam-docker-matrix": 13,
    "feat/pam-bindings-upgrade": 14,
    "feat/pam-bindings-migration": 14,
    "feat/vision-pad": 15,
    "chore/production-hardening": 16,
    "fix/daemon-fail-closed": 17,
    "feat/model-deployment": 18,
    "fix/enrollment-cli-model-ids": 19,
    "fix/socket-toctou": 20,
    "fix/async-cancel-safety": 21,
    "feat/camera-format-negotiation": 22,
    "fix/camera-thread-shutdown": 23,
    "fix/vision-zeroize-frames": 24,
}


def run_gh_cmd(args):
    """Executes a gh command with PAGER=cat and returns stdout string."""
    env = os.environ.copy()
    env["PAGER"] = "cat"
    env["GH_PAGER"] = "cat"
    env["GH_NO_PAGER"] = "1"
    gh_bin = "gh"
    if not os.path.exists("/usr/bin/gh") and os.path.exists("/home/hadrien/.local/bin/gh"):
        gh_bin = "/home/hadrien/.local/bin/gh"
    try:
        res = subprocess.run(
            [gh_bin] + args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            check=False,
        )
    except FileNotFoundError:
        print("[WARN] GitHub CLI ('gh') is not installed. Skipping remote GitHub issue sync.", file=sys.stderr)
        return None
    if res.returncode != 0:
        print(f"[WARN] gh command failed: {res.stderr.strip()}", file=sys.stderr)
        return None
    return res.stdout.strip()


def sync_local_backlog_subissues(subissue_list, dry_run=False):
    """Checks off - [x] **#X.Y** in AI/BACKLOG.md for each subissue in list."""
    if not os.path.isfile(BACKLOG_PATH):
        print(f"[ERROR] {BACKLOG_PATH} not found.", file=sys.stderr)
        return False

    with open(BACKLOG_PATH, "r", encoding="utf-8") as f:
        content = f.read()

    modified = False
    new_content = content
    for subissue_str in subissue_list:
        if f"- [x] **#{subissue_str}**" in new_content:
            print(f"[INFO] Local {BACKLOG_PATH}: #{subissue_str} already checked off.")
            continue

        pattern = rf"- (?:\[ \] )?\*\*#{re.escape(subissue_str)}\*\*"
        replacement = f"- [x] **#{subissue_str}**"
        if re.search(pattern, new_content):
            new_content = re.sub(pattern, replacement, new_content, count=1)
            modified = True
            print(f"[OK] Local {BACKLOG_PATH}: checked off #{subissue_str}")
        else:
            print(f"[WARN] Sub-issue #{subissue_str} not found in {BACKLOG_PATH}.")

    if modified and not dry_run:
        with open(BACKLOG_PATH, "w", encoding="utf-8") as f:
            f.write(new_content)
    return True


def sync_github_issue_subissues(github_issue_num, subissue_list, comment_msg=None, dry_run=False):
    """Fetches GitHub issue body, checks off specified sub-issues, and updates via API."""
    body = run_gh_cmd(["api", f"repos/{REPO}/issues/{github_issue_num}", "--jq", ".body"])
    if not body:
        print(f"[WARN] Could not retrieve GitHub Issue #{github_issue_num}.", file=sys.stderr)
        return False

    new_body = body
    modified = False
    for subissue_str in subissue_list:
        pattern = rf"- \[ \] \*\*#{re.escape(subissue_str)}\*\*"
        replacement = f"- [x] **#{subissue_str}**"
        if re.search(pattern, new_body):
            new_body = re.sub(pattern, replacement, new_body)
            modified = True
            print(f"[OK] GitHub Issue #{github_issue_num}: checked off #{subissue_str}")
        elif f"- [x] **#{subissue_str}**" in new_body:
            print(f"[INFO] GitHub Issue #{github_issue_num}: #{subissue_str} already checked off.")
        else:
            print(f"[WARN] Sub-issue #{subissue_str} not found in GitHub Issue #{github_issue_num} body.")

    if modified and not dry_run:
        res = run_gh_cmd([
            "api",
            "-X",
            "PATCH",
            f"repos/{REPO}/issues/{github_issue_num}",
            "-f",
            f"body={new_body}",
        ])
        if res is not None:
            print(f"[OK] GitHub Issue #{github_issue_num} updated with checked sub-issues.")
        else:
            print(f"[ERROR] Failed to update GitHub Issue #{github_issue_num}", file=sys.stderr)
            return False

    if comment_msg and not dry_run:
        run_gh_cmd([
            "issue",
            "comment",
            str(github_issue_num),
            "--body",
            comment_msg,
        ])
        print(f"[OK] Added progress comment to GitHub Issue #{github_issue_num}")

    return True


def get_current_branch():
    res = subprocess.run(
        ["git", "symbolic-ref", "--short", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if res.returncode == 0:
        return res.stdout.strip()
    return None


def main():
    parser = argparse.ArgumentParser(description="Synchronize issues/sub-issues on GitHub & BACKLOG.md")
    parser.add_argument("--subissue", type=str, help="Sub-issue ID e.g. 1.1")
    parser.add_argument("--issue", type=int, help="Backlog issue number e.g. 1")
    parser.add_argument("--github-issue", type=int, help="Direct GitHub issue number e.g. 8")
    parser.add_argument("--complete-all", action="store_true", help="Complete all sub-issues for the given issue")
    parser.add_argument("--auto", action="store_true", help="Detect issue from current git branch and sync")
    parser.add_argument("--comment", type=str, help="Optional progress comment to post on GitHub issue")
    parser.add_argument("--branch", type=str, help="Specify branch name if auto-detecting")
    parser.add_argument("--dry-run", action="store_true", help="Simulate sync without modifying files or GitHub")

    args = parser.parse_args()

    backlog_issue = args.issue
    github_issue = args.github_issue

    if args.auto:
        branch = args.branch or get_current_branch()
        if branch and branch in BRANCH_TO_ISSUE:
            backlog_issue = BRANCH_TO_ISSUE[branch]
            github_issue = BACKLOG_TO_GITHUB.get(backlog_issue)
            print(f"[INFO] Auto-detected branch '{branch}' -> Backlog Issue #{backlog_issue} / GitHub Issue #{github_issue}")
        else:
            print(f"[WARN] Branch '{branch}' does not map to a known backlog issue.")

    if args.subissue:
        # Infer backlog issue from subissue e.g. "1.1" -> 1
        parts = args.subissue.split(".")
        if len(parts) >= 1 and parts[0].isdigit():
            backlog_issue = int(parts[0])
            if not github_issue:
                github_issue = BACKLOG_TO_GITHUB.get(backlog_issue)

        sync_local_backlog_subissues([args.subissue], dry_run=args.dry_run)
        if github_issue:
            comment = args.comment or f"Sub-issue **#{args.subissue}** completed and verified."
            sync_github_issue_subissues(github_issue, [args.subissue], comment_msg=comment, dry_run=args.dry_run)

    elif (args.complete_all or args.auto) and backlog_issue:
        if not github_issue:
            github_issue = BACKLOG_TO_GITHUB.get(backlog_issue)

        # Find all sub-issues under this issue in BACKLOG.md
        if os.path.isfile(BACKLOG_PATH):
            with open(BACKLOG_PATH, "r", encoding="utf-8") as f:
                content = f.read()
            subpattern = rf"- (?:\[[ x]\] )?\*\*(#{backlog_issue}\.[0-9]+)\*\*"
            matches = re.findall(subpattern, content)
            sub_ids = [m.replace("#", "") for m in matches]
        else:
            sub_ids = []

        if sub_ids:
            sync_local_backlog_subissues(sub_ids, dry_run=args.dry_run)
            if github_issue:
                comment = args.comment or f"All sub-issues for Issue #{backlog_issue} completed and verified. Ready for PR merge."
                sync_github_issue_subissues(github_issue, sub_ids, comment_msg=comment, dry_run=args.dry_run)
        else:
            print(f"[INFO] No sub-issues found in BACKLOG.md for issue #{backlog_issue}")


if __name__ == "__main__":
    main()

