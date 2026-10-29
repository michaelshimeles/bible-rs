//! Escape closes the running app's book filter before the reader handles the key. Its own
//! test binary because it sets `BIBLE_RS_OFFSCREEN` (see `book_picker_app.rs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx, VerseRef};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::theme;
use eframe::egui::{self, Key, Shape};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);

fn sidebar_has_text(h: &Harness<'_, BibleApp>, label: &str) -> bool {
    fn matches(shape: &Shape, clip: egui::Rect, label: &str) -> bool {
        match shape {
            Shape::Vec(v) => v.iter().any(|s| matches(s, clip, label)),
            Shape::Text(t) => {
                t.galley.text() == label
                    && t.pos.x < 280.0
                    && clip.contains(t.visual_bounding_rect().center())
            }
            _ => false,
        }
    }
    h.output().shapes.iter().any(|s| matches(&s.shape, s.clip_rect, label))
}

#[test]
fn escape_closes_the_filter_without_clearing_the_selected_verse() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-sidebar-filter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.translation = "kjv".into();
    init.settings.sidebar_open = true;
    init.settings.chapter = ChapterIdx::new(JOHN, 1).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 1000.0)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    h.run_steps(8);
    h.state_mut().reader.set_selection(Some(VerseRef::new(JOHN, 1, 1)));
    assert!(sidebar_has_text(&h, "Matthew"));

    h.state_mut().sidebar.open_study("filter");
    h.run_steps(2);
    let filter = egui::Id::new("filter-books");
    assert!(h.ctx.memory(|m| m.has_focus(filter)));
    h.event(egui::Event::Text("phil".into()));
    h.run_steps(2);
    assert!(sidebar_has_text(&h, "phil"), "the nonempty filter is visible");
    assert!(sidebar_has_text(&h, "Philippians"));
    assert!(!sidebar_has_text(&h, "Matthew"), "the book list is filtered");

    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!sidebar_has_text(&h, "phil"), "Escape closes the nonempty filter");
    assert!(sidebar_has_text(&h, "Matthew"), "Escape restores the book list");
    assert!(!h.ctx.memory(|m| m.has_focus(filter)), "the hidden field releases focus");
    assert!(!h.state().reader.selected_refs().is_empty(), "the reader keeps its selection");

    // Reopening starts empty and takes focus again; Escape also closes an empty filter.
    h.state_mut().sidebar.open_study("filter");
    h.run_steps(2);
    assert!(sidebar_has_text(&h, "Filter books"));
    assert!(h.ctx.memory(|m| m.has_focus(filter)));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!sidebar_has_text(&h, "Filter books"));
    assert!(!h.ctx.memory(|m| m.has_focus(filter)));
    assert!(!h.state().reader.selected_refs().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
