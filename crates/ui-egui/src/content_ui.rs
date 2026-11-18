//! Edit a PDF ▸ Add content: the Text tool (click on the page and type), Image (pick a file;
//! it lands in the middle of the page), and editing what was added: select, move, resize
//! (images keep their proportions), double-click text to retype it, format it from the panel,
//! Delete to remove it. Each change is one undoable engine edit.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, Vec2};
use pdfcraft_engine::{Added, AddedContent, AddedText, Edit, FontFamily, TextAlign};
use pdfcraft_render::DocInfo;

use crate::canvas::{DocView, PageXform};
use crate::theme::Tokens;
use crate::widgets;

/// The colour of the warning under text the standard fonts can't draw.
const UNDRAWABLE: Color32 = Color32::from_rgb(0xD7, 0x37, 0x3F);

/// The first character of a draft that can't be drawn, as the engine decides it (it refuses
/// such text, #125): outside the standard fonts and not shaped with the Arabic face.
pub(crate) fn undrawable(d: &TextDraft, memo: &UndrawableMemo) -> Option<char> {
    // Checked as `finish` will commit it: without trailing whitespace.
    let t = AddedText { text: d.text.trim_end().to_string(), rect: d.rect, ..d.style.clone() };
    if let Some((key, c)) = memo.borrow().as_ref()
        && *key == t
    {
        return *c;
    }
    let c = pdfcraft_engine::first_undrawable(&t);
    *memo.borrow_mut() = Some((t, c));
    c
}

/// The last draft [`undrawable`] checked and its answer: the check lays the text out (and shapes
/// Arabic), too slow to repeat every frame for long text, so it reruns only when the text or
/// anything that changes its layout does.
pub(crate) type UndrawableMemo = std::cell::RefCell<Option<(AddedText, Option<char>)>>;

/// The warning shown for text with `c` in it.
pub(crate) fn undrawable_message(c: char) -> String {
    crate::i18n::fmt(
        tl!("The PDF text fonts can't draw “{c}” ({code}). Choose supported text."),
        &[("c", &c.to_string()), ("code", &format!("U+{:04X}", u32::from(c)))],
    )
}

/// The warning after an edit that saved text its font can't draw: BDF drew `?` for it (a form
/// field, which other viewers redraw from its value, or a comment, where the `?` shows everywhere).
pub(crate) fn undrawn_message(n: &pdfcraft_engine::TextNotice) -> String {
    let template = if n.redrawn_elsewhere {
        tl!(
            "This field's font can't draw “{c}” ({code}), so BDF shows ? in its place. The value is saved, and other PDF viewers redraw the field from it."
        )
    } else {
        tl!("The fonts BDF writes with can't draw “{c}” ({code}), so it shows as ? in every PDF viewer. The text is saved.")
    };
    crate::i18n::fmt(template, &[("c", &n.ch.to_string()), ("code", &format!("U+{:04X}", u32::from(n.ch)))])
}

const SELECT_BLUE: Color32 = Color32::from_rgb(0x14, 0x73, 0xE6);

