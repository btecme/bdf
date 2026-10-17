# BUILD-003 — independent Codex review

Status: **R1–R3 corrections pass independent retest for slices 1–3; BUILD-003 remains incomplete.**

Reviewed 2026-10-16: [PR #2](https://github.com/btecme/bdf/pull/2) at
`e2063c687aa357eaafc61ddd3c067e4184993af6` (workflow ownership/pinning), and stacked
[PR #3](https://github.com/btecme/bdf/pull/3) at
`222435679ba099e6dcd671925bc2a08368461b3b` (capability inventory). The local checkout matched
PR #3 and was clean before review. PR #3 targets PR #2's branch; it must be retargeted after
that dependency merges. Neither BUILD-003 PR was merged or pushed by this review.

The earlier PR #1 is already merged as `f698f3a8`; independently checked GitHub results show
all four configured workflows succeeded on its final head `7e85027d`, with fuzz skipped by
the existing event condition. The owner's BUILD-001/002 acceptance is preserved.

Latest: [the final correction retest](BUILD-003-codex-retest.md#final-correction-retest-and-merge-preparation--2026-10-17) closes R1–R3. PR #2 is merged; #3 is retargeted to `main` and undergoing merge preparation. Initial findings below are historical.

## Findings

### R1 — P1 — install the package whose hash was verified (PR #2)

Location: `packaging/windows/install-wix.ps1:38`.

The script verifies the downloaded `wix.5.0.2.nupkg`, then invokes
`dotnet tool install --global wix --version $Version --add-source $dir --ignore-failed-sources`.
`--add-source` adds a feed; it does not restrict installation to the verified package. The
normal NuGet configuration hierarchy remains active, so another configured source can supply
the same package/version. Microsoft explicitly documents that sources are queried in parallel
and the fastest feed wins. Consequently, the verified download and the executable subsequently
run by `wix --version` / packaging are not necessarily the same bytes. A green download hash
does not establish the claimed installation integrity.

Independent evidence: a PowerShell probe executed the actual helper with mocked download,
`dotnet` and `wix` functions. Its synthetic bytes pass the hash check, and the captured install
arguments contain `--add-source` without an isolated source configuration. No real WiX was
downloaded, installed or executed locally. This is an argument-flow reproduction supported by
[Microsoft's source-selection documentation](https://learn.microsoft.com/en-us/dotnet/core/tools/dotnet-tool-install#options),
not a claim that the hosted package was compromised.

Required correction: make the installation consume only the verified package, isolate other
feeds and relevant caches, and prove that an alternative source cannot supply the tool. Retain
the existing mismatch refusal and add positive/negative source-selection coverage. Merely
changing source order or disabling parallelism does not bind installation to verified bytes.

### R2 — P2 — parse YAML keys rather than recognizing one line spelling (PR #2)

Location: `xtask/src/pins.rs:29–34`.

`scan` recognizes only a trimmed line beginning with literal `uses:` (after optional `- `).
It silently ignores equivalent YAML such as:

```yaml
- "uses": actions/checkout@main
- uses : actions/checkout@main
- {uses: actions/checkout@main}
```

The independent Rust probe imports the actual module: the ordinary spelling reports one
violation; all three alternatives report zero references and zero violations. A complete
temporary repository fixture containing twelve pinned references plus one floating quoted-key
reference reports **12 scanned / 0 violations**. It also satisfies the test's minimum-reference
count, so that safeguard does not close the gap. These are YAML mapping forms, not comments;
see the [YAML specification](https://yaml.org/spec/1.2.2/#742-flow-mappings).

Required correction: inspect actual workflow/composite-action mappings with a YAML parser,
or reject unsupported representations explicitly. Cover quoted keys, whitespace, flow maps,
multiline representations, and `uses:` text inside a `run` block. Preserve source locations in
diagnostics and the repository-level test. A legitimate formatting change must not bypass the
pinning requirement or mistake shell text for an action reference.

### R3 — P2 — evaluate effective modes, including defaults (PR #3)

Location: `crates/automation/src/capability.rs:597–600`; the print classification is at line 173.

The guard checks enum rules only when the corresponding argument is present and non-null.
`doc_print` classifies `comments_forms = "document-and-markups"` as excluded, but its actual
implementation defaults to that exact value (`crates/automation/src/printing.rs:107`). Thus:

```text
check_tool_call("doc_print", {"doc":1,"path":"print.pdf"}) => Allow
check_tool_call("doc_print", {"doc":1,"path":"print.pdf",
  "comments_forms":"document-and-markups"}) => Deny
```

Both requests select the same execution mode. Explicit null has the same gap. This contradicts
the new profile's excluded-mode contract and will survive a straightforward caller-side wiring
of `check_tool_call`. The profile is currently unused, so this is not a new live runtime
restriction or a claim that the existing application already enforces the profile.

Required correction: classify/evaluate normalized effective defaults consistently. Decide
whether the actual default belongs in the core or must be replaced/refused; do not change the
product split accidentally. Test explicit, omitted and null forms of every defaulted mode,
using execution defaults as the same source of truth.

## Independent validation

Windows x64, Rust 1.99.0, locked/offline Cargo, two build jobs and finite timeouts:

| Check | Result |
|---|---|
| `cargo test -p pdfcraft-automation --lib capability::tests -j 2` | 10 pass; 23 other unit tests filtered |
| `cargo test -p xtask pins::tests -j 2` | 4 pass; 95 other tests filtered |
| Scoped automation/xtask Clippy, all targets, `-D warnings` | Pass |
| `cargo fmt --all -- --check` | Pass |
| `cargo xtask pins` | Pass: 28 references, zero reported unpinned |
| Independent Rust counterexamples against the actual modules | Reproduce R2/R3; also confirm the documented command-parameter gap |
| WiX helper with mocked external commands | Reproduces R1's non-exclusive source selection; no installation |

The existing tests pass but do not cover the three counterexamples. No implementation was
changed or reverted, and no full workspace sweep was run locally. New regression tests should
first fail on these reviewed heads, then pass with the corrections.

GitHub run/job APIs independently confirm the following **exact-head** results:

| Head / scope | Observed hosted result |
|---|---|
| PR #2 `e2063c68` | [CI](https://github.com/btecme/bdf/actions/runs/38937240634), [FreeBSD](https://github.com/btecme/bdf/actions/runs/38937240593), [Windows ARM64](https://github.com/btecme/bdf/actions/runs/38937240952), [unsigned x64 package](https://github.com/btecme/bdf/actions/runs/38937240786), [packaging lint](https://github.com/btecme/bdf/actions/runs/38937240849): all pass |
| PR #3 `22243567` | [CI](https://github.com/btecme/bdf/actions/runs/38940663683): Windows/Ubuntu tests, wasm and dependency policy pass; packaging/FreeBSD/ARM64 workflows are not runs on this head |

Fuzz is skipped by its existing PR-event condition on both heads. It is not counted as a pass.
The x64 package, native ARM64 install, packaging-lint and FreeBSD logs on PR #2 were fetched
and inspected. The Windows logs show PE architecture checks for both executables, installed
models, CLI version execution and successful uninstall. This is CLI/package qualification,
not a GUI/document-workflow test, and the portable zip is built rather than exercised separately.

Private evidence is under `target/bdf-evidence/BUILD-003-codex-review/`: exact commands and
exits, probes, source fingerprints, hosted metadata and four decoded logs. Its 25-file
`MANIFEST.sha256` has SHA-256
`1ccd2ef714034182b22d65a09910467b494f14c85d8597c654d3d5af54f6c6d7`.
This is a new review manifest; no builder manifest digest was supplied for comparison.

## Remaining scope and bounded follow-up

1. Correct R1/R2 on PR #2, with counterexamples proven red first; rerun the relevant hosted
   package and lint jobs on the corrected head. Keep read-only permissions, no signing secrets,
   no publishing, and no automatic MCP registration.
2. Rebase/retarget the dependent inventory PR as necessary, correct R3, and rerun its focused
   tests and CI. Keep product classification changes explicit.
3. Preserve the acknowledged gaps: the profile has no runtime callers; command dispatch checks
   IDs but not `params`; route completeness is hand-maintained. The probe confirms that
   `command_run(file.save, params.flatten_fill_sign=true)` is allowed while the equivalent direct
   tool request is denied. This is already disclosed in PR #3 and needs closure in the planned
   enforcement work, not a claim of policy parity now.
4. BUILD-003 deliverables 2, 4 and 5 remain open: hosted qualification/parity job contracts and
   protection documentation, BDF identity migration, and network/MCP default verification.
   The runtime-fetched FreeBSD bootstrap and the Docker action's dependencies remain recorded
   supply-chain limitations. Signed distribution, native GUI coverage and full package
   qualification are not established by these drafts.

The workflow-ownership direction and inventory structure fit the build specification. Green
CI and completion of these partial slices do not establish completion of BUILD-003. No source
implementation or workflow was modified; the review adds only documentation and private probes.

## Revision history

| Date | Change |
|---|---|
| 2026-10-16 | Reviewed both draft heads, reran scoped gates, inspected hosted logs and reproduced R1–R3. Changes requested; overall work item remains incomplete. |
