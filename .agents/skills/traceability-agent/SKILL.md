---
name: traceability-agent
description: >
  Phase 6 documentation and traceability sub-agent for the soos workspace. Use
  after the candid review is APPROVED to update AI/VERIFICATION_MATRIX.md with
  test evidence, check off AI/BACKLOG.md and GitHub sub-issues via
  scripts/sync_issue.py, update Docs/, and author the next numbered walkthrough
  in AI/walkthroughs/. English only.
---

# Traceability Sub-Agent — soos

## Mission

Make the repository's written record match what the code and tests now prove — nothing more.
Every "Verified" claim must point to a test that exists and passes. Shared facts:
[`../dev-workflow/references/project-facts.md`](../dev-workflow/references/project-facts.md).

## Procedure

1. **Collect evidence.** For every test named in the tester contract, confirm it exists and passes:
   `cargo test --locked -p <package> --all-features <test_name> -- --exact` (or the module path).
   Use the output as evidence; never cite a test that does not exist.
2. **Verification matrix** (`AI/VERIFICATION_MATRIX.md`):
   - Update the rows named by the issue (e.g. `CLP1`, `NGM13`) to `✅ Verified`, citing
     `file::test_name` evidence. Every citation of a claimed row must resolve (invariant
     `test_matrix_claimed_rows_cite_only_existing_evidence`, walkthrough 104); put historical
     names inside an italic `*( ... )*` annotation.
   - For a new component, add a section `## Component: \`<name>\` (Issue #N / GitHub #M)` using the
     next free IDs of the relevant prefix. Do not rewrite older rows' markers (`☑ Validated`).
3. **Backlog & GitHub sync** (network; requires an authenticated `gh`):
   ```bash
   python3 scripts/sync_issue.py --check      # offline mapping self-check, must pass
   python3 scripts/sync_issue.py --subissue <X.Y> --comment "Sub-issue #<X.Y> completed and verified."
   ```
   Run `--subissue` once per delivered item **before** the commit so the `AI/BACKLOG.md` edit is
   committed (`--local-only` skips GitHub). `--auto` never ticks anything: `save.sh` mirrors the
   committed checkboxes to GitHub after the push. A failed GitHub call exits 2 with
   `GitHub sync failed` and leaves the backlog as it was; re-run it once `gh` works. Branches for
   GitHub-only review findings are not in `BRANCH_TO_ISSUE`; they use `Closes #N` in the commit.
4. **Technical docs** — update the affected `Docs/*.md` page (API signatures, constants with their
   location, behavior, matrix mapping). If a value changed (timeout, threshold, class index), grep
   `Docs/`, `AI/ARCHITECTURE.md` and `README.md` for the old value and fix or flag every occurrence.
5. **Walkthrough** — next number = `$(ls AI/walkthroughs | sort -n | tail -1)` + 1; never reuse a number
   (11 and 54 are duplicated historically). File: `AI/walkthroughs/NN_snake_case_topic.md`.
6. **Self-check** before hand-off:
   ```bash
   ./scripts/candid_review.sh            # English policy & invariants on the docs you touched
   grep -rn "/home/" AI/walkthroughs/NN_*.md Docs/ | grep -v "^Binary" || true   # must be empty
   ```

## Walkthrough Template

```markdown
# Walkthrough NN — <Title>

- **Date**: YYYY-MM-DD
- **Issue**: Backlog #N (GitHub #M) — **Branch**: `<type>/<name>`
- **Matrix criteria**: <IDs>

## 1. Context & Objectives        (problem statement from BACKLOG)
## 2. Architect Design            (types, constants, invariants touched)
## 3. Plan Evaluation             (verdict + key findings)
## 4. Tester Contract             (test names → criteria; Red evidence; migrated tests)
## 5. Auditor Constraints         (numbered list and how each was met)
## 6. Implementation              (files changed, notable decisions)
## 7. Candid Review               (fingerprint, verdict, findings and their resolution)
## 8. Verification Results        (exact commands + pass counts)
## 9. Known Limitations / Follow-ups
```

Every section must be filled with facts from this change; write "none" rather than omitting a
section. Use repository-relative paths only.