/// The text being typed or retyped in place.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDraft {
    pub page: usize,
    /// `None` for new text; the item index for retyping.
    pub index: Option<usize>,
    /// Display space.
    pub rect: [f64; 4],
    pub text: String,
    pub style: AddedText,
    pub focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grab {
    Move,
    /// Which corner: (left, top) in screen terms.
    Corner(bool, bool),
    /// Turning the selected text with its rotate grip: the box centre on screen, the pointer's angle around it when
    /// the turn began (degrees, counter-clockwise) and the item's rotation then.
    Rotate {
        center: Pos2,
        start: f32,
        base: f64,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentView {
    /// The selected added item: (page, index among the page's items).
    pub selected: Option<(usize, usize)>,
    pub draft: Option<TextDraft>,
    /// Select the item an add creates (the page's item count before it).
    pub select_added: Option<(usize, usize)>,
    grab: Option<(usize, Grab, Pos2)>,
    undrawable: UndrawableMemo,
    /// Where the page's context menu was last opened in Edit mode: the page and the display-space point.
    pub menu_at: Option<(usize, [f64; 2])>,
}

impl ContentView {
    /// The draft's first character the standard fonts can't draw. Such a draft is kept open, and
    /// counts as unsaved work, until it is corrected or discarded.
    pub(crate) fn blocked(&self) -> Option<char> {
        self.draft.as_ref().and_then(|d| undrawable(d, &self.undrawable))
    }

    /// Keeps a blocked draft open and focused instead of finishing or replacing it.
    pub(crate) fn hold_blocked(&mut self) -> bool {
        let blocked = self.blocked().is_some();
        if let Some(d) = self.draft.as_mut().filter(|_| blocked) {
            d.focus = true;
        }
        blocked
    }
}

/// While an added item is moved, the page without it, rendered once per gesture: the item's own
/// pixels follow the pointer over it instead of staying put until release (#1488).
#[derive(Default)]
pub(crate) struct MoveBackground {
    key: Option<(u64, usize, usize)>,
    renderer: Option<pdfcraft_render::RenderPool>,
    texture: Option<egui::TextureHandle>,
    tag: Option<u64>,
    /// The box (display space) that holds the item's pixels when the gesture started: for turned text, the bounds of
    /// the turned box.
    from: Option<[f64; 4]>,
    /// The item's rotation when the gesture started.
    from_rotation: f64,
    /// The applied change's bounds and the turn it made, in degrees counter-clockwise. The page raster on screen still
    /// has the item where it was until its replacement arrives, so the preview stays up until then.
    settling: Option<([f64; 4], f32)>,
}

impl MoveBackground {
    pub fn pending(&self) -> bool {
        self.renderer.is_some() && self.texture.is_none()
    }

    pub fn prepare(&mut self, ctx: &egui::Context, doc: &pdfcraft_engine::Document, cv: &ContentView, scale: f32) {
        let key = match (cv.grab, cv.selected) {
            (Some((gp, Grab::Move | Grab::Rotate { .. }, _)), Some((sp, si))) if gp == sp => Some((doc.edit_generation(), sp, si)),
            _ => None,
        };
        if !(key.is_none() && self.settling.is_some()) && self.key != key {
            *self = Self { key, ..Default::default() };
            if let Some((_, page, index)) = key {
                self.renderer = doc.added_content_preview(page, index).ok().map(|p| p.renderer());
                let item = on_page(&doc.added, page).into_iter().find(|(i, _)| *i == index).map(|(_, a)| &a.content);
                self.from = item.map(content_bounds);
                self.from_rotation = item.map_or(0.0, item_rotation);
            }
        }
        if self.settling.is_some() {
            return;
        }
        let (Some((_, page, _)), Some(renderer)) = (self.key, self.renderer.as_ref()) else { return };
        let Some(p) = doc.info.pages.get(page) else { return };
        let max_side = ctx.input(|i| i.max_texture_side) as f32;
        let scale = scale.min(crate::canvas::BASE_SIDE.min(max_side) / p.width.max(p.height).max(1.0));
        let tag = (scale * 1000.0) as u64;
        if self.tag != Some(tag) {
            renderer.set_queue(vec![pdfcraft_render::RenderRequest { page, scale, tag, ..Default::default() }]);
            self.tag = Some(tag);
        }
        if let Some(out) = renderer.try_recv() {
            if out.error.is_some() {
                // Without a background the item stays put while dragged, as before.
                self.renderer = None;
                return;
            }
            if out.request.tag == tag {
                self.texture = Some(ctx.load_texture(
                    "added-content-move-background",
                    crate::canvas::texture_image([out.width as usize, out.height as usize], out.rgba),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        if self.pending() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// The move was applied: keep the preview, now at the new generation, until the page is
    /// rendered again. Any other edit ends it.
    pub fn committed(&mut self, edit: &Edit, generation: u64) {
        match edit {
            Edit::UpdateContent { page, index, content } if self.texture.is_some() && self.key.is_some_and(|(_, p, i)| (p, i) == (*page, *index)) => {
                self.key = Some((generation, *page, *index));
                self.settling = Some((content_bounds(content), signed_turn(self.from_rotation, item_rotation(content))));
            }
            _ => *self = Self::default(),
        }
    }

    pub fn page_received(&mut self, page: usize) {
        if self.settling.is_some() && self.key.is_some_and(|(_, p, _)| p == page) {
            *self = Self::default();
        }
    }

    /// The page without the moving item, then the item's pixels taken from `page_tex` (the page
    /// as rendered, item included where it was) at the dragged or applied position.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        xf: &PageXform,
        page: usize,
        info: &DocInfo,
        cv: &ContentView,
        page_tex: Option<egui::TextureId>,
    ) {
        let (Some((_, kp, _)), Some(background), Some(page_tex), Some(from)) = (self.key, &self.texture, page_tex, self.from) else { return };
        if kp != page {
            return;
        }
        let r = screen_rect(xf, info, page, from);
        // How far the item's pixels are carried, and how far they are turned about the box's centre.
        let (d, turn) = if let Some((_, grab, start)) = cv.grab {
            let Some(p) = ui.input(|i| i.pointer.hover_pos()) else { return };
            match grab {
                Grab::Move => (p - start, 0.0),
                Grab::Rotate { center, start: from_angle, base } => {
                    let now = turned(base, from_angle, pointer_angle(center, p), ui.input(|i| i.modifiers.shift));
                    (Vec2::ZERO, signed_turn(base, now))
                }
                Grab::Corner(..) => return,
            }
        } else if let Some((to, turn)) = self.settling {
            // A moved text box keeps its top edge (its height follows its lines).
            (screen_rect(xf, info, page, to).center() - r.center(), turn)
        } else {
            return;
        };
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
        xf.paint_image(&painter, background.id(), 0.0, 0.0, 1.0, 1.0);
        let r = r.expand(1.0);
        let mut mesh = egui::Mesh::with_texture(page_tex);
        for c in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
            let (u, v) = xf.screen_to_norm(c);
            mesh.vertices.push(egui::epaint::Vertex { pos: rot_about(c, r.center(), turn) + d, uv: Pos2::new(u, v), color: Color32::WHITE });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Text style for new text (the panel's Format controls set it).
pub fn default_style() -> AddedText {
    AddedText { size: 12.0, ..AddedText::default() }
}

/// Display-space rect → screen.
fn screen_rect(xf: &PageXform, info: &DocInfo, page: usize, r: [f64; 4]) -> Rect {
    let ph = info.pages[page].height as f64;
    xf.view_rect([r[0] as f32, (ph - r[3]) as f32, r[2] as f32, (ph - r[1]) as f32])
}

/// Screen point → display space.
fn to_display(xf: &PageXform, info: &DocInfo, page: usize, p: Pos2) -> [f64; 2] {
    let (vx, vy) = xf.screen_to_view(p);
    [vx as f64, info.pages[page].height as f64 - vy as f64]
}

fn on_page(added: &[Added], page: usize) -> Vec<(usize, &Added)> {
    added.iter().filter(|a| a.page == page).enumerate().collect()
}

/// How far above a selected text box its rotate grip sits, and how big it is, in screen points.
const GRIP_OFFSET: f32 = 26.0;
const GRIP_RADIUS: f32 = 7.0;

/// `v` turned `degrees` counter-clockwise as it looks on screen (screen y points down).
fn rot_vec(v: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    egui::vec2(v.x * cos + v.y * sin, -v.x * sin + v.y * cos)
}

/// `p` turned about `center`.
fn rot_about(p: Pos2, center: Pos2, degrees: f32) -> Pos2 {
    center + rot_vec(p - center, degrees)
}

fn handles(r: Rect, degrees: f32) -> [(Pos2, bool, bool); 4] {
    [(r.left_top(), true, true), (r.right_top(), false, true), (r.left_bottom(), true, false), (r.right_bottom(), false, false)]
        .map(|(c, left, top)| (rot_about(c, r.center(), degrees), left, top))
}

/// The box's outline, turned, in drawing order.
fn outline(r: Rect, degrees: f32) -> Vec<Pos2> {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()].map(|c| rot_about(c, r.center(), degrees)).to_vec()
}

/// Where the rotate grip of a text box on screen is.
fn grip(r: Rect, degrees: f32) -> Pos2 {
    rot_about(egui::pos2(r.center().x, r.min.y - GRIP_OFFSET), r.center(), degrees)
}

/// The angle of `p` around `center` in degrees, counter-clockwise from the right.
fn pointer_angle(center: Pos2, p: Pos2) -> f32 {
    let d = p - center;
    (-d.y).atan2(d.x).to_degrees()
}

/// The rotation a turn gives an item: `base` plus how far the pointer has gone round. Within 3° of a right angle it
/// snaps to it, and Shift snaps to every 15°. Always a turn in [0, 360).
fn turned(base: f64, start: f32, now: f32, fine_steps: bool) -> f64 {
    let raw = base + f64::from(now - start);
    let snapped = if fine_steps {
        (raw / 15.0).round() * 15.0
    } else {
        let right = (raw / 90.0).round() * 90.0;
        if (raw - right).abs() < 3.0 { right } else { raw }
    };
    pdfcraft_engine::normalize_rotation((snapped * 10.0).round() / 10.0)
}

/// The shortest signed turn from one rotation to another, in degrees.
fn signed_turn(from: f64, to: f64) -> f32 {
    let d = (to - from).rem_euclid(360.0);
    (if d > 180.0 { d - 360.0 } else { d }) as f32
}

/// How far an item is turned (only text can be), as a turn in [0, 360).
fn item_rotation(c: &AddedContent) -> f64 {
    match c {
        AddedContent::Text(t) => pdfcraft_engine::normalize_rotation(t.rotation),
        AddedContent::Image(_) => 0.0,
    }
}

/// The smallest upright box (display space) that holds `rect` turned `degrees` about its centre.
fn bounds(rect: [f64; 4], degrees: f64) -> [f64; 4] {
    let (x0, x1) = (rect[0].min(rect[2]), rect[0].max(rect[2]));
    let (y0, y1) = (rect[1].min(rect[3]), rect[1].max(rect[3]));
    let (cx, cy, hw, hh) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (ex, ey) = (hw * cos.abs() + hh * sin.abs(), hw * sin.abs() + hh * cos.abs());
    [cx - ex, cy - ey, cx + ex, cy + ey]
}

fn content_bounds(c: &AddedContent) -> [f64; 4] {
    bounds(c.rect(), item_rotation(c))
}

/// Whether the screen point `p` is on an item's box, turned as the item is, give or take `slack` points.
fn on_box(r: Rect, degrees: f32, p: Pos2, slack: f32) -> bool {
    r.expand(slack).contains(rot_about(p, r.center(), -degrees))
}

/// The on-screen box and rotation of an added item.
fn item_box(xf: &PageXform, info: &DocInfo, page: usize, a: &Added) -> (Rect, f32) {
    (screen_rect(xf, info, page, a.content.rect()), item_rotation(&a.content) as f32)
}

/// The unturned box of a turned item after its corner is dragged: the corner opposite the one dragged stays where it is
/// on screen, whatever the turn, and the box stays as turned.
fn dragged_turned(r: Rect, degrees: f32, grab: Grab, d: Vec2, keep_aspect: bool) -> Rect {
    let Grab::Corner(left, top) = grab else { return dragged(r, grab, d, keep_aspect) };
    if degrees == 0.0 {
        return dragged(r, grab, d, keep_aspect);
    }
    // The pointer's movement in the box's own frame.
    let local = dragged(r, grab, rot_vec(d, -degrees), keep_aspect);
    let fixed = egui::pos2(if left { r.max.x } else { r.min.x }, if top { r.max.y } else { r.min.y });
    let anchor = rot_about(fixed, r.center(), degrees);
    let centre = anchor - rot_vec(fixed - local.center(), degrees);
    local.translate(centre - local.center())
}

/// The rect while moving or resizing (images keep their proportions).
fn dragged(r: Rect, grab: Grab, d: egui::Vec2, keep_aspect: bool) -> Rect {
    match grab {
        Grab::Move => r.translate(d),
        Grab::Rotate { .. } => r,
        Grab::Corner(left, top) => {
            let (mut dx, mut dy) = (if left { -d.x } else { d.x }, if top { -d.y } else { d.y });
            if keep_aspect && r.width() > 0.0 && r.height() > 0.0 {
                let k = ((r.width() + dx) / r.width()).max((r.height() + dy) / r.height()).max(0.05);
                dx = r.width() * k - r.width();
                dy = r.height() * k - r.height();
            }
            let mut out = r;
            if left {
                out.min.x -= dx;
            } else {
                out.max.x += dx;
            }
            if top {
                out.min.y -= dy;
            } else {
                out.max.y += dy;
            }
            Rect::from_two_pos(out.min, out.max)
        }
    }
}

/// Pointer input on one page in Edit mode (`adding_text` with the Text tool). `true` when it
/// used the input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    added: &[Added],
    adding_text: bool,
    style: &AddedText,
    view: &mut DocView,
) -> bool {
    let mut consumed = false;
    let pointer = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
    let cv = &mut view.content;
    if let Some((gp, grab, start)) = cv.grab
        && gp == page
    {
        consumed = true;
        if resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
            cv.grab = None;
            let end = pointer.unwrap_or(start);
            if let Some((sp, si)) = cv.selected
                && sp == page
                && let Some((_, a)) = on_page(added, page).into_iter().find(|(i, _)| *i == si)
            {
                if let Grab::Rotate { center, start: from_angle, base } = grab {
                    let turn = turned(base, from_angle, pointer_angle(center, end), ui.input(|i| i.modifiers.shift));
                    if let AddedContent::Text(t) = &a.content
                        && signed_turn(base, turn).abs() >= 0.05
                    {
                        let content = AddedContent::Text(AddedText { rotation: turn, ..t.clone() });
                        view.pending_edit = Some(Edit::UpdateContent { page, index: si, content });
                    }
                } else if (end - start).length() >= 1.0 {
                    let keep = matches!(a.content, AddedContent::Image(_));
                    let (on_screen, degrees) = item_box(xf, info, page, a);
                    let r = dragged_turned(on_screen, degrees, grab, end - start, keep);
                    let (p0, p1) = (to_display(xf, info, page, r.left_bottom()), to_display(xf, info, page, r.right_top()));
                    let mut rect = [p0[0], p0[1], p1[0], p1[1]];
                    if let AddedContent::Text(t) = &a.content
                        && grab == Grab::Move
                    {
                        // Text height follows its lines: keep the top edge's offset only.
                        rect = [rect[0], rect[3] - (t.rect[3] - t.rect[1]), rect[2], rect[3]];
                    }
                    view.pending_edit = Some(Edit::UpdateContent { page, index: si, content: a.content.with_rect(rect) });
                }
            }
        }
        return consumed;
    }
    let Some(p) = pointer.filter(|p| xf.rect.contains(*p)) else { return consumed };
    let items = on_page(added, page);
    let over = |q: Pos2| {
        items
            .iter()
            .rev()
            .find(|(_, a)| {
                let (r, degrees) = item_box(xf, info, page, a);
                on_box(r, degrees, q, 2.0)
            })
            .map(|(i, a)| (*i, *a))
    };
    let hit = over(p);
    // The selected item: its box on screen and rotation, whether it is text, and its rotation as a turn.
    let selected = cv
        .selected
        .filter(|s| s.0 == page)
        .and_then(|(_, si)| items.iter().find(|(i, _)| *i == si))
        .map(|(_, a)| (item_box(xf, info, page, a), matches!(a.content, AddedContent::Text(_)), item_rotation(&a.content)));
    let selected_rect = selected.map(|((r, _), ..)| r);
    let selected_turn = selected.map_or(0.0, |((_, degrees), ..)| degrees);
    let corner = selected_rect.and_then(|r| handles(r, selected_turn).into_iter().find(|(c, ..)| c.distance(p) <= 6.0));
    // The rotate grip over a selected text box.
    let at_grip = |q: Pos2| selected.is_some_and(|((r, degrees), is_text, _)| is_text && grip(r, degrees).distance(q) <= GRIP_RADIUS + 3.0);
    if resp.secondary_clicked() {
        // Right-click picks what is under the pointer (or nothing) for the page menu, and remembers where.
        if !at_grip(p) {
            cv.selected = hit.map(|(i, _)| (page, i));
        }
        cv.menu_at = Some((page, to_display(xf, info, page, p)));
        consumed = true;
    }
    if adding_text && hit.is_none() && !at_grip(p) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        if resp.clicked() {
            // Text that can't be drawn stays where it is being typed (#125).
            if cv.hold_blocked() {
                return true;
            }
            // A click elsewhere keeps the text being typed and starts a new box (#74). The click
            // can reach the page before the editor sees it lose focus, so finish it here.
            let typed = cv.draft.take().and_then(|d| finish(cv, d, added));
            // The Format panel styles the new text, not the box just kept.
            (cv.selected, cv.select_added) = (None, None);
            let at = to_display(xf, info, page, p);
            let mut style = style.clone();
            style.text.clear();
            cv.draft = Some(TextDraft {
                page,
                index: None,
                rect: [at[0], at[1] - style.size * 1.2, at[0] + 200.0, at[1]],
                text: String::new(),
                style,
                focus: true,
            });
            consumed = true;
            if typed.is_some() {
                view.pending_edit = typed;
            }
        }
        return consumed;
    }
    if at_grip(p) {
        ui.ctx().set_cursor_icon(if cv.grab.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
    } else if let Some((_, left, top)) = corner {
        ui.ctx().set_cursor_icon(if left == top { egui::CursorIcon::ResizeNwSe } else { egui::CursorIcon::ResizeNeSw });
    } else if hit.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
        if let Some(((r, _), _, base)) = selected.filter(|_| at_grip(origin)) {
            let center = r.center();
            cv.grab = Some((page, Grab::Rotate { center, start: pointer_angle(center, origin), base }, origin));
            consumed = true;
        } else if let Some((_, left, top)) =
            selected_rect.and_then(|r| handles(r, selected_turn).into_iter().find(|(c, ..)| c.distance(origin) <= 6.0))
        {
            cv.grab = Some((page, Grab::Corner(left, top), origin));
            consumed = true;
        } else if let Some((i, _)) = over(origin) {
            cv.selected = Some((page, i));
            cv.grab = Some((page, Grab::Move, origin));
            consumed = true;
        }
        return consumed;
    }
    if resp.double_clicked()
        && let Some((i, a)) = hit
        && let AddedContent::Text(t) = &a.content
    {
        if cv.hold_blocked() {
            return true;
        }
        cv.draft = Some(TextDraft { page, index: Some(i), rect: t.rect, text: t.text.clone(), style: t.clone(), focus: true });
        cv.selected = Some((page, i));
        consumed = true;
        return consumed;
    }
    if resp.clicked() {
        match hit {
            Some((i, _)) => {
                cv.selected = Some((page, i));
                consumed = true;
            }
            None if corner.is_none() => cv.selected = None,
            None => consumed = true,
        }
    }
    consumed || hit.is_some() && resp.is_pointer_button_down_on()
}

/// Outlines of added items (hover and selection with handles and, on text, the rotate grip) and the box being dragged.
pub(crate) fn paint_page(ui: &egui::Ui, painter: &egui::Painter, xf: &PageXform, page: usize, info: &DocInfo, added: &[Added], view: &DocView) {
    let cv = &view.content;
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let shift = ui.input(|i| i.modifiers.shift);
    for (i, a) in on_page(added, page) {
        if cv.draft.as_ref().is_some_and(|d| d.page == page && d.index == Some(i)) {
            continue;
        }
        let (mut r, mut degrees) = item_box(xf, info, page, a);
        let selected = cv.selected == Some((page, i));
        let is_text = matches!(a.content, AddedContent::Text(_));
        let mut turning = false;
        if selected
            && let (Some((gp, g, start)), Some(p)) = (cv.grab, pointer)
            && gp == page
        {
            match g {
                Grab::Rotate { center, start: from_angle, base } => {
                    degrees = turned(base, from_angle, pointer_angle(center, p), shift) as f32;
                    turning = true;
                }
                _ => r = dragged_turned(r, degrees, g, p - start, !is_text),
            }
        }
        if selected {
            let stroke = Stroke::new(1.5, SELECT_BLUE);
            painter.add(egui::Shape::closed_line(outline(r.expand(1.0), degrees), stroke));
            for (c, ..) in handles(r.expand(1.0), degrees) {
                painter.circle(c, 3.5, Color32::WHITE, stroke);
            }
            if is_text {
                paint_grip(painter, r, degrees, turning);
            }
        } else if pointer.is_some_and(|p| on_box(r, degrees, p, 2.0)) {
            painter.add(egui::Shape::closed_line(outline(r.expand(1.0), degrees), Stroke::new(1.0, SELECT_BLUE.gamma_multiply(0.7))));
        }
    }
}

/// The rotate grip: a round handle on a stem above the box's top edge, with a circular arrow on it; while it is being
/// dragged it is filled and shows the angle.
fn paint_grip(painter: &egui::Painter, r: Rect, degrees: f32, turning: bool) {
    let stroke = Stroke::new(1.5, SELECT_BLUE);
    let g = grip(r, degrees);
    painter.line_segment([rot_about(egui::pos2(r.center().x, r.min.y - 1.0), r.center(), degrees), g], Stroke::new(1.0, SELECT_BLUE));
    painter.circle(g, GRIP_RADIUS, if turning { SELECT_BLUE } else { Color32::WHITE }, stroke);
    let ink = if turning { Color32::WHITE } else { SELECT_BLUE };
    // Three quarters of a circle with an arrow head at its end.
    let arc: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = 0.5 + k as f32 / 12.0 * 4.2;
            g + egui::vec2(a.cos(), -a.sin()) * 3.2
        })
        .collect();
    if let [.., before, end] = arc[..] {
        let t = (end - before).normalized();
        let n = egui::vec2(-t.y, t.x);
        painter.add(egui::Shape::line(arc, Stroke::new(1.3, ink)));
        painter.add(egui::Shape::line(vec![end - t * 2.2 + n * 2.2, end + t * 1.2, end - t * 2.2 - n * 2.2], Stroke::new(1.3, ink)));
    }
    if turning {
        let galley = painter.layout_no_wrap(format!("{:.0}°", degrees), egui::FontId::proportional(12.0), Color32::WHITE);
        let at = egui::Align2::LEFT_BOTTOM.anchor_size(g + egui::vec2(12.0, -10.0), galley.size());
        painter.rect_filled(at.expand(3.0), CornerRadius::same(3), SELECT_BLUE);
        painter.galley(at.min, galley, Color32::WHITE);
    }
}

/// A text box on the clipboard, and where it came from: the document and the page.
#[derive(Clone)]
struct Clip {
    item: AddedText,
    source: (egui::Id, usize),
}

/// How far down and to the right a pasted copy lands from the box it was copied from (points, display space).
const PASTE_OFFSET: f64 = 14.0;
/// The most characters a text box made by pasting plain text from outside holds.
const PASTE_LIMIT: usize = 5000;

fn clip_key() -> egui::Id {
    egui::Id::new("bdf-added-text-clipboard")
}

/// Whether a text box is waiting to be pasted.
pub(crate) fn clip_ready(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Clip>(clip_key()).is_some())
}

/// The selected item, when it is text.
fn selected_text(view: &DocView, added: &[Added]) -> Option<(usize, usize, AddedText)> {
    let (page, i) = view.content.selected?;
    match &on_page(added, page).into_iter().find(|(k, _)| *k == i)?.1.content {
        AddedContent::Text(t) => Some((page, i, t.clone())),
        AddedContent::Image(_) => None,
    }
}

/// Copy the selected text box: for BDF's own paste, with its style and turn, and as plain text for everything else.
fn copy_selected(ctx: &egui::Context, view: &DocView, added: &[Added]) -> bool {
    let Some((page, _, t)) = selected_text(view, added) else { return false };
    ctx.copy_text(t.text.clone());
    ctx.data_mut(|d| d.insert_temp(clip_key(), Clip { item: t, source: (egui::Id::new(view.id.0), page) }));
    true
}

/// The edit that puts a copy of `item` on `page`, and selects the copy once it exists. It lands with its top-left corner
/// at `at` (display space) when given; otherwise a little down and right of where it was copied from, when that is this
/// page, and else exactly where it was. It always stays on the page. `keep` makes it what Paste pastes next, so
/// repeated pastes step across the page.
#[allow(clippy::too_many_arguments)]
fn paste_item(
    ctx: &egui::Context,
    cv: &mut ContentView,
    added: &[Added],
    item: &AddedText,
    from: (egui::Id, usize),
    here: (egui::Id, usize),
    at: Option<[f64; 2]>,
    page_size: (f64, f64),
    keep: bool,
) -> Edit {
    let mut t = item.clone();
    let (left, top) = (t.rect[0].min(t.rect[2]), t.rect[1].max(t.rect[3]));
    let (w, h) = ((t.rect[2] - t.rect[0]).abs(), (t.rect[3] - t.rect[1]).abs());
    let (x, y) = match at {
        Some([x, y]) => (x, y),
        None if from == here => (left + PASTE_OFFSET, top - PASTE_OFFSET),
        None => (left, top),
    };
    // Whole on the page, as far as it fits.
    let (x, y) = (x.min(page_size.0 - w).max(0.0), y.max(h).min(page_size.1));
    t.rect = [x, y - h, x + w, y];
    if keep {
        ctx.data_mut(|d| d.insert_temp(clip_key(), Clip { item: t.clone(), source: here }));
    }
    cv.select_added = Some((here.1, on_page(added, here.1).len()));
    Edit::AddText { page: here.1, text: t }
}

/// Whether pasted text is what was copied: the clipboard may have turned its line ends into CR LF or trimmed the end.
fn same_text(pasted: &str, copied: &str) -> bool {
    let plain = |s: &str| s.replace("\r\n", "\n").trim_end().to_string();
    plain(pasted) == plain(copied)
}

/// The keys of Edit mode on added content: Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste a text box (pasting text copied
/// elsewhere makes a new box of it), Delete removes the selected item and Escape clears the selection. Nothing here
/// acts while a text field has the keyboard.
pub(crate) fn keys(ctx: &egui::Context, view: &mut DocView, info: &DocInfo, added: &[Added], style: &AddedText) {
    if view.content.draft.is_some() || ctx.egui_wants_keyboard_input() {
        return;
    }
    let (copy, cut, pasted, del, esc) = ctx.input(|i| {
        (
            i.events.iter().any(|e| matches!(e, egui::Event::Copy)),
            i.events.iter().any(|e| matches!(e, egui::Event::Cut)),
            i.events.iter().find_map(|e| if let egui::Event::Paste(text) = e { Some(text.clone()) } else { None }),
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            i.key_pressed(egui::Key::Escape),
        )
    });
    // Handled here, so nothing else copies the page's selected text or pastes pages as well.
    let take =
        |ctx: &egui::Context| ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))));
    if (copy || cut) && copy_selected(ctx, view, added) {
        take(ctx);
        if cut && let Some((page, index, _)) = selected_text(view, added) {
            view.content.selected = None;
            view.pending_edit = Some(Edit::DeleteContent { page, index });
            return;
        }
    }
    if let Some(text) = pasted {
        let page = view.content.selected.map_or(view.current, |s| s.0);
        let Some(p) = info.pages.get(page) else { return };
        let page_size = (f64::from(p.width), f64::from(p.height));
        let here = (egui::Id::new(view.id.0), page);
        let clip = ctx.data(|d| d.get_temp::<Clip>(clip_key()));
        // What BDF itself copied comes back whole; text from anywhere else becomes a new box.
        let edit = match clip {
            Some(c) if text.is_empty() || same_text(&text, &c.item.text) => {
                Some(paste_item(ctx, &mut view.content, added, &c.item, c.source, here, None, page_size, true))
            }
            _ if !text.trim().is_empty() => {
                let mut t = style.clone();
                t.text = text.chars().take(PASTE_LIMIT).collect();
                t.rect = [0.0, 0.0, 200.0, t.size * 1.2];
                let top = (page_size.1 - 72.0).max(t.size * 1.2);
                Some(paste_item(ctx, &mut view.content, added, &t, here, here, Some([72.0, top]), page_size, false))
            }
            _ => None,
        };
        if edit.is_some() {
            take(ctx);
            view.pending_edit = edit;
        }
        return;
    }
    if view.content.selected.is_none() {
        return;
    }
    if del && let Some((page, index)) = view.content.selected.take() {
        view.pending_edit = Some(Edit::DeleteContent { page, index });
    } else if esc {
        view.content.selected = None;
    }
}

