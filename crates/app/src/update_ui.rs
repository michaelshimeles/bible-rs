//! The update prompt. "Check for Updates…" and the daily check answer in the app, never in a
//! browser: over the main window as a sheet, and on Settings › About.

use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Galley, Order, Painter, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::pages::text;
use crate::theme::{self, Palette};
use crate::updater::{self, CURRENT, Progress, Retry, Status};
use crate::widgets::{Button, Kind};

const WIDTH: f32 = 440.0;
const PAD: f32 = 24.0;
const ICON: f32 = 56.0;
const FOOTER: f32 = 64.0;
/// The About page's card: its padding and button row.
const CARD_PAD: f32 = 16.0;
const BUTTON_H: f32 = 32.0;
const TITLE_LH: f32 = 20.0;
const DETAIL_LH: f32 = 18.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Update,
    Retry,
    Close,
}

/// A button: what it does, its label, and whether Return presses it.
type Spec = (Choice, &'static str, bool);

/// What the prompt says.
struct Words {
    title: String,
    detail: String,
    buttons: &'static [Spec],
}

fn words(status: &Status) -> Option<Words> {
    const CANCEL: &[Spec] = &[(Choice::Close, "Cancel", false)];
    const OK: &[Spec] = &[(Choice::Close, "OK", true)];
    let (title, detail, buttons): (String, String, &'static [Spec]) = match status {
        Status::Idle => return None,
        Status::Checking => {
            ("Checking for updates…".into(), "Asking GitHub for the newest version of Bible.".into(), CANCEL)
        }
        Status::UpToDate => {
            ("You’re up to date".into(), format!("Bible {CURRENT} is the newest version."), OK)
        }
        Status::Available { version } => (
            format!("Bible {version} is available"),
            format!("You have {CURRENT}. Update downloads and installs it, then opens Bible again."),
            &[(Choice::Close, "Later", false), (Choice::Update, "Update", true)],
        ),
        Status::Downloading { version, .. } => {
            (format!("Downloading Bible {version}…"), String::new(), CANCEL)
        }
        Status::Installing { version } => (
            format!("Installing Bible {version}…"),
            "Bible closes and opens again by itself in a moment.".into(),
            &[],
        ),
        Status::Failed { message, retry } => (
            match retry {
                Retry::Check => "Couldn’t check for updates",
                Retry::Download => "Couldn’t update Bible",
            }
            .into(),
            message.clone(),
            &[(Choice::Close, "Close", false), (Choice::Retry, "Retry", true)],
        ),
        Status::Unsupported if cfg!(windows) => (
            "This build can’t update itself".into(),
            "Updates work in the app the Bible installer sets up.".into(),
            OK,
        ),
        Status::Unsupported if cfg!(target_os = "macos") => {
            ("This build can’t update itself".into(), "Updates work in the released app.".into(), OK)
        }
        Status::Unsupported => (
            "This copy can’t update itself".into(),
            "Get new versions the same way you installed this one, such as your package manager.".into(),
            OK,
        ),
    };
    Some(Words { title, detail, buttons })
}

fn detail_galley(painter: &Painter, s: &str, width: f32, p: &Palette) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple(s.into(), theme::ui_font(13.0), p.ink_2, width);
    job.sections[0].format.line_height = Some(DETAIL_LH);
    painter.layout_job(job)
}

/// The text's height at `width`: the title, then the detail or the progress bar.
fn text_height(painter: &Painter, p: &Palette, w: &Words, status: &Status, width: f32) -> f32 {
    TITLE_LH
        + if matches!(status, Status::Downloading { .. }) {
            8.0 + 6.0 + 8.0 + 16.0
        } else if w.detail.is_empty() {
            0.0
        } else {
            4.0 + detail_galley(painter, &w.detail, width, p).size().y
        }
}

fn draw_text(painter: &Painter, p: &Palette, at: egui::Pos2, width: f32, w: &Words, status: &Status) {
    text(painter, at.x, at.y, TITLE_LH, &w.title, theme::ui_semibold(15.0), p.ink, false);
    let y = at.y + TITLE_LH;
    if let Status::Downloading { progress, .. } = status {
        let track = Rect::from_min_size(pos2(at.x, y + 8.0), vec2(width, 6.0));
        painter.rect_filled(track, 3.0, p.fill_strong);
        let done = Rect::from_min_size(track.min, vec2((width * fraction(progress)).floor(), 6.0));
        painter.rect_filled(done, 3.0, p.accent);
        let mb = |bytes: u64| bytes as f64 / 1_000_000.0;
        let label = format!("{:.1} of {:.1} MB", mb(progress.done()), mb(progress.total()));
        text(painter, at.x, track.bottom() + 8.0, 16.0, &label, theme::ui_font(12.0), p.ink_3, false);
    } else if !w.detail.is_empty() {
        painter.galley(pos2(at.x, y + 4.0), detail_galley(painter, &w.detail, width, p), p.ink_2);
    }
}

fn fraction(progress: &Progress) -> f32 {
    if progress.total() == 0 {
        0.0
    } else {
        (progress.done() as f64 / progress.total() as f64).min(1.0) as f32
    }
}

