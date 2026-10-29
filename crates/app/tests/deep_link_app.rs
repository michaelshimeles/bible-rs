//! A `bible://` link opens its passage in the middle of the reader, wherever the reader was.
//! Its own test binary because it sets `BIBLE_RS_OFFSCREEN` (no macOS menu bar, which needs the
//! main thread) before anything else runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx, VerseRef};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::{deep_link, theme};
use eframe::egui;
use egui_kittest::Harness;

const ROMANS: BookId = BookId(44);
const FIRST_CORINTHIANS: BookId = BookId(45);
/// Tall enough that a verse placed near the top, as other jumps place it, is well off centre.
const HEIGHT: f32 = 1100.0;
/// The reader's toolbar, floating over its top edge.
const TOOLBAR: f32 = 44.0;

#[test]
fn a_link_centres_its_passage_on_a_cold_and_a_warm_open() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-deep-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.chapter = ChapterIdx::new(FIRST_CORINTHIANS, 13).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    // Cold: the link is waiting before the first frame, as when it launched the app.
    assert!(deep_link::open("bible://1co.13.4-7"));
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, HEIGHT)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    harness.run_steps(6);
    let v = |n| VerseRef::new(FIRST_CORINTHIANS, 13, n);
    assert_eq!(harness.state().reader.selection, Some((v(4), v(7))));
    assert_centred(&harness, "cold");

    // Warm: scrolled elsewhere in the chapter, then the same link again.
    harness.state_mut().reader.set_selection(None);
    harness.state_mut().reader.jump(ChapterIdx::new(FIRST_CORINTHIANS, 13).unwrap(), None, false);
    harness.run_steps(4);
    assert!(deep_link::open("bible://1co.13.4-7"));
    harness.run_steps(6);
    assert_eq!(harness.state().reader.selection, Some((v(4), v(7))));
    assert_centred(&harness, "warm");

    // A chapter link drops the selection the last link made, so N, B and 1–6 can't act on a
    // passage that is no longer on screen.
    assert!(deep_link::open("bible://rom.8"));
    harness.run_steps(6);
    assert_eq!(harness.state().reader.location.0, ChapterIdx::new(ROMANS, 8).unwrap());
    assert_eq!(harness.state().reader.selection, None);

    // A verse past the chapter's end is no passage: nothing opens, nothing is selected.
    assert!(!deep_link::open("bible://rom.8.40"));
    assert!(!deep_link::open("bible://rom8:99"));
    harness.run_steps(4);
    assert_eq!(harness.state().reader.location.0, ChapterIdx::new(ROMANS, 8).unwrap());
    assert_eq!(harness.state().reader.selection, None);

    // A range running past the end selects up to the last verse (Romans 8 has 39).
    assert!(deep_link::open("bible://rom8:38-99"));
    harness.run_steps(6);
    let r = |n| VerseRef::new(ROMANS, 8, n);
    assert_eq!(harness.state().reader.selection, Some((r(38), r(39))));
    let _ = std::fs::remove_dir_all(dir);
}

/// The selection's middle is the middle of the view below the toolbar (within half a line).
fn assert_centred(harness: &Harness<'_, BibleApp>, when: &str) {
    let sel = harness.state().reader.selection_rect.expect("the selection is on screen");
    let middle = (TOOLBAR + HEIGHT) / 2.0;
    assert!((sel.center().y - middle).abs() < 16.0, "{when}: selection {sel:?}, view middle {middle}");
}
