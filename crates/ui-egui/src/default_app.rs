//! "Make BDF the default PDF app" (BUILD-018): the shortcut at the bottom right of the window, and the action that
//! Preferences ▸ Windows integration shares with it.
//!
//! **What it does.** The shortcut shows on Windows while Windows has not confirmed BDF as the default PDF app. Clicking it
//! runs the same two steps as the Preferences button: register this copy for the person (per user, nothing else), then open
//! the Settings page where only the person can make the choice. Opening Settings is not success: the shortcut goes only
//! when Windows itself reports BDF as the default, which is read again when BDF regains focus after the Settings page, and
//! after anything BDF itself changed. A cancelled choice, or a registration that failed, leaves it where it was.
//!
//! **What it costs.** The registry is read at start, when the window regains focus, and after a choice or a removal, never
//! per frame; every frame draws from the cached answer. The desktop app supplies [`DefaultAppHost`] (it owns the registry
//! code), so the web build and the tests never touch the real registry.

use std::sync::Arc;

use pdfcraft_platform::windows_integration as win;

use crate::PdfCraftApp;

/// What the person's click did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// This copy is registered and Windows' Default apps page is open for the person to choose BDF.
    SettingsOpened,
    /// This copy is registered, but the Settings page could not be opened (the reason is Windows' own).
    SettingsFailed(String),
}

/// What the desktop app provides for the default-app shortcut and Preferences ▸ Windows integration. Calls read or write
/// the registry, so the window makes them on events, not per frame.
pub trait DefaultAppHost: Send + Sync {
    /// Where the registration and the default stand now (read from Windows; never written).
    fn status(&self) -> win::Status;
    /// Register this copy for the person, then open Windows' Default apps page. `Err` is what to tell them.
    fn choose(&self) -> Result<Chosen, String>;
    /// Take this copy's registration out again. `Ok` is what to tell them.
    fn remove(&self) -> Result<(), String>;
}

/// The real thing: this program, this user's registry.
struct System;

impl DefaultAppHost for System {
    fn status(&self) -> win::Status {
        match std::env::current_exe() {
            Ok(exe) => win::status(&exe),
            Err(_) => win::Status::default(),
        }
    }

    fn choose(&self) -> Result<Chosen, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        win::register(&exe).map_err(|e| e.to_string())?;
        Ok(match win::open_default_apps() {
            Ok(()) => Chosen::SettingsOpened,
            Err(e) => Chosen::SettingsFailed(e.to_string()),
        })
    }

    fn remove(&self) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        win::unregister(&exe).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// The host for this program on this system, for the desktop app to set. Off Windows its status says unsupported, so
/// nothing shows.
pub fn system_host() -> Arc<dyn DefaultAppHost> {
    Arc::new(System)
}

/// The cached answer and what makes it old.
#[derive(Default)]
pub(crate) struct DefaultApp {
    status: Option<win::Status>,
    /// Read again at the next frame (something BDF did changed the registration).
    stale: bool,
    /// The window had the focus at the last frame.
    focused: bool,
}

impl DefaultApp {
    pub(crate) fn status(&self) -> Option<&win::Status> {
        self.status.as_ref().filter(|s| s.supported)
    }
}

impl PdfCraftApp {
    /// Each frame: read the status again when there is a reason to (first frame, the window regained the focus, a choice or
    /// a removal), and otherwise keep the cached one.
    pub(crate) fn poll_default_app(&mut self) {
        let Some(host) = self.default_app_host.clone() else { return };
        let Some(ctx) = self.ctx.clone() else { return };
        let focused = ctx.input(|i| i.focused);
        let regained = focused && !self.default_app.focused;
        self.default_app.focused = focused;
        if self.default_app.status.is_none() || self.default_app.stale || regained {
            self.default_app.stale = false;
            self.default_app.status = Some(host.status());
        }
    }

    /// The shortcut is showing: Windows integration exists here and Windows has not confirmed BDF as the default.
    pub(crate) fn default_app_shortcut_shown(&self) -> bool {
        self.default_app_host.is_some() && self.default_app.status().is_some_and(|s| !s.bdf_is_default())
    }

    /// The click, from the shortcut or from Preferences: register, open Settings, say what happened. The status is read
    /// again next frame; the shortcut stays until Windows reports BDF as the default.
    pub(crate) fn make_default_pdf_app(&mut self) {
        let Some(host) = self.default_app_host.clone() else { return };
        let message = match host.choose() {
            Ok(Chosen::SettingsOpened) => tl!("BDF is registered. Choose it in Windows' Default apps to make it the default.").to_string(),
            Ok(Chosen::SettingsFailed(why)) => crate::i18n::fmt(tl!("BDF is registered, but Windows Settings did not open: {why}"), &[("why", &why)]),
            Err(why) => why,
        };
        self.default_app.stale = true;
        self.notify(message);
    }

    /// Preferences ▸ Remove Windows integration (the desktop app's Preferences only).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn remove_windows_integration(&mut self) {
        let Some(host) = self.default_app_host.clone() else { return };
        let message = match host.remove() {
            Ok(()) => tl!("Windows integration removed. Other apps' settings were not touched.").to_string(),
            Err(why) => why,
        };
        self.default_app.stale = true;
        self.notify(message);
    }
}
