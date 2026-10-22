//! H and L, and [ and ], turn chapters like ← and →, crossing into the next or previous book,
//! and wait while a text field has the keyboard. This is its own test binary because it sets
//! `BIBLE_RS_OFFSCREEN` (no macOS menu bar, which needs the main thread) before anything else runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::theme;
use eframe::egui::{self, Key};
use egui_kittest::Harness;

const MALACHI: BookId = BookId(38);
const MATTHEW: BookId = BookId(39);

#[test]
fn letter_and_bracket_keys_turn_chapters_across_books_but_not_while_typing() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-chapter-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    let malachi_4 = ChapterIdx::new(MALACHI, 4).unwrap();
    let matthew_1 = ChapterIdx::new(MATTHEW, 1).unwrap();
    let matthew_2 = ChapterIdx::new(MATTHEW, 2).unwrap();
    init.settings.chapter = malachi_4.0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    harness.run_steps(4);
    let press = |harness: &mut Harness<'_, BibleApp>, key: Key| {
        harness.key_press(key);
        harness.run_steps(2);
        harness.state().reader.location.0
    };

    assert_eq!(press(&mut harness, Key::L), matthew_1, "L from the last chapter of Malachi opens Matthew");
    assert_eq!(press(&mut harness, Key::L), matthew_2, "L turns to the next chapter");
    assert_eq!(press(&mut harness, Key::H), matthew_1, "H turns to the previous chapter");
    assert_eq!(press(&mut harness, Key::H), malachi_4, "H from Matthew 1 goes back to Malachi");
    assert_eq!(press(&mut harness, Key::CloseBracket), matthew_1, "] turns to the next chapter");
    assert_eq!(press(&mut harness, Key::OpenBracket), malachi_4, "[ turns to the previous chapter");

    // With the search field focused, the letters are typed rather than turning chapters.
    press(&mut harness, Key::Slash);
    harness.run_steps(2);
    assert!(harness.ctx.egui_wants_keyboard_input(), "the search field has the keyboard");
    assert_eq!(press(&mut harness, Key::L), malachi_4, "L in the search field stays put");
    assert_eq!(press(&mut harness, Key::H), malachi_4, "H in the search field stays put");
    assert_eq!(press(&mut harness, Key::CloseBracket), malachi_4, "] in the search field stays put");
    let _ = std::fs::remove_dir_all(&dir);
}
