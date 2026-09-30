#!/usr/bin/env python3
"""
scripts/sync_issue.py — Dual Synchronization for Local AI/BACKLOG.md & GitHub Issues

Synchronizes task completion across:
  1. Local AI/BACKLOG.md (checks off - [x] **#X.Y**)
  2. GitHub Issues API (checks off - [x] **#X.Y** in the issue body & adds progress comment)

Completion is always explicit: only --subissue and --complete-all tick sub-issues in
AI/BACKLOG.md, and they are run before the commit so the change is part of the pushed diff.
--auto never writes AI/BACKLOG.md: it mirrors the sub-issues already checked in the
branch's backlog section to the mapped GitHub issue and reports the ones still open.

Usage:
  python3 scripts/sync_issue.py --check                       # offline mapping self-check
  python3 scripts/sync_issue.py --subissue 1.1 [--comment "Optional message"] [--local-only]
  python3 scripts/sync_issue.py --issue 1 --complete-all [--local-only]
  python3 scripts/sync_issue.py --auto [--branch <branch_name>] [--local-only]
  python3 scripts/sync_issue.py --print-github-issue [--branch <branch_name>]

Branches that fix a GitHub-only issue (review findings without a backlog id) are
intentionally NOT registered in BRANCH_TO_ISSUE: --auto is a visible no-op for them and the
commit message references the GitHub issue with an explicit "Closes #N".

Exit status: 0 success, 1 self-check or usage failure, 2 GitHub sync failed (the local
backlog is not modified by the failure and nothing on GitHub is overwritten).
"""

import argparse
import os
import re
import shutil
import subprocess
import sys

REPO = "Mysticaly622/soos"
BACKLOG_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "AI", "BACKLOG.md"
)
GH_TIMEOUT_SECS = 30

EXIT_OK = 0
EXIT_CHECK_FAILED = 1
EXIT_GITHUB_FAILED = 2

# Backlog issue number -> GitHub issue number.
# Every value must be a distinct GitHub *issue* (never a pull request); --check rejects
# duplicates. Backlog issues without a GitHub issue are omitted: #50 (delivered by PR #140)
# and #51 (the soos-gui work, formerly the second "Issue #22" heading).
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
    36: 102,
    37: 103,
    38: 104,
    39: 105,
    40: 106,
    41: 107,
    42: 108,
    43: 109,
    44: 110,
    45: 111,
    46: 132,
    47: 134,
    48: 136,
    49: 138,
}

# Topic branch -> backlog issue id. Several branches may deliver the same backlog issue, but
# every id must exist as a "### Issue #N" heading in AI/BACKLOG.md (enforced by --check).
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
    "fix/biometric-store-security": 25,
    "feat/install-script": 26,
    "feat/distro-packages": 27,
    "fix/policy-concurrency": 28,
    "fix/pam-uid-resolution": 29,
    "fix/evidence-store-safety": 30,
    "test/physical-hardware-validation": 31,
    "test/distro-validation": 32,
    "fix/policy-hardening": 33,
    "fix/pam-ffi-timeout": 34,
    "fix/cli-security": 35,
    "feat/nextgen-models-manifest": 36,
    "feat/scrfd-face-detector": 37,
    "refactor/remove-ort-landmark-detector": 38,
    "feat/embedding-512d-w600k": 39,
    "feat/pad-minifasnet-v2": 40,
    "refactor/vision-pipeline-3-model": 41,
    "feat/vision-letterbox-and-bbox-crop": 42,
    "refactor/model-ids-nextgen": 43,
    "refactor/mock-backends-nextgen": 44,
    "docs/nextgen-model-documentation": 45,
    "feat/admin-debug-gui": 51,
    "feat/guided-enrollment-production-unlock": 51,
    "feat/biometric-reliability-and-camera-lifecycle": 46,
    "feat/anti-spoof-ir-gdm-integration": 47,
    "feat/gdm-lockscreen-feedback-and-stability": 48,
    "feat/camera-latency-lockscreen-gui-preview": 49,
    "fix/gui-camera-auto-resolution-and-packaging": 50,
}

# Branch prefixes allowed by AGENTS.md / Docs/DEVELOPMENT_WORKFLOW.md (GitHub #239).
ALLOWED_BRANCH_PREFIXES = ("feat/", "fix/", "test/", "chore/")

# Merged historical branches registered before the prefix rule was enforced. The set is
# frozen: --check rejects any new BRANCH_TO_ISSUE entry with another prefix.
LEGACY_BRANCHES = frozenset(
    {
        "refactor/remove-ort-landmark-detector",
        "refactor/vision-pipeline-3-model",
        "refactor/model-ids-nextgen",
        "refactor/mock-backends-nextgen",
        "docs/nextgen-model-documentation",
    }
)

