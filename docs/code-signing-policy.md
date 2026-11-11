# BDF code signing policy

Status: DRAFT for the SignPath Foundation application. Items in [square brackets] are for the owner to fill in before this is
published. Revision history at the end.

Free code signing for BDF is provided by [SignPath.io](https://signpath.io), and the certificate is issued by the
[SignPath Foundation](https://signpath.org).

## What this covers

BDF is an open-source, local-first PDF application for Windows (a portable ZIP and an MSI), licensed under MIT OR Apache-2.0.
Source: <https://github.com/btecme/bdf>. Downloads: <https://github.com/btecme/bdf/releases>.

Only programs built from this repository's source code, by the repository's own build workflow, are signed:

* `bdf.exe`, `bdf-cli.exe`, `bdf-worker.exe` and `bdf-updater.exe`
* the Windows installer (`.msi`) that carries them

Nothing else is signed. In particular, no third-party binary and no file that is not built from this repository is submitted for signing, and no part of BDF is proprietary: every dependency is open source and listed in `Cargo.lock`, `ATTRIBUTION.toml` and the notices shipped in each package.

## How a release is built and signed

1. The release is built by the GitHub Actions workflow in `.github/workflows/` from a commit on `main`. The build runs on GitHub-hosted runners, every action it uses is pinned to a full commit SHA, and the build log is public.
2. The unsigned programs are submitted to SignPath from that workflow run. SignPath records which repository, commit and workflow run the files came from and refuses files that do not come from this build.
3. A person listed under *Approvers* below approves the signing request. Nothing is signed automatically.
4. The signed files are packaged, checked (contents, file list and hashes) and published with a `SHA256SUMS.txt`.

## Roles

| Role | Who | What they may do |
|---|---|---|
| Authors | Anyone who contributes through a pull request | Propose changes. They cannot merge or request signing. |
| Committers and reviewers | [list GitHub accounts of the maintainers] | Review and merge pull requests into `main`. [Confirm that `main` is protected so it accepts changes only through a pull request, then keep this sentence.] |
| Approvers | [name and GitHub account of each approver] | Approve a signing request for a release. |

An approver checks that the request is for a tagged release built from `main`, that the workflow run is the public one for that commit, and that the files are the expected programs. [Confirm before publishing:] each approver uses multi-factor authentication on GitHub and on SignPath.

## Privacy policy

BDF opens and edits PDF files on the computer it runs on. It does not collect or send documents, file names, usage data or telemetry.

The only connection BDF makes to another system is the update check, which is on by default and can be switched off in Preferences. It fetches one public file (`bdf-update.json`) from the project's GitHub releases page, and, when the person asks for the update, downloads the release package from the same place. Nothing about the person or their documents is sent apart from what any web request to GitHub carries (the IP address and the standard request headers). GitHub's own privacy statement applies to that connection. A downloaded update is installed only if its signature, made with the project's update key, and its checksums verify.

BDF installs and updates for the current Windows account only and needs no administrator rights.

## Reporting a problem

If you believe a signed BDF file is malicious or was signed by mistake, report it at
<https://github.com/btecme/bdf/issues> or by email to [security contact]. Signed files that turn out to be wrong are reported to SignPath and revoked.

## Relationship to the update signature

BDF's in-app updater verifies releases with its own update key (a separate key kept by the project). That is independent of the
Windows code signature described here: Authenticode signing lets Windows show the publisher and build reputation, and the update
signature lets BDF check what it installs. Neither replaces the other.

## Revision history

* [11-11-2026]: first draft for the SignPath Foundation application.
