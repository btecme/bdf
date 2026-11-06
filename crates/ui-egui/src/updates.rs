//! Updates in the window (BUILD-018): the green Update button, the Updates dialog, the install flow and the
//! Preferences for automatic checks and Windows integration.
//!
//! The desktop app supplies [`UpdateHost`] (it owns the network code and the updater service), so this crate has
//! none; without a host (the web build, tests) Help ▸ Check for updates opens the releases page.
//!
//! **The flow.** A check (automatic in the background when enabled, or Help ▸ Check for updates) finds a newer
//! signed release. The button appears only then, and only for a copy that can apply it. *Update* downloads and
//! verifies the whole package; *Close BDF and update* then asks the existing close flow, which asks about every
//! unsaved document (Save, Don't save, Cancel) and never saves over an original by itself; when BDF has really
//! closed, the updater starts, waits for anything still running from the folder (it never ends a process), replaces
//! the program, and starts BDF again with the documents that were open. *Later* leaves everything as it is.

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use egui::{Align, Color32, Layout};
pub use pdfcraft_update::apply::LastResult;
pub use pdfcraft_update::service::Status;

use crate::{PdfCraftApp, theme, widgets};

/// Where every BDF release is listed.
pub const RELEASES_PAGE: &str = "https://github.com/btecme/bdf/releases";

/// How often a running BDF asks again (the host also keeps its own schedule and backoff).
#[cfg(not(target_arch = "wasm32"))]
const AUTO_INTERVAL_SECS: f64 = 12.0 * 3600.0;
/// Seconds after start before the first automatic check, so opening documents is never delayed.
#[cfg(not(target_arch = "wasm32"))]
const FIRST_CHECK_DELAY: f64 = 6.0;

/// The update capabilities the desktop app provides. Calls that touch the network or the disk block, so the
/// window runs them on a thread of their own.
pub trait UpdateHost: Send + Sync {
    /// The state from what is on disk (cheap, offline).
    fn status(&self) -> Status;
    /// Ask the feed. An automatic check (`manual == false`) does nothing until one is due.
    fn check(&self, manual: bool) -> Result<Status, String>;
    /// Download and verify the release on offer; `progress(received, total)` returns `false` to cancel.
    fn download(&self, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String>;
    /// Start the updater for the verified package; BDF is about to close, and `reopen` are the files to open again.
    fn install(&self, reopen: &[PathBuf]) -> Result<(), String>;
    /// What the last update attempt reported, once.
    fn take_last_result(&self) -> Option<LastResult>;
}

/// Whether release `latest` (a tag such as `v0.2.0`) is newer than version `current` (`0.1.1`).
/// Pre-release and build suffixes are ignored; a version that doesn't parse is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok(),
        None if required => None,
        None => Some(0),
    };
    let version = (next(true)?, next(false)?, next(false)?);
    parts.next().is_none().then_some(version)
}

/// What the background work is doing.
#[derive(Default)]
pub(crate) enum Job {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Checking { rx: std::sync::mpsc::Receiver<Result<Status, String>>, manual: bool },
    #[cfg(not(target_arch = "wasm32"))]
    Downloading { rx: std::sync::mpsc::Receiver<Result<(), String>>, got: Arc<AtomicU64>, total: Arc<AtomicU64>, cancel: Arc<AtomicBool> },
}

#[derive(Default)]
pub(crate) struct Updates {
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    pub(crate) status: Option<Status>,
    pub(crate) job: Job,
    /// The last problem, shown in the dialog until the next try.
    pub(crate) error: Option<String>,
    /// A manual check finished with nothing to offer: say so in the dialog.
    pub(crate) checked_up_to_date: bool,
    /// Start-up work (first status, last result) has run.
    started: bool,
    /// When the next automatic check is due (egui time, seconds).
    next_auto: Option<f64>,
    /// The person chose to install: BDF is closing (after the unsaved-work questions) and the updater starts then.
    pub(crate) install_pending: bool,
    /// The close flow was started for the install; a cancelled close withdraws the install.
    quit_asked: bool,
}

impl Updates {
    pub(crate) fn busy(&self) -> bool {
        !matches!(self.job, Job::Idle)
    }
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: open the dialog and ask now.
    pub fn check_for_updates(&mut self) {
        if self.update_host.is_none() {
            self.open_url(RELEASES_PAGE);
            return;
        }
        self.updates.open = true;
        self.updates.error = None;
        self.updates.checked_up_to_date = false;
        #[cfg(not(target_arch = "wasm32"))]
        self.spawn_check(true);
    }

