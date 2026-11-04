# BDF progress check — 2026-11-04

Status: **local Windows developer preview built and smoke-tested; reported corrections pass; broader v1 acceptance pending**. This is a focused progress check and correction retest,
not whole-product v1 acceptance or a full audit of every inherited feature.

Checkout: `aee35194bc547c6c6ddc0aac0b0ae530f9823bb8` (clean at start, detached managed
worktree). This includes PR #14's correction head `1d4d9479` plus three read-only
migration commits. No implementation source, branch, commit, push, merge, installation,
default-handler registration or MCP registration was changed by this check.

## What has actually advanced

| Area | Current evidence | Meaning for delivery |
|---|---|---|
| Save/conflict/recovery foundation | BUILD-004 through BUILD-007 fixes are present; the three BUILD-007 correction findings were closed in the recorded final retest. | Build on the existing app; these are not reasons to restart the foundation. |
| Access and audit | BUILD-008 is merged. PR #14 adds a grant-authenticated MCP adapter and fixes the later shared-audit/refusal and failed-revocation findings. | Correction retest is recorded below; whole-product agent access is not released. |
| Read-only service work | 15 typed service operations; recent groups add list/links/bookmarks, text/search and page render/preview. | Useful groundwork, but desktop, ordinary CLI, run/batch, control and old MCP still use older routes. |
| Windows packaging | PR #13 has a successful unsigned x64 package at `53ea2533`, separate from the latest service branch. | A package exists; its age/head must not be confused with current-head desktop qualification. |
| Integrated v1 | Desktop service migration, remaining core operations, worker supervision and BUILD-011–015 qualification remain open. | A private hands-on preview and a qualified public v1 are different checkpoints. |

The upstream Acrobat-parity percentages/hours at the top of ROADMAP.md are inherited
estimates, not a measured completion percentage or schedule for BDF's office-focused v1.

## Checks performed

Fresh desktop and CLI builds pass. Scoped suites pass **289 tests / 4 ignored**:

- service.log: **120 pass / 0 ignored**.
- client.log: **19 pass / 0 ignored**.
- cli-access.log: **13 pass / 0 ignored**.
- ui.log: **137 pass / 4 ignored**.

Commands use `--locked --offline -j 2`, normal Windows user permissions and bounded
subprocesses. The existing Cargo cache was reused with no other Cargo process running;
the source/branches of both checkouts were left alone. The first build reached its
900-second compile timeout; its log/result are preserved. The cached retry completed
in about 124 seconds. No test was called passing on the basis of that timeout.
These are scoped checks, not a local full-workspace sweep or current-head CI.

A local developer preview is staged at
`target/bdf-preview-aee35194/` in this worktree (desktop + CLI + portable marker +
README; about 132 MB of executables, before settings). Binary SHA-256 values are in
`preview-artifacts.json`. It is a local test folder, not a signed release or installer.
The test's portable settings were retained separately and the preview was reset for
the owner, so normal settings/recovery are separate from this evaluation.

Native Windows desktop smoke **passes** through the actual app's opt-in control
channel: open/render the two-page fixture, click and type into the Contact form
field, rotate/undo/redo, save in place, close, reopen and quit. A separate CLI process
reads the saved file as 90° and finds the exact value `Desktop Form`. The screenshots
were visually inspected: page content and the filled value are visible, with no page
error reported. This is the older desktop adapter, not a service-migration claim.

The **16-step owner-side CLI workflow passes**: open/search, rotate/undo/redo, add a
note, fill Contact, reorder pages, save a new copy, close/reopen, inspect fields and
comments, find the new first-page text and render the rotated page. Readback has
rotations `[0, 90]`, value `Office Preview`, the note, and a 792×612 PNG; the original
fixture's hash is unchanged. I inspected the rendered output. Reopening here uses a
fresh session in the same engine, not an independent external reader.

Native creation of notes/highlights, desktop search and page reorder, failed-save
interaction, physical printing and print-preview layout were **not** exercised by
that native smoke. They remain checklist items; CLI/model tests are not substituted
for them.

The three original public-library counterexamples now **pass** against the current
Cargo-selected libraries:

- **R14-1 closed:** two client servers opened before recording preserve both operations
  and the audit chain. A second independent check starts two real CLI/MCP processes;
  four successful calls (including a saved copy) and one malformed refusal all survive,
  with five records in order, an intact chain, and both clients represented.
- **R14-2 closed:** an authenticated malformed `page_rotate` refusal adds one audit
  record. The real-process check also retains its malformed refusal.
