# BDF portable deployment, Windows integration and updates (BUILD-018)

Status: implemented 2026-11-06 on `feat/build-018-portable-updates` (version 0.8.0); corrected 2026-11-07 after the independent
review (findings R18-1 to R18-4, listed in [the evidence record](bdf-evidence/BUILD-018.md)). Locally tested; hosted checks and
the real-Windows flows that need a disposable machine are listed there with what has and has not been run. No signed release
has been published: see [Owner setup](#owner-setup-to-activate-production-releases).

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
  The same action is also one click away: while Windows has not confirmed BDF as the default PDF app, a quiet *Make BDF the
  default PDF app* link sits in a thin strip at the bottom right of the window (with or without a document open; the
  strip is a panel of its own, so it takes room from the page and covers nothing). It runs exactly the Preferences action. Opening
  Settings is not success: the link goes only when Windows itself reports BDF (this registered copy, or the installed BDF) as the
  default, read again when BDF regains the focus. A cancelled choice or a failed registration leaves it in place. The registry is
  read at start, on regaining the focus and after a choice or removal, never per frame. A gear in the toolbar, between Document
  properties and Save, opens Preferences (Ctrl+,) with or without a document.
* **The previous version is kept** in `BDFPrevious` (`bdf-cli update rollback` goes back to it).
* **The dialog says what is true.** Not configured (a build with no trust root: it cannot verify, so it installs nothing), never
  checked, failed (with the reason), up to date, or available: `bdf-cli update status` reports the same as `check_state`.
  An empty state is never shown as "up to date".
* **A test offer is labelled a test offer.** A development build's offer reads **Test update** on the button, the tooltip and the
  dialog ("Test offer from a development feed. This is not a BDF release."), and a development release without notes of its own
  carries one line saying so. A development build remembers an offer only for the feed it came from: start the same copy
  without `BDF_UPDATE_FEED` (the ordinary way) and there is nothing on offer and nothing downloadable. A release build asks one
  fixed feed, so what it last verified still shows offline, exactly as before.

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

### Hosted build

`.github/workflows/release.yml` (manual, or on a pull request that touches packaging, `crates/update`, `apps/pdfcraft-updater` or
`xtask/src/release.rs`) builds the MSI and this portable package through `packaging/windows/package.ps1`: the four programs
(`bdf.exe`, `bdf-cli.exe`, `bdf-worker.exe`, `bdf-updater.exe`), `portable.txt`, the notices and `bdf-package.json`, written by
`cargo xtask portable-package` and checked by `portable-verify`. `packaging/windows/test-portable-package.ps1` then tests **that
exact ZIP**: contents against its own list, the four executables' machine type and subsystem, and (where the machine can run
them) an extracted copy that reports its version, is a managed copy, carries the expected update trust, has nothing on offer,
and whose `bdf-updater` runs; finally the Action1 script deploys the same ZIP under a throwaway test signature and the deployed
copy runs. The workflow then asserts the artifact holds exactly the MSI and the ZIP.

* The trust root is the repository **variable** `BDF_UPDATE_PUBKEY` (a public key). Unset, the artifact is built with no trust
  root, says `not_configured`, refuses every update and is labelled unsigned; the test expects exactly that. Set, the test
  expects `configured` and the key id of that key.
* Signing is **owner-activated and publishes nothing**: a manual run on `main` with `sign_release` and the secret
  `BDF_UPDATE_SIGNING_KEY` signs this package's `bdf-update.json`, verifies it like the app does, and uploads it as the artifact
  `bdf-windows-x64-RELEASE-INFORMATION`. The secret is visible to that one step, is written outside the workspace and deleted.
  No private key is in the repository and none is created by the workflow.

## Action1 deployment (Local System)

`packaging/windows/deploy-bdf.ps1` is the script; `packaging/windows/test-deploy.ps1` tests it against synthetic packages.

1. Put the release's `bdf-<version>-windows-x64-portable.zip` and `bdf-update.json` where the script can read them (an Action1
   package, a share). They must be the **same files** the in-app updater uses.
2. Create an Action1 *Run script* (PowerShell) with the contents of `deploy-bdf.ps1` and run it with:
   `-PackageZip <zip> -ReleaseFile <bdf-update.json> -PublicKey <release public key> -IncludeDefaultProfile`
   (add `-Channel dev` only for test releases; `-Users alice,bob` limits it; `-WhatIf` reports without changing anything).