    /// The Update button is showing: a newer, verified-signed release is on offer and this copy can apply it.
    pub(crate) fn update_button_shown(&self) -> bool {
        self.updates.status.as_ref().is_some_and(|s| s.update_available.is_some() && s.managed && s.trust == "configured")
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_check(&mut self, manual: bool) {
        let Some(host) = self.update_host.clone() else { return };
        if self.updates.busy() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // The receiver may be gone (the app quit): nothing to report to then.
            let _ = tx.send(host.check(manual));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        self.updates.job = Job::Checking { rx, manual };
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_download(&mut self) {
        let Some(host) = self.update_host.clone() else { return };
        if self.updates.busy() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (got, total, cancel) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (g, t, c) = (got.clone(), total.clone(), cancel.clone());
        let ctx = self.ctx.clone();
        self.updates.error = None;
        std::thread::spawn(move || {
            let result = host.download(&mut |received, size| {
                g.store(received, Ordering::Relaxed);
                t.store(size, Ordering::Relaxed);
                if let Some(ctx) = &ctx {
                    ctx.request_repaint();
                }
                !c.load(Ordering::Relaxed)
            });
            let _ = tx.send(result);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        self.updates.job = Job::Downloading { rx, got, total, cancel };
    }

    /// Each frame: first-run work, the schedule, and finished background work.
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(host) = self.update_host.clone() else { return };
            let now = self.ctx.as_ref().map(|c| c.input(|i| i.time)).unwrap_or(0.0);
            if !self.updates.started {
                self.updates.started = true;
                self.updates.status = Some(host.status());
                if let Some(result) = host.take_last_result() {
                    self.notify(result.message);
                }
                self.updates.next_auto = self.auto_update_check.then_some(now + FIRST_CHECK_DELAY);
            }
            if let Some(due) = self.updates.next_auto
                && now >= due
            {
                self.updates.next_auto = self.auto_update_check.then_some(now + AUTO_INTERVAL_SECS);
                if self.auto_update_check {
                    self.spawn_check(false);
                }
            }
            if self.updates.next_auto.is_none() && self.auto_update_check && self.updates.started {
                self.updates.next_auto = Some(now + AUTO_INTERVAL_SECS);
            }
            // An idle window draws nothing, so wake it when the next automatic check is due.
            if let (Some(due), Some(ctx)) = (self.updates.next_auto, &self.ctx)
                && self.auto_update_check
                && due > now
            {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).clamp(0.1, 3600.0)));
            }
            match &self.updates.job {
                Job::Checking { rx, manual } => {
                    let manual = *manual;
                    let result = match rx.try_recv() {
                        Ok(r) => r,
                        Err(std::sync::mpsc::TryRecvError::Empty) => return,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update check stopped unexpectedly".into()),
                    };
                    self.updates.job = Job::Idle;
                    match result {
                        Ok(status) => {
                            self.updates.checked_up_to_date = manual && status.update_available.is_none();
                            self.updates.status = Some(status);
                            self.updates.error = None;
                        }
                        // A failed automatic check is silent: it must never interrupt work. The dialog (manual) says why.
                        Err(e) => {
                            self.updates.status = Some(host.status());
                            self.updates.error = manual.then_some(e);
                        }
                    }
                }
                Job::Downloading { rx, .. } => {
                    let result = match rx.try_recv() {
                        Ok(r) => r,
                        Err(std::sync::mpsc::TryRecvError::Empty) => return,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the download stopped unexpectedly".into()),
                    };
                    self.updates.job = Job::Idle;
                    self.updates.status = Some(host.status());
                    if let Err(e) = result
                        && e != pdfcraft_update::UpdateError::Cancelled.to_string()
                    {
                        self.updates.error = Some(e);
                    }
                }
                Job::Idle => {}
            }
            // A close that was cancelled (the unsaved-work prompt's Cancel) withdraws the install.
            if self.updates.install_pending && self.updates.quit_asked && self.close_request.is_none() && !self.allow_quit {
                self.updates.install_pending = false;
                self.updates.quit_asked = false;
                self.notify(tl!("Update postponed. Choose Update again when you are ready."));
            }
        }
    }

    /// Start closing BDF for the install; the existing close flow asks about unsaved documents first.
    fn begin_install(&mut self) {
        let Some(ctx) = self.ctx.clone() else { return };
        self.updates.install_pending = true;
        self.updates.quit_asked = true;
        self.updates.open = false;
        self.request_quit(&ctx);
    }

    /// Called as BDF is really closing: hand the verified package to the updater. `false` keeps BDF open (the
    /// updater could not be started), so a failed update never costs the person their session.
    pub(crate) fn launch_pending_update(&mut self) -> bool {
        if !self.updates.install_pending {
            return true;
        }
        self.updates.install_pending = false;
        self.updates.quit_asked = false;
        let Some(host) = self.update_host.clone() else { return true };
        let reopen: Vec<PathBuf> =
            self.views.iter().filter_map(|v| self.session.get(v.id)).filter_map(|d| d.path.clone().map(PathBuf::from)).collect();
        match host.install(&reopen) {
            Ok(()) => true,
            Err(e) => {
                self.updates.error = Some(e);
                self.updates.open = true;
                false
            }
        }
    }
}

