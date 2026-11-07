//! Memorising and recalling in the running app: a recall hides a memorised verse in place, a click
//! reveals a word and a drag across the reader the rest, and Return grades it Good, scheduling it
//! days on; M memorises the selected verse, and the verse menu stops it. A passage the translation
//! doesn't have all of is skipped, parallel columns wait, and Back returns to the reading place.
//! Its own test binary because it sets `BIBLE_RS_OFFSCREEN` (see `book_picker_app.rs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx, VerseRef};
use bible_core::memory::interval_label;
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::reader::TextPos;
use bible_rs::theme;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);

fn press(h: &mut Harness<'_, BibleApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    h.step();
}

fn drag(h: &mut Harness<'_, BibleApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    press(h, from, true);
    for i in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    press(h, to, false);
    // The reveal springs to rest.
    h.run_steps(90);
}

fn click(h: &mut Harness<'_, BibleApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    press(h, at, true);
    press(h, at, false);
    h.run_steps(2);
}

/// A trackpad scroll moving the content by `delta` points.
fn wheel(h: &mut Harness<'_, BibleApp>, delta: egui::Vec2) {
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta,
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
    h.step();
}

/// Where the app draws `label`, if it does.
fn text_at(h: &Harness<'_, BibleApp>, label: &str) -> Option<Pos2> {
    fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, label)),
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|s| find(&s.shape, label))
}

fn key(h: &mut Harness<'_, BibleApp>, key: Key) {
    h.key_press(key);
    h.run_steps(2);
}

