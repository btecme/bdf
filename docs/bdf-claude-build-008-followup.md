# Claude Code follow-up: finish BUILD-008 without reopening settled choices

**2026-11-01 review update:** original R1–R4 are corrected; do not redo them. The
[BUILD-008 retest](bdf-evidence/BUILD-008-codex-retest.md) records the additional
failed-revocation defect B8-R5. The [PR #14 review](bdf-evidence/PR-014-codex-review.md)
records the new MCP audit corrections. The product/packaging direction below stays
settled; follow those current findings rather than the historical four-finding list.

Continue in the same local BDF repository. Read `AGENTS.md`, `CLAUDE.md`,
`docs/bdf-build-spec.md` and `docs/bdf-evidence/BUILD-008-codex-review.md`.
Complete the corrections in that review before expanding the feature set. Keep
BUILD-008 and BUILD-009 separate: these changes do not release agent access or
claim that the existing adapters have migrated.

The product direction is Windows 11 desktop first, local-first, general-office PDF
workflows, with BDF branding. Implement ordinary engineering choices yourself,
consistent with that direction and the repository rules. Report concrete results,
remaining product limitations and any genuine blocker. Do not send the owner a menu
of Rust APIs, model hosts, folder-name constants or branch-management procedures.

## Settled direction for this work

1. **Core packaging:** omit deferred OCR model downloads and payloads from the BDF
   v1 core package. Apply the profile consistently to Windows x64 and ARM64 packaging,
   the MSI/portable manifests and their payload/install checks. Keep the build,
   architecture, core smoke, install and uninstall checks real. Remove the assumption
   that an OCR models directory is mandatory for a core package. Do not switch to
   different Hugging Face models, weaken asset hashes/licensing checks, suppress
   download errors, or mark a failed/skipped required job green. Model absence is not
   proof that deferred operations are blocked at runtime; keep those qualification
   claims separate. Keep this packaging repair identifiable and scoped.
2. **Persistent identity:** new BDF application state belongs under BDF, including
   `%LOCALAPPDATA%\BDF\Access` on Windows and the corresponding BDF application-data
   namespace on other systems. This is already required by build-spec section 5;
   it is not an unresolved product-name decision. Update code, tests and docs together.
   Use a shared identity value where appropriate. Keep internal `pdfcraft-*` package
   and compatible tool names during stabilization. Record existing settings/recovery
   namespace migration deliberately; do not silently import upstream credentials,
   grants or signing identity, and do not discard recovery data during a rename.
3. **Integration:** #11 precedes #12. After both pass independent review and the
   required checks, integrate #11 first, retarget #12 to `main`, reconcile any changes
   and check the actual resulting head before integrating #12. Handle routine branch
   work without escalating merge-order choices. Follow the owner's merge authorization;
   this prompt alone does not authorize merging or bypassing a required check.

## Correction and delivery requirements

- Correct every finding in `BUILD-008-codex-review.md`, including alternate public
  service entry points and grant persistence concurrency. Do not defer these access
  control contracts to adapter migration.
- Add focused regressions for the reported counterexamples and demonstrate that
  restoring each defect makes its regression fail. Preserve the independent failed
  probes as the historical red baseline.
- Run scoped tests and required gates; identify the exact PR heads and hosted jobs.
  Distinguish tests on the stacked branch from tests on an isolated slice. A green
  suite does not close a counterexample that it never exercises.
- Update evidence, work-item status, roadmap and PR descriptions to the final scope.
  State limitations plainly. Do not mark the whole BUILD-008 work item verified
  while its application/adapter acceptance remains incomplete.
  Say that the new grant policy is not yet applied to legacy adapters; do not say
  that no agent can reach a document through any shipped surface. The existing
  opt-in CLI/MCP still uses the automation engine until BUILD-009 migrates it.
- Hand back a concise summary: what changed, what passed, what remains, and the
  next reviewable PR/head. Ask the owner only for missing authorization or a material
  product trade-off that the existing direction cannot resolve. If a repository rule
  requires approval, identify the exact rule and explain the user-visible consequence.
