# BUILD-001 Codex review

Reviewed 2026-10-13. Branch: `chore/build-001-baseline`. Reviewed HEAD: `2a73127fef0e6e7a54de826b36358c9cefae32f4`; upstream baseline/main: `7437bde378f43856eada40af25b983812e615e8a`. The working tree was clean before this review. This review adds documentation/status only; no production or test implementation was changed.

## Verdict

**Changes requested before the known-green baseline gate. BUILD-002 remains pending.** The build/toolchain/branding evidence is credible, and the report correctly discloses failed/unrun checks. The evidence collection is complete enough to diagnose the next bounded task; BUILD-001's verification acceptance is not complete.

Do not accept the failing tests as a baseline exception. Fix the test synchronization while preserving the production asynchronous open path and every behavioral assertion. The repository's green-commit/merge gates remain unmet on this branch; do not merge or rewrite the existing commits to conceal that history.

## Independent checks performed

- Inspected the three branch commits, code/configuration diff and evidence report. The production changes are branding removal; the toolchain pin is 1.99.0 in both local configuration and three CI locations. No file-opening implementation was changed by the branch.
- Confirmed `main` still names `7437bde3` and the branch has no additional production feature changes.
- Verified all **109** files in the original evidence manifest against SHA-256. No missing files or mismatches. Manifest SHA-256: `4012349f83a9393ba45c4f0641d0d63a3e7b840773adc1bd313bddbc2352ef58`.
- Parsed the recorded full-suite and pristine-baseline logs: **16** failures on the pending tree, **14** on pristine baseline. Log hashes match the report.
- Independently reran two targeted tests using the recorded branch test binaries, one test per process, `--exact --nocapture --test-threads=1`, with a 30-second subprocess timeout. Both failed with exit 101 and the same absent-document assertions:
  - `file_open_returns_at_once_and_opens_the_pick_on_the_next_frame`: `pickers.rs:70`, views 0 instead of 1.
  - `dropping_a_pdf_on_the_window_opens_it`: `ui.rs:685`, views 0 instead of 1.
- Inspected `schedule_open`/`poll_pending_opens` and the fixed-frame assertions. Preparation runs on a worker and is polled with `try_recv`; two or three headless frames do not establish worker completion.
- Inspected the disposable baseline experiment: it changed waiting only in those two tests; the recorded experiment shows both passing after a bounded conditional wait. That supports the timing diagnosis for these two tests.
- `git diff --check main...HEAD` passed.

Independent rerun results and binary/log hashes are in the ignored `target/bdf-evidence/BUILD-001-codex-review/results.json`, with `pickers.log` and `ui.log`. These were existing test binaries; this review did **not** perform a fresh compilation, whole-suite rerun, new desktop launch or hosted run. Binary reuse and the builder's full-suite evidence are distinguished deliberately.

## Findings that require follow-up

### R1 — Baseline completion is blocked by asynchronous test synchronization

The failing tests still assert final state after fixed frame counts. Replace those assumptions with shared bounded waits for the specific terminal outcome: the intended document/tab and destination, the correct password/recovery prompt, or an expected refusal/error. Do not wait only for any nonempty view: several tests already have a document open.

Retain assertions that the initiating command returns before completion. Do not set `run_inline` globally, force production opening synchronous, enlarge fixed sleeps/frame counts, ignore tests, or swallow failed predicates. A helper must pump UI frames, yield to the worker, use a monotonic deadline, and fail with useful state when the expected condition does not arrive. Use deterministic worker gating where feasible for nonblocking/pending assertions; keep integration coverage of the actual async path.

Review all six affected binaries: commands, document_links, editing, pickers, recovery and ui. Their test flows include password retries/cancellation, startup URL focus, recovery attachment and invalid-file consent; each requires its own correct completion condition.

### R2 — The cause of all 16 failures is not yet established equally

Fourteen have pristine-baseline failure evidence. Two were not observed failing in the single pristine run: `the_open_recent_menu_lists_files_and_opens_one` and `consenting_to_absolute_gotor_still_validates_the_target`. Both have asynchronous fixed-frame assumptions in their source and appear compatible with the same diagnosis, but lack the same baseline evidence. Avoid asserting that every failure is definitively inherited until repeat/properly synchronized results support it.

The two-test wait experiment is not proof that all password/recovery/navigation assertions will pass. Preserve those checks and report any failure left after synchronization as a separate finding.

### R3 — Required verification still needs explicit outcomes

Fresh scoped fmt/clippy/tests after the correction are required. Rerun the six affected binaries under normal scheduling repeatedly and under single-thread scheduling, then the full scoped UI suite once. Rerun desktop/CLI tests if their behavior or dependencies change. Run layers/assets/parity; obtain wasm/dependency-check and Windows/Linux hosted outcomes before claiming those gates. Do not treat upstream runs as fork CI evidence, or skip required gates silently.

The pin has only local evidence; hosted CI, optional fonts and MSRV remain unverified. BUILD-001 does not need unrelated production work to address every later qualification item, but its evidence must distinguish passed, failed and unavailable checks.

## Bounded next assignment

Use [the BUILD-001 follow-up prompt](../bdf-build-001-followup.md). Implement test synchronization and its evidence only; preserve the existing asynchronous production behavior. Stop for Codex review after the resulting scoped suite is green or a concrete residual product defect is reproduced. Do not start BUILD-002 or advance status to verified on an incomplete rerun.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | Independent artifact verification and two targeted failure reproductions; baseline gate remains open. |
