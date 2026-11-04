# Windows Open picker — online-only OneDrive selection

Status: reproduced, not fixed. Owner-reported preview blocker, 2026-11-04.
Candidate: `aee35194`, copied to the owner's working repository target preview folder.

## Observed in the actual native dialog

- The file-type filter was correctly `PDF (*.pdf)`.
- Selecting the owner's online-only PDFs in the OneDrive shell location disabled
  the native Open button. The items were labelled `Available when online`.
- Replacing the filename with the existing local synthetic office PDF's full path
  enabled Open. After the desktop was unlocked, confirming it through the native
  dialog opened `office.pdf` in the app and rendered the fixture's text and form.
- Reopening the dialog showed the local fixture's filesystem directory normally.
  The OneDrive navigation entry instead exposed a friendly cloud-location name in
  its address editor. No filesystem path was established for the selected cloud items.
- The diagnostic picker was cancelled afterwards. The local synthetic PDF remains
  open. No personal PDF content was opened, downloaded or copied by this check.

This disproves the broad interpretation that the preview cannot open PDFs at all.
It confirms a user-visible cloud-selection compatibility problem. It does not yet
prove the exact Windows option/namespace cause, and no corrected binary is claimed.
The earlier path/control smoke did not cover this picker path.

## Source boundary to investigate

`PdfCraftApp::open_dialog` in `crates/ui-egui/src/lib.rs` asks for multiple files via
`rfd::AsyncFileDialog`, using the shared asynchronous picker in `pickers.rs`.
The locked rfd 0.17.2 Windows backend returns filesystem `PathBuf`s: its shell-item
`get_path` uses `SIGDN_FILESYSPATH` (`file_dialog/com.rs`), and `get_results` expects
that for every selected item. Its multi-file builder sets `FOS_ALLOWMULTISELECT`
explicitly. Whether preserving native defaults or choosing filesystem-backed items
fixes this case must be tested; the flag assignment alone is not proof of the cause.
Do not blindly add streaming flags when the consumer still requires a file path.

## Next Claude correction

Make ordinary Windows File > Open handle the supported OneDrive selection case,
including Windows' normal download/materialization where appropriate, or give a
clear, actionable supported route when an item is virtual/stream-only. A silently
disabled button is not an adequate explanation. Check the native picker/backend
before designing any cloud workspace, Graph authentication or new service framework.

Preserve local-file opening, multiple-file opening, asynchronous UI responsiveness,
private staging, save-conflict protection and existing agent confinement. Native
owner-selected file access must not be implemented by weakening delegated-client
reparse/root checks. Use maintained safe APIs; repository unsafe restrictions stand.

Prove the correction using the actual native picker: local PDFs, synced/downloaded
OneDrive PDFs, the online-only shell location, cancel, and multiple selection. Use
synthetic or approved samples and never commit private filenames/content/screenshots.
Agent-created cloud samples/uploads require the owner's specific authorization.
Do not call this fixed solely because `ui.open` or command-line paths pass.

For the owner's immediate evaluation, a PDF downloaded/synced to an ordinary local
folder is a supported comparison route. Actual native local opening is now verified.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Reproduce online-only cloud selection failure; verify native local selection opens and renders; define bounded correction and native acceptance cases. |
