# BUILD-008 — independent review of the first two slices

Status: **changes requested: four P1 access-control findings; packaging is also red.**

Reviewed 2026-10-31:

| PR | Head | Base | Disposition |
|---|---|---|---|
| [#11](https://github.com/btecme/bdf/pull/11) | `be477d71ddc70c164fd507792b640b02e2014a22` | `main`, `ccfc041b15bfee06ecf507f5ec1cf5ee4ab33a79` | R1–R3 require correction. |
| [#12](https://github.com/btecme/bdf/pull/12) | `586a00d665f3e04f80511a7c2f72537089d70aef` | #11 branch at `be477d71` | Inherits R1–R3; R4 requires correction. Apply the BDF state-directory requirement. |

Both PRs are open drafts. The checkout was clean at the start, on the stacked #12
branch. The local runs below exercise that combined tree. R1–R3 affect code unchanged
from #11; its diff and exact-head hosted results were reviewed separately. This is
not a claim of an isolated local build of #11. No adapter migration or release of
agent access is accepted here; those are explicitly outside these slices.
The submission's statement that no agent can reach a document through any shipped
surface is too broad: the existing opt-in CLI/MCP still constructs the legacy
automation session (`apps/pdfcraft-cli/src/main.rs:813`). The accurate limit is
that the new service grant policy is not yet applied to those adapters. This is
known BUILD-009 work, not an additional implementation finding against these slices.

## R1 — P1 — folder grants must constrain the actual read/write

Locations: `crates/app-service/src/grants.rs:85–88`,
`crates/app-service/src/service.rs:568` and `:866–874`.

`FileAccess` approves paths by `starts_with` and rejects `..`, but a service made with
`Service::new().with_policy(...)` reads and writes through ordinary filesystem paths.
The strict reader/directory pin/confined writer are selected only when the service's
separate optional global root is set. Granting a folder does not establish that root.
This is the same configuration used by the public agent integration tests.

Independent probes on the stacked head both reproduce an escape, with actual native
Windows links and no race: (1) an agent granted only `allowed/` opens
`allowed/alias.pdf` and receives all three pages of a PDF in sibling `outside/`;
(2) a directory link `allowed/linked` points to `outside/`, and saving to
`allowed/linked/escaped.pdf` succeeds and creates the outside file. Both safety
assertions exit **101**. Ordinary grant checks passed; neither test bypasses the
service API or changes its implementation.

The service must bind each granted folder to the I/O it authorizes, including the
opened leaf and the destination parent through commit. Reuse the shared platform
confinement primitives; a preliminary canonicalization alone leaves a replacement
race. The global root is an additional restriction, not a substitute for each
client's grant. Keep the existing ordinary-file behavior and explicitly stated
hard-link boundary. Cover both reads and writes through links, plus replacement
during the operation. This service contract cannot be deferred to BUILD-009.

## R2 — P1 — document-specific grants lose the session identity

Locations: `crates/app-service/src/grants.rs:42–58`;
`crates/app-service/src/policy.rs` (`Facts.document` and document-scope matching).

`DocumentKey` is session-qualified, but `DocumentScope::Only(Vec<u64>)` retains only
the document number, and the policy receives only that number. A persistent grant
cannot distinguish document 1 in one window/session from document 1 in another.
Checking that the supplied key belongs to the receiving service does not restore
the session identity lost from the grant itself.

Independent reproduction: two services share the grant store and open different
PDFs (one page and three pages). Their keys have different session IDs but both have
local document number 1. A `DocumentRead` grant limited to the first document
authorizes querying the second service with its own valid key and returns the
other PDF's three-page information. A strengthened variant adds page-edit permission
limited to the same first document: it also rotates the other session's first page
to 90° (`[90, 0, 0]`). Both variants fail the refusal assertion with exit **101**.

Bind document-specific scopes to full session-qualified document identities, or
fail closed when their originating session is no longer applicable. Do not turn a
limited grant into an implicit all-sessions grant. Preserve the intentionally broad
meaning of an explicitly requested all-documents scope. Add cross-session and
restart regressions with colliding local document numbers.

## R3 — P1 — public entry points bypass the failed-audit gate

Locations: `crates/app-service/src/service.rs:229–249`, `:279–285`, `:393–417`.

`run` checks `audit_failed` for clients. `prepare_save_inner` checks capability and
folder permissions but not that flag, and its write watch checks only the grant.
The actor-aware `document`/`documents` facade likewise bypasses the audit-failure
gate. These are real public entry points, including the intended worker save API.

Independent reproduction: attach an audit sink that reports a synthetic write
failure, then open a document as the human (which is intentionally allowed).
`audit_failed=true`, and an agent's ordinary query is correctly refused with
`audit_unavailable`. The same live agent obtains a direct document view, prepares
a save, writes `unaudited.pdf` and finishes successfully while the audit remains
failed. The no-write assertion exits **101**. This is a subsequent client effect,
not the acknowledged one-record failure window.

Apply the same admission/delivery rule to every public agent path and to uncommitted
prepared writes when auditing becomes unavailable. Preserve the human's ability to
work and report any effect already committed. The documented one-record failure
window does not justify continuing unrecorded client saves afterward. Add regressions
for direct saves, direct reads and a plan queued before the audit fails.

## R4 — P1 — concurrent grant writers can undo a successful revocation

Locations: `crates/app-service/src/grants.rs:341–367`, `:373–385`, `:493–509`.

The second slice refreshes a store before mutation, then modifies its private
snapshot and writes the whole file. Its mutex protects clones of one store only;
another `GrantStore::open`, including another CLI process, has a different mutex.
No transaction/serialization or version guard protects the read-modify-write.

Independent deterministic reproduction: pause issuer A after it refreshed the file
but before its write, using the public clock seam. A real child process opens a
separate store and successfully revokes an existing grant; a fresh reader confirms
the old secret is refused. Resume A to issue an unrelated grant. A writes its stale
snapshot over the revocation, and another fresh store successfully authenticates
the revoked secret again. The safety assertion exits **101**. Both writers are
cooperating BDF APIs; there is no direct JSON tampering or hostile external writer.

Make cooperating BDF grant writers serialize a complete load/validate/mutate/commit
transaction, or detect a conflicting version and retry safely. Cover issue, revoke,
prune and initial creation; preserve private-file checks and failed-write behavior.
A successful revoke must not be reversed by an unrelated issue or prune. Include a
deterministic overlapping-writer test with a real second process. Watching length
and modification time at request entry is not a write transaction.

## Product direction and the three questions

1. **OCR packaging:** the failing check is BDF's ARM64 packaging job, not an external
   review status. It fails when the inherited package script fetches
   `text-detection.rten` from the pinned OCR host and receives HTTP 403. OCR is
   deferred: omit its model download/payload from the BDF core packaging profile,
   consistently for x64/ARM64, manifests and install expectations. Keep the build,
   architecture, core smoke, install and uninstall checks. Do not substitute different
   models, weaken hashes/licensing checks or waive the failed job. Packaging must
   become green before merge.
2. **BDF state directory:** use BDF now. Build-spec section 5 already requires a
   BDF-specific per-user directory, and the owner confirmed the branding on 2026-10-31.
   The `PdfCraft` constant in `crates/platform/src/state.rs` is inconsistent with
   that requirement. Update its code/tests/docs and record any legacy-state migration
   explicitly. Do not silently import upstream grants/credentials or lose recovery
   data. Internal crate renaming is outside this correction.
3. **Merge order:** #11 first, then retarget #12 to `main`, reconcile changes and
   verify its actual resulting head. Both need their fixes and required checks first.
   This is routine engineering coordination, not a new product decision. This review
   does not merge either PR or create a watcher.

## Validation, scope and handoff

Local existing suites pass: service **35 unit + 9 agent + 2 identity-gate + 26
service integration tests**, platform **62**, CLI access **7**: **141 passed,
0 failed, 0 ignored**. Commands use `--locked --offline -j 2`, bounded subprocesses
and the normal Windows user token. Platform/service used `--nocapture`; no skip
notice was printed. The five independent safety assertions all fail with exit
**101**, for the reasons above. The document-scope probe was strengthened from a
read-only case to include a limited page-edit grant; its initial read-only source
and log are preserved as well. Engine/UI suites were not separately rerun locally
for this review; the exact-head hosted workspace jobs are listed below.

Current-head hosted evidence was read at job level, with Windows/Ubuntu test-log
excerpts confirming the exact merge heads:

- #11: [CI 40519910328](https://github.com/btecme/bdf/actions/runs/40519910328) and
  [FreeBSD 40519910314](https://github.com/btecme/bdf/actions/runs/40519910314) pass;
  [ARM64 40519910291](https://github.com/btecme/bdf/actions/runs/40519910291) fails
  on the OCR HTTP 403, with native install skipped. CI fuzz is skipped by event.
- #12: [CI 40531824806](https://github.com/btecme/bdf/actions/runs/40531824806) and
  [FreeBSD 40531824618](https://github.com/btecme/bdf/actions/runs/40531824618) pass;
  [ARM64 40531824644](https://github.com/btecme/bdf/actions/runs/40531824644) fails
  on the same download, with native install skipped. CI fuzz is skipped by event.

The failing probes use public APIs, synthetic PDFs and files beneath ignored
`target/bdf-evidence/BUILD-008-codex-review/`. Their assertions are intentional
review failures, not application crashes. Native Windows file/directory links are
required to be created successfully; there is no silent skip. The concurrent-writer
case uses the public clock seam to pause the issuer at a known point and a real
child process to revoke. No grant secret is printed. No product source was changed
to trigger these cases. No native macOS or independently executed Unix experiment
is claimed; hosted checks are identified separately.

The 20-file text manifest SHA-256 is
`9dcf7e4428526839a61f818523080e841f4c9a5dee014b0e8cb667e9916faa03` (`manifest.json` / `manifest.sha256`).
It identifies commands, logs, Cargo-selected library hashes, shared-source comparisons
between PR heads, hosted metadata/excerpts and the probe source/results. Generated
fixture files and binaries are excluded. Product source remained unchanged.

[Focused Claude follow-up](../bdf-claude-build-008-followup.md): correct R1–R4, apply
the settled branding/core-package direction, demonstrate the red cases green and
refresh the actual PR heads/evidence. Keep the first-two-slices scope. No production
fix, commit, push, merge, grant registration or agent access release was made by Codex.

## Revision history

| Date | Change |
|---|---|
| 2026-10-31 | Independent review of #11/#12; access-control corrections and focused product direction recorded. |