- **B8-R5 closed for the reported in-process failure:** a real read-only grant file
  makes revoke fail; the actor is denied, and an unrelated issue that also fails no
  longer reactivates it. A failed durable revoke still reports failure and cannot
  promise revocation in another process; that documented limit is unchanged.

These are correction retests on `aee35194`, not acceptance of every added read-only
operation or the remaining BUILD-009 migration.

The original counterexamples are reused, with their output paths moved into this
worktree and their build artifacts selected from the current Cargo JSON output.
The two-process probe uses a staged copy of the newly built CLI so it does not hold
Cargo's build output open during later test linking. All fixtures are synthetic and
all grants/control credentials remain in private ignored test folders; no secret is
included in this record or the preview folder.

## Hosted status

[PR #14](https://github.com/btecme/bdf/pull/14) remains draft, targets main and is
mergeable at `1d4d9479`. Its [CI run](https://github.com/btecme/bdf/actions/runs/41323030895)
passes Windows, Ubuntu, wasm and dependency policy; FreeBSD and ARM64 runs also pass;
fuzz is skipped by event. The connector returns no PR-triggered run for checkout head
`aee35194`; this is not an exhaustive search of manual/push-triggered runs. The latest
read-only groups are explicitly recorded as not hosted in the author evidence.

The [PR #13 x64 package run](https://github.com/btecme/bdf/actions/runs/41196970579)
produced artifact `pdfcraft-windows-x64-UNSIGNED`, artifact id `13041274237`, at
`53ea2533`; it is a separate candidate, not proof of the latest branch. No installer
from that run was installed during this check.

## Keep the next checkpoint focused

The repository's previous CC focus prompt still assigned BUILD-004 corrections from
October 18. The [updated proposed handoff](../bdf-claude-focus-prompt.md) replaces that
stale assignment with a Windows preview checkpoint. It has not been sent to CC.
The [owner checklist](../bdf-windows-preview-checklist.md) describes the office tasks
in product terms.

Recommended immediate outcome: a portable Windows x64 candidate with an exact commit,
launch path, checksum and short desktop pass/fail/not-tested list, proposed by Friday,
2026-11-06. Finish only changes that block those workflows. Keep option A and the
shared authorization boundary as v1 release requirements; do not make completing
all agent catalogue migration, web work or the reusable support module prerequisites
for the owner's first private hands-on evaluation. Do not enable or release an
unqualified agent route to reach the date.

Limits: no broad real-document corpus, hardware printing, independent external-reader
qualification, clean-machine installation, signing, full performance/large-plan test,
macOS qualification or whole-product release is certified here. The current desktop
still has inherited branding and interface breadth. No new architectural/product
choice is requested from the owner in this check.

Ignored raw evidence is under `target/bdf-evidence/2026-11-04-progress/`.
The 34-file manifest SHA-256 is
`1cde165d0d41b0879fb918fff4725b217c0fb4d1dc05c655a930d059943197ae` (`manifest.json` / `manifest.sha256`).
It pins the commands, both build attempts, logs, source identities, artifact identities,
probe results, native/CLI results and synthetic screenshots. Executables, credential
files, grant state, portable user data and generated PDF fixtures are excluded.

## Owner-reported blocker: native Open picker

The owner subsequently reported that selecting any PDF greys out the Open button.
Treat this as a blocker for the ordinary File > Open workflow until reproduced and
corrected. The earlier native smoke used a launch path and `ui.open`; it did **not**
exercise the native Windows file-selection dialog and does not close this report.

Native inspection subsequently reproduced the disabled Open control with the PDF
filter correctly selected and online-only PDFs in the OneDrive shell view. After
unlock, selecting the local synthetic PDF by its full path through that same native
dialog opened and rendered it successfully. The issue is narrowed to cloud selection;
local native opening is now verified. No personal PDF content was opened or copied.
See [the reproduced picker issue and next correction](windows-open-picker-issue.md).
No production fix or corrected binary is claimed.

The requested repository sync is still pending: automatic approval review rejected
the validation command because its usage limit prevented review; the command was
not executed. This was not a determination that the command was unsafe. No merge,
push or commit has been performed in this sync attempt.

## Owner follow-up: BDF identity

After opening the preview, the owner confirmed that the fork must present itself as
BDF throughout the product. The next Claude assignment is
[the BDF branding build spec](../bdf-branding-build-spec.md): visible text, executable
names, storage and Windows packaging/support/update identity. The current preview
still has inherited branding; this commit records the requirement and does not claim
that implementation is finished. Required legal credits remain accurate.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Verify current branch progress, retest the bounded prior corrections, and focus the next delivery on a local Windows preview. |
