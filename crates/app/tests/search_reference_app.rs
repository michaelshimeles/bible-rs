//! Searching for a reference in the running app: "1:14" (in the book being read), "genesis 1:14"
//! and "gen 1:14" list that verse above the hits, and return goes to it. Its own test binary
//! because it sets `BIBLE_RS_OFFSCREEN` before anything else runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx, VerseRef};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::theme;
use eframe::egui::{self, Key};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);

#[test]
fn return_goes_to_the_verse_a_search_names() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-search-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.chapter = ChapterIdx::new(JOHN, 3).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    harness.run_steps(4);

    let mut search = |harness: &mut Harness<'_, BibleApp>, query: &str| {
        // Typed into the toolbar's field, which opens the sidebar's results.
        harness.state_mut().sidebar.focus_search(Some(String::new()));
        harness.run_steps(2);
        assert!(harness.state().sidebar.field_focused, "the search field has focus");
        harness.event(egui::Event::Text(query.to_string()));
        harness.run_steps(4);
        harness.key_press(Key::Enter);
        harness.run_steps(4);
        // Landing on a verse selects it.
        harness.state().reader.selected_refs().first().copied()
    };

    let verse = |book: BookId, c: u8, v: u8| Some(VerseRef::new(book, c, v));
    assert_eq!(search(&mut harness, "1:14"), verse(JOHN, 1, 14), "a bare reference is in this book");
    assert_eq!(harness.state().reader.location.0, ChapterIdx::new(JOHN, 1).unwrap());
    assert_eq!(search(&mut harness, "genesis 1:14"), verse(BookId::GENESIS, 1, 14));
    assert_eq!(search(&mut harness, "rom 8:28"), verse(BookId(44), 8, 28));
    // A word search still goes to its first hit.
    assert_eq!(search(&mut harness, "bethesda"), verse(JOHN, 5, 2));
    let _ = std::fs::remove_dir_all(&dir);
}