3. What it does. As Local System it verifies the release itself in PowerShell (signature against `-PublicKey`, package hash, every
   file) and stops on anything unsigned or altered. It publishes the verified release to `C:\ProgramData\BDF\release\<version>-<id>`
   (only Local System and Administrators can change it; everyone can read it) and registers one task for the **Users group** that
   runs **when anyone signs in**: `\BDF\BDF-apply-release`. That task runs as the person who signed in, in their own session and
   with their own rights (no password, no administrator): it checks the signature, and if their copy is older it installs or updates
   `%LOCALAPPDATA%\Programs\BDF`. The files it writes are the person's own, so no ownership or permission change is needed and
   in-app updates need no administrator rights. A person already on this version or newer costs a signature check and nothing else.
   People who are signed in when the script runs have the task started in their session at once and are waited for (up to
   `-WaitSeconds`, default 180) so the run can report what happened; everyone else is reported as **pending until they sign in**,
   which is not a failure. `-Users` limits it to named people; `-WhatIf` reports without registering anything.
   **Why the install runs as the person, not as Local System (corrected after review round 2).** The folder is the user's, so they
   can change it at any moment. A Local System script that checks a path ("is this a link?") and then uses it can always be
   raced: the check is out of date by the time of the use. `packaging/windows/test-deploy-race.ps1` measures this against the
   previous in-place version: with a second process swapping a folder for a junction while the script ran, a Local System run was
   steered through the link in a measurable share of trials (zero hits in a sample proves nothing). Checking harder only narrows
   the window. Running the install with the user's own rights makes the window harmless instead: it can reach nothing the user
   cannot already change. **`-SkipLoggedOnUsers` is not a defence against this and does not remove any file-system race**: it only
   decides whether the task is started now in the sessions of people who are signed in. A task that runs as another person
   without their password was tried first and **does not work on current Windows**: from Local System, Task Scheduler refuses
   ("Access is denied") to register an S4U task for another user, with or without the batch-logon right, and .NET's S4U needs a
   domain (hosted-runner probe on Windows Server 2025, build 26100; see `docs/bdf-evidence/BUILD-018.md` section 8).
   Inside the person's own task the older checks remain, because they protect the person from a damaged copy: the installed
   `bdf-package.json` belongs to the user, so it is checked like the signed one before anything moves (schema, version, count, and
   every path: no `BDFData`/`BDFUpdate`/`BDFPrevious`, no absolute, drive, stream, 8.3-short or back-slash names, no traversal, no
   duplicates), and an installed copy whose list fails is refused and left exactly as it is; the folder is scanned for junctions
   and symbolic links first (the user's `BDFData` is never read or listed); every path is checked just before it is moved, created
   or deleted; deleting never follows a link. Local System itself reads the profile list from the registry and nothing inside a
   profile, except the Default profile (only administrators can write there, so it is installed directly). The working directory
   is System32 and the log, release and results folders are used only if they are not links and are owned by SYSTEM or
   Administrators. Each person's task leaves a text result in `C:\ProgramData\BDF\user-results` (everyone may add a file, nobody
   may change another person's); it is for the log and is never trusted for anything.
   To stop deploying: `Unregister-ScheduledTask -TaskName BDF-apply-release -TaskPath '\BDF\' -Confirm:$false` and delete
   `C:\ProgramData\BDF\release`; installed copies stay.
4. **Reruns are safe, and so is an interruption.** A user already on this version or newer is untouched. An older managed copy is
   updated by renaming files (previous version kept in `BDFPrevious`, rollback on failure) **only if nothing is running from it**;
   a running copy is skipped and updated at the person's next sign-in. `BDFData` and any file the user keeps in the folder are never touched (a
   regression test plants hostile installed lists, including a native `BDFData\settings.json` path, and checks the data stays where
   it was). A folder at that path that is not a managed BDF copy is left alone and reported as a failure. If a run is stopped
   part-way (the task is ended, the machine stops), it leaves a marker (`BDFUpdate\deploy-pending.json`) and the files it had set
   aside; the next run does not treat the copy as current but finishes the job from the verified package, keeps the displaced
   version whole in `BDFPrevious`, and removes the marker. Until then the copy may be part old, part new; the in-app updater's
   rollback and crash recovery are separate and are unchanged. `packaging/windows/test-deploy-interrupt.ps1` ends the script at
   a swept series of moments and checks this; against the previous script it fails (the previous version was thrown away on
   resume).
5. **New users:** `-IncludeDefaultProfile` also installs into `C:\Users\Default`, which Windows copies into every profile
   created afterwards; the sign-in task covers them too. Running the Action1 policy again publishes a newer release.
6. Not done on purpose: nothing runs BDF as SYSTEM or runs anything from a user's folder as SYSTEM; no default-app change and no registry
   change (people choose in Preferences ▸ Windows integration); no shortcut, service or firewall rule; no Action1 credentials or
   API secrets are in BDF or its package. The log is `C:\ProgramData\BDF\deploy.log`.

## Owner setup to activate production releases

1. **Signing key**: run `cargo xtask update-keygen` once on a trusted machine; store the private key file securely (not in this
   repository; the tool refuses a path inside it). In the repository settings add the public key as the **variable**
   `BDF_UPDATE_PUBKEY` and, to let the workflow sign, the private key as the **secret** `BDF_UPDATE_SIGNING_KEY` (the workflow
   code is in place; no secret has been added by this work). A protected `release` environment with a required reviewer is a
   sensible extra.
2. **Release hosting**: publish releases on `btecme/bdf` (assumed), with the three files above.
3. **Two minutes at a Windows 11 desktop** for the one flow a hosted runner cannot do: in Preferences ▸ Windows integration choose
   *Make BDF my default PDF app*, choose BDF for `.pdf` on the Settings page that opens, and double-click a PDF. The rest of the
   acceptance flows run on disposable hosted Windows machines by `.github/workflows/qualify-windows.yml` (see
   `docs/bdf-evidence/BUILD-018.md` section 8).
4. Optional: an Authenticode certificate for the executables.
