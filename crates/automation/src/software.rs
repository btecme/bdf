//! The software tools: update status, check, download, apply and rollback, and Windows integration (BUILD-018).
//!
//! These act on the installed copy of BDF and the Windows account, not on documents, so they have their own
//! authorisation, separate from everything a document grant or `--root` confers:
//!
//! * `update_status` and `windows_integration_status` read state and change nothing: always available.
//! * every other tool needs the software authorisation (`--allow-software`, or
//!   [`Automation::with_software_changes`]). Without it the call is refused with a message that says how to
//!   permit it, and the MCP server does not list the tool.
//! * the grant-limited server (`mcp --client`) has a fixed tool list that contains none of them.
//!
//! The tools and the app's buttons call the same code: [`pdfcraft_update::service::Updater`] and
//! [`pdfcraft_platform::windows_integration`].

use serde_json::{Value, json};

use crate::{Args, Automation, Result, ToolError, failed};

/// What the software tools act on: the updater for this copy and the program Windows should open PDFs with.
/// A test supplies its own; otherwise it is made from the running executable's folder.
#[cfg(not(target_arch = "wasm32"))]
pub struct SoftwareEnv {
    pub updater: pdfcraft_update::service::Updater,
    /// `bdf.exe` beside `bdf-cli.exe`: what a registration points at.
    pub gui_exe: std::path::PathBuf,
}

/// The refusal for a software-changing tool called without the authorisation.
pub const NOT_AUTHORISED: &str = "not authorised: this tool changes the installed BDF or this Windows account, which no document grant or --root allows. \
Start the command or server with --allow-software to permit it (update_status and windows_integration_status are always available)";

impl Automation {
    /// Permit the tools that change the installed software or the Windows account (updates, rollback, Windows
    /// registration). Off by default; a document grant never turns it on.
    pub fn with_software_changes(mut self, allowed: bool) -> Self {
        self.software_allowed = allowed;
        self
    }

    /// Whether those tools are permitted.
    pub fn software_changes_allowed(&self) -> bool {
        self.software_allowed
    }

    /// Act on this environment instead of the running executable's (tests).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_software_env(mut self, env: SoftwareEnv) -> Self {
        self.software_env = Some(env);
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn software(&mut self) -> Result<&SoftwareEnv> {
        if self.software_env.is_none() {
            let updater = pdfcraft_update::service::Updater::for_current_exe().map_err(failed)?;
            let gui_exe = updater.app_dir.join(if cfg!(windows) { "bdf.exe" } else { "bdf" });
            self.software_env = Some(SoftwareEnv { updater, gui_exe });
        }
        self.software_env.as_ref().ok_or_else(|| failed("no software environment"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn software_call(&mut self, name: &str, a: &Args) -> Result<Value> {
        use pdfcraft_platform::windows_integration as win;
        let env = self.software()?;
        match name {
            "update_status" => status_json(&env.updater.status()),
            "update_check" => {
                let manual = a.opt_bool("manual")?.unwrap_or(true);
                let status = env.updater.check(manual).map_err(failed)?;
                Ok(json!({ "checked": true, "status": status_json(&status)? }))
            }
            "update_download" => {
                let prepared = env.updater.prepare(&mut |_, _| true).map_err(failed)?;
                Ok(json!({ "ready": true, "version": prepared.version.to_string(), "status": status_json(&env.updater.status())? }))
            }
            "update_apply" => {
                env.updater.launch_apply(&[]).map_err(failed)?;
                Ok(json!({
                    "scheduled": true,
                    "note": "The updater installs the update once BDF, the command line and the worker have closed, then starts BDF again. Nothing is ended to make room; the result is shown the next time BDF starts."
                }))
            }
            "update_rollback" => {
                env.updater.launch_rollback(&[]).map_err(failed)?;
                Ok(json!({ "scheduled": true, "note": "The updater returns to the previous version once BDF and the command line have closed." }))
            }
            "windows_integration_status" => {
                let s = win::status(&env.gui_exe);
                Ok(integration_json(&s))
            }
            "windows_integration_register" => {
                let done = win::register(&env.gui_exe).map_err(failed)?;
                let opened = a.opt_bool("open_settings")?.unwrap_or(false) && win::open_default_apps().is_ok();
                Ok(json!({
                    "registered": true,
                    "replaced": done.replaced,
                    "default_changed": false,
                    "settings_opened": opened,
                    "note": "BDF is now offered in Open with and Default apps for this Windows account. BDF does not make itself the default: choose it in Windows' Default apps.",
                    "status": integration_json(&win::status(&env.gui_exe)),
                }))
            }
            "windows_integration_remove" => {
                let done = win::unregister(&env.gui_exe).map_err(failed)?;
                Ok(
                    json!({ "removed": !done.nothing_to_remove, "nothing_to_remove": done.nothing_to_remove, "status": integration_json(&win::status(&env.gui_exe)) }),
                )
            }
            "windows_open_default_apps" => {
                win::open_default_apps().map_err(|e| failed(format!("couldn't open Windows Settings: {e}")))?;
                Ok(json!({ "opened": true, "note": "Choose BDF there to make it the default; nothing was changed." }))
            }
            other => Err(ToolError::UnknownTool(other.into())),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn software_call(&mut self, name: &str, _a: &Args) -> Result<Value> {
        let _ = name;
        Err(failed("updates and Windows integration are not available in the browser"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn status_json(status: &pdfcraft_update::service::Status) -> Result<Value> {
    serde_json::to_value(status).map_err(failed)
}

#[cfg(not(target_arch = "wasm32"))]
fn integration_json(s: &pdfcraft_platform::windows_integration::Status) -> Value {
    json!({
        "supported": s.supported,
        "registered": s.registered,
        "this_copy": s.this_copy,
        "registered_exe": s.registered_exe,
        "stale": s.stale,
        "is_default": s.is_default,
        "default_progid": s.default_progid,
        "summary": s.summary(),
    })
}
