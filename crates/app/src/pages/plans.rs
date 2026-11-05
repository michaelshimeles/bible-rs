//! Study › Reading Plans: the plans under way with their calendar and a catalogue to start from
//! (frame 20), and a plan's day — its readings, a reflection, progress and what's next (frame 21).

use std::collections::HashSet;

use eframe::egui::{self, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use bible_core::plans::{self, CustomPlan, Passage, PlanIcon, ResolvedPlan};
use bible_core::study::PlanProgress;
use bible_core::{ChapterIdx, Translation, VerseRef};

use super::{PageEnv, group_label, text};
use crate::actions::Action;
use crate::icons;
use crate::theme::{self, Palette};
use crate::time;
use crate::widgets::{self, BadgeKind, Button, Kind, css_baseline, flow, paint_runs};

const DAY: i64 = 86_400;
const CARD_W: f32 = 300.0;
/// Reading speed for time estimates, in words a minute.
const WPM: usize = 230;

/// What leaving does: `stop_plan` drops the plan and its ticks; starting again begins from day 1.
const LEAVE_TITLE: &str = "Leave this plan?";
const LEAVE_BODY: &str = "Leaving removes this plan from Active and clears its reading ticks. You can start it again from the beginning.";
const LEAVE_BODY_SHORT: &str = "This clears your reading ticks.";
const LEAVE_HINT: &str = "Clears its reading ticks.";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Active,
    Mine,
    Browse,
    Completed,
}

#[derive(Default)]
pub struct State {
    tab: Tab,
    /// The plan and day open in the day view.
    pub open: Option<(String, usize)>,
    /// A plan the reader is writing or editing.
    editor: Option<Editor>,
    /// A destructive action waiting for a second click.
    confirm: Option<Confirm>,
    /// The reflection being written: its verse, the note's text it started from, and the draft.
    reflection: Option<(VerseRef, String, String)>,
    /// The text the last reflection save was sent over, until the next frame shows whether it
    /// landed.
    reflection_sent: Option<(String, String)>,
    /// Plan the user asked to leave, waiting for a confirm (`None`: none).
    leaving: Option<String>,
}

/// Create or edit a plan: title, optional description, a schedule to fill from, and the days.
struct Editor {
    /// `None` while creating.
    id: Option<String>,
    title: String,
    description: String,
    /// Books and passages to turn into days ("John, Matthew 5–7").
    source: String,
    /// 0–2: that many passages a day; 3: finish in `in_days` days.
    pace_idx: usize,
    in_days: String,
    days: Vec<String>,
    error: Option<String>,
    confirm_delete: bool,
}

impl Editor {
    fn blank() -> Self {
        Self {
            id: None,
            title: String::new(),
            description: String::new(),
            source: String::new(),
            pace_idx: 0,
            in_days: "30".into(),
            days: vec![String::new()],
            error: None,
            confirm_delete: false,
        }
    }

    fn from_plan(plan: &CustomPlan) -> Self {
        let name = |b: bible_core::BookId| b.ref_name().to_string();
        Self {
            id: Some(plan.id.clone()),
            title: plan.title.clone(),
            description: plan.description.clone(),
            source: String::new(),
            pace_idx: 0,
            in_days: "30".into(),
            days: plan.days.iter().map(|d| plans::edit_label(d, name)).collect(),
            error: None,
            confirm_delete: false,
        }
    }
}

enum Confirm {
    Delete { id: String, title: String, started: bool },
    Reset { id: String, title: String },
}

/// A plan with its progress, as of today.
struct Under<'a> {
    plan: ResolvedPlan,
    state: &'a PlanProgress,
    reads: HashSet<(u16, u8)>,
    /// Chapters read to the end since the plan began.
    finished: HashSet<ChapterIdx>,
    /// Today's day number (1-based; beyond the plan's length once it's over).
    today: usize,
}

impl Under<'_> {
    fn new<'a>(plan: ResolvedPlan, state: &'a PlanProgress, env: &PageEnv<'_>) -> Under<'a> {
        let reads = env.study.plan_reads(&state.id);
        let finished = env
            .study
            .reading_since(state.started)
            .into_iter()
            .filter(|s| s.finished())
            .filter_map(|s| s.chapter_idx())
            .collect();
        let today = ((time::day_start(bible_core::study::now()) - state.started) / DAY).max(0) as usize + 1;
        Under { plan, state, reads, finished, today }
    }

    fn passage_done(&self, day: usize, i: usize) -> bool {
        let p = self.plan.day(day).get(i);
        self.reads.contains(&(day as u16, i as u8))
            || p.is_some_and(|p| p.verses.is_none() && self.finished.contains(&p.chapter))
    }

    fn day_done(&self, day: usize) -> bool {
        let n = self.plan.day(day).len();
        n > 0 && (0..n).all(|i| self.passage_done(day, i))
    }

    fn done_days(&self) -> usize {
        (1..=self.plan.len()).filter(|d| self.day_done(*d)).count()
    }

    /// Days read in a row, up to today (or yesterday, while today is still to do).
    fn streak(&self) -> usize {
        let mut d = self.today.min(self.plan.len());
        if !self.day_done(d) {
            d = d.saturating_sub(1);
        }
        let mut n = 0;
        while d >= 1 && self.day_done(d) {
            n += 1;
            d -= 1;
        }
        n
    }

    /// The day to continue with: today's, or the first unfinished one before it.
    fn current_day(&self) -> usize {
        let today = self.today.min(self.plan.len()).max(1);
        if self.day_done(today) {
            return today;
        }
        (1..today).find(|d| !self.day_done(*d)).filter(|d| today - d <= 1).unwrap_or(today)
    }
}

fn icon_for(icon: PlanIcon) -> widgets::IconFn {
    match icon {
        PlanIcon::Calendar => icons::calendar,
        PlanIcon::Book => icons::book,
        PlanIcon::Quote => icons::quote,
        PlanIcon::History => icons::history,
        PlanIcon::Flame => icons::flame,
        PlanIcon::Note => icons::note,
    }
}

fn day_label(passages: &[Passage], t: Option<&Translation>) -> String {
    plans::label(passages, |b| {
        t.map(|t| t.meta.ref_name(b).to_string()).unwrap_or_else(|| b.ref_name().to_string())
    })
}

/// Verses in a passage and the words in them.
fn passage_size(p: &Passage, t: Option<&Translation>) -> (usize, usize) {
    let Some(t) = t else { return (0, 0) };
    let (a, b) = p.verses.unwrap_or((1, u8::MAX));
    let verses: Vec<String> =
        t.verses_in(p.chapter).filter(|v| v.number >= a && v.number <= b).map(|v| v.plain_text()).collect();
    (verses.len(), verses.iter().map(|v| v.split_whitespace().count()).sum())
}