/// The page's right-click menu in Edit mode: Copy, Cut, Paste, Duplicate, turning and Delete for the item that was
/// clicked, or Paste alone where nothing is. `false` when there is nothing to offer, so the page's ordinary menu shows.
pub(crate) fn context_menu(ui: &mut egui::Ui, view: &mut DocView, info: &DocInfo, added: &[Added], can_modify: bool) -> bool {
    let Some((page, at)) = view.content.menu_at else { return false };
    let selected = view
        .content
        .selected
        .filter(|s| s.0 == page)
        .and_then(|(_, i)| on_page(added, page).into_iter().find(|(k, _)| *k == i))
        .map(|(i, a)| (i, a.content.clone()));
    let can_paste = can_modify && clip_ready(ui.ctx());
    if selected.is_none() && !can_paste {
        return false;
    }
    let Some(p) = info.pages.get(page) else { return false };
    let page_size = (f64::from(p.width), f64::from(p.height));
    let here = (egui::Id::new(view.id.0), page);
    let ctx = ui.ctx().clone();
    let shortcut = |key: egui::Key| ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key));
    let item = |ui: &mut egui::Ui, label: &str, shortcut: Option<String>, enabled: bool| {
        let mut b = egui::Button::new(label);
        if let Some(s) = shortcut {
            b = b.shortcut_text(s);
        }
        ui.add_enabled(enabled, b).clicked()
    };
    if let Some((index, AddedContent::Text(t))) = &selected {
        let index = *index;
        if item(ui, tl!("Copy"), Some(shortcut(egui::Key::C)), true) {
            copy_selected(&ctx, view, added);
            ui.close();
        }
        if item(ui, tl!("Cut"), Some(shortcut(egui::Key::X)), can_modify) {
            copy_selected(&ctx, view, added);
            view.content.selected = None;
            view.pending_edit = Some(Edit::DeleteContent { page, index });
            ui.close();
        }
        if item(ui, tl!("Paste"), Some(shortcut(egui::Key::V)), can_paste) {
            if let Some(c) = ctx.data(|d| d.get_temp::<Clip>(clip_key())) {
                view.pending_edit = Some(paste_item(&ctx, &mut view.content, added, &c.item, c.source, here, None, page_size, true));
            }
            ui.close();
        }
        if item(ui, tl!("Duplicate"), None, can_modify) {
            view.pending_edit = Some(paste_item(&ctx, &mut view.content, added, t, here, here, None, page_size, false));
            ui.close();
        }
        ui.separator();
        let turn = |degrees: f64| {
            let content = AddedContent::Text(AddedText { rotation: pdfcraft_engine::normalize_rotation(t.rotation + degrees), ..t.clone() });
            Edit::UpdateContent { page, index, content }
        };
        if item(ui, tl!("Rotate Clockwise"), None, can_modify) {
            view.pending_edit = Some(turn(-90.0));
            ui.close();
        }
        if item(ui, tl!("Rotate Counterclockwise"), None, can_modify) {
            view.pending_edit = Some(turn(90.0));
            ui.close();
        }
        if item(ui, tl!("Reset rotation"), None, can_modify && pdfcraft_engine::normalize_rotation(t.rotation) != 0.0) {
            view.pending_edit = Some(turn(-t.rotation));
            ui.close();
        }
        ui.separator();
    } else if selected.is_none() && can_paste {
        if item(ui, tl!("Paste"), Some(shortcut(egui::Key::V)), true) {
            if let Some(c) = ctx.data(|d| d.get_temp::<Clip>(clip_key())) {
                view.pending_edit = Some(paste_item(&ctx, &mut view.content, added, &c.item, c.source, here, Some(at), page_size, true));
            }
            ui.close();
        }
        return true;
    }
    if let Some((index, _)) = selected
        && item(ui, tl!("Delete"), Some("Del".to_string()), can_modify)
    {
        view.content.selected = None;
        view.pending_edit = Some(Edit::DeleteContent { page, index });
        ui.close();
    }
    true
}

