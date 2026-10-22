//! Toasts in the running app: a stack of toasts of different widths stays on one centre line as
//! the oldest goes, and toggling a bookmark again and again shows one toast with its latest
//! state. Its own test binary because it sets `BIBLE_RS_OFFSCREEN` (see `book_picker_app.rs`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::theme;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;

const JOHN: BookId = BookId(42);

/// The centre line of each toast capsule painted last frame: 40 high, radius 20, unblurred (the
/// shadow beneath is blurred). Where it was painted, not where its area ended up: an area moved
/// after its content was laid out records the new place but showed the old one.
fn centres(h: &Harness<'_, BibleApp>) -> Vec<f32> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r)
                if r.rect.height() == 40.0 && r.corner_radius.nw == 20 && r.blur_width == 0.0 =>
            {
                Some(r.rect.center().x)
            }
            _ => None,
        })
        .collect()
}

fn assert_one_centre_line(h: &Harness<'_, BibleApp>) {
    let c = centres(h);
    assert_eq!(c.len(), h.state().toast_areas().len(), "a capsule for each toast");
    assert!(c.iter().all(|x| (x - c[0]).abs() < 1.0), "toasts off the centre line: {c:?}");
}

#[test]
fn toasts_stay_centred_and_bookmark_toasts_collapse() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-toasts-{}", std::process::id()));
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

    // Alternating widths, as toggling a bookmark used to stack them. Each step is 0.25 s.
    for (i, m) in ["Bookmarked John 3:8", "Removed bookmark John 3:8"].iter().cycle().take(4).enumerate() {
        let ctx = h.ctx.clone();
        h.state_mut().toast(&ctx, *m);
        h.step();
        if i == 0 {
            h.run_steps(3);
        }
    }
    h.step(); // a new area's first frame only measures it
    assert_eq!(centres(&h).len(), 4);
    assert_one_centre_line(&h);

    // Every frame as the oldest goes: the ones left keep to the line (they zigzagged by half the
    // difference in width when the oldest went).
    for _ in 0..16 {
        h.step();
        if !h.state().toast_areas().is_empty() {
            assert_one_centre_line(&h);
        }
    }
    h.run_steps(20);
    assert!(h.state().toast_areas().is_empty());

    // ⌘D four times on one verse: one toast, saying where it ended up.
    for _ in 0..4 {
        h.key_press_modifiers(Modifiers::COMMAND, Key::D);
        h.step();
    }
    let shown: Vec<_> = h.state().toast_areas().into_iter().map(|(m, _)| m.to_string()).collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].starts_with("Removed bookmark "), "{shown:?}");
    h.key_press_modifiers(Modifiers::COMMAND, Key::D);
    h.step();
    let shown: Vec<_> = h.state().toast_areas().into_iter().map(|(m, _)| m.to_string()).collect();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].starts_with("Bookmarked "), "{shown:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
