# BUILD-002 Codex review

Reviewed 2026-10-15. Branch: `chore/build-002-fidelity-resources`; reviewed HEAD `50660d134c1393ab03203b24b678d5da9236706a`, implementation tip `3d41257362b76b78e9c21df92e25eadab928ff7f`, branch point `c2dcc7a3c5f11163b34aaa8363c97c273e7a4ae7`. Working tree was clean at the start. This review changes documentation/status only; production and test implementation are left for Claude Code.

## Verdict and proposed decisions

**Changes requested for BUILD-002 verification.** The measurement work is useful and its disclosed limitations are appropriate, but independent counterexamples found faults in the parent supervisor and false-positive fidelity checks. Correct those before treating the prototype guarantees or harness as acceptance evidence. Keep BUILD-003 pending under the existing dependency gate; BUILD-001 remains separately pending review.

| Proposed ADR | Recommendation | Conditions |
|---|---|---|
| BDF-0001: retain/adapt renderer | Retain provisionally; no replacement project is justified by this corpus | Correct the harness gaps below and rerun comparisons. Track downscaling, missing widget appearances and bounded page-tree repair separately; each needs synthetic before/after and independent-reader evidence. Thin-line/plan fidelity remains a qualification question, not a proven nonissue. Expand to real-world and missing codec/language coverage before release claims. |
| BDF-0002: supervised worker | Adopt the process-boundary direction; request correction of this prototype | Fix overflow, fail-open job setup and blocked-upload cancellation. Preserve read-only retry limits and parent-owned state. Hard commit limits, process restrictions and removal of ambient file/network access remain open BUILD-010 gates. Do not call polling a hard cap or process separation a sandbox. |
| BDF-0003: budgets plus pixel cap | Keep PRD ceilings as engineering targets; support an explicit whole-page pixel cap | Enforce a cap before raster allocation, with tiled/strip handling for legitimate larger high-resolution output. The suggested 16,000,000 pixels is an initial design value, not a measured optimal limit. Qualify combined process-tree working set/commit on the reference hardware; warm timings on this faster GPU machine do not ratify cold launch, frame time, input latency or recovery. |

The ADRs remain proposed for the owner; this review does not record owner approval or increase any budget. The application engine was not changed by BUILD-002; the prototype is not integrated. This verdict is about the new implementation/evidence, not a finding that a currently shipped UI already accepts these worker frames.

## Evidence verified and independent work

- Verified the supplied manifest SHA-256 `c871cc69e0b38f8c81e54f204da10811fa01314b1a0df63d1792cc44dde6f101`, all **425** listed files, and every listed file hash: no mismatch or missing file.
- Parsed the raw oracle JSON: 100 cases, 306 semantic passes, zero semantic failures, 103 visual comparisons and 91 visual passes. No render failure is recorded in that original run. These counts are accurate for the checks actually implemented, not proof of full PDF semantics or color fidelity.
- Inspected the new supervisor/protocol/worker, image comparator, oracle verdict logic and benchmark paths, plus corpus provenance/dependency changes and the three ADRs. This was targeted review, not a line-by-line audit of all approximately 11,000 added lines.
- Built small native review executables under ignored `target/bdf-evidence/BUILD-002-codex-review/`, linking the recorded worker library and using the current image-comparison source. Counterexamples and logs are described below. No new dependency or download was needed.
- Reran the recorded worker equality integration test: one test passed. An attempted cancellation test selected a default-feature binary and ran zero tests; another stale fault-enabled test binary was incompatible with the currently installed default-feature worker and reported a protocol error. Neither is counted as a positive cancellation verification or as a newly established product defect. The dedicated blocked-upload counterexample does not depend on fault-injection features and reproduced independently.
- A direct attempt to link the existing thin-LTO release library into a standalone executable failed at linking and is not runtime evidence. The release-arithmetic counterexample instead compiles the unchanged supervisor/protocol sources with overflow checks disabled, using the cached dependencies; it reproduces the wrapped calculation described below. This is not a full Cargo release rebuild.
- `git diff --check c2dcc7a3..HEAD` passed. No new full Cargo test/build sweep, benchmark campaign, UI launch or hosted CI run was performed.

## Findings

### R1 — P1: worker dimensions can panic the parent or bypass reply validation

At `apps/pdfcraft-worker/src/supervisor.rs:444`, `width * height * 4` is calculated with unchecked u64 multiplication before the result limit is validated. A worker can send width and height both `2147483648`, total `0`, the SHA-256 of empty bytes, and a matching RenderEnd.

**Reproduced:** the recorded debug supervisor panics with `attempt to multiply with overflow`. The same supervisor source compiled without overflow checks accepts a `2147483648 × 2147483648` render containing zero bytes. Thus a malformed worker response can break the parent boundary the prototype is meant to protect.

Use checked multiplication/conversion and explicit dimensions/pixel/result bounds before allocation. Reject an overflow as a protocol fault and terminate the peer. Add real fake-worker regression coverage in debug and release arithmetic configurations; assert parent/session survival and no accepted render.

### R2 — P1: cancellation does not interrupt a blocked document upload

`open_inner` (`supervisor.rs:413`) synchronously writes the whole document before reaching `await_frame`. The watcher observes deadline/memory but not cancellation; `CancelHandle` only sets a flag. If the peer stops reading and the pipe fills, the caller cannot reach the cancellation logic.

**Reproduced:** a fake worker completes Hello then stops reading. Uploading 16 MiB, requesting cancel after 100 ms, with a 100 ms cancellation grace and 4-second call deadline, returns `Timeout` about **3.91 seconds after cancellation**. The default deadline is 30 seconds. The existing render-hang test does not cover this upload boundary.

