//! Notes in the running app: return or a double-click opens a verse's note, escape closes it, a
//! new note highlights its verse, and colouring a passage leaves its noted verses in their own
//! colour. Its own test binary because it sets `BIBLE_RS_OFFSCREEN` before anything else runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx, VerseRef};
use bible_core::study::HighlightColor;
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::theme;
use eframe::egui::{self, Key};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);

/// Types `text` into whatever has the keyboard, then waits out the autosave.
fn type_and_close(harness: &mut Harness<'_, BibleApp>, text: &str) {
    // Half a second for the popover to open and its editor to take the keyboard.
    harness.run_steps(30);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(4);
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().note_open(), None, "escape closes the note");
}

#[test]
fn notes_open_with_return_highlight_their_verse_and_stay_over_a_passage() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.chapter = ChapterIdx::new(JOHN, 1).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut harness =
        Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
            cc.egui_ctx.set_fonts(theme::font_definitions());
            BibleApp::new(cc, init, online_counter(), startup)
        });
    harness.run_steps(4);
    let v = |n| VerseRef::new(JOHN, 1, n);

    // Return on a verse without a note does nothing; with no highlight yet, a new note takes the
    // first colour.
    harness.state_mut().reader.set_selection(Some(v(3)));
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    harness.key_press(Key::N);
    harness.run_steps(2);
    assert_eq!(harness.state().note_open(), Some(v(3)), "N opens the note");
    type_and_close(&mut harness, "All things were made by him.");
    let s = &harness.state().study;
    assert_eq!(s.note(v(3)).map(|n| n.body.as_str()), Some("All things were made by him."));
    assert_eq!(s.highlight(v(3)), Some(HighlightColor::ALL[0]), "a new note highlights its verse");

    // The next note takes the colour used last.
    harness.state_mut().reader.set_selection(Some(v(9)));
    harness.key_press(Key::Num4);
    harness.run_steps(2);
    harness.state_mut().reader.set_selection(Some(v(5)));
    harness.key_press(Key::N);
    harness.run_steps(2);
    type_and_close(&mut harness, "Light and darkness.");
    assert_eq!(harness.state().study.highlight(v(5)), Some(HighlightColor::ALL[3]));

    // Return on a verse that has a note opens it: typing goes into the note.
    harness.state_mut().reader.set_selection(Some(v(3)));
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    type_and_close(&mut harness, " See Col 1:16.");
    assert_eq!(
        harness.state().study.note(v(3)).map(|n| n.body.as_str()),
        Some("All things were made by him. See Col 1:16.")
    );

    // Colouring the passage around them leaves the noted verses in their own colour.
    harness.state_mut().reader.select_range(v(1), v(6));
    harness.key_press(Key::Num2);
    harness.run_steps(2);
    let s = &harness.state().study;
    assert_eq!(s.highlight(v(3)), Some(HighlightColor::ALL[0]));
    assert_eq!(s.highlight(v(5)), Some(HighlightColor::ALL[3]));
    for n in [1, 2, 4, 6] {
        assert_eq!(s.highlight(v(n)), Some(HighlightColor::ALL[1]), "verse {n} takes the passage colour");
    }
    // One noted verse on its own still changes colour.
    harness.state_mut().reader.set_selection(Some(v(3)));
    harness.key_press(Key::Num2);
    harness.run_steps(2);
    assert_eq!(harness.state().study.highlight(v(3)), Some(HighlightColor::ALL[1]));

    // Double-clicking a verse that is already selected opens its note, even when the clicks are
    // a little slower than egui's own double-click.
    harness.state_mut().reader.set_selection(Some(v(7)));
    harness.run_steps(2);
    let at = harness.state().reader.selection_rect.expect("the selection is on screen").center();
    for _ in 0..2 {
        click(&mut harness, at);
        // About 0.4 s between the clicks.
        harness.run_steps(24);
    }
    assert_eq!(harness.state().note_open(), Some(v(7)), "a double-click opens the note");
    assert_eq!(harness.state().reader.selected_refs(), vec![v(7)], "and leaves the verse selected");
    let _ = std::fs::remove_dir_all(&dir);
}

fn click(harness: &mut Harness<'_, BibleApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
}
