# BUILD-005 evidence: local credentials and bounded control/MCP framing

Status: **in progress (2026-10-19). Implemented and tested locally on Windows 11 x64; not yet run hosted; not reviewed by Codex. The two-account Windows test the work item asks for was not run (section 5), so the release gate for the UI control channel stays open.** Work item BUILD-005 in [bdf-work-items.toml](../bdf-work-items.toml) stays `pending`; it depends on BUILD-004, which is merged (PR #5, `396576d3`) but not marked verified by the owner. Branch `feat/build-005-credentials-framing`, from `main` at `396576d3`. Findings F01, F04 and F05 of the [foundation review](../bdf-foundation-review.md). Local commands ran with Rust 1.99.0, `--offline --locked` (one relock for a new dependency edge), two jobs.

## 1. F01: the control token goes only into a file shown to be private

**Before.** `write_control_file` created the file `0600` on Unix and did nothing on Windows, so the folder's inherited permissions decided who could read the port and token. On this machine the default temporary folder inherits entries for other accounts (seen with `icacls` during the BUILD-004 work: `CodexSandboxUsers` and three further SIDs with modify access), so a control file written there would have been readable by them.

**Now** (`crates/platform/src/private_files.rs`, new; used by `apps/pdfcraft`'s `--control`): the token is written only into a new, exclusively created file that has been checked private **while it is still empty**, then flushed and put in place of the destination.

* **Unix:** owned by this user and no group or other bits (`0600` from creation).
* **Windows:** the file's access list is read and every entry that grants access must belong to this user, the local system, the built-in Administrators, or the creator-owner placeholders. The list is read by running `icacls.exe` from the system folder with the file's path as its only argument and `/save` (which writes SDDL, so it does not depend on the display language); no safe API for reading an access list is available to this workspace, and `unsafe` is forbidden. Inherited entries are what the check is for: a folder that hands its files to Users, Authenticated Users or Everyone refuses the write with a message naming the group.
* **Fail closed:** if the list cannot be read or parsed, or is empty text, null or malformed, nothing is written. A refusal leaves no file behind (not even an empty one) and the error contains the folder and the reason, never the token. The destination is replaced, not opened, so a link or hard link planted there is replaced and its target left alone.
* The current user's SID comes from the process token through `winsafe` (feature `advapi`, already a workspace dependency); `rustix` gains the `process` feature on Unix. No new crate.

Tests (Windows, real access lists): a folder that grants Users read access is refused before the secret exists on disk; a folder only the current user can use is accepted and the file verifies private; the same file in a shared folder fails the check, so it is not a rubber stamp; a planted hard link is replaced. The app-level test shows the startup message tells the user what to do and contains no token. Pure SDDL tests (run everywhere) cover the allowed principals, every group refused and named, deny and audit entries not widening access, and missing, null and malformed lists. The existing control-file tests now use a private folder (a temp folder inherits what its parent grants). Mutation: skipping the verification fails three tests.

**Not shown:** that a second Windows account really cannot read the file. That needs two accounts and was not available (stop condition of the work item): the check reads the access list that governs the question, and a restricted-token observer in the BUILD-004 review showed the same approach decides it, but this item has not had its own two-account run. **The release gate stays open.**

## 2. F04: the control transport is bounded

`crates/ui-egui/src/control.rs`, with the limits in one struct (`Limits`, defaults in brackets) so the tests can use small ones:

* **Connections** [8]: counted in the accept loop; the ninth gets error -32003 ("too many control connections (limit 8)") and is closed, without a thread. A finished connection gives its place back.
* **Authentication deadline** [5 s from connect], absolute: a client that sends one byte at a time is cut off at the deadline, with error -32001, not after the dribble.
* **Idle timeout** after authentication [300 s], **write timeout** [10 s].
* **Request line** after authentication [1 MiB]; before it the existing 4 KiB cap. An over-long line gets -32600, is skipped without being kept (up to 16 MiB, then the connection is closed) and the next request on the same connection is served; malformed JSON does not stop later valid requests either.
* **Queue to the app** [64]: a bounded channel; a full one answers -32000 "the app is busy: 64 requests are already waiting" at once instead of growing.

The line reader (`read_line_limited`, `skip_line`) moved to `pdfcraft_platform::framing` so the MCP server uses the same code. Tests use real sockets: idle and dribbling clients closed at the deadline with a reason; extra connections refused with an error, a flood of 100 more refused quickly, held connections still working, a freed place reusable; an authenticated client that goes silent is closed; a request exactly at the limit is served, one byte over, ten times over and two megabytes over are refused and skipped, each followed by a served request; the queue answers "busy" at once. Mutations, each reverted: no connection limit, no framing recovery, no auth or idle deadline (the last one exposed that my `is_closed` helper counted a read timeout as a close; fixed and re-proved).

## 3. F05: MCP framing is bounded

`McpServer::serve` read with `input.lines()`, which allocates a whole line before parsing. It now reads with the same bounded reader, limit 16 MiB by default (`DEFAULT_MAX_MESSAGE`, `with_max_message` for tests). A longer line gets one JSON-RPC error (-32600, id null) and is skipped without being buffered; the next message is served. A line that is not UTF-8 gets a parse error and the server carries on (it used to end the session). The bound applies before the message is parsed, so it covers every kind of message.

Tests: through the real `serve` loop, an ordinary `tools/call`, the compact `tool_search` and `tool_call` meta tools, `resources/read` and `tools/list` each at the limit (served), one byte over and a hundred times over (refused with the limit named), then a valid request (served under its own id); a 64 MiB line generated lazily is refused and skipped without existing in memory; the non-UTF-8 case; and an end-to-end test through the real `pdfcraft-cli mcp` process over stdin and stdout, plain and `--compact`, with the 16 MiB default and 17 to 32 MiB lines. Mutation: removing the bound fails the two framing tests.

## 4. Checks (Windows, working tree)

rustfmt and `clippy -D warnings` over platform, ui-egui, automation, the CLI and the desktop app are clean. Platform 43 tests, automation 40 unit + 156 + 8 + 4 integration, the `pdfcraft-cli` suites, the control module 9 unit tests and 47 integration tests, and the app's 4 control-file tests pass. Not run locally: the workspace sweep, wasm, MSRV, Unix and macOS code (the Unix mode check is written but only a Linux run exercises it). The token was searched for in every `log::`, `println!`, `format!` and error path of the app, the control module and the CLI: it is only ever written to the private file and sent over loopback in the `auth` request.

**First hosted run (`578bfe73`) found three defects of mine, all fixed.** (1) On Linux clippy failed: `PathBuf` was imported for Windows-only code (an unused import everywhere else). (2) On the Windows runner the check refused its own user's folder with "another account (LA)": the runner's user is the machine's built-in Administrator, which SDDL prints as the alias `LA`, so the user's own SID never matched. `LA` (an administrator account that can take any file anyway) is now a trusted principal, with a test using the alias. (3) An existing app test (`a_control_channel_that_cant_be_published_stops_the_app`) published into the shared temp folder, which the new check correctly refuses on a machine whose temp folder grants other accounts; I had only run the control-file tests locally, not that one. It now uses a private folder like the others. FreeBSD, wasm, ARM64 and dependency policy were green on that head.

## 5. Not done, and not claimed

* **No two-account Windows test** (above). Credential confinement is shown through the access list, not by a second account reading or failing to read the file.
* **`icacls.exe` is a subprocess**, run from the system folder once per control-file publication (its cost was not measured). If it is missing or fails, the control channel refuses to publish rather than writing the token unchecked.
* **MCP cancellation and long-running operations** (the rest of F05) are not done: tool calls still run to completion synchronously and `notifications/cancelled` is still ignored. The work item's deliverables ask for bounded reads; deadlines and cancellation belong with the shared service and jobs (BUILD-007, BUILD-010).
* **Output bounds** (large tool results) are not changed here.
* The MCP server still exposes the CLI's full tool table, not the desktop-core profile (BUILD-007).
* The new messages are not translated; they come from the control channel and MCP, which are developer-facing.

## Revision history

| Date | Change |
|---|---|
| 2026-10-19 | First version: verified-private control file, bounded control transport, bounded MCP framing, evidence and limits. |