/// The green Update button for the tab strip; returns whether it was clicked. The label says what it does and the
/// colour is never the only cue.
pub(crate) fn button(app: &PdfCraftApp, ui: &mut egui::Ui, height: f32) -> bool {
    let Some(offer) = app.updates.status.as_ref().and_then(|s| s.update_available.as_ref()) else { return false };
    let label = tl!("Update");
    let font = theme::semibold(12.5);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE).size().x);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w + 38.0, (height - 4.0).max(22.0)), egui::Sense::click());
    let tip = crate::i18n::fmt(tl!("Update to BDF {v}"), &[("v", &offer.version)]);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &tip));
    // One green for both themes: white text on it is above 4.5:1.
    let base = Color32::from_rgb(0x1f, 0x7a, 0x3d);
    let fill = if resp.is_pointer_button_down_on() {
        base.lerp_to_gamma(Color32::BLACK, 0.2)
    } else if resp.hovered() {
        base.lerp_to_gamma(Color32::WHITE, 0.12)
    } else {
        base
    };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(5), fill);
    crate::icons::paint(
        ui,
        egui::Rect::from_min_size(rect.min + egui::vec2(8.0, (rect.height() - 16.0) / 2.0), egui::vec2(16.0, 16.0)),
        "cloud",
        15.0,
        Color32::WHITE,
    );
    ui.painter().text(rect.left_center() + egui::vec2(28.0, 0.0), egui::Align2::LEFT_CENTER, label, font, Color32::WHITE);
    theme::hand(resp.on_hover_text(tip)).clicked()
}

/// The width the strip keeps free for the "b-tec Built" mark and, when it is showing, the Update button.
pub(crate) fn reserved_width(app: &PdfCraftApp, ui: &mut egui::Ui) -> f32 {
    let t = theme::Tokens::get(ui.ctx());
    let measure = |text: &str, font: egui::FontId| ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, t.text).size().x);
    let brand = measure(tl!("b-tec Built"), theme::medium(12.0)) + 8.0 + 4.0;
    let update = if app.update_button_shown() { measure(tl!("Update"), theme::semibold(12.5)) + 38.0 + 4.0 } else { 0.0 };
    brand + update
}

