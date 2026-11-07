//! Recalling a memorised passage in place: the reader dims the chapter around it and hides the
//! passage's words, which a horizontal drag (or a two-finger swipe) reveals phrase by phrase.
//!
//! The text is typeset as for reading and never moves: hiding a word only changes its glyphs'
//! colour, so each block's galley is painted again with its glyphs faded by how far the reveal
//! has passed them ([`bible_core::memory::Reveal`] says where each char appears).

use std::sync::Arc;

use bible_core::library::{Chapter, Library};
use bible_core::memory::{Grade, Memorised, Reveal, interval_label};
use eframe::egui::{Color32, Id, Painter, Pos2, Rect, Sense, Ui, pos2, vec2};
use eframe::epaint::TextShape;
use eframe::epaint::text::Galley;

use super::layout::Block;
use crate::actions::Action;
use crate::theme::{self, Palette, text};
use crate::widgets::{Button, Kind};

/// How many chars the reveal's soft edge spans.
const SOFT: f32 = 5.0;
/// How strongly text that isn't being recalled is dimmed.
pub const DIM: f32 = 0.28;
/// A whole passage is revealed by a drag this many column widths long.
const SWIPE: f32 = 1.0;

/// What the keyboard asks of a recall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    NextPhrase,
    PreviousPhrase,
    NextWord,
    PreviousWord,
}

struct Built {
    translation: String,
    reveal: Reveal,
    /// The passage's verses in this translation: global verse index and number.
    verses: Vec<(u32, u8)>,
}

pub struct Recall {
    pub passage: Memorised,
    /// Which of the session's passages this is (from 1), and how many it has.
    pub index: usize,
    pub total: usize,
    position: f32,
    velocity: f32,
    target: Option<f32>,
    built: Option<Built>,
    /// A drag under way: where the reveal and the pointer were when it began.
    drag: Option<(f32, f32)>,
    /// When the last two-finger swipe moved the reveal.
    swiped_at: Option<f64>,
    /// The translation being read doesn't have all of the passage, so it can't be recalled or
    /// graded, only skipped.
    unavailable: bool,
}

impl Recall {
    pub fn new(passage: Memorised, index: usize, total: usize) -> Self {
        Self {
            passage,
            index,
            total,
            position: 0.0,
            velocity: 0.0,
            target: None,
            built: None,
            unavailable: false,
            drag: None,
            swiped_at: None,
        }
    }

    /// Makes the reveal for the passage as `translation` has it, once its chapter has loaded.
    pub fn build(&mut self, library: &Library, translation: &str) {
        match self.built.as_ref().map(|b| b.translation == translation) {
            Some(true) => return,
            // Another translation's words: the reveal starts the passage over.
            Some(false) => {
                self.built = None;
                self.position = 0.0;
                self.velocity = 0.0;
                self.target = None;
            }
            None => {}
        }
        let chapter = self.passage.first.chapter_idx();
        let (first, last) = (self.passage.first.verse, self.passage.last.verse);
        let t = match library.chapter(translation, chapter) {
            Chapter::Ready(t) => t,
            Chapter::Pending => return,
            _ => {
                self.unavailable = true;
                return;
            }
        };
        // Only all of it: a grade from part of the passage would schedule the whole.
        self.unavailable = !t.has_passage(chapter, first, last);
        if self.unavailable {
            return;
        }
        let views: Vec<_> = t.verses_in(chapter).filter(|v| (first..=last).contains(&v.number)).collect();
        let verses = views.iter().map(|v| (v.index, v.number)).collect();
        let reveal = Reveal::of_verses(&views, SOFT);
        self.position = self.position.min(reveal.end);
        self.built = Some(Built { translation: translation.to_string(), reveal, verses });
    }

    fn reveal(&self) -> Option<&Reveal> {
        self.built.as_ref().map(|b| &b.reveal)
    }

    /// How far the reveal has gone.
    pub fn shown(&self) -> f32 {
        self.position
    }

    pub fn unavailable(&self) -> bool {
        self.unavailable && self.built.is_none()
    }

    pub fn started(&self) -> bool {
        self.position > 0.01
    }

    pub fn done(&self) -> bool {
        self.reveal().is_some_and(|r| r.is_done(self.shown()))
    }

