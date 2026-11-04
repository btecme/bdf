# Windows Open picker — online-only OneDrive selection

Status: root cause found (the OneDrive sync client was not connected, not a BDF or `rfd` option); actionable message added; opening verified through the native dialog. Owner-reported preview blocker, 2026-11-04.
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

## Findings (2026-11-04, second pass)

Reproduced on the owner's machine with the `aee35194` build, then isolated by driving the
real native Open dialog through UI Automation (names and content of the owner's files are not
recorded here; counts and states only).

| Condition | Open button | Notes |
|---|---|---|
| OneDrive client (`OneDrive.exe`) **not running**; one local PDF selected | enabled | opens |
| same; one or three online-only PDFs selected ("Available when online") | **disabled** | tooltip says "Available when online" |
| same, in Paint's Open dialog and in a stock .NET `OpenFileDialog` (neither is BDF or `rfd`) | **disabled** | the OS dialog itself |
| same, raw `IFileOpenDialog` with the default options, `FOS_FORCEFILESYSTEM`, `FOS_NOVALIDATE`, `FOS_ALLOWNONSTORAGEITEMS` or no options at all | **disabled** | no option set changes it |
| same, reading an online-only file from the file system | fails | `The cloud file provider is not running.` (Win32 error 362) |
| OneDrive client running and connected (it took a few minutes after the client was started) | enabled | for one and for three online-only PDFs, and for local PDFs |
| same, `aee35194` build, one online-only PDF confirmed | opens | the file downloads and renders |

Conclusion: the dialog's Open button is disabled by Windows for a cloud placeholder whose
sync provider is not connected. It is the same in every application, and no dialog option
changes it, so the earlier hypothesis about the `rfd` option flags is disproved. With the
provider connected, the unchanged build selects and opens online-only OneDrive PDFs
(single and multiple selection), and cancel leaves the app unchanged.

What BDF can and does do: when it reads a file itself (drag and drop, a path argument, a
recent file, Open after the picker) and Windows answers error 362, the message now says what
to do (start OneDrive, or open a copy saved on this PC) instead of only the system text
(`open_worker::read_error_text`, unit-tested). It cannot enable a button that Windows
disables, and it does not start a user's sync client.

## Source boundary that was investigated

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
| 2026-11-04 | Second pass: root cause is the OneDrive provider not running (OS-level, all apps); native-dialog matrix with the provider connected; actionable message for error 362. |
