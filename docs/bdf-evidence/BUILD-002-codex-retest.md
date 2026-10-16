# BUILD-002 — independent Codex retest

Status: **R1–R5 corrections pass review; overall BUILD-002 acceptance remains pending.**

Reviewed 2026-10-15 on `chore/build-002-fidelity-resources`, clean starting revision
`1142fc8ca63e8d874faca0862bc759404bbc3a48`. The implementation corrections are
`da9afa4a`, `6ecac565`, `c1096b44`, `878b1acc` and `24ccf9a1`; later commits record evidence.
This review closes the five findings in [the first review](BUILD-002-codex-review.md), within
the scope of [the correction assignment](../bdf-build-002-followup.md). No blocking defect was
found in those corrections. This is not production-worker qualification or acceptance of the ADRs.

## Independent counterexamples

The original reviewer probe was rebuilt against the current protocol, supervisor and image
comparison sources, separately with Rust overflow checks **on and off**. Its malicious peer and
16 MiB blocked-upload workload were unchanged.

| Finding | Independent result | Disposition |
|---|---|---|
| R1: reply dimensions overflow | `2147483648 × 2147483648`, declared zero bytes, returns a protocol error with either overflow setting. No accepted render or parent panic. | Closed |
| R2: blocked-upload cancellation | The peer handshakes then stops reading. With 100 ms grace, the call returns `Canceled` about 109 ms after cancellation, with either overflow setting. | Closed |
| R3: job setup fails open | Zero CPU affinity is refused as `InvalidConfig`. Fresh fault-injection tests independently pass job-creation and job-assignment failure, child cleanup, explicit uncontained opt-in and real parent-death containment. | Closed |
| R4: missing renders receive `match` | With a one-case corpus and empty exception list, the healthy control receives `match`/exit 0; the original render-refusal shim receives `incomplete`/exit 1; an independently broken oracle also receives `incomplete`/exit 1. Neither failure is caused merely by selecting part of the corpus. | Closed |
| R5: grayscale misses wrong colors | Original red `(255,0,0)` versus green `(0,130,0)` PNGs still have identical grayscale metrics, but color difference fraction is 1, color MAE 255 and maximum color block difference 255. The actual acceptance-path tests reject the substitution in all three classes, and alpha-compositing tests pass. | Closed |

The code now validates public limits/configuration, checks RGBA arithmetic before reserving reply
memory, supervises cancellation independently of a blocked pipe writer and propagates required
containment errors. The strict oracle records failed/missing comparisons explicitly; positive and
negative controls use the same color-aware acceptance rule.

## Fresh Cargo checks

Commands used Rust/Cargo 1.99.0, locked/offline dependencies, two build jobs and finite outer
timeouts. Worker configurations ran sequentially through Cargo, rebuilding examples as needed;
old test executables were not substituted for the current feature configuration. Exact argument
arrays, exit codes, durations and logs are in the ignored retest directory.

| Check | Independent result |
|---|---|
| `cargo test --locked --offline -p pdfcraft-worker -j 2` | 9 unit + 26 integration tests pass |
| Same with `--features fault-injection` | 9 unit + 16 fault + 25 other integration tests pass |
| `cargo test --locked --offline -p xtask -j 2` | 90 tests pass |
| `cargo fmt --check` | Pass |
| Scoped worker + xtask clippy, all targets, `-D warnings` | Pass |
| Worker fault-injection clippy, all targets, `-D warnings` | Pass |
| `cargo xtask layers`, `assets`, `parity` through locked/offline Cargo | Pass |
| `cargo +1.95.0 check --locked --offline -p pdfcraft-worker -p xtask -j 2 --all-targets` | Pass; compile check, not tests on MSRV |
| MSRV worker check with `--features fault-injection` | Pass |
| `cargo xtask wasm` | 31 crates pass |
| Offline `cargo deny` with the existing writable advisory DB copy | Advisories, bans, licenses and sources pass |