/// After an add: select the new item.
pub(crate) fn after_refresh(view: &mut DocView, added: &[Added]) {
    if let Some((page, before)) = view.content.select_added.take()
        && on_page(added, page).len() > before
    {
        view.content.selected = Some((page, before));
    }
    if let Some((p, i)) = view.content.selected
        && on_page(added, p).len() <= i
    {
        view.content.selected = None;
    }
}

/// The in-place text editor, with Done and Discard beside it. Returns the edit once the text is
/// committed (a click away, or Done), and whether Done finished adding text. Discard or Escape
/// throws the draft away.
pub(crate) fn editor(ctx: &egui::Context, view: &mut DocView, info: &DocInfo, added: &[Added]) -> (Option<Edit>, bool) {
    let Some((page, rect)) = view.content.draft.as_ref().map(|d| (d.page, d.rect)) else { return (None, false) };
    let Some(xf) = view.page_xform(page) else { return (None, false) };
    let r = screen_rect(&xf, info, page, rect);
    let zoom = xf.rect.width() / xf.pw.max(1.0);
    let tokens = Tokens::get(ctx);
    let (mut commit, mut cancel, mut done) = (false, false, false);
    let mut undrawable = None;
    let memo = &view.content.undrawable;
    egui::Area::new(egui::Id::new(("added-text", view.id.0))).order(egui::Order::Foreground).fixed_pos(r.min).show(ctx, |ui| {
        let Some(t) = view.content.draft.as_mut() else { return };
        // The box is white in every theme, so the caret must not follow the theme (pale in dark mode) (#655).
        ui.visuals_mut().text_cursor.stroke.color = crate::edit_text_ui::caret_color_on(Color32::WHITE);
        let [cr, cg, cb] = t.style.color.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let resp = ui.add(
                egui::TextEdit::multiline(&mut t.text)
                    .font(egui::FontId::proportional((t.style.size as f32 * zoom).max(8.0)))
                    .desired_width(r.width().max(60.0))
                    .desired_rows(1)
                    .background_color(Color32::from_rgba_unmultiplied(255, 255, 255, 235))
                    .text_color(Color32::from_rgb(cr, cg, cb))
                    .hint_text(tl!("Type text"))
                    .id_salt("added-text-edit"),
            );
            if t.focus {
                resp.request_focus();
                t.focus = false;
            }
            let buttons = egui::Frame::NONE
                .fill(tokens.card)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(2)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
                    ui.horizontal(|ui| {
                        done = crate::icons::button(ui, "check", 24.0, false, tl!("Done adding text")).clicked();
                        cancel = crate::icons::button(ui, "x", 24.0, false, tl!("Discard this text (Esc)")).clicked();
                    });
                })
                .response
                .rect;
            // Pressing a button takes the focus from the text: that is not a click away.
            let on_buttons = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| buttons.contains(p));
            cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
            commit = done || resp.lost_focus() && !on_buttons && !view.objects.hold_editor;
        });
        // Page text is drawn with the standard fonts; anything else would be saved as `?` (#125).
        undrawable = self::undrawable(t, memo);
        if let Some(c) = undrawable {
            ui.label(egui::RichText::new(undrawable_message(c)).small().color(UNDRAWABLE));
        }
    });
    if cancel {
        view.content.draft = None;
        return (None, false);
    }
    // Text that can't be drawn stays in the editor, with the warning, rather than being lost.
    if !commit || undrawable.is_some() {
        return (None, false);
    }
    let edit = view.content.draft.take().and_then(|d| finish(&mut view.content, d, added));
    (edit, done)
}

