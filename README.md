# BDF

BDF is a local-first PDF workbench, focused on a dependable Windows 11 desktop app.
The core workflows are viewing and searching PDFs, reviewing with annotations,
filling existing AcroForms, organizing pages, printing, and trustworthy saving with
undo/redo and recovery. Web and shared cloud workspaces follow desktop qualification.

This repository is an independent fork with its own product direction. The current
build is a developer preview; implementation and release qualification are tracked
separately. The product identity (names, executables, storage, Windows identity, links and
artwork) is BDF throughout ([BDF branding assignment](docs/bdf-branding-build-spec.md);
what still carries an inherited technical name, and why, is in
[docs/bdf-branding-residuals.md](docs/bdf-branding-residuals.md)).

## Current checkpoint

- [Verified progress and limits](docs/bdf-evidence/2026-11-04-progress-check.md)
- [Windows preview checklist](docs/bdf-windows-preview-checklist.md)
- [Open-picker issue: online-only OneDrive selection](docs/bdf-evidence/windows-open-picker-issue.md)
- [Focused Claude build handoff](docs/bdf-claude-focus-prompt.md)

The preview opens local PDFs and, through the native Open dialog, online-only OneDrive
PDFs while the OneDrive client is connected. Windows itself greys out Open for an
online-only file whose sync client is not running (in every application), so start
OneDrive first. Do not treat a passing path/control-channel test as proof of that
cloud-selection workflow.

Generated previews live under `target/` and are not included in Git synchronization.
Each preview folder holds `bdf.exe`, `bdf-cli.exe` and `bdf-worker.exe`; its README,
BUILD.json and verification notes identify the binaries and checks actually performed.

## Build and contribute

Read [AGENTS.md](AGENTS.md) and [CLAUDE.md](CLAUDE.md). Start from the
[build specification](docs/bdf-build-spec.md), [work items](docs/bdf-work-items.toml)
and [BDF PRD](docs/bdf-prd-draft.md). Claude implements; Codex independently reviews.

Cargo package and crate names keep their inherited technical identifiers; the executables
they build are `bdf.exe`, `bdf-cli.exe` and `bdf-worker.exe`:

```sh
cargo build -p pdfcraft -p pdfcraft-cli -p pdfcraft-worker --locked -j 2
cargo run -p pdfcraft -- file.pdf        # opens bdf
```

Use scoped, bounded tests with two build jobs. Hosted CI owns the full workspace
sweep. Keep MCP and the UI control channel opt-in. Do not install, register or publish
a build as a side effect of ordinary development or testing.

Project issues and development: [btecme/bdf](https://github.com/btecme/bdf) ·
[issues](https://github.com/btecme/bdf/issues).

## Origin, license and credits

BDF is based on [PdfCraft](https://github.com/storytold/pdfcraft) by the ArtCraft team
and contributors. BDF is an independent fork.

BDF is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors. Required notices are in [NOTICE](NOTICE).
Original copyright and contributor acknowledgements remain intact.

Bundled fonts, icons, images and other assets keep their own open licenses; their
authors, sources and licenses are listed in [ATTRIBUTION.md](ATTRIBUTION.md).
The optional shared font input [craft-fonts](https://github.com/storytold/craft-fonts)
retains its own attribution and licenses. No upstream trademark artwork is licensed
for use by this fork; see the asset rules in AGENTS.md.

## Docking developer notes

Tools and Inspector panels can be dragged into tab groups, split beside the PDF, or floated within the app window. Read and full-screen modes keep their existing focused document view. Drag a panel header to a target edge or another header. Its context menu offers Float panel, Move to group, and Close panel. Existing panel toggles reopen panels at their saved location; Reset Panel Layout restores the app layout. Floating panels remain inside the current app window.

Advanced `ui.dock` operations use the same validated transaction as pointer gestures:
`moveFloating` updates `rect`, `resizeSplit` accepts a boolean child `path` and
`size` (`{"Ratio":0.5}`, `{"FixedFirst":280}`, or `{"FixedSecond":280}`),
`setStackOpen` takes `open`, and `resizeStack` takes a point `height` or null.
A center `move` can include `before` (panel ID or null to append) for tab order.
The equivalent `{"action": ...}` form accepts a serialized shared docking action;
this is also the form emitted by the renderer. Protected-canvas and app-specific panel
rules apply equally to both forms.