    /// Whether verse `number` of the passage's chapter is in it.
    pub fn contains(&self, number: u8) -> bool {
        (self.passage.first.verse..=self.passage.last.verse).contains(&number)
    }

    /// The reveal's index of a verse, by global verse index.
    fn verse_slot(&self, index: u32) -> Option<usize> {
        self.built.as_ref()?.verses.iter().position(|(i, _)| *i == index)
    }

    fn animate_to(&mut self, target: f32) {
        let end = self.reveal().map_or(0.0, |r| r.end);
        self.target = Some(target.clamp(0.0, end));
    }

    pub fn step(&mut self, step: Step) {
        let Some(r) = self.reveal() else { return };
        let from = self.target.unwrap_or_else(|| self.shown());
        let target = match step {
            Step::NextPhrase => r.next_stop(from),
            Step::PreviousPhrase => r.previous_stop(from),
            Step::NextWord => r.next_word(from),
            Step::PreviousWord => r.previous_word(from),
        };
        self.animate_to(target);
    }

    /// Hides the passage again.
    pub fn reset(&mut self) {
        self.animate_to(0.0);
    }

    /// A drag moving across the reader: `x` is the pointer's, `width` the text column's.
    pub fn drag(&mut self, x: f32, width: f32, dt: f32) {
        let Some(end) = self.reveal().map(|r| r.end) else { return };
        let (from, x0) = *self.drag.get_or_insert((self.position, x));
        let before = self.position;
        self.position = (from + (x - x0) * end / (SWIPE * width.max(200.0))).clamp(0.0, end);
        self.track(before, dt);
    }

    /// A two-finger swipe moved the reveal in the last moment.
    pub fn swiping(&self) -> bool {
        self.swiped_at.is_some()
    }

    /// A two-finger horizontal swipe of `dx` points: positive shows more.
    pub fn swipe(&mut self, dx: f32, width: f32, dt: f32, now: f64) {
        let Some(end) = self.reveal().map(|r| r.end) else { return };
        let before = self.position;
        self.position = (self.position + dx * end / (SWIPE * width.max(200.0))).clamp(0.0, end);
        self.track(before, dt);
        self.swiped_at = Some(now);
    }

    fn track(&mut self, before: f32, dt: f32) {
        self.target = None;
        if dt > 0.0 {
            let v = (self.position - before) / dt;
            self.velocity = self.velocity * 0.5 + v * 0.5;
        }
    }