fn minutes(words: usize) -> usize {
    words.div_ceil(WPM).max(1)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

impl State {
    /// Shows the list on the `active`, `browse` or `completed` tab (demo captures), closing any
    /// plan's day view, which hides the tabs.
    pub fn open_new_plan(&mut self) {
        self.confirm = None;
        self.editor = Some(Editor::blank());
    }

    /// Reopens the editor after a save the store refused, so the draft is not lost.
    pub fn restore_editor(&mut self, plan: CustomPlan) {
        self.editor = Some(Editor::from_plan(&plan));
        self.tab = Tab::Mine;
    }

    pub fn set_tab(&mut self, name: &str) {
        self.open = None;
        self.editor = None;
        self.confirm = None;
        self.tab = match name {
            "browse" => Tab::Browse,
            "completed" => Tab::Completed,
            "mine" | "my" | "my-plans" => Tab::Mine,
            _ => Tab::Active,
        };
    }

    pub fn show(&mut self, ui: &mut Ui, content: Rect, env: &PageEnv<'_>, actions: &mut Vec<Action>) -> f32 {
        let started = env.study.plans();
        if self.editor.is_some() {
            return self.editor_view(ui, content, env, actions);
        }
        if let Some((id, day)) = self.open.clone()
            && let (Some(plan), Some(state)) =
                (env.study.resolve_plan(&id), started.iter().find(|s| s.id == id))
        {
            let under = Under::new(plan, state, env);
            return self.day_view(ui, content, env, &under, day, actions);
        }
        self.open = None;
        self.list(ui, content, env, &started, actions)
    }

    fn list(
        &mut self,
        ui: &mut Ui,
        content: Rect,
        env: &PageEnv<'_>,
        started: &[PlanProgress],
        actions: &mut Vec<Action>,
    ) -> f32 {
        let p = env.settings.theme.palette();
        let painter = ui.painter().clone();
        let (left, right, top) = (content.left(), content.right(), content.top());
        super::title(&painter, p, left, top, "Reading Plans", "Read a little every day");
        let active: Vec<&PlanProgress> = started.iter().filter(|s| s.finished.is_none()).collect();
        let completed: Vec<&PlanProgress> = started.iter().filter(|s| s.finished.is_some()).collect();
        let q = env.query.trim().to_lowercase();
        let matches = |title: &str, description: &str| {
            q.is_empty() || title.to_lowercase().contains(&q) || description.to_lowercase().contains(&q)
        };
        let mine: Vec<CustomPlan> =
            env.study.custom_plans().into_iter().filter(|p| matches(&p.title, &p.description)).collect();
        let browse: Vec<ResolvedPlan> = plans::catalogue()
            .iter()
            .map(ResolvedPlan::from_builtin)
            .filter(|plan| !active.iter().any(|s| s.id == plan.id))
            .filter(|plan| matches(&plan.title, &plan.description))
            .collect();

        // Tabs, at the right of the header.
        let tabs = [
            (Tab::Active, "Active", active.len()),
            (Tab::Mine, "My Plans", mine.len()),
            (Tab::Browse, "Browse", browse.len()),
            (Tab::Completed, "Completed", completed.len()),
        ];
        let widths: Vec<f32> = tabs
            .iter()
            .map(|(t, l, n)| {
                widgets::pill_tab_width(&painter, l, Some(&n.to_string()), self.tab == *t, false)
            })
            .collect();
        let mut x = right - widths.iter().sum::<f32>() - 4.0 * (tabs.len() as f32 - 1.0);
        for ((t, l, n), w) in tabs.into_iter().zip(widths) {
            let r = Rect::from_min_size(pos2(x, top + 30.0), vec2(w, 28.0));
            let n = n.to_string();
            if widgets::pill_tab(
                ui,
                p,
                r,
                l,
                Some(&n),
                None,
                self.tab == t,
                ui.scope_id().with(("plans-tab", l)),
            )
            .clicked()
            {
                self.tab = t;
            }
            x = r.right() + 4.0;
        }

        let mut y = top + 58.0 + 20.0;
        let t = super::reading_text(env);
        match self.tab {
            Tab::Active => {
                for s in &active {
                    let Some(plan) = env.study.resolve_plan(&s.id) else { continue };
                    if !matches(&plan.title, &plan.description) {
                        continue;
                    }
                    let under = Under::new(plan, s, env);
                    y = self.hero(
                        ui,
                        p,
                        Rect::from_min_size(pos2(left, y), vec2(right - left, 232.0)),
                        env,
                        t.as_deref(),
                        &under,
                        actions,
                    ) + 20.0;
                }
                if active.is_empty() {
                    text(
                        &painter,
                        left,
                        y,
                        20.0,
                        "No plan under way — start one from Browse, or write your own.",
                        theme::ui_font(13.0),
                        p.ink_2,
                        false,
                    );
                    y += 40.0;
                    let (_, resp) = Button::new(Kind::Secondary, "New plan").icon(icons::plus).show_left(
                        ui,
                        p,
                        left,
                        y + 20.0,
                        ui.scope_id().with("new-plan-active"),
                    );
                    if resp.clicked() {
                        self.editor = Some(Editor::blank());
                    }
                    y += 52.0;
                }
                // Everything not under way, finished plans included (they can be read again).
                let others = browse.clone();
                if !others.is_empty() {
                    let heading = if active.is_empty() { "Start a plan" } else { "Start another plan" };
                    text(&painter, left, y, 20.0, heading, theme::ui_semibold(15.0), p.ink, false);
                    let count =
                        format!("{} {}", others.len(), if others.len() == 1 { "plan" } else { "plans" });
                    text(&painter, right, y + 2.0, 16.0, &count, theme::ui_font(12.0), p.ink_3, true);
                    y = self.grid(ui, p, left, right, y + 40.0, &others, "Start", started, actions);
                }
            }
            Tab::Mine => {
                y = self.mine_list(ui, p, left, right, y, &mine, started, env, actions);
            }
            Tab::Browse => {
                y = self.grid(ui, p, left, right, y, &browse, "Start", started, actions);
            }
            Tab::Completed => {
                let done: Vec<ResolvedPlan> = completed
                    .iter()
                    .filter_map(|s| env.study.resolve_plan(&s.id))
                    .filter(|plan| matches(&plan.title, &plan.description))
                    .collect();
                if done.is_empty() {
                    // None finished, or none of them matching the toolbar's field.
                    let (title, body) = if completed.is_empty() {
                        ("No finished plans yet", "Plans you finish show up here, ready to read again.")
                    } else {
                        ("No finished plans match", "Try other words in the search field.")
                    };
                    let empty = super::Empty {
                        glyph: icons::checklist,
                        tone: super::Tone::Accent,
                        title,
                        body,
                        buttons: &[Button::new(Kind::Secondary, "Browse plans")],
                        hints: &[],
                        width: 360.0,
                    };
                    let room = right - left;
                    let h = empty.height(&painter, room);
                    let at = super::optical_top(super::under_toolbar(ui, content), h, y);
                    if empty.show(ui, p, content.center().x, at, room).first().is_some_and(|r| r.clicked()) {
                        self.tab = Tab::Browse;
                    }
                    y = at + h;
                } else {
                    y = self.grid(ui, p, left, right, y, &done, "Restart", started, actions);
                }
            }
        }
        self.confirm_sheet(ui, p, content, actions);
        y
    }

    fn is_leaving(&self, id: &str) -> bool {
        self.leaving.as_deref() == Some(id)
    }

    fn request_leave(&mut self, id: &str) {
        self.leaving = Some(id.to_string());
    }

    fn cancel_leave(&mut self) {
        self.leaving = None;
    }

    fn confirm_leave(&mut self, id: &str, actions: &mut Vec<Action>) {
        actions.push(Action::StopPlan(id.to_string()));
        if self.open.as_ref().is_some_and(|(open_id, _)| open_id == id) {
            self.open = None;
        }
        self.leaving = None;
    }

    /// Leave control on an Active card, on the Continue row so it never covers the day badge.
    fn leave_on_hero(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        plan_id: &str,
        left: f32,
        cy: f32,
        actions: &mut Vec<Action>,
    ) -> bool {
        if self.is_leaving(plan_id) {
            let (cancel_b, cancel) = Button::new(Kind::Plain, "Cancel").small().show_left(
                ui,
                p,
                left,
                cy,
                ui.scope_id().with(("leave-cancel", plan_id)),
            );
            let (_, leave) = Button::new(Kind::Destructive, "Leave plan").small().show_left(
                ui,
                p,
                cancel_b.right() + 8.0,
                cy,
                ui.scope_id().with(("leave-confirm", plan_id)),
            );
            if leave.clicked() {
                self.confirm_leave(plan_id, actions);
                return true;
            }
            if cancel.clicked() {
                self.cancel_leave();
                return true;
            }
            false
        } else {
            let (_, resp) = Button::new(Kind::Secondary, "Leave plan").small().show_left(
                ui,
                p,
                left,
                cy,
                ui.scope_id().with(("leave-plan", plan_id)),
            );
            if resp.clicked() {
                self.request_leave(plan_id);
                return true;
            }
            false
        }
    }

    /// Leave control on a plan's day: "Leave…", then a confirm that explains the ticks go too.
    #[allow(clippy::too_many_arguments)]
    fn leave_on_page(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        left: f32,
        top: f32,
        width: f32,
        plan_id: &str,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let painter = ui.painter().clone();
        let confirming = self.is_leaving(plan_id);
        let inner = width - 32.0;
        let body = if confirming { LEAVE_BODY } else { LEAVE_HINT };
        let mut job = egui::text::LayoutJob::simple(body.into(), theme::ui_font(12.0), p.ink_3, inner);
        job.sections[0].format.line_height = Some(16.0);
        let galley = painter.layout_job(job);
        let h = if confirming { 12.0 + 20.0 + 4.0 + galley.size().y + 12.0 + 24.0 + 12.0 } else { 60.0 };
        let card = Rect::from_min_size(pos2(left, top), vec2(width, h));
        painter.rect_filled(card, 12.0, p.surface);
        painter.rect_stroke(card, 12.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
        if confirming {
            text(&painter, left + 16.0, top + 12.0, 20.0, LEAVE_TITLE, theme::ui_medium(13.0), p.ink, false);
            painter.galley(pos2(left + 16.0, top + 36.0), galley, p.ink_3);
            let cy = card.bottom() - 24.0;
            let (leave_b, leave) = Button::new(Kind::Destructive, "Leave plan").small().show_right(
                ui,
                p,
                card.right() - 16.0,
                cy,
                ui.scope_id().with(("leave-confirm", plan_id)),
            );
            let (_, cancel) = Button::new(Kind::Plain, "Cancel").small().show_right(
                ui,
                p,
                leave_b.left() - 8.0,
                cy,
                ui.scope_id().with(("leave-cancel", plan_id)),
            );
            if leave.clicked() {
                self.confirm_leave(plan_id, actions);
            }
            if cancel.clicked() {
                self.cancel_leave();
            }
        } else {
            text(&painter, left + 16.0, top + 12.0, 20.0, "Leave plan", theme::ui_medium(13.0), p.ink, false);
            text(&painter, left + 16.0, top + 32.0, 16.0, LEAVE_HINT, theme::ui_font(12.0), p.ink_3, false);
            let (_, resp) = Button::new(Kind::Secondary, "Leave…").small().show_right(
                ui,
                p,
                card.right() - 16.0,
                card.center().y,
                ui.scope_id().with(("leave-plan", plan_id)),
            );
            if resp.clicked() {
                self.request_leave(plan_id);
            }
        }
        card.bottom()
    }

    /// The plan under way: its day, progress, what to read and a calendar.
    #[allow(clippy::too_many_arguments)]
    fn hero(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        card: Rect,
        _env: &PageEnv<'_>,
        t: Option<&Translation>,
        u: &Under<'_>,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let painter = ui.painter().clone();
        let resp = ui.interact(card, ui.scope_id().with(("hero", u.plan.id.as_str())), Sense::click());
        painter.rect_filled(card, 20.0, p.sidebar);
        let x = card.left() + 28.0;
        let y = card.top() + 28.0;
        let len = u.plan.len();
        let day = u.today.min(len);
        let badge = if u.today > len { "Last day passed".to_string() } else { format!("Day {day} of {len}") };
        widgets::badge(&painter, p, x, y + 10.0, BadgeKind::Accent, &badge);
        let g = widgets::line(
            &painter,
            &u.plan.title,
            FontId::new(28.0, theme::family(theme::SERIF_HEADING)),
            p.ink,
            -0.01,
        );
        widgets::paint_on_baseline(&painter, x, css_baseline(y + 26.0, 36.0, 28.0, true), g, p.ink);
        let s = time::local(u.state.started);
        let desc =
            format!("{} Started {} {}.", u.plan.description, s.day, MONTHS[(s.month as usize + 11) % 12]);
        let calendar_w = 334.0;
        let text_w = card.width() - 56.0 - 32.0 - calendar_w;
        let desc = super::elide(&painter, &desc, &theme::ui_font(13.0), text_w);
        text(&painter, x, y + 68.0, 20.0, &desc, theme::ui_font(13.0), p.ink_2, false);
        if self.is_leaving(&u.plan.id) {
            text(&painter, x, y + 88.0, 16.0, LEAVE_BODY_SHORT, theme::ui_font(12.0), p.ink_2, false);
        }
        let done = u.done_days();
        let track = Rect::from_min_size(pos2(x, y + 107.0), vec2(320.0, 6.0));
        painter.rect_filled(track, 3.0, p.fill_strong);
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(320.0 * done as f32 / len as f32, 6.0)),
            3.0,
            p.accent,
        );
        text(
            &painter,
            track.right() + 12.0,
            y + 102.0,
            16.0,
            &format!("{done} of {len} days"),
            theme::ui_medium(12.0),
            p.ink_2,
            false,
        );

        // Continue, the streak and today's time.
        let current = u.current_day();
        let label = format!("Continue · {}", day_label(u.plan.day(current), t));
        let cy = y + 136.0 + 20.0;
        let (button, resp_c) = Button::new(Kind::Primary, &label).large().icon(icons::book).show_left(
            ui,
            p,
            x,
            cy,
            ui.scope_id().with(("continue", u.plan.id.as_str())),
        );
        if resp_c.clicked() {
            let next = (0..u.plan.day(current).len()).find(|i| !u.passage_done(current, *i)).unwrap_or(0);
            if let Some(passage) = u.plan.day(current).get(next) {
                open_passage(passage, actions);
            }
        }
        let mut sx = button.right() + 16.0;
        let streak = u.streak();
        if streak > 0 {
            let flame = Rect::from_min_size(pos2(sx, cy - 8.0), vec2(16.0, 16.0));
            icons::heavy(|| {
                icons::flame(&painter, flame, p.swatch(bible_core::study::HighlightColor::Orange))
            });
            let w = text(
                &painter,
                sx + 22.0,
                cy - 8.0,
                16.0,
                &format!("{streak}-day streak"),
                theme::ui_semibold(13.0),
                p.ink,
                false,
            );
            sx += 22.0 + w + 16.0;
        }
        let words: usize = u.plan.day(day.max(1)).iter().map(|ps| passage_size(ps, t).1).sum();
        if words > 0 {
            let mins = format!("~{} min today", minutes(words));
            let w = text(&painter, sx, cy - 8.0, 16.0, &mins, theme::ui_font(13.0), p.ink_2, false);
            sx += w + 16.0;
        }
        // Reserve room for Cancel + Leave plan so a narrow card does not lose the confirm
        // off the right edge (a 620 pt window leaves little after Continue, streak and time).
        let needed = Button::new(Kind::Plain, "Cancel").small().width(&painter)
            + 8.0
            + Button::new(Kind::Destructive, "Leave plan").small().width(&painter);
        let row_right = card.right() - 28.0 - calendar_w - 12.0;
        let (leave_x, leave_cy) = if sx + needed <= row_right { (sx, cy) } else { (x, cy + 36.0) };
        let leave_used = self.leave_on_hero(ui, p, &u.plan.id, leave_x, leave_cy, actions);

        // The calendar: this month of the plan as dots.
        let cal = Rect::from_min_size(
            pos2(card.right() - 28.0 - calendar_w, card.center().y - 63.0),
            vec2(calendar_w, 126.0),
        );
        painter.rect_filled(cal, 14.0, p.paper);
        painter.rect_stroke(cal, 14.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
        let block = (day.max(1) - 1) / 30;
        let first = block * 30 + 1;
        let last = (first + 29).min(len);
        let first_date = time::local(u.state.started + (first as i64 - 1) * DAY);
        group_label(
            &painter,
            p,
            cal.left() + 20.0,
            cal.top() + 20.0,
            MONTHS[(first_date.month as usize + 11) % 12],
        );
        for d in first..=last {
            let k = d - first;
            let c = pos2(
                cal.left() + 20.0 + 7.0 + 20.0 * (k % 15) as f32,
                cal.top() + 46.0 + 7.0 + 20.0 * (k / 15) as f32,
            );
            if d == u.today {
                painter.circle_filled(c, 7.0, p.raised);
                painter.circle_stroke(c, 6.0, Stroke::new(2.0, p.accent));
            } else if u.day_done(d) {
                painter.circle_filled(c, 7.0, p.accent);
            } else if d < u.today {
                painter.circle_stroke(c, 6.25, Stroke::new(1.5, p.ink_3));
            } else {
                painter.circle_filled(c, 7.0, p.fill_strong);
            }
        }
        let mut lx = cal.left() + 20.0;
        let ly = cal.top() + 90.0;
        for (label, kind) in [("Read", 0), ("Missed", 1), ("Today", 2)] {
            let c = pos2(lx + 4.0, ly + 8.0);
            match kind {
                0 => painter.circle_filled(c, 4.0, p.accent),
                1 => painter.circle_stroke(c, 3.25, Stroke::new(1.5, p.ink_3)),
                _ => painter.circle_stroke(c, 3.0, Stroke::new(2.0, p.accent)),
            };
            let w = text(&painter, lx + 12.0, ly, 16.0, label, theme::ui_medium(11.0), p.ink_3, false);
            lx += 12.0 + w + 12.0;
        }
        if resp.clicked() && !leave_used {
            self.open = Some((u.plan.id.to_string(), current));
        }
        card.bottom()
    }

    /// Plan cards, three to a row.
    #[allow(clippy::too_many_arguments)]
    fn grid(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        left: f32,
        right: f32,
        top: f32,
        list: &[ResolvedPlan],
        action: &str,
        started: &[PlanProgress],
        actions: &mut Vec<Action>,
    ) -> f32 {
        let painter = ui.painter().clone();
        let cols = (((right - left) + 16.0) / (CARD_W + 16.0)).floor().max(1.0) as usize;
        let mut y = top;
        for row in list.chunks(cols) {
            let heights: Vec<f32> = row
                .iter()
                .map(|plan| {
                    let mut job = egui::text::LayoutJob::simple(
                        plan.description.clone(),
                        theme::ui_font(13.0),
                        p.ink_2,
                        CARD_W - 32.0,
                    );
                    job.sections[0].format.line_height = Some(20.0);
                    104.0 + painter.layout_job(job).size().y + 16.0
                })
                .collect();
            let h = heights.iter().copied().fold(0.0, f32::max);
            for (i, plan) in row.iter().enumerate() {
                let card = Rect::from_min_size(pos2(left + i as f32 * (CARD_W + 16.0), y), vec2(CARD_W, h));
                let hovered = ui.rect_contains_pointer(card);
                if hovered {
                    p.shadows.popover.paint(&painter, card, 14.0);
                } else {
                    painter.add(
                        egui::epaint::RectShape::filled(
                            card.translate(vec2(0.0, 1.0)),
                            14.0,
                            Color32::from_black_alpha(10),
                        )
                        .with_blur_width(4.0),
                    );
                    painter.rect_stroke(card, 14.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
                }
                painter.rect_filled(card, 14.0, p.surface);
                let tile = Rect::from_min_size(card.min + vec2(16.0, 16.0), vec2(40.0, 40.0));
                painter.rect_filled(tile, 10.0, p.accent_soft);
                (icon_for(plan.icon))(
                    &painter,
                    Rect::from_center_size(tile.center(), vec2(20.0, 20.0)),
                    p.accent,
                );
                let (_, resp) = Button::new(Kind::Secondary, action).small().show_right(
                    ui,
                    p,
                    card.right() - 16.0,
                    card.top() + 36.0,
                    ui.scope_id().with(("start-plan", plan.id.as_str())),
                );
                if resp.clicked() {
                    if action == "Restart" || started.iter().any(|s| s.id == plan.id) {
                        self.confirm =
                            Some(Confirm::Reset { id: plan.id.clone(), title: plan.title.clone() });
                    } else {
                        actions.push(Action::StartPlan(plan.id.clone()));
                        self.tab = Tab::Active;
                    }
                }
                let x = card.left() + 16.0;
                text(
                    &painter,
                    x,
                    card.top() + 60.0,
                    20.0,
                    &plan.title,
                    theme::ui_semibold(15.0),
                    p.ink,
                    false,
                );
                text(
                    &painter,
                    x,
                    card.top() + 84.0,
                    16.0,
                    &plan.meta,
                    theme::ui_medium(12.0),
                    p.ink_3,
                    false,
                );
                let mut job = egui::text::LayoutJob::simple(
                    plan.description.clone(),
                    theme::ui_font(13.0),
                    p.ink_2,
                    CARD_W - 32.0,
                );
                job.sections[0].format.line_height = Some(20.0);
                painter.galley(pos2(x, card.top() + 104.0), painter.layout_job(job), p.ink_2);
            }
            y += h + 16.0;
        }
        y
    }

    /// A plan's day (frame 21).
    fn day_view(
        &mut self,
        ui: &mut Ui,
        content: Rect,
        env: &PageEnv<'_>,
        u: &Under<'_>,
        day: usize,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let p = env.settings.theme.palette();
        let painter = ui.painter().clone();
        let (left, right, top) = (content.left(), content.right(), content.top());
        let t = super::reading_text(env);
        let t = t.as_deref();
        let len = u.plan.len();
        let day = day.clamp(1, len.max(1));
        let passages = u.plan.day(day);

        // Breadcrumb.
        let w = text(&painter, left, top, 16.0, "Reading Plans", theme::ui_medium(13.0), p.accent, false);
        let crumb = Rect::from_min_size(pos2(left, top), vec2(w, 16.0));
        if ui.interact(crumb, ui.scope_id().with("crumb"), Sense::click()).clicked() {
            self.open = None;
        }
        let chev = Rect::from_min_size(pos2(left + w + 6.0, top + 2.0), vec2(12.0, 12.0));
        icons::disclosure(&painter, chev, p.ink_3);
        text(&painter, chev.right() + 6.0, top, 16.0, &u.plan.title, theme::ui_medium(13.0), p.ink_2, false);

        // Title, date and time; previous and next day.
        let y = top + 36.0;
        let label = day_label(passages, t);
        let g = widgets::line(
            &painter,
            &format!("Day {day} · {label}"),
            FontId::new(28.0, theme::family(theme::SERIF_HEADING)),
            p.ink,
            -0.01,
        );
        widgets::paint_on_baseline(&painter, left, css_baseline(y, 36.0, 28.0, true), g, p.ink);
        let date = time::local(u.state.started + (day as i64 - 1) * DAY);
        let words: usize = passages.iter().map(|ps| passage_size(ps, t).1).sum();
        let mut sub = format!(
            "{}, {} {}",
            time::weekday(date.weekday),
            date.day,
            MONTHS[(date.month as usize + 11) % 12]
        );
        if words > 0 {
            sub.push_str(&format!(" · about {} minutes", minutes(words)));
        }
        text(&painter, left, y + 38.0, 20.0, &sub, theme::ui_font(13.0), p.ink_2, false);
        let prev = Rect::from_min_size(pos2(right - 72.0, y + 26.0), vec2(32.0, 32.0));
        if day > 1
            && widgets::round_button(ui, p, prev, icons::chevron_left, ui.scope_id().with("prev-day"))
                .clicked()
        {
            self.open = Some((u.plan.id.to_string(), day - 1));
        }
        let next = prev.translate(vec2(40.0, 0.0));
        if day < len
            && widgets::round_button(ui, p, next, icons::chevron_right, ui.scope_id().with("next-day"))
                .clicked()
        {
            self.open = Some((u.plan.id.to_string(), day + 1));
        }

        // The day strip: nine days around this one.
        let y = top + 114.0;
        let start = day.saturating_sub(4).max(1).min(len.saturating_sub(8).max(1));
        for (k, d) in (start..=(start + 8).min(len)).enumerate() {
            let cell = Rect::from_min_size(pos2(left + 60.0 * k as f32, y), vec2(56.0, 68.0));
            let resp = ui.interact(cell, ui.scope_id().with(("strip", d)), Sense::click());
            let selected = d == day;
            if selected {
                painter.rect_filled(cell, 12.0, p.accent_soft);
            } else if let Some(fill) = widgets::hover_fill(ui, &resp, p, false) {
                painter.rect_filled(cell, 12.0, fill);
            }
            let dd = time::local(u.state.started + (d as i64 - 1) * DAY);
            let wd = time::weekday_short(dd.weekday);
            let ink = if d == u.today { p.accent } else { p.ink_3 };
            let g = widgets::line(&painter, wd, theme::ui_medium(11.0), ink, 0.0);
            let gx = cell.center().x - g.size().x / 2.0;
            widgets::paint_on_baseline(
                &painter,
                gx,
                css_baseline(cell.top() + 8.0, 16.0, 11.0, false),
                g,
                ink,
            );
            let c = pos2(cell.center().x, cell.top() + 44.0);
            if selected {
                painter.circle_filled(c, 16.0, p.accent);
                painter.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    d.to_string(),
                    theme::ui_semibold(13.0),
                    p.on_accent,
                );
            } else if u.day_done(d) {
                painter.circle_filled(c, 16.0, p.accent.gamma_multiply(0.55));
                icons::check(&painter, Rect::from_center_size(c, vec2(12.0, 12.0)), p.on_accent);
            } else {
                painter.circle_filled(c, 16.0, p.fill);
                painter.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    d.to_string(),
                    theme::ui_semibold(13.0),
                    p.ink_2,
                );
            }
            if resp.clicked() {
                self.open = Some((u.plan.id.to_string(), d));
            }
        }

        // Readings.
        let side_w = 280.0;
        let main_w = (right - left - 32.0 - side_w).max(360.0);
        let mut ry = top + 202.0;
        let first_open = (0..passages.len()).find(|i| !u.passage_done(day, *i));
        for (i, ps) in passages.iter().enumerate() {
            let row = Rect::from_min_size(pos2(left, ry), vec2(main_w, 70.0));
            let done = u.passage_done(day, i);
            let mut pp = painter.clone();
            if done {
                pp.set_opacity(0.75);
            }
            pp.rect_filled(row, 14.0, p.surface);
            pp.rect_stroke(row, 14.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
            let check = Rect::from_min_size(pos2(row.left() + 16.0, row.center().y - 8.0), vec2(16.0, 16.0));
            let cr = ui.interact(check.expand(6.0), ui.scope_id().with(("tick", day, i)), Sense::click());
            if done {
                pp.rect_filled(check, 4.0, p.accent);
                icons::check(&pp, Rect::from_center_size(check.center(), vec2(12.0, 12.0)), p.on_accent);
            } else {
                pp.rect_filled(check, 4.0, p.field);
                let ring = if cr.hovered() { p.ink_3 } else { p.border };
                pp.rect_stroke(check, 4.0, Stroke::new(1.0, ring), StrokeKind::Inside);
            }
            if cr.clicked() {
                actions.push(Action::SetPlanRead {
                    plan: u.plan.id.to_string(),
                    day: day as u16,
                    passage: i as u8,
                    read: !done,
                });
            }
            // The button and size at the right; the title and opening line in what's left.
            let primary = Some(i) == first_open;
            let (kind, label) = if done {
                (Kind::Plain, "Read again")
            } else if primary {
                (Kind::Primary, "Read")
            } else {
                (Kind::Plain, "Read")
            };
            let (b, resp) = Button::new(kind, label).small().show_right(
                ui,
                p,
                row.right() - 16.0,
                row.center().y,
                ui.scope_id().with(("read", day, i)),
            );
            if done {
                // Dim the button with the row.
                pp.rect_filled(b, 12.0, p.surface.gamma_multiply(0.25));
            }
            if resp.clicked() {
                open_passage(ps, actions);
            }
            let (verses, words) = passage_size(ps, t);
            let meta = format!("{verses} verses · {} min", minutes(words));
            let mw = text(
                &pp,
                b.left() - 14.0,
                row.center().y - 8.0,
                16.0,
                &meta,
                theme::ui_font(12.0),
                p.ink_3,
                true,
            );
            let tx = check.right() + 14.0;
            let tw = b.left() - 14.0 - mw - 14.0 - tx;
            let title = day_label(std::slice::from_ref(ps), t);
            text(&pp, tx, row.top() + 14.0, 20.0, &title, theme::ui_semibold(15.0), p.ink, false);
            let first = t
                .and_then(|t| t.verses_in(ps.chapter).find(|v| ps.verses.is_none_or(|(a, _)| v.number >= a)))
                .map(|v| v.plain_text())
                .unwrap_or_default();
            let serif = FontId::new(13.0, theme::family(theme::SERIF));
            if let Some(l) = flow(&pp, &first, &serif, 10.0, tw, 1).first() {
                paint_runs(&pp, tx, css_baseline(row.top() + 36.0, 20.0, 13.0, true), l, p.ink_2);
            }
            ry += 70.0 + 12.0;
        }

        // Reflection: a note on the day's first verse.
        let ry = ry + 0.0;
        text(&painter, left, ry, 20.0, "Reflection", theme::ui_semibold(13.0), p.ink, false);
        let bottom = match passages.first() {
            Some(ps) => {
                let v =
                    VerseRef::new(ps.chapter.book(), ps.chapter.chapter(), ps.verses.map_or(1, |(a, _)| a));
                self.reflection(
                    ui,
                    p,
                    Rect::from_min_size(pos2(left, ry + 28.0), vec2(main_w, 0.0)),
                    env,
                    t,
                    v,
                    actions,
                )
            }
            None => ry + 28.0,
        };

        // Side: progress, up next, the reminder.
        let sx = right - side_w;
        let card = Rect::from_min_size(pos2(sx, top + 202.0), vec2(side_w, 184.0));
        p.shadows.panel.paint(&painter, card, 16.0);
        painter.rect_filled(card, 16.0, p.sidebar);
        let done = u.done_days();
        let frac = done as f32 / len.max(1) as f32;
        let c = pos2(card.center().x, card.top() + 20.0 + 60.0);
        painter.circle_stroke(c, 55.0, Stroke::new(10.0, p.fill_strong));
        if frac > 0.0 {
            let n = (frac * 96.0).ceil().max(2.0) as usize;
            let pts: Vec<egui::Pos2> = (0..=n)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n as f32;
                    c + vec2(a.cos(), a.sin()) * 55.0
                })
                .collect();
            painter.circle_filled(pts[0], 5.0, p.accent);
            painter.circle_filled(*pts.last().unwrap(), 5.0, p.accent);
            painter.add(egui::Shape::line(pts, Stroke::new(10.0, p.accent)));
        }
        let pct = format!("{} %", (frac * 100.0).round());
        let g = widgets::line(&painter, &pct, FontId::new(28.0, theme::family(theme::SERIF)), p.ink, 0.0);
        let gx = c.x - g.size().x / 2.0;
        widgets::paint_on_baseline(&painter, gx, css_baseline(card.top() + 40.0, 36.0, 28.0, true), g, p.ink);
        let days = format!("{done} of {len} days");
        let g = widgets::line(&painter, &days, theme::ui_medium(12.0), p.ink_2, 0.0);
        let gx = c.x - g.size().x / 2.0;
        widgets::paint_on_baseline(
            &painter,
            gx,
            css_baseline(card.top() + 148.0, 16.0, 12.0, false),
            g,
            p.ink_2,
        );

        let mut y = card.bottom() + 16.0;
        if day < len {
            text(&painter, sx, y, 20.0, "Up next", theme::ui_semibold(13.0), p.ink, false);
            y += 22.0;
            for d in (day + 1)..=(day + 3).min(len) {
                text(
                    &painter,
                    sx,
                    y + 6.0,
                    16.0,
                    &format!("Day {d}"),
                    theme::ui_medium(12.0),
                    p.ink_3,
                    false,
                );
                text(
                    &painter,
                    sx + 56.0,
                    y + 4.0,
                    20.0,
                    &day_label(u.plan.day(d), t),
                    theme::ui_font(13.0),
                    p.ink,
                    false,
                );
                y += 30.0;
            }
            y += 14.0;
        }
        let reminder = Rect::from_min_size(pos2(sx, y), vec2(side_w, 60.0));
        painter.rect_filled(reminder, 12.0, p.surface);
        painter.rect_stroke(reminder, 12.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
        text(&painter, sx + 16.0, y + 12.0, 20.0, "Daily reminder", theme::ui_medium(13.0), p.ink, false);
        text(
            &painter,
            sx + 16.0,
            y + 32.0,
            16.0,
            "7:00 AM · while the app is open",
            theme::ui_font(12.0),
            p.ink_3,
            false,
        );
        let mut on = env.settings.plan_reminder;
        if widgets::toggle(
            ui,
            p,
            pos2(reminder.right() - 16.0, reminder.center().y),
            ui.scope_id().with("reminder"),
            &mut on,
        )
        .clicked()
        {
            actions.push(Action::SetPlanReminder(on));
        }
        let leave_bottom =
            self.leave_on_page(ui, p, sx, reminder.bottom() + 16.0, side_w, &u.plan.id, actions);
        bottom.max(leave_bottom)
    }

    /// The reflection card: the verse quoted, and a note on it.
    #[allow(clippy::too_many_arguments)]
    fn reflection(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        at: Rect,
        env: &PageEnv<'_>,
        t: Option<&Translation>,
        v: VerseRef,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let painter = ui.painter().clone();
        let saved = env.study.note(v).map(|n| n.body.clone()).unwrap_or_default();
        // The save didn't land (the note isn't the text sent): keep the draft unsaved.
        if let Some((sent, body)) = self.reflection_sent.take()
            && let Some((rv, base, _)) = &mut self.reflection
            && *rv == v
            && saved != body
        {
            *base = sent;
        }
        // New, or a sync changed the note under a draft with nothing typed: start from the note.
        if self
            .reflection
            .as_ref()
            .is_none_or(|(rv, base, draft)| *rv != v || (*base != saved && draft == base))
        {
            self.reflection = Some((v, saved.clone(), saved.clone()));
        }
        let Some((_, base, draft)) = &mut self.reflection else { return at.top() };
        let x = at.left() + 16.0;
        let w = at.width() - 32.0;
        let verse = t.and_then(|t| t.find(v)).map(|v| v.plain_text()).unwrap_or_default();
        let mut job = egui::text::LayoutJob::simple(
            verse,
            FontId::new(14.0, theme::family(theme::SERIF_ITALIC)),
            p.ink_2,
            w,
        );
        job.sections[0].format.line_height = Some(20.0);
        let quote = painter.layout_job(job);
        let id = egui::Id::unique(("reflection", v.key()));
        let focused = ui.memory(|m| m.has_focus(id));
        let editor_h = 20.0f32.max(ui.ctx().data(|d| d.get_temp::<f32>(id.with("h"))).unwrap_or(20.0));
        let h = 16.0 + 16.0 + 10.0 + quote.size().y + 10.0 + editor_h + 10.0 + 24.0 + 16.0;
        let card = Rect::from_min_size(at.min, vec2(at.width(), h));
        if focused {
            p.shadows.popover.paint(&painter, card, 12.0);
            painter.rect_filled(card.expand(4.0), 16.0, p.focus);
            painter.rect_filled(card.expand(1.0), 13.0, p.accent);
        } else {
            painter.rect_stroke(card, 12.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
        }
        painter.rect_filled(card, 12.0, p.surface);
        let y = card.top() + 16.0;
        icons::heavy(|| icons::note(&painter, Rect::from_min_size(pos2(x, y), vec2(16.0, 16.0)), p.accent));
        let rw = text(&painter, x + 22.0, y, 16.0, &v.display(), theme::ui_semibold(13.0), p.ink, false);
        let abbrev = t.map(|t| t.meta.abbrev.clone()).unwrap_or_default();
        text(&painter, x + 22.0 + rw + 6.0, y, 16.0, &abbrev, theme::ui_medium(11.0), p.ink_3, false);
        let when = env.study.note(v).map(|n| time::ago(n.updated)).unwrap_or_else(|| "Today".into());
        text(&painter, card.right() - 16.0, y, 16.0, &when, theme::ui_font(12.0), p.ink_3, true);
        let qh = quote.size().y;
        painter.galley(pos2(x, y + 26.0), quote, p.ink_2);
        let ey = y + 26.0 + qh + 10.0;
        let edit = Rect::from_min_size(pos2(x, ey), vec2(w, editor_h));
        let resp = ui
            .scope_builder(egui::UiBuilder::new().max_rect(edit.with_max_y(edit.top() + 400.0)), |ui| {
                ui.visuals_mut().text_cursor.stroke = Stroke::new(1.5, p.accent);
                ui.add(
                    egui::TextEdit::multiline(draft)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(w)
                        .desired_rows(1)
                        .font(theme::ui_font(14.0))
                        .text_color(p.ink)
                        .hint_text(
                            egui::RichText::new("Write a note…").font(theme::ui_font(14.0)).color(p.ink_3),
                        ),
                )
            })
            .inner;
        ui.ctx().data_mut(|d| d.insert_temp(id.with("h"), resp.rect.height()));
        let fy = card.bottom() - 16.0 - 24.0;
        text(&painter, x, fy + 4.0, 16.0, "Markdown supported", theme::ui_font(12.0), p.ink_3, false);
        let changed = *draft != saved;
        let (save, sr) = Button::new(Kind::Primary, "Save").small().enabled(changed).show_right(
            ui,
            p,
            card.right() - 16.0,
            fy + 12.0,
            ui.scope_id().with("save-reflection"),
        );
        let (_, cr) = Button::new(Kind::Plain, "Cancel").small().enabled(changed).show_right(
            ui,
            p,
            save.left() - 8.0,
            fy + 12.0,
            ui.scope_id().with("cancel-reflection"),
        );
        if sr.clicked() {
            let sent = std::mem::replace(base, draft.clone());
            actions.push(Action::SaveNote(v, draft.clone(), sent.clone()));
            self.reflection_sent = Some((sent, draft.clone()));
        }
        if cr.clicked() {
            *base = saved.clone();
            *draft = saved;
            ui.memory_mut(|m| m.surrender_focus(id));
        }
        card.bottom()
    }

    #[allow(clippy::too_many_arguments)]
    fn mine_list(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        left: f32,
        right: f32,
        top: f32,
        mine: &[CustomPlan],
        started: &[PlanProgress],
        env: &PageEnv<'_>,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let painter = ui.painter().clone();
        let (_, new) = Button::new(Kind::Primary, "New plan").icon(icons::plus).show_left(
            ui,
            p,
            left,
            top + 20.0,
            ui.scope_id().with("new-plan"),
        );
        if new.clicked() {
            self.confirm = None;
            self.editor = Some(Editor::blank());
        }
        let mut y = top + 52.0;
        if mine.is_empty() {
            text(
                &painter,
                left,
                y,
                20.0,
                "No plans of your own yet — write one from books you choose, or list each day's passages.",
                theme::ui_font(13.0),
                p.ink_2,
                false,
            );
            return y + 40.0;
        }
        let w = right - left;
        for plan in mine {
            let progress = started.iter().find(|s| s.id == plan.id);
            let card = Rect::from_min_size(pos2(left, y), vec2(w, 132.0));
            painter.rect_filled(card, 14.0, p.surface);
            painter.rect_stroke(card, 14.0, Stroke::new(0.5, p.border), StrokeKind::Outside);
            let tile = Rect::from_min_size(card.min + vec2(16.0, 16.0), vec2(40.0, 40.0));
            painter.rect_filled(tile, 10.0, p.accent_soft);
            icons::note(&painter, Rect::from_center_size(tile.center(), vec2(20.0, 20.0)), p.accent);
            let x = tile.right() + 14.0;
            text(&painter, x, card.top() + 16.0, 20.0, &plan.title, theme::ui_semibold(15.0), p.ink, false);
            text(&painter, x, card.top() + 38.0, 16.0, &plan.meta(), theme::ui_medium(12.0), p.ink_3, false);
            if !plan.description.is_empty() {
                let desc =
                    super::elide(&painter, &plan.description, &theme::ui_font(13.0), (w - 220.0).max(120.0));
                text(&painter, x, card.top() + 58.0, 20.0, &desc, theme::ui_font(13.0), p.ink_2, false);
            }
            let by = card.bottom() - 28.0;
            let rx = card.right() - 16.0;
            let (del_r, del) = Button::new(Kind::Ghost, "Delete").small().show_right(
                ui,
                p,
                rx,
                by,
                ui.scope_id().with(("mine-del", plan.id.as_str())),
            );
            if del.clicked() {
                self.confirm = Some(Confirm::Delete {
                    id: plan.id.clone(),
                    title: plan.title.clone(),
                    started: progress.is_some(),
                });
            }
            let (dup_r, dup) = Button::new(Kind::Plain, "Duplicate").small().show_right(
                ui,
                p,
                del_r.left() - 8.0,
                by,
                ui.scope_id().with(("mine-dup", plan.id.as_str())),
            );
            if dup.clicked() {
                actions.push(Action::DuplicateCustomPlan(plan.id.clone()));
            }
            let (edit_r, edit) = Button::new(Kind::Plain, "Edit").small().show_right(
                ui,
                p,
                dup_r.left() - 8.0,
                by,
                ui.scope_id().with(("mine-edit", plan.id.as_str())),
            );
            if edit.clicked() {
                self.confirm = None;
                self.editor = Some(Editor::from_plan(plan));
            }
            let (label, restart) = match progress {
                Some(s) if s.finished.is_some() => ("Restart", true),
                Some(_) => ("Continue", false),
                None => ("Start", false),
            };
            let (_, go) = Button::new(Kind::Secondary, label).small().show_right(
                ui,
                p,
                edit_r.left() - 8.0,
                by,
                ui.scope_id().with(("mine-go", plan.id.as_str())),
            );
            if go.clicked() {
                if restart {
                    self.confirm = Some(Confirm::Reset { id: plan.id.clone(), title: plan.title.clone() });
                } else if let Some(s) = progress {
                    let under = Under::new(plan.resolved(), s, env);
                    self.open = Some((plan.id.clone(), under.current_day()));
                } else {
                    actions.push(Action::StartPlan(plan.id.clone()));
                    self.tab = Tab::Active;
                }
            }
            y = card.bottom() + 12.0;
        }
        y
    }

    fn editor_view(
        &mut self,
        ui: &mut Ui,
        content: Rect,
        env: &PageEnv<'_>,
        actions: &mut Vec<Action>,
    ) -> f32 {
        let p = env.settings.theme.palette();
        let painter = ui.painter().clone();
        let (left, right, top) = (content.left(), content.right(), content.top());
        let w = (right - left).min(640.0);
        let x = left;

        let creating = self.editor.as_ref().is_some_and(|e| e.id.is_none());
        let crumb = if creating { "New plan" } else { "Edit plan" };
        let cw = text(&painter, left, top, 16.0, "Reading Plans", theme::ui_medium(13.0), p.accent, false);
        let crumb_hit = ui.interact(
            Rect::from_min_size(pos2(left, top), vec2(cw, 16.0)),
            ui.scope_id().with("editor-crumb"),
            Sense::click(),
        );
        let chev = Rect::from_min_size(pos2(left + cw + 6.0, top + 2.0), vec2(12.0, 12.0));
        icons::disclosure(&painter, chev, p.ink_3);
        text(&painter, chev.right() + 6.0, top, 16.0, crumb, theme::ui_medium(13.0), p.ink_2, false);

        let mut close = crumb_hit.clicked();
        let mut y = top + 40.0;
        let mut save_plan = None;
        let mut delete_id = None;
        if let Some(editor) = self.editor.as_mut() {
            text(&painter, x, y, 16.0, "Title", theme::ui_medium(12.0), p.ink_2, false);
            y += 22.0;
            widgets::text_field(
                ui,
                p,
                Rect::from_min_size(pos2(x, y), vec2(w, 36.0)),
                &mut editor.title,
                "Wisdom in a month",
                None,
                false,
                false,
                ui.scope_id().with("plan-title"),
            );
            y += 48.0;
            text(&painter, x, y, 16.0, "Description", theme::ui_medium(12.0), p.ink_2, false);
            y += 22.0;
            widgets::text_field(
                ui,
                p,
                Rect::from_min_size(pos2(x, y), vec2(w, 36.0)),
                &mut editor.description,
                "Optional — what this plan is for",
                None,
                false,
                false,
                ui.scope_id().with("plan-desc"),
            );
            y += 56.0;
            text(&painter, x, y, 20.0, "Build a schedule", theme::ui_semibold(13.0), p.ink, false);
            y += 24.0;
            text(
                &painter,
                x,
                y,
                20.0,
                "Books, chapters, or verse ranges — “John, Matthew 5–7, Proverbs 3:5–6”.",
                theme::ui_font(13.0),
                p.ink_2,
                false,
            );
            y += 28.0;
            widgets::text_field(
                ui,
                p,
                Rect::from_min_size(pos2(x, y), vec2(w, 36.0)),
                &mut editor.source,
                "John, Matthew 5–7",
                None,
                false,
                false,
                ui.scope_id().with("plan-source"),
            );
            y += 48.0;
            text(&painter, x, y + 8.0, 16.0, "Pace", theme::ui_medium(12.0), p.ink_2, false);
            let labels = ["1 a day", "2 a day", "3 a day", "In N days"];
            let (track, picked) = widgets::segmented_tight(
                ui,
                p,
                pos2(x + w, y + 16.0),
                ui.scope_id().with("plan-pace"),
                &labels,
                editor.pace_idx,
            );
            if let Some(i) = picked {
                editor.pace_idx = i;
            }
            y = track.bottom() + 12.0;
            if editor.pace_idx == 3 {
                text(&painter, x, y + 8.0, 16.0, "Days", theme::ui_medium(12.0), p.ink_2, false);
                widgets::text_field(
                    ui,
                    p,
                    Rect::from_min_size(pos2(x + 48.0, y), vec2(80.0, 36.0)),
                    &mut editor.in_days,
                    "30",
                    None,
                    false,
                    false,
                    ui.scope_id().with("plan-ndays"),
                );
                y += 48.0;
            }
            let (_, fill) = Button::new(Kind::Secondary, "Fill days").show_left(
                ui,
                p,
                x,
                y + 20.0,
                ui.scope_id().with("plan-fill"),
            );
            if fill.clicked() {
                match plans::parse_passages(&editor.source) {
                    Ok(passages) => {
                        let pace = if editor.pace_idx < 3 {
                            plans::Pace::ChaptersPerDay((editor.pace_idx as u16) + 1)
                        } else {
                            plans::Pace::InDays(editor.in_days.trim().parse().unwrap_or(0))
                        };
                        match plans::schedule(&passages, pace) {
                            Ok(days) => {
                                let name = |b: bible_core::BookId| b.ref_name().to_string();
                                editor.days = days.iter().map(|d| plans::label(d, name)).collect();
                                if editor.days.is_empty() {
                                    editor.days.push(String::new());
                                }
                                editor.error = None;
                            }
                            Err(e) => editor.error = Some(e),
                        }
                    }
                    Err(e) => editor.error = Some(e),
                }
            }
            y += 52.0;
            text(
                &painter,
                x,
                y,
                20.0,
                &format!("Days ({})", editor.days.len()),
                theme::ui_semibold(13.0),
                p.ink,
                false,
            );
            y += 28.0;
            let mut remove = None;
            let mut swap = None;
            for i in 0..editor.days.len() {
                text(
                    &painter,
                    x,
                    y + 10.0,
                    16.0,
                    &format!("{}", i + 1),
                    theme::ui_medium(12.0),
                    p.ink_3,
                    false,
                );
                widgets::text_field(
                    ui,
                    p,
                    Rect::from_min_size(pos2(x + 28.0, y), vec2(w - 28.0 - 108.0, 36.0)),
                    &mut editor.days[i],
                    "John 1 · Proverbs 3",
                    None,
                    false,
                    false,
                    ui.scope_id().with(("plan-day", i)),
                );
                let bx = x + w;
                if i > 0 {
                    let up = Rect::from_min_size(pos2(bx - 108.0, y + 4.0), vec2(28.0, 28.0));
                    if widgets::round_button(ui, p, up, icons::chevron_up, ui.scope_id().with(("day-up", i)))
                        .clicked()
                    {
                        swap = Some((i - 1, i));
                    }
                }
                if i + 1 < editor.days.len() {
                    let down = Rect::from_min_size(pos2(bx - 76.0, y + 4.0), vec2(28.0, 28.0));
                    if widgets::round_button(
                        ui,
                        p,
                        down,
                        icons::chevron_down,
                        ui.scope_id().with(("day-down", i)),
                    )
                    .clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                }
                let trash = Rect::from_min_size(pos2(bx - 44.0, y + 4.0), vec2(28.0, 28.0));
                if widgets::round_button(ui, p, trash, icons::trash, ui.scope_id().with(("day-del", i)))
                    .clicked()
                {
                    remove = Some(i);
                }
                y += 44.0;
            }
            if let Some((a, b)) = swap {
                editor.days.swap(a, b);
            }
            if let Some(i) = remove {
                editor.days.remove(i);
                if editor.days.is_empty() {
                    editor.days.push(String::new());
                }
            }
            let (_, add) = Button::new(Kind::Plain, "Add day").icon(icons::plus).show_left(
                ui,
                p,
                x,
                y + 16.0,
                ui.scope_id().with("add-day"),
            );
            if add.clicked() {
                editor.days.push(String::new());
            }
            y += 48.0;
            if let Some(err) = &editor.error {
                text(&painter, x, y, 20.0, err, theme::ui_medium(13.0), p.danger, false);
                y += 28.0;
            }
            let (_, cancel) = Button::new(Kind::Plain, "Cancel").show_left(
                ui,
                p,
                x,
                y + 20.0,
                ui.scope_id().with("plan-cancel"),
            );
            if cancel.clicked() {
                close = true;
            }
            let (_, save) = Button::new(Kind::Primary, "Save plan").show_left(
                ui,
                p,
                x + 100.0,
                y + 20.0,
                ui.scope_id().with("plan-save"),
            );
            if save.clicked() {
                match Self::plan_from_editor(editor) {
                    Ok(plan) => {
                        save_plan = Some(plan);
                        close = true;
                    }
                    Err(e) => editor.error = Some(e),
                }
            }
            if editor.id.is_some() {
                if editor.confirm_delete {
                    text(
                        &painter,
                        x,
                        y + 52.0,
                        20.0,
                        "This deletes the plan. If you have started it, its progress is removed too.",
                        theme::ui_font(13.0),
                        p.ink_2,
                        false,
                    );
                    let (_, yes) = Button::new(Kind::Destructive, "Delete plan").show_left(
                        ui,
                        p,
                        x,
                        y + 96.0,
                        ui.scope_id().with("plan-del-yes"),
                    );
                    let (_, no) = Button::new(Kind::Plain, "Keep it").show_left(
                        ui,
                        p,
                        x + 140.0,
                        y + 96.0,
                        ui.scope_id().with("plan-del-no"),
                    );
                    if yes.clicked() {
                        delete_id = editor.id.clone();
                        close = true;
                    }
                    if no.clicked() {
                        editor.confirm_delete = false;
                    }
                    y += 140.0;
                } else {
                    let (_, del) = Button::new(Kind::Ghost, "Delete this plan").show_left(
                        ui,
                        p,
                        x + 220.0,
                        y + 20.0,
                        ui.scope_id().with("plan-del"),
                    );
                    if del.clicked() {
                        editor.confirm_delete = true;
                    }
                    y += 52.0;
                }
            } else {
                y += 52.0;
            }
        }
        if let Some(plan) = save_plan {
            actions.push(Action::SaveCustomPlan(plan));
        }
        if let Some(id) = delete_id {
            actions.push(Action::DeleteCustomPlan(id));
        }
        if close {
            self.editor = None;
            self.tab = Tab::Mine;
        }
        y
    }

    fn plan_from_editor(editor: &Editor) -> Result<CustomPlan, String> {
        let title = editor.title.trim().to_string();
        if title.is_empty() {
            return Err("Give the plan a title".into());
        }
        if title.chars().count() > plans::MAX_TITLE {
            return Err("That title is too long".into());
        }
        let description = editor.description.trim().to_string();
        if description.chars().count() > plans::MAX_DESCRIPTION {
            return Err("That description is too long".into());
        }
        let mut days = Vec::new();
        for (i, input) in editor.days.iter().enumerate() {
            if input.trim().is_empty() {
                continue;
            }
            match plans::parse_passages(input) {
                Ok(ps) if !ps.is_empty() => days.push(ps),
                Ok(_) => return Err(format!("Day {} has no passages", i + 1)),
                Err(e) => return Err(format!("Day {}: {e}", i + 1)),
            }
        }
        if days.is_empty() {
            return Err("Add at least one day with a passage".into());
        }
        if days.len() > plans::MAX_DAYS {
            return Err(format!("A plan can have at most {} days", plans::MAX_DAYS));
        }
        if days.iter().any(|d| d.len() > plans::MAX_PASSAGES_PER_DAY) {
            return Err(format!("A day can hold at most {} passages", plans::MAX_PASSAGES_PER_DAY));
        }
        Ok(CustomPlan { id: editor.id.clone().unwrap_or_default(), title, description, days, created: 0 })
    }

    fn confirm_sheet(&mut self, ui: &mut Ui, p: &Palette, content: Rect, actions: &mut Vec<Action>) {
        let Some(confirm) = &self.confirm else { return };
        let painter = ui.painter().clone();
        let (title, body, action) = match confirm {
            Confirm::Delete { title, started, .. } => {
                let body = if *started {
                    "This deletes the plan from My Plans and removes its reading progress. Leaving a plan you started (without deleting it) is separate — that only takes it off Active."
                } else {
                    "This deletes the plan from My Plans. It was not started, so no reading progress is lost."
                };
                (format!("Delete “{title}”?"), body, "Delete plan")
            }
            Confirm::Reset { title, .. } => (
                format!("Reset progress on “{title}”?"),
                "This forgets which days you have marked read and starts the plan again today.",
                "Reset progress",
            ),
        };
        let card_w = 420.0;
        let card =
            Rect::from_center_size(pos2(content.center().x, content.bottom() - 140.0), vec2(card_w, 176.0));
        p.shadows.modal.paint(&painter, card, 16.0);
        painter.rect_filled(card, 16.0, p.surface);
        text(
            &painter,
            card.left() + 20.0,
            card.top() + 20.0,
            22.0,
            &title,
            theme::ui_semibold(16.0),
            p.ink,
            false,
        );
        let mut job =
            egui::text::LayoutJob::simple(body.to_string(), theme::ui_font(13.0), p.ink_2, card_w - 40.0);
        job.sections[0].format.line_height = Some(20.0);
        painter.galley(pos2(card.left() + 20.0, card.top() + 52.0), painter.layout_job(job), p.ink_2);
        let (_, cancel) = Button::new(Kind::Plain, "Cancel").show_right(
            ui,
            p,
            card.right() - 148.0,
            card.bottom() - 20.0,
            ui.scope_id().with("confirm-cancel"),
        );
        let (_, go) = Button::new(Kind::Destructive, action).show_right(
            ui,
            p,
            card.right() - 20.0,
            card.bottom() - 20.0,
            ui.scope_id().with("confirm-go"),
        );
        if cancel.clicked() {
            self.confirm = None;
        }
        if go.clicked() {
            match self.confirm.take() {
                Some(Confirm::Delete { id, .. }) => actions.push(Action::DeleteCustomPlan(id)),
                Some(Confirm::Reset { id, .. }) => {
                    actions.push(Action::StartPlan(id));
                    self.tab = Tab::Active;
                }
                None => {}
            }
        }
    }
}

fn open_passage(p: &Passage, actions: &mut Vec<Action>) {
    let verse = p.verses.map(|(a, _)| a);
    actions.push(Action::Jump { chapter: p.chapter, verse, flash: verse.is_some() });
}
