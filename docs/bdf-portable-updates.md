# BDF portable deployment, Windows integration and updates (BUILD-018)

Status: implemented 2026-11-06 on `feat/build-018-portable-updates` (version 0.8.0). Locally tested; **not** hosted-verified,
not independently reviewed, and the real-Windows flows listed in [the evidence record](bdf-evidence/BUILD-018.md) that need a
disposable machine have **not** been run. No signed release has been published: see [Owner setup](#owner-setup-to-activate-production-releases).

## What a person gets

* **The portable ZIP is the product.** Extract it anywhere you can write and open `bdf.exe`. No installer, no administrator
  rights, nothing written outside the folder. `portable.txt` keeps settings, logs and recovery files in `BDFData` beside the
  program; updates never touch that folder.
* **A green Update button** (top right of the window, beside the "b-tec Built" mark, which links to
  <https://www.b-tec.org/bdf>) appears only when a newer, signed, applicable release is on offer. *Update* downloads and verifies
  the whole package; *Close BDF and update* uses the normal close flow (Save / Don't save / Cancel for every unsaved document;
  it never saves over an original by itself), then closes BDF, replaces the program, and opens BDF again with the documents
  that were open. *Later* leaves everything alone. Automatic checks are on by default, cached (12 h) and backed off after
  failures (1, 2, 4 … 24 h); a failed check is silent and never delays opening a PDF. Preferences has the off switch; Help ▸
  Check for updates always asks.
* **Optional Windows integration** (Preferences ▸ Windows integration): *Make BDF my default PDF app* registers this copy for
  this Windows account (Open with, Default apps) and opens Windows' Default apps page, where the person confirms. BDF never
  sets the default and never writes the protected `UserChoice`. *Remove Windows integration* removes only what this copy
  registered; *Refresh* re-points a registration after the folder moved. An extracted copy never touches the registry.
* **The previous version is kept** in `BDFPrevious` (`bdf-cli update rollback` goes back to it).

## Trust model (what is verified, and what happens without a key)

* A release is `bdf-update.json`, one envelope: `{ "payload": base64(manifest JSON), "signature": hex(ECDSA P-256 r||s over SHA-256(payload)) }`.
  The manifest carries product, channel, version, notes and, for each package, its **size and SHA-256**. So the package
  checksum comes from signed bytes, never from an unauthenticated place.
* The public key is compiled into the build (`BDF_UPDATE_PUBKEY`). **A build without one refuses every update operation**
  (status says `trust: missing`; no Update button; `update check` fails). Nothing falls back to "trust the download".
* A `dev` build (`BDF_UPDATE_CHANNEL=dev`) trusts a throwaway test key, checks exactly as hard, labels itself in the dialog,
  and is the only build that reads its feed from `BDF_UPDATE_FEED` (HTTPS or loopback). A release build ignores that variable
  and only fetches a package from `https://github.com/btecme/bdf/releases/download/`.
* Before anything is written: signature, channel, product, version, architecture, package size and hash, every file's path,
  size and hash against the package's own list, no unlisted files, no path outside the folder, no `BDFData`/`BDFUpdate`/
  `BDFPrevious` entries, no links. A release **older than the copy**, or older than one already seen (a replay), is rejected.
* Replacement is by journaled renames (never overwrite in place), the launcher last; an interrupted run is rolled back at the
  next start of `bdf.exe` or `bdf-cli.exe`; the result is verified against the new list or rolled back. BDF, `bdf-cli` and
  `bdf-worker` still running from the folder make the updater **wait** (up to 10 minutes) and then give up unchanged; nothing is
  ever ended to make room. Read-only folders and a full disk are detected before or during staging and reported.
* The updater (`bdf-updater.exe`) has no network code and no privileges beyond the user's; there is no service.

## Agent, CLI and MCP surface

`bdf-cli update status|check|download|apply|rollback`, `bdf-cli windows status|register [--open-settings]|unregister|open-settings`
are the same tools as `update_*` and `windows_*` in `bdf-cli tools`. **`update_status` and `windows_integration_status` are
read-only and always available. Every other tool needs the separate software authorisation**: typing the `bdf-cli update …`
command yourself implies it; an agent route (`bdf-cli run`, `bdf-cli mcp`) needs `--allow-software`, and the MCP server then lists
the tools (without it they are neither listed nor callable). `--root` and document grants never imply it, and the grant-limited
server (`mcp --client`) has a fixed tool list that contains none of them (tested).

## Release process (owner)

```sh
cargo xtask update-keygen --out C:\secure\bdf-update-signing.key        # once; keep the file secret and backed up; prints the public key
$env:BDF_UPDATE_PUBKEY = "<the printed public key>"                     # release builds only; never BDF_UPDATE_CHANNEL
cargo build --release --locked -p pdfcraft -p pdfcraft-cli -p pdfcraft-worker -p pdfcraft-updater
cargo xtask portable-package --bin-dir target\release --version 0.8.0 --out dist
cargo xtask update-sign --zip dist\bdf-0.8.0-windows-x64-portable.zip --version 0.8.0 --key C:\secure\bdf-update-signing.key `
  --expect-pubkey $env:BDF_UPDATE_PUBKEY --notes release-notes.txt --out dist\release
cargo xtask update-verify --release dist\release\bdf-update.json --zip dist\release\bdf-0.8.0-windows-x64-portable.zip --pubkey $env:BDF_UPDATE_PUBKEY
```

Upload `bdf-0.8.0-windows-x64-portable.zip`, `bdf-update.json` and `SHA256SUMS.txt` to the GitHub Release `v0.8.0` of
`btecme/bdf`; BDF reads `https://github.com/btecme/bdf/releases/latest/download/bdf-update.json`. A published `latest` release
is what people are offered, so publish last. Authenticode signing of the executables is separate and not required for updates
(the update signature is the trust root); it is still worth doing so SmartScreen is quiet.

## Action1 deployment (Local System)

`packaging/windows/deploy-bdf.ps1` is the script; `packaging/windows/test-deploy.ps1` tests it against synthetic packages.

1. Put the release's `bdf-<version>-windows-x64-portable.zip` and `bdf-update.json` where the script can read them (an Action1
   package, a share). They must be the **same files** the in-app updater uses.
2. Create an Action1 *Run script* (PowerShell) with the contents of `deploy-bdf.ps1` and run it with:
   `-PackageZip <zip> -ReleaseFile <bdf-update.json> -PublicKey <release public key> -IncludeDefaultProfile`
   (add `-Channel dev` only for test releases; `-Users alice,bob` limits it; `-WhatIf` reports without changing anything).
3. What it does, as Local System: verifies the release itself in PowerShell (signature against `-PublicKey`, package hash,
   every file) and stops on anything unsigned or altered; then, for each real user profile (local, domain and Azure AD
   accounts from the profile list: never SYSTEM or service accounts), installs into `%LOCALAPPDATA%\Programs\BDF` of **that user**
   (not Action1's or SYSTEM's `LOCALAPPDATA`), gives that user ownership and Modify on their own copy so in-app updates need no
   administrator rights, and changes no other permission. It refuses to write through a junction or link.
4. **Reruns are safe.** A user already on this version or newer is untouched. An older managed copy is updated by renaming files
   (previous version kept in `BDFPrevious`, rollback on failure) **only if nothing is running from it**; a running copy is skipped
   and updated by the next run. `BDFData` and any file the user keeps in the folder are never touched. A folder at that path
   that is not a managed BDF copy is left alone and reported as a failure.
5. **New users:** `-IncludeDefaultProfile` also installs into `C:\Users\Default`, which Windows copies into every profile
   created afterwards. Users who already exist are covered by running the Action1 policy on a schedule.
6. Not done on purpose: nothing runs BDF as SYSTEM or runs anything from a user's folder; no default-app change and no registry
   change (people choose in Preferences ▸ Windows integration); no shortcut, service or firewall rule; no Action1 credentials or
   API secrets are in BDF or its package. The log is `C:\ProgramData\BDF\deploy.log`.

## Owner setup to activate production releases

1. **Signing key**: run `cargo xtask update-keygen` once on a trusted machine; store the private key file securely (not in this
   repository; the tool refuses a path inside it). Give builds the public key as `BDF_UPDATE_PUBKEY`
   (`.github/workflows/release.yml` still needs that variable added; no signing secret has been added to the repository).
2. **Release hosting**: publish releases on `btecme/bdf` (assumed), with the three files above.
3. **A disposable Windows machine** (Windows Sandbox or a VM, or a standard test account) to run the remaining acceptance flows.
4. Optional: an Authenticode certificate for the executables.