#[test]
fn a_memorised_verse_is_recalled_in_place() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("BIBLE_RS_OFFSCREEN", "1");
        std::env::set_var("BIBLE_RS_BACKGROUND", "1");
        std::env::set_var("BIBLE_RS_DEMO", "memorise:jn 3:16;recall");
    }
    let dir = std::env::temp_dir().join(format!("bible-rs-recall-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.translation = "kjv".into();
    let c = ChapterIdx::new(JOHN, 3).unwrap();
    init.settings.chapter = c.0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    h.run_steps(4);
    h.state_mut().reader.jump(c, Some(16), false);
    h.run_steps(6);
    let v16 = VerseRef::new(JOHN, 3, 16);

    // The demo steps memorise John 3:16 and start recalling what is due: it, hidden in place.
    h.run_steps(10);
    let recall = h.state().reader.recall.as_ref().expect("recalling");
    assert_eq!(recall.passage.first, v16);
    assert!(!recall.started() && !recall.done());

    // A drag across the reader reveals all of it, wherever it starts.
    let text = bible_core::bundled::load("kjv").unwrap().find(v16).unwrap().text.to_string();
    let at = |h: &Harness<'_, BibleApp>, offset: usize| {
        h.state().reader.point_at(TextPos { verse: v16, offset, column: 0 }).expect("on screen")
    };
    let start = at(&h, 0);

    // A two-finger swipe to the left shows more, and its drift up or down doesn't scroll the page.
    let anchor = h.state().reader.anchor;
    h.event(Event::PointerMoved(start));
    for _ in 0..6 {
        wheel(&mut h, vec2(-40.0, 6.0));
    }
    let recall = h.state().reader.recall.as_ref().unwrap();
    assert!(recall.started(), "a swipe left reveals");
    assert_eq!(h.state().reader.anchor, anchor, "the page holds still while swiping");
    // Once the swipe has settled, scrolling up or down moves the page again.
    h.run_steps(40);
    for _ in 0..4 {
        wheel(&mut h, vec2(0.0, -40.0));
    }
    h.run_steps(20);
    assert_ne!(h.state().reader.anchor, anchor, "a vertical scroll still scrolls");
    h.state_mut().reader.recall.as_mut().unwrap().reset();
    h.run_steps(90);
    assert!(!h.state().reader.recall.as_ref().unwrap().started());
    // A click shows a word.
    h.state_mut().reader.jump(c, Some(16), false);
    h.run_steps(6);
    let p = at(&h, text.find("God").unwrap());
    click(&mut h, p);
    h.run_steps(90);
    let recall = h.state().reader.recall.as_ref().unwrap();
    assert!(recall.started() && !recall.done(), "one click shows one word");
    // A drag reveals wherever it starts in the reader.
    let middle = Pos2::new(300.0, 450.0);
    drag(&mut h, middle, middle + vec2(1000.0, 0.0));
    assert!(h.state().reader.recall.as_ref().unwrap().done(), "a long drag shows the whole passage");
    // Another translation's words start the passage over, so it can't be graded on the old reveal.
    h.state_mut().settings.translation = "bsb".into();
    h.run_steps(20);
    let recall = h.state().reader.recall.as_ref().unwrap();
    assert!(!recall.started() && !recall.done(), "switching translation starts over");
    drag(&mut h, middle, middle + vec2(1000.0, 0.0));
    assert!(h.state().reader.recall.as_ref().unwrap().done());

    // Return grades it Good and ends the session: a first Good comes back in two days.
    key(&mut h, Key::Enter);
    assert!(h.state().reader.recall.is_none(), "the only passage due is done");
    let m = h.state().study.memorised_at(v16).unwrap().clone();
    assert_eq!(m.schedule.reps, 1);
    assert_eq!(interval_label(bible_core::study::now(), m.schedule.due), "2 days");
    assert!(h.state().study.due(bible_core::study::now()).is_empty(), "it waits days now");

    // With nothing due, recalling practises early; Again brings it back in the same session.
    h.state_mut().settings.sidebar_open = true;
    h.run_steps(30);
    let row = text_at(&h, "Memory").expect("the sidebar's Memory row");
    click(&mut h, row);
    h.state_mut().settings.sidebar_open = false;
    h.run_steps(30);
    let recall = h.state().reader.recall.as_ref().expect("practising early");
    assert_eq!((recall.passage.first, recall.total), (v16, 1));
    drag(&mut h, middle, middle + vec2(1000.0, 0.0));
    key(&mut h, Key::Num1);
    let recall = h.state().reader.recall.as_ref().expect("Again comes back");
    assert_eq!((recall.passage.first, recall.index, recall.total), (v16, 2, 2));
    assert!(!recall.started());
    assert_eq!(h.state().study.due(bible_core::study::now()).len(), 1, "and stays due");
    key(&mut h, Key::Escape);
    assert!(h.state().reader.recall.is_none());

    // M memorises the selected verse. Pressing it again, or on part of a passage, never resets it.
    let v17 = VerseRef::new(JOHN, 3, 17);
    h.state_mut().reader.set_selection(Some(v17));
    key(&mut h, Key::M);
    let m = h.state().study.memorised_at(v17).cloned().expect("memorised");
    assert_eq!(m.translation, "bsb");
    h.state_mut().study.review(v17, bible_core::memory::Grade::Good, bible_core::study::now()).unwrap();
    key(&mut h, Key::M);
    assert_eq!(h.state().study.reviews(v17).len(), 1, "a stray M keeps the passage and its recalls");

    // Stopping is the verse menu's Stop Memorising.
    let p = h.state().reader.point_at(TextPos { verse: v17, offset: 0, column: 0 }).expect("on screen");
    h.event(Event::PointerMoved(p));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run_steps(10);
    let stop = text_at(&h, "Stop Memorising").expect("the verse menu offers Stop Memorising");
    click(&mut h, stop);
    assert!(h.state().study.memorised_at(v17).is_none());

    // Recalling from Genesis 1 in the WEB, beside a parallel column. Luke 17:35–36 is memorised from
    // the KJV, but the WEB leaves out verse 36; Romans 8:38–39 is due after it.
    h.state_mut().study.review(v16, bible_core::memory::Grade::Good, bible_core::study::now()).unwrap();
    let luke = VerseRef::new(BookId(41), 17, 35);
    let romans = VerseRef::new(BookId(44), 8, 38);
    h.state_mut().study.memorise(luke, VerseRef::new(BookId(41), 17, 36), "kjv").unwrap();
    h.state_mut().study.memorise(romans, VerseRef::new(BookId(44), 8, 39), "kjv").unwrap();
    h.state_mut().settings.translation = "web".into();
    h.state_mut().settings.parallel_on = true;
    let genesis = ChapterIdx::new(BookId(0), 1).unwrap();
    h.state_mut().reader.jump(genesis, Some(1), false);
    h.run_steps(20);
    assert_eq!(h.state().columns().len(), 2);
    h.state_mut().settings.sidebar_open = true;
    h.run_steps(30);
    let row = text_at(&h, "Memory").expect("the sidebar's Memory row");
    click(&mut h, row);
    h.state_mut().settings.sidebar_open = false;
    h.run_steps(30);
    assert_eq!(h.state().columns().len(), 1, "one column, so another translation can't give it away");

    // Part of Luke 17:35–36 can't be graded for all of it; Return skips it and the session goes on.
    let recall = h.state().reader.recall.as_ref().expect("recalling");
    assert_eq!((recall.passage.first, recall.index, recall.total), (luke, 1, 2));
    assert!(recall.unavailable() && !recall.done());
    assert!(text_at(&h, "Not all of it is in this translation").is_some());
    key(&mut h, Key::Enter);
    let recall = h.state().reader.recall.as_ref().expect("the rest of the session");
    assert_eq!((recall.passage.first, recall.index, recall.total), (romans, 1, 1));
    assert!(!recall.unavailable());
    assert!(h.state().study.reviews(luke).is_empty(), "skipped, not graded");
    assert!(h.state().study.memorised_at(luke).is_some());

    // Back ends the recall and returns to where you were reading.
    h.key_press_modifiers(Modifiers::COMMAND, Key::OpenBracket);
    h.run_steps(10);
    assert!(h.state().reader.recall.is_none());
    assert_eq!(h.state().reader.location.0, genesis);
    assert_eq!(h.state().columns().len(), 2, "parallel again");
    let _ = std::fs::remove_dir_all(&dir);
}
