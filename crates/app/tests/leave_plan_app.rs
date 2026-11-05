//! Leaving a reading plan in the running app: Active and the plan's day both offer Leave, a
//! confirm explains that ticks are cleared, Cancel keeps the plan, and confirming removes it.
//! Its own test binary because it sets `BIBLE_RS_OFFSCREEN` (see `book_picker_app.rs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::pages::Page;
use bible_rs::theme;
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Shape};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);
const PLAN: &str = "john-21";

fn find_rects(h: &Harness<'_, BibleApp>, label: &str) -> Vec<egui::Rect> {
    fn walk(shape: &Shape, clip: egui::Rect, label: &str, out: &mut Vec<egui::Rect>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, label, out)),
            Shape::Text(t) if t.galley.text() == label => {
                let r = t.visual_bounding_rect();
                if clip.contains(r.center()) {
                    out.push(r);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, c.clip_rect, label, &mut out);
    }
    out.sort_by(|a, b| a.center().y.total_cmp(&b.center().y).then(a.center().x.total_cmp(&b.center().x)));
    out
}

fn find(h: &Harness<'_, BibleApp>, label: &str) -> Vec<Pos2> {
    find_rects(h, label).iter().map(|r| r.center()).collect()
}

fn one(h: &Harness<'_, BibleApp>, label: &str) -> Pos2 {
    let found = find(h, label);
    assert_eq!(found.len(), 1, "{label:?} drawn at {found:?}");
    found[0]
}

fn shown(h: &Harness<'_, BibleApp>, label: &str) -> bool {
    !find(h, label).is_empty()
}

fn click(h: &mut Harness<'_, BibleApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(4);
}

fn app(dir: &std::path::Path) -> Harness<'static, BibleApp> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let mut init = initialize(dir.to_path_buf(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.translation = "kjv".into();
    init.settings.sidebar_open = true;
    init.settings.chapter = ChapterIdx::new(JOHN, 3).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 1000.0)).build_eframe(move |cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    h.run_steps(4);
    h
}

fn start_plan(h: &mut Harness<'_, BibleApp>) {
    let today = bible_rs::time::day_start(bible_core::study::now());
    h.state_mut().study.start_plan(PLAN, today).unwrap();
    h.state_mut().study.set_plan_read(PLAN, 1, 0, true).unwrap();
    h.state_mut().pages.open(Page::Plans);
    h.run_steps(4);
    assert!(h.state().study.plans().iter().any(|p| p.id == PLAN && p.finished.is_none()));
    assert!(h.state().study.plan_reads(PLAN).contains(&(1, 0)));
    assert!(shown(h, "Gospel of John"), "the active plan's card");
}

fn assert_left(h: &Harness<'_, BibleApp>) {
    assert!(h.state().study.plans().iter().all(|p| p.id != PLAN), "the plan should be gone");
    assert!(h.state().study.plan_reads(PLAN).is_empty(), "leaving clears reading ticks");
}

#[test]
fn leaving_from_active_asks_then_clears_the_plan() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-leave-plan-active-{}", std::process::id()));
    let mut h = app(&dir);
    start_plan(&mut h);
    let at = one(&h, "Leave plan");
    click(&mut h, at);
    assert!(shown(&h, "This clears your reading ticks."), "the confirm explains progress");
    assert!(h.state().study.plans().iter().any(|p| p.id == PLAN), "not left until confirmed");
    assert!(h.state().study.plan_reads(PLAN).contains(&(1, 0)));

    let at = one(&h, "Leave plan");
    click(&mut h, at);
    assert_left(&h);
    assert!(!shown(&h, "This clears your reading ticks."));
}

#[test]
fn cancel_keeps_the_plan_and_its_ticks() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-leave-plan-cancel-{}", std::process::id()));
    let mut h = app(&dir);
    start_plan(&mut h);

    let at = one(&h, "Leave plan");
    click(&mut h, at);
    assert!(shown(&h, "This clears your reading ticks."));
    let at = one(&h, "Cancel");
    click(&mut h, at);
    assert!(!shown(&h, "This clears your reading ticks."));
    assert!(h.state().study.plans().iter().any(|p| p.id == PLAN));
    assert!(h.state().study.plan_reads(PLAN).contains(&(1, 0)));
}

#[test]
fn leaving_from_the_plan_page_asks_then_returns_to_the_list() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-leave-plan-day-{}", std::process::id()));
    let mut h = app(&dir);
    start_plan(&mut h);

    // The title on the Active card opens the day; avoid the Leave plan button on its right.
    let at = one(&h, "Gospel of John");
    click(&mut h, at);
    assert!(shown(&h, "Leave…"), "the plan page offers leave");
    assert!(shown(&h, "Daily reminder"));

    let at = one(&h, "Leave…");
    click(&mut h, at);
    assert!(shown(&h, "Leave this plan?"));
    assert!(h.state().study.plans().iter().any(|p| p.id == PLAN), "not left until confirmed");

    let at = one(&h, "Leave plan");
    click(&mut h, at);
    assert_left(&h);
    assert!(h.state().pages.plans.open.is_none(), "the day view closes");
}
