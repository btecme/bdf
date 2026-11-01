# PR #14 — independent Codex review

Status: **changes requested: one P1 and one P2 in this adapter slice.** A separate
P1 found while retesting the merged BUILD-008 dependency is recorded in the
[BUILD-008 retest](BUILD-008-codex-retest.md); it is not presented as introduced by #14.

Reviewed 2026-11-01: [PR #14](https://github.com/btecme/bdf/pull/14), head
`633cfd2c4567dc603307ab2f30ff35c73445fc38`, base `main` at
`c1c38b07ec72cf321793082e8ed576a2c3858cee`. The checkout was clean at the start.
The scope is the new grant-authenticated MCP adapter, its CLI bootstrap, and the
shared framing refactor. This does not accept the whole BUILD-009 migration or
release agent access through the product.

## R14-1 — P1 — concurrent MCP clients silently replace each other's audit history

Location: `apps/pdfcraft-cli/src/access.rs:251–252`; underlying sink behavior in
`crates/app-service/src/audit.rs` (`FileAudit::open`, `start_segment`, `record`).

Every `mcp --client` process creates its own `FileAudit` for the same per-user
`activity` directory. The sink caches its sequence, segment index and handle with
no coordination between writers. If two servers start before either records an
operation, both believe the log has no segment. Each first record creates segment
1 through `write_private_file`, which replaces that filename. The second client
therefore removes the first client's visible history without an audit error.

Independent reproduction through the **actual Cargo-selected CLI binary**:

1. Issue distinct grants for clients alpha and beta in a private fixture state folder.
2. Start both `mcp --client` child processes and initialize both before any tool call.
3. Alpha opens a generated PDF, rotates a page, and saves a new file. All three
   replies succeed and the saved file exists.
4. Beta opens another generated PDF, also successfully.
5. Close both servers and run `activity verify` / `activity show` on the shared state.

The log contains **one record, beta only**. Verification returns `intact=true`,
`last_seq=1`, `records=1`. Alpha's three records, including its committed save,
are gone. A smaller two-open case has the same result. A public-library MCP probe
also reproduces it. These are cooperating clients, not an attacker rewriting a log,
and this is not the documented one-record window after a failed audit write:
every operation and every log write returned success.

Coordinate audit creation, sequence allocation, append and rotation across writers,
or fail closed on a conflicting writer before it serves operations. An append-only
change by itself does not fix stale sequence/segment state. Preserve private access,
bounds and the failed-audit safeguard. Add a real two-process regression: either both
clients' complete records survive and verify, or a conflicting client is explicitly
refused before performing its operations. Never report an intact complete history
after silently replacing another active client's segment.

## R14-2 — P2 — adapter-level document-tool refusals are not audited

Location: `crates/automation/src/client_mcp.rs:153–158`.

Unknown argument keys and argument-builder errors return before `run`, so they
never reach `Service::execute` or its audit path. The submission's claim that every
client call is recorded is therefore false for recognized document tools whose
arguments are rejected by the adapter.

Independent reproduction: an authenticated client calls `page_rotate` with
`revision=-1`. It receives `invalid_argument` as expected, but the attached file
audit has **zero records**. The refusal-coverage assertion fails. Unknown argument
names have another early return with the same omission.

Record authenticated document-tool attempts rejected by adapter validation, using
the trusted actor, a stable known operation identifier and a bounded error category.
Do not log raw arguments, passwords, secrets or document content. Use the service's
audit/health contract so a recording failure affects subsequent client access too.
Public help and protocol housekeeping may remain explicitly separate; the case above
is a recognized document operation, not an initialize or ping request.

## Merged BUILD-008 dependency retest

The original R1–R4 counterexamples now pass on this head: leaf/directory links are
refused inside a granted root; document-specific grants retain the full session key;
the failed-audit gate covers direct reads and saves; and an overlapping successful
issue/revoke retains both the new grant and the revocation. The last case uses a
real revoking child process while the issuer is paused inside its transaction.

A further persistence-failure case remains: after a revoke cannot be written, the
store initially refuses the actor in memory, but a later unrelated transaction
reloads the older file and makes that actor valid again—even when that transaction
also fails. See **B8-R5** in the [separate retest](BUILD-008-codex-retest.md).
The dependency's source is unchanged by #14; this review does not assign that
regression to the adapter diff or mark BUILD-008 accepted.

## Validation and limits

Local existing suites: **386 tests pass, 0 fail, 0 ignored**:

- app-service: 37 unit, 15 agent, 2 identity-gate and 26 service integration;
- automation: 40 unit, 163 main integration, 8 batch, 8 client-MCP, 4 conventions,
  2 sign-oracle and 3 service-parity;
- platform: 62;
- CLI: 12 access, 3 compact-MCP and 1 framing tests.

Commands use `--locked --offline -j 2`, bounded subprocesses and Cargo JSON artifact
records. Library tests use `--nocapture`. Tests/probes run under the normal Windows
user token so native links and restrictive ACLs are actually exercised.

The new audit-sharing and malformed-tool audit assertions fail with exit **101**;
the separate failed-revocation assertion also fails with exit **101**. The two
real-binary audit assertions fail with Python exit **1** after inspecting the
successful replies and the incomplete log. These are intentional test assertions,
not application crashes. All generated PDFs, grant files and logs are private
synthetic fixtures under ignored `target/bdf-evidence/PR-014-codex-review/`.
All child MCP processes are closed by their harness; nothing is registered or left
listening. Grant secrets are supplied only through child environments and are not
printed or included in the evidence manifest.

Current-head hosted checks pass:

- [CI 41262532599](https://github.com/btecme/bdf/actions/runs/41262532599): Windows,
  Ubuntu, wasm and dependency policy pass; fuzz is skipped by event.
- [FreeBSD 41262532590](https://github.com/btecme/bdf/actions/runs/41262532590): build
  and tests pass.
- [ARM64 41262532579](https://github.com/btecme/bdf/actions/runs/41262532579): both
  cross-packaging and native install/run pass. Logs confirm the core package omits
  OCR models and the native install checks their absence.

The logs identify GitHub merge `94a225f` of this exact head into `c1c38b07`.
Hosted success does not cover the failing cases above. Native macOS and independent
Unix experiments were not run locally. Engine/UI suites were not separately rebuilt
locally; the current-head hosted workspace checks are identified above.

Known first-slice limits—eight service tools, no client rendering/text/forms/printing
yet, unmigrated legacy CLI/UI/MCP routes—are explicitly documented and are not added
as new findings here. They still prevent whole-product migration/release acceptance.

The 34-file text manifest SHA-256 is
`a72273dfa4f3376d1bd05462d88c6a3d5a1dddf0ab365223e4e0befa5d91ab7b` (`manifest.json` / `manifest.sha256`).
It includes commands, logs, Cargo-selected library/binary hashes, source identities,
hosted metadata/excerpts and probe results. Binaries and generated fixture files
are excluded. The original BUILD-008 red-evidence manifest and all its 20 text-file
hashes were verified unchanged.

## Focused Claude follow-up

Fix R14-1/R14-2 in the adapter/audit path and B8-R5 in the merged grant-store dependency.
Preserve the now-passing original BUILD-008 regressions; do not redesign those contracts
or reopen settled branding/core-packaging choices. Add the missing failure/concurrency
regressions, demonstrate that restoring each defect fails them, then rerun scoped
tests and current-head CI. Keep the migration groups and release limits explicit.
These are implementation corrections, not new product decisions for the owner.

No production fix, commit, push or merge was made by Codex.

## Revision history

| Date | Change |
|---|---|
| 2026-11-01 | Independent PR #14 review: audit history loss and unaudited adapter refusals; merged dependency retest recorded separately. |
