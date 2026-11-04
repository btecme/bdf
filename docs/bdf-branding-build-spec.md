# BUILD-015 — BDF product identity

Status: ready for Claude implementation. Owner decision, 2026-11-04: this fork is
BDF; first-party product surfaces must not present it as Craft, PdfCraft or ArtCraft.
This is the next build assignment after synchronizing the tested Windows preview.
Claude builds; Codex independently checks. No further brand-name decision is needed.

## Outcome

A person launching, using, saving from, installing or asking for help with this
application sees BDF consistently. The app must identify this fork and its own
repository. Complete the visible identity pass before expanding another backend group.

## Required changes

- Use **BDF** in window titles, Home, About, menus, dialogs, notifications, accessibility
  labels, translated product strings, default document names, log headers and version
  output. Check generated-PDF creator/producer strings where BDF creates them; preserve
  the metadata and content of documents the person opens.
- Deliver **bdf.exe**, **bdf-cli.exe** and **bdf-worker.exe**. Update CLI help/examples,
  child-process discovery, build/package scripts, tests and CI to agree. Do not ship a
  second differently branded launcher that makes the candidate ambiguous.
- Use BDF settings, recovery, clipboard-cache and logs folders; portable data is
  **BDFData**, with `portable.txt` and `BDF.portable` markers. Keep existing upstream
  data intact. Do not silently import upstream grants/credentials or treat upstream
  application state as BDF state. Any supported old-preview recovery/migration path
  must preserve recoverable work and be explicitly documented.
- Give the Windows product BDF file-version metadata, application identity, shortcuts,
  uninstall/display names, install folder, registry capabilities and `BDF.Document`
  file-type identity. Use a new BDF MSI UpgradeCode and keep it stable after generation;
  this fork must coexist with an upstream installation. Do not reuse the upstream
  upgrade identity or change a user's default PDF handler without consent.
- Point project/help/issues/update links at **btecme/bdf**, not the upstream product.
  Remove the upstream Discord/support entry if BDF has no configured destination;
  do not invent a support address or channel. A missing BDF release must be handled
  clearly; never offer an upstream installer as a BDF update.
- Update the current README, launch/test instructions and active first-party user
  documentation to BDF. Preserve historical review/commit records as historical,
  and preserve accurate references to upstream repositories and dependencies.
- Inspect product wordmarks, icons and packaged artwork. Remove any upstream trademark
  asset the fork may not ship. Use contributor-original BDF artwork or existing
  compliant generic assets; follow AGENTS.md attribution/hashes and font rules.

Required copyright, license, author and contributor acknowledgements retain their
legal names. Dependency identifiers and historical quotations must remain accurate.
Do not mechanically replace text inside third-party source, licenses, original PDF
content or required attribution. Inventory residual first-party technical names and
explain any compatibility reason; they must not leak into the normal product identity.
Keep the first delivery bounded; no new architecture, UI framework or blanket rename
of unrelated shared Craft dependencies is required to deliver the visible BDF product.

## Acceptance

1. A clean portable Windows launch uses `bdf.exe` and BDFData; the window and About say
   BDF and point at this fork. Normal launch starts no MCP/control listener.
2. The CLI's help/version, Windows executable properties and worker discovery use BDF.
   The package contents, installation identity and uninstall metadata agree. Static
   package checks do not count as an actual installation test.
3. Open, annotate, fill a form, rotate/reorder, undo/redo, save a copy, close and reopen
   still work; a failed save preserves work. Keep the existing privacy/conflict gates.
4. Search the active first-party product strings, localization, launchers, Windows
   packaging and current user docs for old branding. Report the remaining matches by
   their real role (required attribution/history/dependency/compatibility), not as a
   broad exemption for missed visible labels.
5. Provide the runnable BDF preview, exact commit/checksum, screenshots of BDF's own UI
   with synthetic content and system-font fallback disabled, scoped checks and hosted
   CI. Keep implementation choices with Claude. Do not install/register/publish the
   candidate without the corresponding owner authorization.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Owner establishes BDF as the complete product identity; define bounded implementation and independent acceptance checks. |