/// A finished draft as an edit: new text is added (and selected once it exists), retyped text
/// updated, and text emptied by retyping removed. Empty new text adds nothing.
fn finish(cv: &mut ContentView, d: TextDraft, added: &[Added]) -> Option<Edit> {
    let text = d.text.trim_end().to_string();
    match d.index {
        None if text.trim().is_empty() => None,
        None => {
            cv.select_added = Some((d.page, on_page(added, d.page).len()));
            Some(Edit::AddText { page: d.page, text: AddedText { rect: d.rect, text, ..d.style } })
        }
        Some(i) if text.trim().is_empty() => Some(Edit::DeleteContent { page: d.page, index: i }),
        Some(i) => {
            let (_, a) = on_page(added, d.page).into_iter().find(|(k, _)| *k == i)?;
            let AddedContent::Text(old) = &a.content else { return None };
            (old.text != text).then(|| Edit::UpdateContent { page: d.page, index: i, content: AddedContent::Text(AddedText { text, ..old.clone() }) })
        }
    }
}

/// Format text (shown in the Edit panel while a text item is selected, or for new text).
/// Returns the changed style; for a selected item the caller turns it into an update.
pub(crate) fn format_panel(ui: &mut egui::Ui, t: &Tokens, style: &AddedText) -> Option<AddedText> {
    let mut s = style.clone();
    widgets::section_title(ui, tl!("Format text"));
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("font-family").selected_text(s.family.label()).width(110.0).show_ui(ui, |ui| {
            for f in [FontFamily::Helvetica, FontFamily::Times, FontFamily::Courier] {
                ui.selectable_value(&mut s.family, f, f.label());
            }
        });
        // A list rather than a drag value: every change is an undo step.
        egui::ComboBox::from_id_salt("font-size").selected_text(format!("{} pt", s.size)).width(70.0).show_ui(ui, |ui| {
            for size in [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 36.0, 48.0, 72.0] {
                ui.selectable_value(&mut s.size, size, format!("{size} pt"));
            }
        });
    });
    ui.horizontal(|ui| {
        if ui.selectable_label(s.bold, egui::RichText::new("B").strong()).on_hover_text(tl!("Bold")).clicked() {
            s.bold = !s.bold;
        }
        if ui.selectable_label(s.italic, egui::RichText::new("I").italics()).on_hover_text(tl!("Italic")).clicked() {
            s.italic = !s.italic;
        }
        ui.separator();
        for (a, icon, tip) in [
            (TextAlign::Left, "align-left", tl!("Align left")),
            (TextAlign::Center, "align-center", tl!("Centre")),
            (TextAlign::Right, "align-right", tl!("Align right")),
            (TextAlign::Justify, "align-justify", tl!("Justify")),
        ] {
            if crate::icons::button(ui, icon, 26.0, s.align == a, tip).clicked() {
                s.align = a;
            }
        }
    });
    let c = s.color;
    if let Some(picked) = crate::comments::swatch_grid(ui, Some(c)) {
        s.color = picked;
    }
    ui.label(egui::RichText::new(tl!("Thai and Arabic use regular fallback fonts when available.")).small().color(t.text_faint));
    (s != *style).then_some(s)
}