HEADING_RE = re.compile(r"^### Issue #(\d+)\b", re.MULTILINE)
SECTION_END_RE = re.compile(r"^#{1,3} ", re.MULTILINE)
SUBISSUE_RE = re.compile(r"^\s*- (?:\[([ x])\] )?\*\*#(\d+)\.(\d+)\*\*", re.MULTILINE)


def run_gh_cmd(args):
    """Executes a gh command with PAGER=cat and returns stdout, or None on any failure."""
    env = os.environ.copy()
    env["PAGER"] = "cat"
    env["GH_PAGER"] = "cat"
    env["GH_NO_PAGER"] = "1"
    gh_bin = shutil.which("gh")
    if gh_bin is None:
        print("[WARN] GitHub CLI ('gh') is not installed.", file=sys.stderr)
        return None
    try:
        res = subprocess.run(
            [gh_bin] + args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            check=False,
            timeout=GH_TIMEOUT_SECS,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        print(f"[WARN] gh command could not run: {exc}", file=sys.stderr)
        return None
    if res.returncode != 0:
        print(f"[WARN] gh command failed: {res.stderr.strip()}", file=sys.stderr)
        return None
    return res.stdout.strip()


def read_backlog(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def backlog_sections(content):
    """Returns {backlog_id: [(start, end), ...]} for every "### Issue #N" heading."""
    sections = {}
    for match in HEADING_RE.finditer(content):
        nxt = SECTION_END_RE.search(content, match.end())
        end = nxt.start() if nxt is not None else len(content)
        sections.setdefault(int(match.group(1)), []).append((match.start(), end))
    return sections


def section_subissues(content, backlog_id):
    """Returns [(sub_id, checked)] for the sub-issues listed under backlog heading #N."""
    result = []
    for start, end in backlog_sections(content).get(backlog_id, []):
        for m in SUBISSUE_RE.finditer(content, start, end):
            if int(m.group(2)) == backlog_id:
                result.append((f"{m.group(2)}.{m.group(3)}", m.group(1) == "x"))
    return result


def check_mappings(content, backlog_to_github=None, branch_to_issue=None):
    """Offline self-check of the mapping tables against AI/BACKLOG.md; returns error strings."""
    backlog_to_github = BACKLOG_TO_GITHUB if backlog_to_github is None else backlog_to_github
    branch_to_issue = BRANCH_TO_ISSUE if branch_to_issue is None else branch_to_issue
    errors = []

    sections = backlog_sections(content)
    for backlog_id, spans in sorted(sections.items()):
        if len(spans) > 1:
            errors.append(
                f"duplicate backlog heading #{backlog_id} "
                f"({len(spans)} '### Issue #{backlog_id}' headings)"
            )

    seen_sub = {}
    for m in SUBISSUE_RE.finditer(content):
        sub_id = f"{m.group(2)}.{m.group(3)}"
        seen_sub[sub_id] = seen_sub.get(sub_id, 0) + 1
    for sub_id, count in sorted(seen_sub.items()):
        if count > 1:
            errors.append(f"duplicate sub-issue #{sub_id} ({count} occurrences)")

    targets = {}
    for backlog_id, github_id in sorted(backlog_to_github.items()):
        if not isinstance(github_id, int) or isinstance(github_id, bool) or github_id <= 0:
            errors.append(f"backlog issue #{backlog_id} has an invalid GitHub target {github_id!r}")
            continue
        targets.setdefault(github_id, []).append(backlog_id)
        if backlog_id not in sections:
            errors.append(f"BACKLOG_TO_GITHUB references unknown backlog issue #{backlog_id}")
    for github_id, ids in sorted(targets.items()):
        if len(ids) > 1:
            joined = ", ".join(f"#{i}" for i in ids)
            errors.append(f"duplicate GitHub target #{github_id} for backlog issues {joined}")

    for branch, backlog_id in sorted(branch_to_issue.items()):
        if backlog_id not in sections:
            errors.append(f"branch '{branch}' maps to unknown backlog issue #{backlog_id}")
        if not branch.startswith(ALLOWED_BRANCH_PREFIXES) and branch not in LEGACY_BRANCHES:
            allowed = ", ".join(ALLOWED_BRANCH_PREFIXES)
            errors.append(
                f"branch '{branch}' uses a non-approved prefix (allowed: {allowed}; "
                "only the frozen LEGACY_BRANCHES are exempt)"
            )

    return errors


def sync_local_backlog_subissues(backlog_path, backlog_id, subissue_list, dry_run=False):
    """Checks off - [x] **#X.Y** under backlog heading #X only. Returns False on error."""
    content = read_backlog(backlog_path)
    spans = backlog_sections(content).get(backlog_id, [])
    if len(spans) != 1:
        print(
            f"[ERROR] Backlog heading #{backlog_id} found {len(spans)} times; run --check.",
            file=sys.stderr,
        )
        return False
    start, end = spans[0]
    section = content[start:end]

    modified = False
    for subissue_str in subissue_list:
        if f"- [x] **#{subissue_str}**" in section:
            print(f"[INFO] Local backlog: #{subissue_str} already checked off.")
            continue
        pattern = rf"- (?:\[ \] )?\*\*#{re.escape(subissue_str)}\*\*"
        if re.search(pattern, section):
            section = re.sub(pattern, f"- [x] **#{subissue_str}**", section, count=1)
            modified = True
            print(f"[OK] Local backlog: checked off #{subissue_str}")
        else:
            print(
                f"[ERROR] Sub-issue #{subissue_str} not found under backlog heading #{backlog_id}.",
                file=sys.stderr,
            )
            return False

    if modified and not dry_run:
        with open(backlog_path, "w", encoding="utf-8") as f:
            f.write(content[:start] + section + content[end:])
    return True


def sync_github_issue_subissues(github_issue_num, subissue_list, comment_msg=None, dry_run=False):
    """Checks off the given sub-issues in the GitHub issue body. Returns False on any failure.

    Non-destructive: the body is only PATCHed after a successful read, only checkbox markers
    change, and the progress comment is only posted after a successful PATCH.
    """
    body = run_gh_cmd(["api", f"repos/{REPO}/issues/{github_issue_num}", "--jq", ".body"])
    if not body:
        print(f"[ERROR] Could not retrieve GitHub Issue #{github_issue_num}.", file=sys.stderr)
        return False

    new_body = body
    changed = False
    for subissue_str in subissue_list:
        pattern = rf"- \[ \] \*\*#{re.escape(subissue_str)}\*\*"
        if re.search(pattern, new_body):
            new_body = re.sub(pattern, f"- [x] **#{subissue_str}**", new_body)
            changed = True
            print(f"[OK] GitHub Issue #{github_issue_num}: checking off #{subissue_str}")
        elif f"- [x] **#{subissue_str}**" in new_body:
            print(f"[INFO] GitHub Issue #{github_issue_num}: #{subissue_str} already checked off.")
        else:
            print(f"[WARN] Sub-issue #{subissue_str} not found in GitHub Issue #{github_issue_num} body.")

    if not changed or dry_run:
        return True

    res = run_gh_cmd([
        "api",
        "-X",
        "PATCH",
        f"repos/{REPO}/issues/{github_issue_num}",
        "-f",
        f"body={new_body}",
    ])
    if res is None:
        print(f"[ERROR] Failed to update GitHub Issue #{github_issue_num}.", file=sys.stderr)
        return False
    print(f"[OK] GitHub Issue #{github_issue_num} updated with checked sub-issues.")

    if comment_msg:
        if run_gh_cmd(["issue", "comment", str(github_issue_num), "--body", comment_msg]) is None:
            print(f"[ERROR] Failed to comment on GitHub Issue #{github_issue_num}.", file=sys.stderr)
            return False
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


def report_check(content):
    """Prints the self-check result; returns True when the mapping tables are consistent."""
    errors = check_mappings(content)
    for err in errors:
        print(f"[FAIL] sync_issue self-check: {err}", file=sys.stderr)
    if not errors:
        print(
            f"[OK] sync_issue self-check: {len(BACKLOG_TO_GITHUB)} GitHub targets, "
            f"{len(BRANCH_TO_ISSUE)} branches, backlog headings and sub-issues unique."
        )
    return not errors


def github_failed(github_issue):
    print(
        f"[ERROR] GitHub sync failed for GitHub Issue #{github_issue}. AI/BACKLOG.md was not "
        "modified by this failure; re-run the same command once 'gh' is authenticated and online.",
        file=sys.stderr,
    )
    return EXIT_GITHUB_FAILED


def run_auto(args, content):
    """Mirrors the branch's already-checked sub-issues to GitHub; never writes the backlog."""
    branch = args.branch or get_current_branch()
    if not branch or branch not in BRANCH_TO_ISSUE:
        print(
            f"[INFO] Branch '{branch}' is not registered in BRANCH_TO_ISSUE (GitHub-only review "
            "finding or tooling branch): nothing to sync. Reference the GitHub issue with "
            "'Closes #N' in the commit message."
        )
        return EXIT_OK
    backlog_issue = BRANCH_TO_ISSUE[branch]
    github_issue = BACKLOG_TO_GITHUB.get(backlog_issue)
    print(f"[INFO] Branch '{branch}' -> Backlog Issue #{backlog_issue} / GitHub Issue #{github_issue}")
    subs = section_subissues(content, backlog_issue)
    done = [sub for sub, checked in subs if checked]
    pending = [sub for sub, checked in subs if not checked]
    if pending:
        listed = ", ".join("#" + s for s in pending)
        print(
            f"[WARN] Still open in AI/BACKLOG.md: {listed}. Tick delivered items explicitly with "
            "--subissue before committing; --auto never checks them off."
        )
    if github_issue is None:
        print(f"[INFO] Backlog Issue #{backlog_issue} has no GitHub issue; nothing to mirror.")
        return EXIT_OK
    if args.local_only or not done:
        return EXIT_OK
    comment = args.comment or (
        f"Sub-issues {', '.join('#' + s for s in done)} are checked in AI/BACKLOG.md "
        f"on branch '{branch}'."
    )
    if not sync_github_issue_subissues(github_issue, done, comment_msg=comment, dry_run=args.dry_run):
        return github_failed(github_issue)
    return EXIT_OK


def main():
    parser = argparse.ArgumentParser(description="Synchronize issues/sub-issues on GitHub & BACKLOG.md")
    parser.add_argument("--check", action="store_true", help="Offline self-check of the mapping tables (no network)")
    parser.add_argument("--subissue", type=str, help="Sub-issue ID e.g. 1.1")
    parser.add_argument("--issue", type=int, help="Backlog issue number e.g. 1 (with --complete-all)")
    parser.add_argument("--github-issue", type=int, help="Override the GitHub issue number e.g. 8")
    parser.add_argument("--complete-all", action="store_true", help="Complete all sub-issues of --issue")
    parser.add_argument(
        "--auto",
        action="store_true",
        help="Mirror the branch's checked sub-issues to GitHub (never writes AI/BACKLOG.md)",
    )
    parser.add_argument("--print-github-issue", action="store_true", help="Print the branch's GitHub issue, if any")
    parser.add_argument("--comment", type=str, help="Optional progress comment to post on GitHub issue")
    parser.add_argument("--branch", type=str, help="Branch name (default: current branch)")
    parser.add_argument("--backlog", type=str, default=BACKLOG_PATH, help="Path to AI/BACKLOG.md")
    parser.add_argument("--local-only", action="store_true", help="Never call GitHub")
    parser.add_argument("--dry-run", action="store_true", help="Simulate sync without modifying files or GitHub")

    args = parser.parse_args()

    if not os.path.isfile(args.backlog):
        print(f"[FAIL] {args.backlog} not found.", file=sys.stderr)
        return EXIT_CHECK_FAILED
    content = read_backlog(args.backlog)

    if args.check:
        return EXIT_OK if report_check(content) else EXIT_CHECK_FAILED

    if args.print_github_issue:
        branch = args.branch or get_current_branch() or ""
        github_issue = BACKLOG_TO_GITHUB.get(BRANCH_TO_ISSUE.get(branch))
        if github_issue:
            print(github_issue)
        return EXIT_OK

    # Every syncing mode refuses to run on inconsistent mapping tables.
    if not report_check(content):
        return EXIT_CHECK_FAILED

    if args.auto:
        return run_auto(args, content)

    if args.subissue:
        parts = args.subissue.split(".")
        if len(parts) != 2 or not all(p.isdigit() for p in parts):
            print(f"[FAIL] Invalid sub-issue id '{args.subissue}' (expected N.M).", file=sys.stderr)
            return EXIT_CHECK_FAILED
        backlog_issue = int(parts[0])
        sub_ids = [args.subissue]
        default_comment = f"Sub-issue **#{args.subissue}** completed and verified."
    elif args.complete_all and args.issue is not None:
        backlog_issue = args.issue
        sub_ids = [sub for sub, _ in section_subissues(content, backlog_issue)]
        if not sub_ids:
            print(f"[FAIL] No sub-issues found under backlog heading #{backlog_issue}.", file=sys.stderr)
            return EXIT_CHECK_FAILED
        default_comment = f"All sub-issues for backlog Issue #{backlog_issue} completed and verified."
    else:
        parser.print_help()
        return EXIT_CHECK_FAILED

    if not sync_local_backlog_subissues(args.backlog, backlog_issue, sub_ids, dry_run=args.dry_run):
        return EXIT_CHECK_FAILED
    github_issue = args.github_issue or BACKLOG_TO_GITHUB.get(backlog_issue)
    if args.local_only or github_issue is None:
        return EXIT_OK
    comment = args.comment or default_comment
    if not sync_github_issue_subissues(github_issue, sub_ids, comment_msg=comment, dry_run=args.dry_run):
        return github_failed(github_issue)
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