The deny config was compared structurally with current `deny.toml`: only its advisory `db-path`
differs, to avoid the sandbox's read-only home-cache lock. This uses the previously reviewed cached
advisory snapshot, without fetching newer advisories. Two initial invocations used unsupported
argument placement/options and exited before checking; `deny-final.json` records the successful
exact command. The wasm gate invokes nested Cargo checks; its outer run is locked/offline with
two jobs, while its inherited implementation does not pass those flags to each nested invocation.

The initial sandbox run could not use Windows `tasklist` (access denied). Its process-existence
helper therefore incorrectly reported a live worker as absent and failed the healthy parent-death
test before cleanup. The exact leftover test helper was stopped after verifying its PID and path.
Normal Windows process visibility allowed the unchanged default suite to pass. A first subsequent
fault-suite build hit the executable lock from that helper; after cleanup the complete fault suite
passed. Both unsuccessful attempts remain in the evidence. No test was skipped or weakened to
obtain the passing runs, and no helper/worker remained afterward.

## Full corpus rerun

Ran through current Cargo-built xtask:

```text
cargo run --locked --offline -p xtask -j 2 -- bdf-oracle --strict --control --out target/bdf-evidence/BUILD-002-codex-retest/oracle
```

Exit 0 in about 86 seconds, with **100 cases**, **306 passing semantic checks**, **103 completed
visual comparisons**, **zero coverage gaps**, and **89 passing / 14 failing visual comparisons**.
There are 13 cases with visual divergence; one has multiple failing pages. The strict gate passes
with the same **16 named exceptions**, not because every file is visually correct. All verdicts,
semantic-check details and per-page visual metrics exactly equal CC's corrected
`BUILD-002-followup/oracle-v3/results.json`; timing fields were excluded from that equality check.

**Reporting correction:** CC's documentation said 90 passing comparisons and 13 failing
comparisons. The raw evidence says 89 and 14. The first run had 91 passes; the two newly detected
annotation-icon color differences reduce that to 89. This review corrects the current report,
renderer ADR and progress/task summaries. It does not change any metric, threshold, exception or
product source.

The different-case negative control still accepts 3 of 69 pairs: the same same-density text pairs
CC disclosed. Visual metrics alone do not establish text correctness. The known-exception gate
checks verdict categories; a larger error within an already excused `visual divergence` is not by
itself a new verdict. Continue to inspect metrics and images for those cases during renderer work.

## Evidence and remaining gates

Evidence is local and unpublished under `target/bdf-evidence/BUILD-002-codex-retest/`. It contains
fresh logs, exact Cargo argument arrays, the rebuilt reviewer source/probes, original color inputs,
positive/negative oracle controls, full-corpus results and pinned artifact/source identities.
`reviewed-artifacts.json` records the worker/xtask source trees and executable hashes; its worker
binary hash is the final fault-injection configuration, not a claim about the earlier default run.
The retest `MANIFEST.sha256` covers 75 files and hashes to
`f35eab57daa7add9d2eabe77c562234fd552876a6c053395c32eba2e72110552`.
It also includes hash inventories of CC's 45 follow-up files and the 292 existing corpus render
files, without copying or publishing those assets. These inventories pin the current files; CC
supplied no separate follow-up manifest digest to compare against.

Reverified **all 425 original BUILD-002 artifact hashes** and **all 43 original review artifact
hashes**, with no mismatches. Their manifest digests remain, respectively:

```text
c871cc69e0b38f8c81e54f204da10811fa01314b1a0df63d1792cc44dde6f101
bf39f9349f1b6e22ea013028d5cfb2ec1e2367a2dd43e6354f917b9e5dd00485
```

BUILD-002 remains `pending_review` for overall acceptance: the ADRs are proposed, budget
ratification/reference-hardware qualification is outstanding and BUILD-001's required qualification
is still open. G1 image sampling, G2 missing widget appearances and G3 page-tree repair remain
separate defects. End-to-end admission before raster allocation, a hard process-tree memory limit
and worker sandboxing remain future work. No full workspace test sweep, hosted CI, optional-font
build, new desktop smoke or performance rerun was claimed by this correction review. BUILD-003
was not started; nothing was committed, pushed, merged, registered or published.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-15 | 0.1 | Independent retest closes R1–R5; corrects visual pass count from raw evidence and preserves overall acceptance limits. |