/// How to use the tools, under the Add content list.
pub(crate) fn hint(ui: &mut egui::Ui, t: &Tokens) {
    ui.label(
        egui::RichText::new(
            tl!("Click on the page to add text; each click starts a new box, and ✓ finishes. Drag items to move them, drag a corner to resize, double-click text to edit it."),
        )
            .small()
            .color(t.text_faint),
    );
    ui.label(egui::RichText::new(tl!("Drag the round handle above a text box to turn it (Shift: 15° steps). Ctrl+C and Ctrl+V copy and paste a box, Delete removes it; right-click shows the same.")).small().color(t.text_faint));
    ui.add_space(4.0);
}

impl crate::PdfCraftApp {
    /// Pick an image file for the active document, read it, and call `then` with its name and
    /// bytes: now when `save_override` answers (tests and automation never see a native dialog),
    /// otherwise on a later frame, and only if that document is still the active one.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_image(&mut self, title: &str, then: impl FnOnce(&mut Self, String, Vec<u8>) + Send + 'static) {
        let read = |app: &mut Self, path: std::path::PathBuf| match std::fs::read(&path) {
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                then(app, name, bytes);
            }
            Err(e) => app.notify_fmt("Couldn't read {name}: {e}", &[("name", &path.display().to_string()), ("e", &e.to_string())]),
        };
        match self.save_override.clone() {
            Some(p) if p.ends_with(".png") || p.ends_with(".jpg") => read(self, p.into()),
            Some(_) => {}
            None => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(tl!("Images"), &["png", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "jp2", "j2k", "jpx"])
                    .set_title(title);
                let target = self.active_ids().map(|(_, id)| id);
                self.ask_one(crate::pickers::Ask::File(dialog), target, read);
            }
        }
    }

    /// Edit a PDF ▸ Add content ▸ Image: pick a file and place it in the middle of the current page.
    pub fn add_image_dialog(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Choose an image"), |app, name, bytes| app.add_image(name, bytes));
        #[cfg(target_arch = "wasm32")]
        self.notify_tr("Adding images arrives on the web with file pickers for images");
    }

    /// Edit text & images ▸ Replace Image: pick a file to draw in a page image's place.
    pub(crate) fn replace_page_image_dialog(&mut self, page: usize, index: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Replace image"), move |app, name, bytes| {
            app.apply_edit(Edit::EditPageImage {
                page,
                index,
                change: pdfcraft_engine::ImageEdit::Replace { name, bytes: std::sync::Arc::new(bytes) },
            });
        });
        #[cfg(target_arch = "wasm32")]
        self.notify_fmt(
            "Replacing images on page {p} arrives on the web with image pickers ({i})",
            &[("p", &(page + 1).to_string()), ("i", &index.to_string())],
        );
    }

    /// Edit text & images ▸ Save Image As.
    pub(crate) fn save_page_image(&mut self, page: usize, index: usize) {
        let Some((_, id)) = self.active_ids() else { return };
        if !self.may_copy(id) {
            return;
        }
        let file = self.session.get(id).map(|d| d.page_image_file(page, index));
        match file {
            Some(Ok((ext, bytes))) => {
                self.write_files(&[(format!("Image page {} #{}.{ext}", page + 1, index + 1), std::sync::Arc::new(bytes))], "Save image");
            }
            Some(Err(e)) => self.notify_fmt("Couldn't save the image: {e}", &[("e", &e.to_string())]),
            None => {}
        }
    }

    /// Copy Image: image `index` on `page`, at its own resolution, to the clipboard.
    pub(crate) fn copy_page_image(&mut self, page: usize, index: usize) {
        let Some((_, id)) = self.active_ids() else { return };
        if !self.may_copy(id) {
            return;
        }
        let Some(pixels) = self.session.get(id).map(|d| d.page_image_rgba(page, index)) else { return };
        let copied = pixels.and_then(|(w, h, rgba)| {
            #[cfg(not(target_arch = "wasm32"))]
            if self.system_clipboard {
                let img = arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(&rgba) };
                arboard::Clipboard::new().and_then(|mut c| c.set_image(img)).map_err(|e| e.to_string())?;
            }
            #[cfg(target_arch = "wasm32")]
            if self.system_clipboard {
                return Err("the web version can't put images on the clipboard yet; use Save Image As".to_string());
            }
            self.last_copied_image = Some((w, h, rgba));
            Ok((w, h))
        });
        match copied {
            Ok((w, h)) => self.notify_fmt("The image has been copied ({w} × {h} pixels)", &[("w", &w.to_string()), ("h", &h.to_string())]),
            Err(e) => self.notify_fmt("Couldn't copy the image: {e}", &[("e", &e)]),
        }
    }

    /// Click an image field: pick a picture for it.
    pub fn choose_field_image(&mut self, name: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let name = name.to_string();
            self.pick_image(tl!("Select Icon"), move |app, _, bytes| {
                app.apply_edit(Edit::SetFieldImage { name, image: std::sync::Arc::new(bytes) });
            });
        }
        #[cfg(target_arch = "wasm32")]
        self.notify_fmt("{name}: choosing images arrives on the web with file pickers for images", &[("name", &name)]);
    }

    /// Edit image ▸ Replace: pick a file for the selected image.
    pub fn replace_image_dialog(&mut self, page: usize, index: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        self.pick_image(tl!("Replace image"), move |app, name, bytes| {
            app.apply_edit(Edit::ReplaceImage { page, index, name, bytes: std::sync::Arc::new(bytes) });
        });
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (page, index);
            self.notify_tr("Replacing images arrives on the web with image file pickers");
        }
    }

    /// Place image `bytes` in the middle of the current page and select it.
    pub fn add_image(&mut self, name: String, bytes: Vec<u8>) {
        let Some((i, id)) = self.active_ids() else { return };
        let page = self.views[i].current;
        let before = self.session.get(id).map_or(0, |d| d.added.iter().filter(|a| a.page == page).count());
        if self.apply_edit(Edit::AddImage { page, rect: None, name, bytes: std::sync::Arc::new(bytes) }) {
            self.views[i].content.select_added = Some((page, before));
            self.left = crate::LeftPanel::Tool("edit");
            self.left_open = true;
        }
    }
}

