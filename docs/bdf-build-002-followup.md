# Claude Code follow-up — BUILD-002 supervisor and fidelity corrections

Prepared 2026-10-15. This prompt is limited to correcting the reviewed BUILD-002 prototype and measurement harness.

## Prompt

Work in `C:\Users\btec\Documents\GitHub\bdf` on `chore/build-002-fidelity-resources`. Read AGENTS.md, CLAUDE.md, docs/bdf-build-spec.md, docs/bdf-evidence/BUILD-002.md, all three proposed ADRs and docs/bdf-evidence/BUILD-002-codex-review.md. Preserve existing commits and all owner/reviewer changes. Do not reset, stash, rewrite history, push, merge, publish or start BUILD-003.

Implement BUILD-002 review findings R1–R5 in small focused corrections:

1. Checked dimensions/byte arithmetic and limits on worker replies. The `2147483648 × 2147483648`, zero-byte response must be a protocol error, never a parent panic or accepted render, with overflow checks on or off.
2. Cancellation across blocked upload/write/hash/receive as well as rendering. A peer that handshakes then stops reading must be terminated within the cancellation budget, classified Canceled, with accepted session state preserved. Do not depend on writing Cancel to a full pipe.
3. Fail-closed Windows job setup. Validate configuration; propagate creation/configuration/assignment failure and kill/reap any started child. Add failure-path coverage alongside parent-death coverage. Do not silently fall back to an uncontained worker.
4. Explicit required render/comparison coverage. A successful open followed by refused/missing/unreadable render output or failed oracle cannot receive match. Separate exploratory reporting from a strict regression gate, with explicit known exceptions if necessary.
5. Color-sensitive visual checks and negative controls. Red `(255,0,0)` versus green `(0,130,0)` must fail color fidelity despite equal grayscale luma. Include alpha/transparency and retain useful geometry/noise tolerances without tuning them merely to make current outputs pass.

Use the existing ignored review probes as reproduction references. Add small Rust regressions at the appropriate boundaries and actually run them failing before each correction. Validate public Limits/Config inputs and apply the required never-panic lint declarations; do not introduce unsafe code or silently weaken containment/limits.

Keep the renderer unchanged. G1 image sampling, G2 widget fallback and G3 page-tree repair remain separately scoped adaptations. The worker remains a prototype; no production integration, broad refactor or new dependency is authorized by this follow-up. The ADRs remain proposed. Document that future pixel admission must occur before raster allocation, and preserve the hard-memory-limit/sandbox limitations.

Run bounded scoped fmt/clippy/tests with two jobs, both normal and fault-injection worker configurations, relevant MSRV/wasm checks and layers/assets/deny/parity. Avoid stale prebuilt binary confusion: run tests through Cargo in a known feature configuration and record exact binaries/toolchains. Rerun the 100-case oracle with explicit coverage/failure reporting and color metrics; retain original results and explain changed counts/thresholds. Rerun benchmarks only where a change affects their measurement or correctness.

Append exact commands, reviewed revision, expected-red/green evidence and artifact hashes to the BUILD-002 record; update task/roadmap status honestly. Commit only green scoped corrections. Return the diff and evidence for Codex review; do not label the full baseline verified or advance later tasks while required reviews/gates remain open.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-15 | 0.1 | Bounded correction prompt for five independently reproduced BUILD-002 findings. |
