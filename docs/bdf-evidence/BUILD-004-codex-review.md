# BUILD-004 — independent Codex review

Status: **historical initial review: two P1 findings at the heads below.**

Latest disposition: [2026-10-19 final correction retest](BUILD-004-codex-final-retest.md) at `26a0797a` closes R1/R2. Earlier findings and their original evidence remain below.

Reviewed 2026-10-18: [PR #5](https://github.com/btecme/bdf/pull/5), initial head
`0d1a64f20af456d86b78c336dc0a7b41db23028d` and follow-up
`676937351e7c200c49c4fe6bef9afe16347cbce5`. The checkout was clean before review.
CC pushed the follow-up during this review; its changes are accounted for below. The
persistence replacement boundary and engine completion code implicated by R1/R2 are unchanged
between these heads. No implementation, test, workflow or dependency fix was made by Codex.

## R1 — P1 — keep staged document bytes private through replacement

Location: `crates/platform/src/persistence.rs:218–229`, with
`crates/platform/src/staging.rs:63–80`.

The writing handle closes before replacement. On Windows `keep_permissions` does nothing;
the staged file then has the folder's inherited DACL until `ReplaceFile` runs. An exclusive
writing handle protects only the time that handle remains open. Preserving the final target's
DACL does not protect these intermediate bytes. A crash in this interval can leave the staging
file behind, so the exposure is not safely bounded to milliseconds.

Independent synthetic probe: a fixture folder grants ordinary Users read access, while the
target's inheritance is removed. The observing restricted process gets **Access denied**
reading the protected target. At the injected replacement boundary, the same process reads
the staged **SYNTHETIC SECRET NEW BYTES**; the stage's DACL contains inherited Users read
access. The service subsequently cleans up the stage after the injected error. The final
privacy assertion fails with exit **101**. No personal document was used and only the ignored
synthetic fixture's permissions were changed. This is not a full two-account end-to-end test.

Required correction: establish private staging before writing sensitive bytes and retain it
through the entire commit/error lifecycle. Evaluate a compliant maintained safe wrapper or a
validated secure staging design; refuse the operation before writing sensitive bytes where
the required guarantee cannot be established. Do not substitute a plaintext copy, a rename
that loses target protection, or a waiver of the privacy requirement. Add adversarial access
checks at the write, closed-handle/precommit and failure boundaries.

The work item's explicit stop condition is:

> Required replacement/ACL primitive cannot be implemented with approved safe dependencies:
> report it rather than using unsafe code or a non-atomic copy fallback.

CC was right to report this limitation. Reporting satisfies the stop instruction; it does not
satisfy the acceptance criterion that sensitive staging bytes are never temporarily public.
The absence of an ACL setter in the selected wrapper is evidence about that wrapper, not a
proof that every compliant design is unavailable. AGENTS.md still requires owner sign-off
for an actual unsafe exception; none is recommended or approved by this review.

## R2 — P1 — publish save completion only after the complete refresh succeeds

Location: `crates/engine/src/lib.rs:3780–3788`.

`mark_saved_at` assigns the new COS document, clears `dirty`, and increments the generation
before `Self::refresh` can fail. The saved bytes can be syntactically readable by the COS
layer but rejected by the renderer/inspection layer. In that case the old working edit is
already gone when the method returns `EditError::Reopen`.

Independent synthetic probe starts with a valid one-page document and a working Title edit
`KEEP WORKING EDIT`. The completion payload has a valid PDF/COS structure with an invalid
page-tree child reference. Observed outcome:

```text
Err(Reopen("the file is not a readable PDF: the document has no pages"))
dirty=false; title=None; pages=1
```

The retained page count is stale view information; the actual working COS state has lost the
Title edit. The regression requiring an unsaved document fails, exit **101**. This surviving
failure-handling flaw is in the method modified by BUILD-004; it is not claimed to have been
newly introduced by the refactor.

A second probe starts with a clean document, as in Save As without edits, and supplies bytes
that cannot reopen at all. The path changes to `save-as.pdf`, but `dirty` remains **false**
after `Reopen`. That assertion also fails, exit **101**. The submitted test starts dirty,
which conceals this case. Returning `false` from the UI save handler is insufficient if later
close/recovery decisions still see a clean document.

Required correction: prepare/validate the candidate rebase and refresh before publishing it
over the working state. On failure retain the working edit/undo/recovery state, make the
unsaved/recoverable outcome explicit even for an initially clean document, and retain the
correct committed destination. Add both failing-before-fix engine cases and focused UI tests
for notice, recovery retention and pending-close behavior. Keep filesystem commit and
postcommit refresh outcomes distinct. Do not introduce background jobs or a new application
service merely to fix this completion boundary.

## Hosted failures and CC's follow-up

Initial head `0d1a64f2`:

| Check | Independent observation |
|---|---|
| [Ubuntu](https://github.com/btecme/bdf/actions/runs/39129020137/job/117680148167) | New-file mode assertion fails: **0600 vs 0644**. The ordinary-mode probe used the private staging creator. |
| [FreeBSD](https://github.com/btecme/bdf/actions/runs/39129020127/job/117680148077) | Same mode assertion fails. |
| [Windows](https://github.com/btecme/bdf/actions/runs/39129020137/job/117680148395) | **23 pass / 1 fail**: the rename contrast has identical ACLs in the hosted fixture. This does not demonstrate a production replacement failure. |
| [Dependency policy](https://github.com/btecme/bdf/actions/runs/39129020137/job/117680148373) | Fails on three inherited informational **unmaintained** advisories: `zvariant`, `zbus_names`, `zvariant_derive` (RUSTSEC-2026-0370/0371/0372). Not a newly demonstrated exploit in BUILD-004. |
| wasm and ARM64 | Pass on that head. Passing ARM64 packaging does not substitute for persistence correctness tests. |

Follow-up `67693735` changes the mode probe to an ordinary exclusive empty-file creation and
adds an explicit ACL entry to make the contrast non-vacuous. These are appropriate bounded
engineering corrections. The follow-up's **24 platform tests pass** in a normal Windows
execution context. Under Codex's restricted token the ACL fixture's replacement is denied;
that environment-specific result is recorded separately, not called a product replacement
defect or hidden by skipping the test. Real privacy qualification still needs the independent
R1 property, not only equality of final ACL text.

Latest-head [CI](https://github.com/btecme/bdf/actions/runs/39130736838),
[FreeBSD](https://github.com/btecme/bdf/actions/runs/39130736800), and
[ARM64](https://github.com/btecme/bdf/actions/runs/39130736804) were running at inspection.
Wasm had passed and dependency policy had failed again; Windows and Ubuntu tests were pending.
No pending run is called passed. The dependency gate needs an engineering disposition that
keeps policy meaningful, not a blanket advisory ignore or a request that the product owner
choose Cargo dependencies.

The advisory database identifies these as maintenance notices with migration to zbus 6:
[zvariant](https://github.com/RustSec/advisory-db/blob/main/crates/zvariant/RUSTSEC-2026-0370.md),
[zbus_names](https://github.com/RustSec/advisory-db/blob/main/crates/zbus_names/RUSTSEC-2026-0371.md),
[zvariant_derive](https://github.com/RustSec/advisory-db/blob/main/crates/zvariant_derive/RUSTSEC-2026-0372.md).

## Verification and limits

Selected private evidence: **15 files**, manifest SHA-256
`191cffd4b585097b46cf3afd1fe8b58244cb36059005e32e77a6942222e1333e`.
The manifest excludes executables, debug symbols and synthetic permission fixtures.

- Fresh scoped engine suite: **235 pass, 1 ignored**. These passing tests miss the two R2 cases.
- Final independent probes: privacy, clean reopen failure and failed refresh each fail their
  intended safety assertion, exit **101**. Synthetic fixture/source and logs are under
  `target/bdf-evidence/BUILD-004-codex-review/`, not committed.
- Platform tests were rerun because the source/fixture changed during review. The initial
  contrast failure, follow-up restricted-token failure, and successful normal-context run
  are separate records. No failing original suite is relabeled green.
- Probe harness setup initially omitted native linker search paths, and its first privacy
  check tried to read the intentionally protected target after the operation. Those harness
  issues were corrected; they are not counted as product defects. The final logs above
  contain the declared safety assertions.
- Windows privacy was tested at a controlled service seam. No second account was provisioned,
  no physical power interruption was performed, and no native macOS test ran. Linux exercises
  Unix branches; it cannot exercise code behind `cfg(target_os = "macos")`.
- No full local workspace sweep, extra renderer work, signing, release, MCP registration,
  merge or push was performed for this review. Required passing gates must be rerun after
  implementation corrections; this review does not pretend that an expected-red safety
  probe is a passing gate.

## Delivery focus

The shared persistence boundary, flush propagation, ACL-preserving final replacement and CLI
anti-truncation work are directly relevant to the product objective. Keep them. Fix R1/R2
before accepting this service. Routine branch retargeting, test fixture design, dependency
triage and evidence bookkeeping belong with the engineering agents. PR #4 remains a separate
BUILD-003 review dependency; this review neither accepts it nor treats its file independence
as permission to claim BUILD-003 verified.

Windows 11 remains the release priority. Preserve existing Linux/wasm gates without adding
macOS feature scope to this task. Hardware power-loss qualification and localization coverage
must be tracked honestly against their actual release requirements; they need not become a
new product-owner question for each persistence change.

For the owner-facing working agreement and bounded correction handoff, use
[the Claude focus prompt](../bdf-claude-focus-prompt.md).

## Revision history

| Date | Change |
|---|---|
| 2026-10-18 | Independently reviewed initial and follow-up heads; reproduced staged-byte exposure and save-completion state loss; scoped suites and hosted failure causes recorded; changes requested. |
