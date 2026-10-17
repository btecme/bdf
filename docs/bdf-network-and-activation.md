# Network and agent activation: what is off by default

Status: **verified by source reading, existing tests and a dependency gate, 2026-10-17.** BUILD-003 deliverable 5. It was **not** verified by watching the running application's traffic; section 4 says what that would add.

The requirement ([PRD](bdf-prd-draft.md), [build spec](bdf-build-spec.md)): network and MCP stay off by default; MCP is stdio and explicitly launched, with no registration or background server; no production support URL is invented; no credentials are imported silently. This page records where the product stands against that, with the evidence for each line and what is still open.

## 1. Every way the product can talk to a network, a socket or an agent

| Path | Default | What starts it | Evidence |
|---|---|---|---|
| Help > Check for updates (HTTPS request to `api.github.com`, the only HTTP client in the product) | **Off.** No check at start. | The user choosing the command. Also reachable by an agent through `command_run` / the control channel, which is why the inventory classifies `app:update-check` and `help.check_updates` as Excluded. | `crates/ui-egui/src/updates.rs` (module doc: "no check at start (the owner's decision)"); test `nothing_is_asked_until_the_user_checks` in `crates/ui-egui/tests/ui_group/updates.rs`; the update source is `None` unless the desktop app supplies it (`crates/ui-egui/src/lib.rs`); `apps/pdfcraft/src/updates.rs` is the only caller of `ureq`. |
| Help menu links (Discord, web page, GitHub, website) and release page | Nothing is fetched. A click hands the address to the operating system's browser. | The user clicking. | `crates/engine/src/links.rs` (the table of four links); `crates/ui-egui/src/lib.rs` `open_url`. |
| Links inside a document | Refused unless the scheme is `https`, `http` or `mailto`; the user is shown the address first. | The user following a link. | `DOCUMENT_SCHEMES`, `MAX_DOCUMENT_URL` and the tests in `crates/engine/src/links.rs`. |
| Cloud AI commands (`ai.ask`, `ai.summary`, `ai.translate`) | **Not present.** They are catalogue entries gated on a provider; no provider and no client exists. | Nothing can start them. | `crates/engine/src/catalog.rs` (`Provider`); classified Excluded in `crates/automation/src/capability.rs`. |
| MCP server (`pdfcraft-cli mcp`) | **Not running.** It speaks over stdin and stdout of the process that launched it, with no socket and no registration. The subcommand exists in default builds and is absent with `--no-default-features`. | An explicit `pdfcraft-cli mcp` launch by the user or by an agent host the user configured. | `apps/pdfcraft-cli/src/main.rs` (module doc: "never starts on its own"); `apps/pdfcraft-cli/Cargo.toml` (`mcp` feature); CI step "CLI without the MCP server still builds". No code writes an agent host's configuration. |
| UI control channel | **Off.** | The app launched with `--control <file>`. Then it listens on `127.0.0.1` on a random port; every connection must first present the random token written to `<file>`. | `crates/ui-egui/src/control.rs` (module doc and `serve`); `apps/pdfcraft/src/main.rs` (`--control`). |
| Single-instance hand-off (Windows) | On, **local only**: a Unix-domain socket in the user's own `%LOCALAPPDATA%\PdfCraft` (or the portable data folder), so other accounts cannot reach it. It carries absolute file paths from a second launch to the first. | Launching the app with file arguments while it is running. | `apps/pdfcraft/src/single_instance.rs` (module doc). No network interface is involved. |
| Credentials | None are read from the environment, the operating system or other software. The control token is generated per run. No settings key holds a provider credential. | n/a | A search of `crates` and `apps` for environment reads shows only `PDFCRAFT_*`, `WGPU_*`, `LOCALAPPDATA` and similar; none matches key, token, secret or password. |
| Support and update addresses | **None invented.** The links and the update request still name the upstream project (`storytold`, `getartcraft.com`, the ArtCraft Discord). | n/a | `crates/engine/src/links.rs`, `apps/pdfcraft/src/updates.rs`. Replacing them needs addresses only the owner can give; see [identity migration](bdf-identity-migration.md). |

## 2. The guard added here

`cargo xtask network` fails when a workspace crate other than the desktop app depends on an HTTP or WebSocket library (`ureq`, `reqwest`, `hyper`, `tungstenite` and others, listed in `xtask/src/network.rs`), and when the allowlist names a dependency that no longer exists. It runs in the `BDF gates` job of `ci.yml` and in `cargo xtask ci`. On the current tree it reports 36 workspace crates, 0 problems. With the allowlist emptied, the live run reports the `pdfcraft` to `ureq` dependency and fails.

It is a dependency-surface check. It cannot see a socket opened with `std::net` (the two local listeners above use it), and it does not look at transitive dependencies. Adding a network feature is therefore a reviewed change to `ALLOWED`, not an accident.

## 3. What is open

* **The update check and the Help links point at the upstream project.** They are user-initiated, so the default is off, but a BDF build that ships them would send a BDF user's click to someone else's repository. They are Excluded from the desktop-core profile; whether to remove them, repoint them or keep them is the owner's decision.
* **A user-initiated request is not blocked for an agent.** `command_run` and the control channel can still reach `help.check_updates` today, because the profile is not enforced at run time (BUILD-005 to BUILD-008).
* **MCP exposes the same tool table as the CLI today**, not the desktop-core profile. The build spec's rule that an absent valid grant yields only public version and capability help is not implemented; it is BUILD-007's work. "Off by default" holds (nothing starts it); "safe when on" does not yet.
* **The web build (`pdfcraft-web`) is out of scope** here; it is not a release target.

## 4. What was not done

No traffic was captured. A runtime check would start the desktop app on a clean profile, open a document, and confirm no outbound connection appears from process start to idle; that needs a Windows packet or socket monitor and a display, and would be new evidence rather than a change to this page's findings. It is listed in [the CI contracts](bdf-ci-contracts.md) only as the missing real-UI qualification, not as a job.

## Revision history

| Date | Change |
|---|---|
| 2026-10-17 | First version (BUILD-003 deliverable 5): defaults verified from source and tests; `cargo xtask network` added. |