/// The "b-tec Built" mark beside it, linking to the BDF page on b-tec's site. Returns whether it was clicked.
pub(crate) fn brand_link(ui: &mut egui::Ui) -> bool {
    let t = theme::Tokens::get(ui.ctx());
    let text = tl!("b-tec Built");
    let font = theme::medium(12.0);
    let w = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), t.text_muted).size().x);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w + 8.0, 22.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), text));
    let color = if resp.hovered() { t.accent } else { t.text_muted };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, font, color);
    if resp.hovered() {
        ui.painter().hline(rect.x_range().shrink(4.0), rect.bottom() - 3.0, egui::Stroke::new(1.0, color));
    }
    theme::hand(resp.on_hover_text(pdfcraft_engine::links::BTEC_BDF)).clicked()
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let mut close = false;
    let mut action = Action::None;
    let modal = crate::widgets::show_modal(ctx, "updates", |ui| {
        ui.set_width(440.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("cloud", 22.0, t.accent));
            ui.label(egui::RichText::new(tl!("Check for updates")).font(theme::semibold(16.0)));
        });
        ui.add_space(8.0);
        action = body(app, ui, &t, current);
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(tl!("Updates come only from BDF's own signed releases. Nothing about you or your documents is sent."))
                .color(t.text_muted)
                .small(),
        );
    });
    if modal.should_close() {
        close = true;
    }
    match action {
        Action::None => {}
        Action::Close => close = true,
        #[cfg(not(target_arch = "wasm32"))]
        Action::Download => app.spawn_download(),
        #[cfg(not(target_arch = "wasm32"))]
        Action::CancelDownload => {
            if let Job::Downloading { cancel, .. } = &app.updates.job {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        Action::Install => app.begin_install(),
        Action::Releases => {
            app.open_url(RELEASES_PAGE);
            close = true;
        }
        #[cfg(target_arch = "wasm32")]
        _ => {}
    }
    if close {
        app.updates.open = false;
    }
}

#[derive(PartialEq, Eq)]
enum Action {
    None,
    Close,
    Download,
    CancelDownload,
    Install,
    Releases,
}

fn body(app: &PdfCraftApp, ui: &mut egui::Ui, t: &theme::Tokens, current: &str) -> Action {
    let mut action = Action::None;
    match &app.updates.job {
        #[cfg(not(target_arch = "wasm32"))]
        Job::Checking { .. } => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tl!("Checking for a newer version…"));
            });
            return action;
        }
        #[cfg(not(target_arch = "wasm32"))]
        Job::Downloading { got, total, .. } => {
            let (got, total) = (got.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
            let fraction = if total > 0 { got as f32 / total as f32 } else { 0.0 };
            ui.label(tl!("Downloading and checking the update…"));
            ui.add(egui::ProgressBar::new(fraction).text(format!("{:.1} / {:.1} MB", got as f64 / 1e6, total as f64 / 1e6)));
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                    action = Action::CancelDownload;
                }
            });
            return action;
        }
        Job::Idle => {}
    }
    let status = app.updates.status.as_ref();
    if let Some(s) = status
        && s.dev_build
    {
        ui.label(egui::RichText::new(tl!("Development build: updates come from a test feed.")).color(t.text_muted).small());
    }
    if let Some(error) = &app.updates.error {
        ui.label(crate::i18n::fmt(tl!("Couldn't update: {e}"), &[("e", error)]));
        ui.add_space(6.0);
    }
    match status.and_then(|s| s.update_available.as_ref().map(|o| (s, o))) {
        Some((s, offer)) => {
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("BDF {v} is available."), &[("v", &offer.version)])).strong());
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("You have version {c}."), &[("c", current)])).color(t.text_muted));
            if !offer.notes.is_empty() {
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                    ui.label(&offer.notes);
                });
            }
            ui.add_space(10.0);
            if !s.managed {
                ui.label(egui::RichText::new(s.managed_note.clone().unwrap_or_default()).color(t.text_muted));
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::pill_button(ui, tl!("Releases page"), true).clicked() {
                        action = Action::Releases;
                    }
                    if widgets::pill_button(ui, tl!("Later"), false).clicked() {
                        action = Action::Close;
                    }
                });
            } else if s.ready_to_install {
                ui.label(
                    egui::RichText::new(tl!(
                        "BDF will ask about any unsaved work, close, update, and reopen your documents. Your settings are kept."
                    ))
                    .color(t.text_muted),
                );
                ui.add_space(8.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::pill_button(ui, tl!("Close BDF and update"), true).clicked() {
                        action = Action::Install;
                    }
                    if widgets::pill_button(ui, tl!("Later"), false).clicked() {
                        action = Action::Close;
                    }
                });
            } else {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::pill_button(ui, tl!("Download update"), true).clicked() {
                        action = Action::Download;
                    }
                    if widgets::pill_button(ui, tl!("Later"), false).clicked() {
                        action = Action::Close;
                    }
                });
            }
        }
        None => {
            if status.is_some_and(|s| s.trust != "configured") {
                ui.label(tl!(
                    "This build cannot verify updates, so it will not install any. Get the newest portable package from the releases page."
                ));
            } else if app.updates.checked_up_to_date || app.updates.error.is_none() {
                ui.label(crate::i18n::fmt(tl!("BDF {c} is up to date."), &[("c", current)]));
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::pill_button(ui, tl!("Close"), true).clicked() {
                    action = Action::Close;
                }
            });
        }
    }
    action
}

/// Preferences: automatic checks, and the optional Windows integration.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn preferences(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let t = theme::Tokens::get(ui.ctx());
    if app.update_host.is_some() {
        ui.add(widgets::checkbox(&mut app.auto_update_check, tl!("Check for updates automatically")));
        ui.label(
            egui::RichText::new(tl!("Asks BDF's release feed in the background. A failed check never interrupts your work."))
                .small()
                .color(t.text_muted),
        );
    }
    if cfg!(windows) {
        windows_preferences(app, ui, &t);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn windows_preferences(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &theme::Tokens) {
    use pdfcraft_platform::windows_integration as win;
    let Ok(exe) = std::env::current_exe() else { return };
    let status = win::status(&exe);
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Windows integration")).font(theme::semibold(13.0)));
    ui.label(egui::RichText::new(status.summary()).small().color(t.text_muted));
    let mut result: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        let label = if status.registered && !status.this_copy { tl!("Refresh Windows integration") } else { tl!("Make BDF my default PDF app") };
        if widgets::pill_button(ui, label, true).clicked() {
            result = Some(match win::register(&exe) {
                Ok(_) => match win::open_default_apps() {
                    Ok(()) => tl!("BDF is registered. Choose it in Windows' Default apps to make it the default.").to_string(),
                    Err(_) => tl!("BDF is registered. Choose it in Windows' Default apps to make it the default.").to_string(),
                },
                Err(e) => e.to_string(),
            });
        }
        if status.registered && widgets::pill_button(ui, tl!("Remove Windows integration"), false).clicked() {
            result = Some(match win::unregister(&exe) {
                Ok(_) => tl!("Windows integration removed. Other apps' settings were not touched.").to_string(),
                Err(e) => e.to_string(),
            });
        }
    });
    if let Some(message) = result {
        app.notify(message);
    }
}