/// Draws the buttons in a row ending at `right`, centred on `cy`; returns the one clicked.
fn draw_buttons(
    ui: &mut Ui,
    p: &Palette,
    right: f32,
    cy: f32,
    buttons: &[Spec],
    id: egui::Id,
) -> Option<Choice> {
    let mut x = right;
    let mut clicked = None;
    for (i, &(choice, label, default)) in buttons.iter().enumerate().rev() {
        let kind = match (default, buttons.len()) {
            (true, _) => Kind::Primary,
            (false, 1) => Kind::Secondary,
            (false, _) => Kind::Plain,
        };
        let (r, resp) = Button::new(kind, label).show_right(ui, p, x, cy, id.with(i));
        if resp.clicked() {
            clicked = Some(choice);
        }
        x = r.left() - 8.0;
    }
    clicked
}

fn act(choice: Option<Choice>) {
    match choice {
        Some(Choice::Update) => updater::update(),
        Some(Choice::Retry) => updater::retry(),
        Some(Choice::Close) => updater::dismiss(),
        None => {}
    }
}

/// Keeps a check or a download moving on screen.
fn animate(ctx: &egui::Context, status: &Status) {
    if matches!(status, Status::Checking | Status::Downloading { .. }) {
        ctx.request_repaint_after_for(Duration::from_millis(100), egui::ViewportId::ROOT);
    }
}

/// Over the main window, when the menu asked or the daily check found a newer version.
pub fn sheet(ctx: &egui::Context, p: &Palette) {
    if !updater::sheet_open() {
        return;
    }
    let status = updater::status();
    let Some(w) = words(&status) else { return };
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("update-scrim")).order(Order::Middle).fixed_pos(screen.min).show(
        ctx,
        |ui| {
            let (r, _) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, 0.0, p.scrim);
        },
    );
    let at = pos2((screen.center().x - WIDTH / 2.0).round(), 72.0);
    let mut choice = None;
    egui::Area::new(egui::Id::new("update-sheet")).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        let painter = ui.painter().clone();
        let text_x = PAD + ICON + 16.0;
        let text_w = WIDTH - text_x - PAD;
        let text_h = text_height(&painter, p, &w, &status, text_w);
        let body = text_h.max(ICON);
        let footer = if w.buttons.is_empty() { 0.0 } else { FOOTER };
        let sheet = Rect::from_min_size(at, vec2(WIDTH, PAD + body + PAD + footer));
        ui.allocate_rect(sheet, Sense::hover());
        p.shadows.modal.paint(&painter, sheet, 16.0);
        painter.rect_filled(sheet, 16.0, p.surface);
        crate::onboarding::app_icon(&painter, Rect::from_min_size(at + vec2(PAD, PAD), vec2(ICON, ICON)));
        let text_at = at + vec2(text_x, PAD + ((body - text_h) / 2.0).round());
        draw_text(&painter, p, text_at, text_w, &w, &status);
        if footer > 0.0 {
            let f = Rect::from_min_max(pos2(sheet.left(), sheet.bottom() - FOOTER), sheet.max);
            painter.rect_filled(f, egui::CornerRadius { nw: 0, ne: 0, sw: 16, se: 16 }, p.fill);
            painter.hline(f.x_range(), f.top() + 0.5, Stroke::new(1.0, p.separator));
            choice = draw_buttons(ui, p, f.right() - PAD, f.center().y, w.buttons, ui.id().with("update"));
        }
    });
    let (enter, escape) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if enter {
        choice = choice.or(w.buttons.iter().find(|b| b.2).map(|b| b.0));
    }
    if escape && w.buttons.iter().any(|b| b.0 == Choice::Close) {
        choice = choice.or(Some(Choice::Close));
    }
    animate(ctx, &status);
    act(choice);
}

/// Settings › About: the prompt under its buttons, as a card, whatever asked.
pub fn about(ui: &mut Ui, p: &Palette) {
    let status = updater::status();
    let Some(w) = words(&status) else { return };
    let painter = ui.painter().clone();
    let width = ui.available_width().min(WIDTH);
    let inner = width - 2.0 * CARD_PAD;
    let text_h = text_height(&painter, p, &w, &status, inner);
    let buttons = if w.buttons.is_empty() { 0.0 } else { CARD_PAD + BUTTON_H };
    ui.add_space(16.0);
    let (row, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), 2.0 * CARD_PAD + text_h + buttons), Sense::hover());
    let card = Rect::from_min_size(
        pos2((row.center().x - width / 2.0).round(), row.top()),
        vec2(width, row.height()),
    );
    painter.rect_filled(card, 12.0, p.surface);
    painter.rect_stroke(card, 12.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
    draw_text(&painter, p, card.min + vec2(CARD_PAD, CARD_PAD), inner, &w, &status);
    let cy = card.bottom() - CARD_PAD - BUTTON_H / 2.0;
    let choice = draw_buttons(ui, p, card.right() - CARD_PAD, cy, w.buttons, ui.id().with("update"));
    animate(ui.ctx(), &status);
    act(choice);
}
