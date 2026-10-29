//! Canon sections in the running app: the testaments by default, then, once the setting asks for
//! sections, disclosure rows, reader following, chapter navigation, filtering and pins, and back.
//! Its own test binary because it sets `BIBLE_RS_OFFSCREEN` before startup.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bible_core::books::{BookId, ChapterIdx};
use bible_rs::app::{BibleApp, Startup, initialize, online_counter};
use bible_rs::sidebar::BookGrouping;
use bible_rs::theme::{self, Theme};
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const JOHN: BookId = BookId(42);
const ROMANS: BookId = BookId(44);
const HEBREWS: BookId = BookId(57);
const SECTIONS: [(&str, u8, u8); 8] = [
    ("Pentateuch", 0, 4),
    ("History", 5, 16),
    ("Wisdom and Poetry", 17, 21),
    ("Major Prophets", 22, 26),
    ("Minor Prophets", 27, 38),
    ("Gospels and Acts", 39, 43),
    ("Pauline Epistles", 44, 56),
    ("General Epistles", 57, 65),
];

/// Visible text in the sidebar, top to bottom (the reader can use the same labels).
fn find(h: &Harness<'_, BibleApp>, label: &str) -> Vec<Pos2> {
    fn walk(shape: &Shape, clip: egui::Rect, label: &str, out: &mut Vec<Pos2>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, clip, label, out)),
            Shape::Text(t) if t.galley.text() == label => {
                let p = t.visual_bounding_rect().center();
                if p.x < 300.0 && clip.contains(p) {
                    out.push(p);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, c.clip_rect, label, &mut out);
    }
    out.sort_by(|a, b| a.y.total_cmp(&b.y));
    out
}

/// Only the book groups, so the History page row cannot be mistaken for the canon section.
fn books(h: &Harness<'_, BibleApp>, label: &str) -> Vec<Pos2> {
    let top = ["Pinned", "Old Testament", "New Testament"]
        .into_iter()
        .flat_map(|label| find(h, label))
        .map(|p| p.y)
        .min_by(f32::total_cmp)
        .unwrap_or(f32::INFINITY);
    find(h, label).into_iter().filter(|p| p.y >= top).collect()
}

fn one(h: &Harness<'_, BibleApp>, label: &str) -> Pos2 {
    let found = books(h, label);
    assert_eq!(found.len(), 1, "{label:?} drawn at {found:?}");
    found[0]
}

fn click(h: &mut Harness<'_, BibleApp>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(4);
}

fn toggle(h: &mut Harness<'_, BibleApp>, label: &str) {
    let at = one(h, label);
    click(h, at, PointerButton::Primary);
}

fn jump(h: &mut Harness<'_, BibleApp>, book: BookId, chapter: u8) {
    h.state_mut().reader.jump(ChapterIdx::new(book, chapter).unwrap(), None, false);
    h.run_steps(6);
}

fn book_label(h: &Harness<'_, BibleApp>, book: BookId) -> String {
    h.state().library.get("kjv").unwrap().meta.book_short(book).into()
}

/// Whether each disclosure row is expanded, as assistive tech sees it.
fn expanded(h: &Harness<'_, BibleApp>, label: &str) -> Option<bool> {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node().data().is_expanded()
}

fn has_focus_outline(h: &Harness<'_, BibleApp>, label: &str) -> bool {
    fn walk(shape: &Shape, rect: egui::Rect, color: egui::Color32) -> bool {
        match shape {
            Shape::Vec(shapes) => shapes.iter().any(|s| walk(s, rect, color)),
            Shape::Rect(r) => r.rect == rect && r.stroke.width > 0.0 && r.stroke.color == color,
            _ => false,
        }
    }
    let rect = h.get_by_label(label).rect();
    let color = h.state().settings.theme.palette().accent;
    h.output().shapes.iter().any(|s| s.clip_rect.contains(rect.center()) && walk(&s.shape, rect, color))
}

