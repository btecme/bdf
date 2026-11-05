# BDF app icon

<img src="bdf.svg" alt="BDF app icon: the letters B, D and F in white on a navy rounded square" width="128">

A generic mark: the letters **B D F** in white, drawn as round-capped strokes, on a navy rounded square. It
is deliberately plain. It is not a logo design and can be replaced by one without touching the packaging.

**Palette:** navy `#1f3a5f`, white `#ffffff`; the document icon adds a light grey fold `#c9d3e0`.

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`. Windows and Linux icons use the full-bleed
tile. macOS icons put it on Apple's grid (an 824 px body centred on a transparent 1024 px canvas).

**Provenance:** contributor-original. The letters are stroked paths (no font, no image); `packaging/icons.py`
renders every raster and `.ico`/`.icns` from the same geometry, with the standard library only. Licence:
[LICENSE.txt](LICENSE.txt) (`MIT OR Apache-2.0`, like the repo). No Adobe, ArtCraft or PdfCraft artwork is
used or imitated (AGENTS.md §1).

## Files

| File | What it is |
|---|---|
| `bdf.svg` | the master vector |
| `bdf-1024.png` | 1024 px on Apple's grid; also the runtime Dock icon on macOS |
| `bdf.icns` | macOS icon (128-1024 px) |
| `bdf.ico` | Windows icon (16-256 px), embedded in `bdf.exe` by `apps/pdfcraft/build.rs` |
| `bdf-document.svg` | Windows PDF file icon master: a page with a folded corner and the lettering PDF, all stroked paths |
| `bdf-document.ico` | Windows PDF file icon (16-256 px), embedded as resource 2 in `bdf.exe` |
| `hicolor/<n>x<n>/apps/ai.storyteller.pdfcraft.png` | Linux hicolor theme, 16-512 px; the 256 px one is the runtime icon on Windows and Linux |
| `hicolor/scalable/apps/ai.storyteller.pdfcraft.svg` | Linux scalable icon (copy of the master) |

The Linux theme name is the packaged application id (`ai.storyteller.pdfcraft`, see
[`docs/bdf-branding-residuals.md`](../../docs/bdf-branding-residuals.md)): it is an internal identifier the
desktop files, Flatpak manifests and packaged icon names agree on, not a name the application shows.

Regenerate everything with `python3 packaging/icons.py assets/app-icon`; then update the SHA-256 values in
`ATTRIBUTION.toml` and run `cargo xtask assets --write`.