/// What the image tools ask for.
pub(crate) enum ImageAction {
    Update(pdfcraft_engine::AddedContent),
    Replace,
}

/// Edit image: rotate, flip, crop, replace (shown while an added image is selected).
pub(crate) fn image_panel(ui: &mut egui::Ui, t: &Tokens, img: &pdfcraft_engine::AddedImage) -> Option<ImageAction> {
    let mut out = None;
    widgets::section_title(ui, tl!("Edit image"));
    ui.horizontal(|ui| {
        let mut i = img.clone();
        let r = i.rect;
        let turn = |i: &mut pdfcraft_engine::AddedImage, k: u8| {
            i.rotation = (i.rotation + k) % 4;
            // The box turns with the picture, around its centre.
            let (cx, cy, w, h) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0, r[2] - r[0], r[3] - r[1]);
            i.rect = [cx - h / 2.0, cy - w / 2.0, cx + h / 2.0, cy + w / 2.0];
        };
        if crate::icons::button(ui, "rotate-ccw", 28.0, false, tl!("Rotate counterclockwise")).clicked() {
            turn(&mut i, 1);
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "rotate-cw", 28.0, false, tl!("Rotate clockwise")).clicked() {
            turn(&mut i, 3);
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "flip-horizontal-2", 28.0, false, tl!("Flip horizontal")).clicked() {
            i.flip_h = !i.flip_h;
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "flip-vertical-2", 28.0, false, tl!("Flip vertical")).clicked() {
            i.flip_v = !i.flip_v;
            out = Some(ImageAction::Update(AddedContent::Image(i.clone())));
        }
        if crate::icons::button(ui, "replace", 28.0, false, tl!("Replace image")).clicked() {
            out = Some(ImageAction::Replace);
        }
    });
    ui.label(egui::RichText::new(tl!("Crop (% trimmed from each side)")).small().color(t.text_muted));
    let mut crop = img.crop.map(|v| (v * 100.0).round());
    let mut changed = false;
    ui.horizontal(|ui| {
        for (k, label) in ["L", "B", "R", "T"].into_iter().enumerate() {
            ui.label(label);
            // Applied when the drag ends, so a drag is one undo step.
            let r = ui.add(egui::DragValue::new(&mut crop[k]).range(0.0..=45.0).speed(0.5).suffix("%"));
            changed |= r.drag_stopped() || (r.changed() && !r.dragged());
        }
    });
    if changed {
        let mut i = img.clone();
        i.crop = crop.map(|v| v / 100.0);
        if i.crop != img.crop {
            out = Some(ImageAction::Update(AddedContent::Image(i)));
        }
    }
    out
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use egui::{pos2, vec2};

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn a_turn_is_counter_clockwise_as_seen_on_screen() {
        // The point to the right of the centre goes up the screen (y down) for a quarter turn.
        assert!(near(rot_about(pos2(11.0, 5.0), pos2(10.0, 5.0), 90.0), pos2(10.0, 4.0)));
        assert!(near(rot_about(pos2(11.0, 5.0), pos2(10.0, 5.0), -90.0), pos2(10.0, 6.0)));
        // The pointer straight above the centre is at 90 degrees, to its right at 0.
        assert!((pointer_angle(pos2(0.0, 0.0), pos2(0.0, -5.0)) - 90.0).abs() < 1e-4);
        assert!(pointer_angle(pos2(0.0, 0.0), pos2(5.0, 0.0)).abs() < 1e-4);
        // The grip of an upright box is above it, and goes round with the box.
        let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 140.0));
        assert!(near(grip(r, 0.0), pos2(150.0, 100.0 - GRIP_OFFSET)));
        assert!(near(grip(r, 90.0), pos2(150.0 - (GRIP_OFFSET + 20.0), 120.0)));
    }

    #[test]
    fn dragging_the_grip_gives_the_pointers_angle_snapped_and_always_a_turn() {
        // Pointer started straight above (90) and moved to the left of the centre (180): a quarter turn counter-clockwise.
        assert_eq!(turned(0.0, 90.0, 180.0, false), 90.0);
        // Within 3 degrees of a right angle it snaps; further it does not.
        assert_eq!(turned(0.0, 90.0, 92.0, false), 0.0);
        assert_eq!(turned(0.0, 90.0, 97.0, false), 7.0);
        // Shift gives steps of 15 degrees.
        assert_eq!(turned(0.0, 90.0, 100.0, true), 15.0);
        assert_eq!(turned(0.0, 90.0, 80.0, true), 345.0);
        // Never outside [0, 360), whatever the pointer does, and never a NaN.
        for base in [0.0, 359.9, 45.0] {
            for now in [-720.0f32, -180.0, 0.0, 179.9, 540.0, 1.0e7] {
                for fine in [false, true] {
                    let t = turned(base, 90.0, now, fine);
                    assert!((0.0..360.0).contains(&t), "{base} {now} {fine}: {t}");
                }
            }
        }
        assert!((0.0..360.0).contains(&turned(10.0, f32::NAN, 5.0, false)));
        // The turn between two rotations is the short way round.
        assert_eq!(signed_turn(350.0, 10.0), 20.0);
        assert_eq!(signed_turn(10.0, 350.0), -20.0);
        assert_eq!(signed_turn(0.0, 180.0).abs(), 180.0);
    }

    #[test]
    fn the_bounds_of_a_turned_box_hold_it_and_swap_at_a_quarter_turn() {
        let r = [10.0, 20.0, 110.0, 60.0];
        assert_eq!(bounds(r, 0.0), r);
        let q = bounds(r, 90.0);
        assert!((q[2] - q[0] - 40.0).abs() < 1e-9 && (q[3] - q[1] - 100.0).abs() < 1e-9, "{q:?}");
        // Same centre, and growing to hold the corners at 45 degrees.
        let d = bounds(r, 45.0);
        assert!(((d[0] + d[2]) / 2.0 - 60.0).abs() < 1e-9 && ((d[1] + d[3]) / 2.0 - 40.0).abs() < 1e-9);
        // A 100 by 40 box at 45 degrees is (100 + 40) / sqrt 2 across, both ways.
        let across = 140.0 / std::f64::consts::SQRT_2;
        assert!((d[2] - d[0] - across).abs() < 1e-6 && (d[3] - d[1] - across).abs() < 1e-6, "{d:?}");
        // A box given with its corners the wrong way round still gives a box.
        assert_eq!(bounds([110.0, 60.0, 10.0, 20.0], 0.0), r);
    }

    #[test]
    fn a_turned_box_is_hit_where_it_is_not_where_it_was() {
        let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 120.0));
        // A long thin box turned a quarter turn stands up: the point above its centre is on it, the old right end is not.
        assert!(on_box(r, 90.0, pos2(150.0, 70.0), 0.0));
        assert!(!on_box(r, 90.0, pos2(195.0, 110.0), 0.0));
        assert!(on_box(r, 0.0, pos2(195.0, 110.0), 0.0));
    }

    /// Dragging a corner of a turned box keeps the opposite corner where it is on screen, for every turn and corner.
    #[test]
    fn resizing_a_turned_box_keeps_the_opposite_corner_still() {
        let r = Rect::from_min_max(pos2(100.0, 200.0), pos2(260.0, 240.0));
        for k in 0..24 {
            let degrees = k as f32 * 15.0 + 7.0;
            for (left, top) in [(true, true), (false, true), (true, false), (false, false)] {
                for d in [vec2(30.0, 10.0), vec2(-12.0, 25.0), vec2(5.0, -9.0)] {
                    let out = dragged_turned(r, degrees, Grab::Corner(left, top), d, false);
                    let before = handles(r, degrees).into_iter().find(|(_, l, t)| *l == !left && *t == !top).map(|(p, ..)| p);
                    let after = handles(out, degrees).into_iter().find(|(_, l, t)| *l == !left && *t == !top).map(|(p, ..)| p);
                    let (Some(a), Some(b)) = (before, after) else { panic!("no opposite corner") };
                    assert!(near(a, b), "turn {degrees}, corner ({left},{top}), drag {d:?}: {a:?} became {b:?}");
                }
            }
        }
        // Upright, it is exactly the plain drag.
        let plain = dragged(r, Grab::Corner(false, false), vec2(20.0, 8.0), false);
        assert_eq!(dragged_turned(r, 0.0, Grab::Corner(false, false), vec2(20.0, 8.0), false), plain);
    }

    #[test]
    fn a_pasted_copy_lands_on_the_page_and_steps_away_from_the_original() {
        let ctx = egui::Context::default();
        let item = AddedText { rect: [100.0, 700.0, 300.0, 720.0], text: "Hello".into(), ..AddedText::default() };
        let (me, other) = (egui::Id::new("a"), egui::Id::new("b"));
        let mut cv = ContentView::default();
        let place =
            |cv: &mut ContentView, from, here, at: Option<[f64; 2]>| match paste_item(&ctx, cv, &[], &item, from, here, at, (612.0, 792.0), false) {
                Edit::AddText { page, text } => (page, text.rect),
                other => panic!("{other:?}"),
            };
        // Same page: down and to the right. Another page or document: exactly where it was.
        assert_eq!(place(&mut cv, (me, 0), (me, 0), None), (0, [114.0, 686.0, 314.0, 706.0]));
        assert_eq!(place(&mut cv, (me, 0), (me, 1), None), (1, [100.0, 700.0, 300.0, 720.0]));
        assert_eq!(place(&mut cv, (other, 0), (me, 0), None), (0, [100.0, 700.0, 300.0, 720.0]));
        // At a point (the top-left corner), and kept whole on the page whatever the point.
        assert_eq!(place(&mut cv, (me, 0), (me, 0), Some([50.0, 400.0])), (0, [50.0, 380.0, 250.0, 400.0]));
        assert_eq!(place(&mut cv, (me, 0), (me, 0), Some([600.0, 5.0])), (0, [412.0, 0.0, 612.0, 20.0]));
        assert_eq!(place(&mut cv, (me, 0), (me, 0), Some([-50.0, 9000.0])), (0, [0.0, 772.0, 200.0, 792.0]));
        assert_eq!(cv.select_added, Some((0, 0)), "the copy is selected once it exists");
        // NaN or infinity as the point never gives a box off the page.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let (_, r) = place(&mut cv, (me, 0), (me, 0), Some([bad, bad]));
            assert!(r.iter().all(|v| !v.is_nan()), "{bad}: {r:?}");
        }
    }

    #[test]
    fn pasted_text_is_the_copied_text_whatever_the_clipboard_did_to_the_line_ends() {
        assert!(same_text("a\r\nb", "a\nb"));
        assert!(same_text("a\nb  \n", "a\nb"));
        assert!(!same_text("a\nb", "a\nc"));
    }
}