    /// The drag ended: the reveal springs back to the end of the last word it fully showed, without
    /// the drag's momentum carrying it further first.
    pub fn release(&mut self) {
        self.drag = None;
        self.swiped_at = None;
        self.velocity = 0.0;
        if let Some(r) = self.reveal() {
            let target = r.settle(self.position);
            self.animate_to(target);
        }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Advances the spring by `dt` seconds; `true` while it is moving. A swipe that has stopped
    /// is released here.
    pub fn tick(&mut self, dt: f32, now: f64, reduce_motion: bool) -> bool {
        if self.swiped_at.is_some_and(|t| now - t > 0.15) {
            self.release();
        }
        if self.swiped_at.is_some() {
            return true;
        }
        let Some(target) = self.target else { return false };
        if reduce_motion {
            self.position = target;
            self.target = None;
            return false;
        }
        let dt = dt.min(1.0 / 30.0);
        let stiffness: f32 = 230.0;
        let damping = 2.0 * stiffness.sqrt() * 0.92;
        let force = stiffness * (target - self.position) - damping * self.velocity;
        self.velocity += force * dt;
        self.position += self.velocity * dt;
        if (target - self.position).abs() < 0.01 && self.velocity.abs() < 0.05 {
            self.position = target;
            self.velocity = 0.0;
            self.target = None;
            return false;
        }
        true
    }

    /// Paints a block of verse text holding some of the passage: its words faded by the reveal
    /// (first letters as hints, with `letters`), and the rest of the chapter dimmed.
    pub fn paint_block(
        &self,
        painter: &Painter,
        block: &Block,
        at: Pos2,
        p: &Palette,
        em: f32,
        letters: bool,
    ) {
        let Some(b) = &self.built else {
            paint_dimmed(painter, at, &block.galley, p.ink);
            return;
        };
        let r = &b.reveal;
        let shown = self.shown();

        let mut galley: Galley = (*block.galley).clone();
        let mut rules: Vec<(Rect, Color32)> = Vec::new();
        let mut base = 0usize;
        for placed in &mut galley.rows {
            let row_pos = placed.pos;
            let row = Arc::make_mut(&mut placed.row);
            for (gi, glyph) in row.glyphs.iter().enumerate() {
                let ch = base + gi;
                // The verse and char of its text this glyph shows, if it is verse text.
                let text = block
                    .runs
                    .iter()
                    .find(|run| (run.at..run.at + run.len).contains(&ch))
                    .map(|run| (run.verse, run.offset + ch - run.at));
                let look = match text {
                    Some((verse, offset)) => match self.verse_slot(verse) {
                        Some(slot) => match r.places[slot].get(offset).copied().flatten() {
                            Some(place) => {
                                Look::Reveal(r.amount(place, shown), r.first_letters[slot][offset])
                            }
                            None => Look::Full,
                        },
                        None => Look::Dim,
                    },
                    // A verse number, or a space the layout added.
                    None => match block.verse_at_char(ch).and_then(|v| self.verse_slot(v)) {
                        Some(_) => Look::Full,
                        None => Look::Dim,
                    },
                };
                if let Look::Reveal(amount, first) = look
                    && letters
                    && !first
                    && amount < 1.0
                    && glyph.chr.is_alphanumeric()
                {
                    let y = at.y + row_pos.y + glyph.pos.y + em * 0.16;
                    let x = at.x + row_pos.x + glyph.pos.x;
                    rules.push((
                        Rect::from_min_size(pos2(x, y), vec2(glyph.advance_width, 1.0)),
                        p.ink_3.gamma_multiply(0.5 * (1.0 - amount)),
                    ));
                }
                if glyph.uv_rect.is_nothing() {
                    continue;
                }
                let vertices = glyph.first_vertex as usize..glyph.first_vertex as usize + 4;
                for v in row.visuals.mesh.vertices.get_mut(vertices).into_iter().flatten() {
                    let color = if v.color == Color32::PLACEHOLDER { p.ink } else { v.color };
                    v.color = match look {
                        Look::Full => color,
                        Look::Dim => color.gamma_multiply(DIM),
                        Look::Reveal(amount, true) if letters => p.ink_2.lerp_to_gamma(color, amount),
                        Look::Reveal(amount, _) => color.gamma_multiply(amount),
                    };
                }
            }
            base += row.glyphs.len() + usize::from(placed.ends_with_newline);
        }
        painter.galley(at, Arc::new(galley), p.ink);
        for (rect, color) in rules {
            painter.rect_filled(rect, 0.0, color);
        }
    }
}

#[derive(Clone, Copy)]
enum Look {
    Full,
    Dim,
    /// How much shows, and whether it is a word's first letter.
    Reveal(f32, bool),
}

/// Paints text around the passage, dimmed.
pub fn paint_dimmed(painter: &Painter, at: Pos2, galley: &Arc<Galley>, color: Color32) {
    painter.add(TextShape::new(at, galley.clone(), color).with_opacity_factor(DIM));
}

/// The bar along the reader's foot while recalling: back, the passage, what to do, and once it is
/// all shown, the grades, each with when the passage would come back.
pub fn bar(
    recall: &Recall,
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    letters: bool,
    study: &bible_core::study::StudyStore,
    actions: &mut Vec<Action>,
) {
    let painter = ui.painter().clone();
    let font = text::UI.font();
    let reference =
        painter.layout_no_wrap(recall.passage.reference(), theme::ui_semibold(text::UI.size), p.ink);
    let count = (recall.total > 1).then(|| {
        painter.layout_no_wrap(format!("{} of {}", recall.index, recall.total), font.clone(), p.ink_3)
    });
    let done = recall.done();
    let unavailable = recall.unavailable();
    let status = if unavailable {
        "Not all of it is in this translation"
    } else if done {
        "How well did you know it?"
    } else if recall.started() {
        "Click for the next word"
    } else {
        "Say it from memory, then click, drag or press Space"
    };
    let status = painter.layout_no_wrap(status.to_string(), font, p.ink_2);

    let now = bible_core::study::now();
    let schedule =
        study.memorised_at(recall.passage.first).map(|m| m.schedule).unwrap_or(recall.passage.schedule);
    let due = schedule.preview(now);
    let labels: Vec<String> = Grade::ALL
        .iter()
        .zip(due)
        .map(|(g, d)| format!("{} · {}", g.name(), interval_label(now, d)))
        .collect();
    let hints = if letters { "Hints: first letters" } else { "Hints: none" };
    // `short`: the grades without when each brings the passage back, for a narrow window.
    let buttons_for = |short: bool| -> Vec<(Button<'_>, Action)> {
        if unavailable {
            return vec![(Button::new(Kind::Primary, "Skip").small(), Action::SkipRecall)];
        }
        if !done {
            return vec![(Button::new(Kind::Plain, hints).small(), Action::ToggleRecallHints)];
        }
        Grade::ALL
            .iter()
            .zip(&labels)
            .map(|(g, label)| {
                // Good is what Return gives.
                let kind = if *g == Grade::Good { Kind::Primary } else { Kind::Secondary };
                let text = if short { g.name() } else { label.as_str() };
                (Button::new(kind, text).small(), Action::RecallGrade(Some(*g)))
            })
            .collect()
    };
    let back = Button::new(Kind::Plain, "").small().icon(crate::icons::chevron_left);

    let gap = 12.0;
    let pad = 12.0;
    let back_w = back.width(&painter);
    let row_w = |buttons: &[(Button<'_>, Action)]| {
        buttons.iter().map(|(b, _)| b.width(&painter)).sum::<f32>()
            + 6.0 * buttons.len().saturating_sub(1) as f32
    };
    let title_w = 10.0 + reference.size().x + count.as_ref().map_or(0.0, |c| 8.0 + c.size().x) + gap * 1.5;
    let status_w = gap * 1.5 + status.size().x;
    // In a narrow window the instruction goes first, then when each grade comes back, then the
    // reference, so nothing overlaps.
    let room = rect.width() - 32.0;
    let mut buttons = buttons_for(false);
    let mut show_status = true;
    let mut show_title = true;
    if pad * 2.0 + back_w + title_w + status_w + row_w(&buttons) > room {
        show_status = false;
    }
    if !show_status && pad * 2.0 + back_w + title_w + row_w(&buttons) > room {
        buttons = buttons_for(true);
    }
    if !show_status && pad * 2.0 + back_w + title_w + row_w(&buttons) > room {
        show_title = false;
    }
    let widths: Vec<f32> = buttons.iter().map(|(b, _)| b.width(&painter)).collect();
    let w = (pad * 2.0
        + back_w
        + if show_title { title_w } else { gap }
        + if show_status { status_w } else { 0.0 }
        + row_w(&buttons))
    .min(room);
    let h = 48.0;
    let panel = Rect::from_center_size(pos2(rect.center().x, rect.bottom() - 24.0 - h / 2.0), vec2(w, h));
    let radius = h / 2.0;
    p.shadows.popover.paint(&painter, panel, radius);
    painter.rect_filled(panel, radius, p.raised);
    let painter = painter.with_clip_rect(panel);

    // Clicks on the bar don't reach the text beneath it.
    ui.interact(panel, Id::unique("recall-bar"), Sense::click_and_drag());
    let cy = panel.center().y;
    let (back_rect, back_resp) = back.show_left(ui, p, panel.left() + pad, cy, Id::unique("recall-back"));
    if back_resp.on_hover_text("Back to reading (Esc)").clicked() {
        actions.push(Action::EndRecall);
    }
    if show_title {
        let mut x = back_rect.right() + 10.0;
        let rw = reference.size().x;
        painter.galley(pos2(x, cy - reference.size().y / 2.0), reference, p.ink);
        x += rw;
        if let Some(c) = count {
            x += 8.0;
            let cw = c.size().x;
            painter.galley(pos2(x, cy - c.size().y / 2.0), c, p.ink_3);
            x += cw;
        }
        if show_status {
            x += gap * 1.5;
            painter.galley(pos2(x, cy - status.size().y / 2.0), status, p.ink_2);
        }
    }

    let mut right = panel.right() - pad + 6.0;
    for (i, (button, action)) in buttons.iter().enumerate().rev() {
        right -= 6.0;
        let (_, resp) = button.show_right(ui, p, right, cy, Id::unique(("recall-bar", i)));
        let resp = if done { resp.on_hover_text(&labels[i]) } else { resp };
        right -= widths[i];
        if resp.clicked() {
            actions.push(action.clone());
        }
    }
}
