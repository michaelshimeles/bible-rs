//! The "All books" menus' type-ahead: typing narrows the books, the arrows move the highlight and
//! return chooses, so a book is a few keys away.

use bible_core::books::BookId;
use bible_rs::book_filter::{self, Books, Pick};
use bible_rs::theme::{self, Theme};
use eframe::egui::{self, Key};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const JOHN: BookId = BookId(42);

/// A select standing in for the search sidebar's, with its menu open; the state is the last pick.
fn open() -> Harness<'static, Option<Pick>> {
    let mut harness = Harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(
        |ui, pick: &mut Option<Pick>| {
            let anchor = ui.button("All books");
            let all: Vec<BookId> = BookId::all().collect();
            let books = Books {
                current: None,
                here: Some(JOHN),
                listed: &all,
                searchable: &all,
                name: &|b: BookId| b.name().to_string(),
                width: 200.0,
            };
            if let Some(p) = book_filter::show(&anchor, Theme::Light.palette(), books) {
                *pick = Some(p);
            }
        },
        None,
    );
    harness.ctx.set_fonts(theme::font_definitions());
    theme::apply(&harness.ctx, Theme::Light);
    harness.run();
    harness.get_by_label("All books").click();
    harness.run();
    harness
}

fn type_text(harness: &mut Harness<'_, Option<Pick>>, text: &str) {
    harness.event(egui::Event::Text(text.into()));
    harness.run();
}

#[test]
fn nothing_typed_lists_every_book_under_all_books_and_this_book() {
    let harness = open();
    assert!(harness.query_by_label("John (this book)").is_some());
    assert!(harness.query_by_label("Genesis").is_some());
}

#[test]
fn typing_narrows_and_return_chooses_the_best_match() {
    let mut harness = open();
    type_text(&mut harness, "1co");
    assert!(harness.query_by_label("1 Corinthians").is_some());
    assert!(harness.query_by_label("Genesis").is_none(), "the list narrows to the matches");
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), Some(Pick::Book(BookId(45))));
}

#[test]
fn the_arrows_move_through_the_matches() {
    let mut harness = open();
    // "jo": John, then Job.
    type_text(&mut harness, "jo");
    harness.key_press(Key::ArrowDown);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), Some(Pick::Book(BookId(17))));
}

#[test]
fn return_with_nothing_typed_keeps_all_books() {
    let mut harness = open();
    harness.key_press(Key::Enter);
    harness.run();
    assert_eq!(*harness.state(), Some(Pick::All));
}
