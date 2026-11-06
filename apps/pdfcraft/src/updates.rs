//! The desktop app's side of updates (BUILD-018): the [`UpdateHost`] the window uses, over the shared updater
//! service. The window never touches the network; this is the only place that does, through the signed release
//! feed in `pdfcraft-update` (HTTPS only, the operating system's certificates, every answer verified).

use std::path::PathBuf;
use std::sync::Arc;

use pdfcraft_ui_egui::updates::{LastResult, Status, UpdateHost};
use pdfcraft_update::service::Updater;

struct Host {
    updater: Updater,
}

impl UpdateHost for Host {
    fn status(&self) -> Status {
        self.updater.status()
    }

    fn check(&self, manual: bool) -> Result<Status, String> {
        self.updater.check(manual).map_err(|e| e.to_string())
    }

    fn download(&self, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
        self.updater.prepare(progress).map(|_| ()).map_err(|e| e.to_string())
    }

    fn install(&self, reopen: &[PathBuf]) -> Result<(), String> {
        self.updater.launch_apply(reopen).map_err(|e| e.to_string())
    }

    fn take_last_result(&self) -> Option<LastResult> {
        self.updater.take_last_result()
    }
}

/// The host for this running copy, or `None` when its folder cannot be determined.
pub fn host() -> Option<Arc<dyn UpdateHost>> {
    Updater::for_current_exe().ok().map(|updater| Arc::new(Host { updater }) as Arc<dyn UpdateHost>)
}

/// Undo an update that was interrupted (a power cut, a killed updater), before anything else runs: the old launcher
/// is the first file put back. Cheap when nothing was interrupted: one file check, nothing created.
pub fn recover_interrupted() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    match pdfcraft_update::apply::recover(dir) {
        Ok(pdfcraft_update::apply::Recovery::Nothing | pdfcraft_update::apply::Recovery::Busy) => {}
        Ok(other) => log::warn!("an interrupted update was dealt with: {other:?}"),
        Err(e) => log::error!("couldn't undo an interrupted update: {e}"),
    }
}
