# BDF Windows preview — owner checklist

Status: proposed private-preview checklist, 2026-11-04. This is a hands-on evaluation,
not v1 release acceptance. The accompanying progress check records what Codex actually ran.

Use copies of PDFs, keep the originals, and save your results under new names. Start
with an ordinary office PDF, an existing fillable form, and a scan or drawing typical
of your work. No cloud workspace or agent setup is needed.

| Task | What should happen | Your result / friction |
|---|---|---|
| Open, navigate and zoom | Pages draw legibly; fit-page/fit-width and page navigation are easy to find; no frozen UI. | |
| Search and select | Search finds embedded text and selection copies the expected text. Image-only scans need OCR, which is outside this preview's core. | |
| Review | Add a note and highlight; change a property; undo and redo. Save a copy, close it and reopen it; content and placement remain correct. | |
| Fill a form | Existing text/checkbox fields can be filled. Save/reopen preserves the values and their appearance. | |
| Organize pages | Rotate and reorder pages; undo/redo. The saved copy has the intended order, orientation and page count. | |
| Save and close | A successful save really persists. Closing an unsaved edit asks what to do. A failed save leaves the edit open and the original intact. | |
| Print preview | Intended pages, scale and comments appear correctly. Physical printer output is a separate check. | |
| Restart | Reopen the saved copy after quitting the app. Report any recovery prompt that loses or misidentifies work. | |

For each problem, record the task, expected result, actual result and whether it repeats.
Include the candidate commit from the preview README. Share a synthetic or approved
sample if useful; do not put confidential PDFs or document content in public issues.

The app shows BDF branding ([the branding assignment](bdf-branding-build-spec.md)). The preview keeps settings and
recovery in its own portable folder. Keep the whole folder together. Ordinary testing
does not require MCP, a grant, a control token or installation/default-handler changes.

## Delivery checkpoint

The implementer supplies the runnable folder/ZIP, exact commit and checksum, launch
steps, completed verification rows, and a short list of remaining problems. Proposed
checkpoint: Friday, 2026-11-06. Do not delay the first hands-on preview for unrelated
agent catalogue work, web work or a reusable support platform; keep their release
requirements tracked separately.

## Revision history

| Date | Change |
|---|---|
| 2026-11-04 | Initial Windows office-workflow preview checklist. |
