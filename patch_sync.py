import sys

with open("scripts/sync_issue.py", "r") as f:
    content = f.read()

new_backlog_to_github = """
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
}"""

content = content.replace("16: 23,\n}", "16: 23," + new_backlog_to_github)

new_branch_to_issue = """
    "test/pam-docker": 13,
    "feat/pam-bindings": 14,
    "feat/pad-liveness": 15,
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
}"""

# Find the end of BRANCH_TO_ISSUE
content = content.replace('    "test/pam-docker": 13,\n    "feat/pam-bindings": 14,\n    "feat/pad-liveness": 15,\n    "chore/production-hardening": 16,\n}', new_branch_to_issue)

with open("scripts/sync_issue.py", "w") as f:
    f.write(content)
