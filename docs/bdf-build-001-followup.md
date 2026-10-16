# Claude Code follow-up — finish the BUILD-001 baseline

Prepared 2026-10-13. Paste the prompt below into Claude Code in the existing local repository. This is a bounded correction to BUILD-001, not authorization to begin BUILD-002.

## Prompt

Work in `C:\Users\btec\Documents\GitHub\bdf` on the existing `chore/build-001-baseline` branch. Read AGENTS.md, CLAUDE.md, docs/bdf-build-spec.md, docs/bdf-evidence/BUILD-001.md and docs/bdf-evidence/BUILD-001-codex-review.md. Preserve the existing commits and any new owner changes. Do not reset, stash, rewrite history, push, merge, publish or register MCP.

Finish BUILD-001 by correcting the asynchronous UI-test synchronization. Fourteen failures have pristine-baseline evidence; two intermittent failures need equivalent diagnosis. Codex independently reproduced the picker and drag/drop failures. The baseline remains non-green and BUILD-002 remains pending.

Inspect all affected tests across commands, document_links, editing, pickers, recovery and ui. Replace fixed-frame assumptions with a shared bounded helper that pumps frames, yields to the real worker and waits for each test's specific expected completion or refusal. Use a monotonic deadline with diagnostic failure output. Tests with documents already open must wait for the intended new document, prompt or error, not simply any view. Preserve every content/security/focus/retry/cancellation assertion and the assertion that file-open returns without blocking.

Do not globally enable run_inline, make production opening synchronous, increase fixed sleeps/frame counts, disable tests, suppress timeouts or weaken assertions. Use deterministic gating where feasible to test pending/nonblocking states, while keeping real async integration coverage. If synchronization reveals an actual product defect, produce a small synthetic regression and report it separately before expanding the correction.

Prove the tests fail before the correction and pass afterward. Cover all 16 reported failures, including the two intermittent cases. Run the six affected binaries repeatedly under normal scheduling and once with single-thread scheduling, then run the full scoped UI suite. Use -j 2 and finite process timeouts. Run fmt, scoped clippy with -D warnings, layers, assets and parity; record wasm/advisory/hosted/optional-font/MSRV outcomes honestly, never as passed when unavailable.

Update BUILD-001 evidence with exact reviewed tree/commit, commands, outcomes and hashes, append rather than erasing the original red results, and update the roadmap/task status. Commit only a green scoped correction. Stop for Codex review; do not begin BUILD-002, merge, publish or push automatically.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-13 | 0.1 | Bounded baseline test-synchronization correction after Codex review. |