Make cancellation effective throughout hashing/upload/writes, including when pipe I/O is blocked. The independent supervisor/watchdog must be able to end the peer within the cancellation budget without first writing a Cancel frame into the blocked pipe. Preserve `Canceled` versus `Timeout` classification and keep the accepted parent document unchanged. Test blocked upload, render, receive and backpressure cases.

### R3 — P1: required Windows job containment silently fails open

`Worker::spawn` (`supervisor.rs:177`) accepts the `Option` returned by `contain`. `contain` (`:578`) converts job creation/configuration/assignment errors into `None`, yet startup and the handshake proceed. Required kill-on-close containment is therefore conditional on setup succeeding, without an error or degraded-mode disclosure.

**Reproduced:** `Config { cores: Some(0), ... }` makes job setup invalid, but `Worker::spawn` still returns a working authenticated peer. A separate direct call through the same wrapper/limit setup reports `Failed to set info for job`, confirming the controlled setup failure. The broader silent-failure path also covers OS assignment/configuration errors.

Validate configuration, return a meaningful error from containment, and fail startup closed when the required job cannot be established. Kill/reap a child already started on every setup failure; do not leave background threads or children behind. Add injected job creation/assignment failure tests in addition to the healthy orphan test. Do not advertise universal parent-death containment from a success-path test alone.

### R4 — P1: a render refusal can receive a passing fidelity verdict

`visual_checks` (`xtask/src/bdf/oracle.rs:402`) compares only successful renders and silently skips missing/unreadable comparisons. An ordinary render exit 1 is recorded as an error but is not a crash; semantic checks can still pass and `verdict` then sees no failed visual checks.

**Reproduced through the real oracle command:** a CLI shim forwards successful opening, metadata and extraction to the real release CLI, but refuses `page_render` with exit 1. For `office-1p-std14`, the harness records `renders[0].ok = false`, the explicit refusal error and an empty visual array, yet reports **`match` and exit 0**.

Represent expected render coverage explicitly. A required render refusal, missing output, decode failure, oracle failure or skipped required comparison must become a failed or clearly incomplete result, never match. Add a strict gate mode or explicit known-failure policy if exploratory runs need to collect divergences without returning failure. Test those negative paths, including a broken oracle and missing PNGs, then rerun the corpus before relying on aggregate fidelity claims.

### R5 — P2: grayscale-only comparison cannot detect wrong colors

`xtask/src/bdf/imgcmp.rs:64` reduces every image to luma; all four acceptance metrics then operate on grayscale. The color information is discarded, so neither tighter thresholds nor the block-maximum metric can catch color substitutions of equal luma.

**Reproduced with actual PNG loading/comparison:** a solid red RGB `(255,0,0)` image and a solid green RGB `(0,130,0)` image both round to luma 76. The comparator returns diff fraction 0, MAE 0, ink ratio 1 and max block 0. All tolerance classes accept them. Text checks cannot detect this substitution.

Retain the grayscale/geometry metrics where useful but add a color-sensitive metric and targeted RGB/alpha/transparency negative controls. Set justified tolerances, disclose any revision, and rerun results. Until then describe these as grayscale visual comparisons, not complete color/render fidelity qualification for design and marketing workflows.

## Other required clarifications before integration

- The worker's configurable pixel limit is checked after `PageRenderer::render` (`worker.rs:260–277`). A lower proposed cap must move ahead of raster work; a post-render refusal does not prevent the allocation that motivated the cap.
- Validate public `Config`/`Limits` values too: for example `max_chunk <= 4` makes `max_chunk - 4` or zero-size chunking unsafe. New crates need the repository's required never-panic lint declarations, alongside actual checked code. Do not rely on clippy to detect every arithmetic issue.
- Keep per-operation output/admission limits distinct from the whole-session/process-tree budget. The measured memory overshoot and lack of a hard commit limit remain meaningful limitations even after the prototype corrections.
- Narrow the evidence report's claim that `git diff` over `apps/pdfcraft*` is empty: that pattern includes the newly added `apps/pdfcraft-worker`. The original desktop/CLI code is unchanged; the new worker is not.

## Reproduction artifacts and next assignment

All reproduction code, logs and tiny synthetic PNGs remain unpublished under `target/bdf-evidence/BUILD-002-codex-review/`. Key files: `probe.rs`, `overflow.log`, `probe_release_source.rs`, `release-source-overflow.log`, `job.log`, `cancel-upload.log`, `color.log`, `render_refusal_cli.rs`, and `refusal-oracle/results.json`. Original BUILD-002 evidence/corpus outputs were not overwritten; the oracle counterexample uses a copied one-case corpus.

The independent review artifact manifest lists 43 files and hashes to `bf39f9349f1b6e22ea013028d5cfb2ec1e2367a2dd43e6354f917b9e5dd00485`. It includes unsuccessful verification attempts explicitly described above, not only passing/reproducing runs.

Use [the bounded Claude follow-up](../bdf-build-002-followup.md). Correct the supervisor/harness and add regressions that first fail on the reviewed implementation; retain the original red evidence and rerun the qualifying cases. Do not fix the renderer's G1–G3 adaptations or integrate a production worker in this follow-up. Return for Codex review before marking BUILD-002 verified or starting BUILD-003.

## Revision history

| Date | Revision | Change |
|---|---|---|
| 2026-10-15 | 0.1 | Verified original manifest/counts; reproduced five supervisor/harness defects; proposed conditional ADR direction and a bounded correction. |