#[test]
fn books_are_grouped_by_section_and_follow_the_reader() {
    // SAFETY: the only test in this binary, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("BIBLE_RS_OFFSCREEN", "1") };
    let dir = std::env::temp_dir().join(format!("bible-rs-section-books-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut init = initialize(dir.clone(), Arc::new(|| {})).expect("loading data");
    init.settings.onboarded = true;
    init.settings.translation = "kjv".into();
    init.settings.sidebar_open = true;
    init.settings.chapter = ChapterIdx::new(JOHN, 3).unwrap().0;
    let startup =
        Startup { t0: Instant::now(), probe: false, init: Duration::ZERO, first_ui: None, frames: 0 };
    // Enough height to open every section together and count all 66 book rows without scrolling.
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 3400.0)).build_eframe(|cc| {
        cc.egui_ctx.set_fonts(theme::font_definitions());
        BibleApp::new(cc, init, online_counter(), startup)
    });
    h.run_steps(8);

    // Testaments by default: both under Books, the reader's one open on John.
    assert_eq!(h.state().settings.book_grouping, BookGrouping::Testament);
    assert_eq!(find(&h, "Books").len(), 1);
    assert_eq!(expanded(&h, "Old Testament"), Some(false));
    assert_eq!(expanded(&h, "New Testament"), Some(true));
    assert!(books(&h, "Gospels and Acts").is_empty());
    assert!(one(&h, "John").y > one(&h, "New Testament").y);

    h.state_mut().settings.book_grouping = BookGrouping::Section;
    h.run_steps(8);
    assert!(find(&h, "Books").is_empty());
    assert!(find(&h, "Pinned").is_empty(), "no empty pinned group");
    for label in ["Old Testament", "New Testament"] {
        h.get_by_role_and_label(egui::accesskit::Role::Heading, label);
    }
    let old = one(&h, "Old Testament");
    let new = one(&h, "New Testament");
    let mut previous = old.y;
    for (i, (label, first, last)) in SECTIONS.iter().enumerate() {
        let at = one(&h, label);
        assert!(at.y > previous, "{label} follows the preceding section");
        assert_eq!(at.y < new.y, i < 5, "{label} belongs under its testament");
        let count = (last - first + 1).to_string();
        assert!(books(&h, &count).iter().any(|p| (p.y - at.y).abs() < 2.0), "{label}'s book count");
        let node = h.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node();
        assert_eq!(node.label().as_deref(), Some(*label));
        assert_eq!(node.data().is_expanded(), Some(i == 5), "only the reader's section starts expanded");
        previous = at.y;
    }
    // Tab reaches the next disclosure; Enter and Space toggle it while keeping visible focus.
    h.get_by_label("Pentateuch").focus();
    h.run_steps(3);
    h.key_press(Key::Tab);
    h.run_steps(3);
    assert!(h.get_by_label("History").is_focused());
    assert!(has_focus_outline(&h, "History"));
    h.key_press(Key::Enter);
    h.run_steps(4);
    assert_eq!(h.get_by_label("History").accesskit_node().data().is_expanded(), Some(true));
    one(&h, "Joshua");
    assert!(h.get_by_label("History").is_focused());
    assert!(has_focus_outline(&h, "History"));
    h.key_press(Key::Space);
    h.run_steps(4);
    assert_eq!(h.get_by_label("History").accesskit_node().data().is_expanded(), Some(false));
    assert!(books(&h, "Joshua").is_empty());
    // Testament labels are plain headers: clicking them leaves their section rows in place.
    let gospels = one(&h, "Gospels and Acts");
    click(&mut h, old, PointerButton::Primary);
    click(&mut h, new, PointerButton::Primary);
    assert_eq!(one(&h, "Gospels and Acts"), gospels);
    assert!(books(&h, "Genesis").is_empty());
    assert!(books(&h, "Romans").is_empty());
    let john = one(&h, "John");
    let acts = one(&h, "Acts");
    for n in 1..=21 {
        assert!(
            books(&h, &n.to_string()).iter().any(|p| p.y > john.y && p.y < acts.y),
            "John's chapter {n} is inline"
        );
    }
    let chapter7 = *books(&h, "7").iter().find(|p| p.y > john.y && p.y < acts.y).unwrap();
    click(&mut h, chapter7, PointerButton::Primary);
    assert_eq!(h.state().reader.location.0, ChapterIdx::new(JOHN, 7).unwrap());

    toggle(&mut h, "Pauline Epistles");
    for b in 44..=56 {
        one(&h, &book_label(&h, BookId(b)));
    }
    assert!(one(&h, "Romans").y < one(&h, "Philemon").y);
    assert!(books(&h, "Hebrews").is_empty(), "Hebrews isn't among Paul's letters");
    jump(&mut h, ROMANS, 8);
    assert!(books(&h, "John").is_empty(), "the automatically opened Gospels close");
    jump(&mut h, HEBREWS, 2);
    assert!(!books(&h, "Romans").is_empty(), "the manually opened Pauline section stays open");
    assert!(one(&h, "Hebrews").y > one(&h, "General Epistles").y);
    jump(&mut h, JOHN, 3);
    assert!(books(&h, "Hebrews").is_empty(), "the automatically opened General Epistles close");
    assert!(!books(&h, "Romans").is_empty(), "the manually opened section still stays open");

    // Close and reopen the reader's section by hand: it now stays open when the reader moves.
    toggle(&mut h, "Gospels and Acts");
    toggle(&mut h, "Gospels and Acts");
    jump(&mut h, ROMANS, 8);
    assert!(!books(&h, "John").is_empty());
    // Close Paul's letters by hand; filtering must still reveal matches in a closed section.
    toggle(&mut h, "Pauline Epistles");
    assert!(books(&h, "Philippians").is_empty(), "closed before filtering");
    h.state_mut().sidebar.open_study("filter");
    h.run_steps(3);
    h.event(Event::Text("phil".into()));
    h.run_steps(6);
    one(&h, "Pauline Epistles");
    one(&h, "Philippians");
    one(&h, "Philemon");
    assert!(books(&h, "Romans").is_empty());
    assert!(books(&h, "Old Testament").is_empty());
    for (label, _, _) in SECTIONS {
        if label != "Pauline Epistles" {
            assert!(books(&h, label).is_empty(), "unmatched {label} is hidden");
        }
    }
    // Clear the filter and leave the empty field to restore the prior disclosure state.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run_steps(4);
    let header = one(&h, "New Testament");
    click(&mut h, header, PointerButton::Primary);
    h.run_steps(6);
    assert!(books(&h, "Philippians").is_empty(), "filtering didn't change disclosure state");
    assert!(!books(&h, "John").is_empty());

    // Open everything and verify each book's single row lies between its section and the next.
    for (label, first, _) in SECTIONS {
        if books(&h, &book_label(&h, BookId(first))).is_empty() {
            toggle(&mut h, label);
        }
    }
    for theme in [Theme::Light, Theme::Sepia, Theme::Dark] {
        h.state_mut().settings.theme = theme;
        theme::apply(&h.ctx, theme);
        h.run_steps(4);
        for (i, (label, first, last)) in SECTIONS.iter().enumerate() {
            let top = one(&h, label).y;
            let bottom = SECTIONS.get(i + 1).map(|s| one(&h, s.0).y).unwrap_or(f32::INFINITY);
            let mut previous = top;
            for b in *first..=*last {
                let label = book_label(&h, BookId(b));
                let at = one(&h, &label);
                assert!(at.y > previous && at.y < bottom, "{theme:?}: {label} under its section");
                previous = at.y;
            }
        }
    }

    // The section's book keeps its right-click pin menu, and its pinned copy precedes both headers.
    let john = one(&h, "John");
    click(&mut h, john, PointerButton::Secondary);
    let pin = find(&h, "Pin to Top");
    assert_eq!(pin.len(), 1);
    click(&mut h, pin[0], PointerButton::Primary);
    assert_eq!(h.state().settings.pinned(), [JOHN]);
    let john = books(&h, "John");
    assert_eq!(john.len(), 2);
    assert!(one(&h, "Pinned").y < john[0].y);
    assert!(john[0].y < one(&h, "Old Testament").y);
    assert!(john[1].y > one(&h, "Gospels and Acts").y);
    h.state_mut().sidebar.open_study("filter");
    h.run_steps(3);
    h.event(Event::Text("phil".into()));
    h.run_steps(6);
    assert!(find(&h, "Pinned").is_empty(), "hide the header when no pins match the filter");
    assert!(books(&h, "John").is_empty());
    one(&h, "Philippians");

    // Back to testaments: Books heads the pins again, and only the reader's testament reopens.
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.run_steps(4);
    h.state_mut().settings.book_grouping = BookGrouping::Testament;
    h.run_steps(8);
    assert!(find(&h, "Pinned").is_empty());
    assert!(books(&h, "Gospels and Acts").is_empty());
    let header = find(&h, "Books");
    assert_eq!(header.len(), 1);
    let john = find(&h, "John");
    assert_eq!(john.len(), 2, "pinned, and in the New Testament");
    assert!(header[0].y < john[0].y && john[0].y < one(&h, "Old Testament").y);
    assert!(john[1].y > one(&h, "New Testament").y);
    assert_eq!(expanded(&h, "Old Testament"), Some(false));
    assert_eq!(expanded(&h, "New Testament"), Some(true));
    let _ = std::fs::remove_dir_all(&dir);
}
