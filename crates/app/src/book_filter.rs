//! The menu under an "All books" select (the search sidebar's scope, Study › Highlights): a field
//! to type a book's name and the books below it. Typing narrows the list, reading names the way
//! the jump palette does ("jn", "1co", "song", "revalation"); ↑ ↓ move the highlight, return
//! chooses it and escape closes, so a book you know is a few keys away.

use bible_core::BookId;
use eframe::egui::{self, Align2, Key, Modifiers, Response, ScrollArea, Sense, Ui, vec2};

use crate::icons;
use crate::menu::{self, ITEM_HEIGHT};
use crate::theme::{Palette, text};
use crate::widgets;

/// What the menu chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    All,
    Book(BookId),
}

/// What the menu offers.
pub struct Books<'a> {
    /// The current choice (`None`: all books).
    pub current: Option<BookId>,
    /// The book being read, offered as "Name (this book)" under "All books".
    pub here: Option<BookId>,
    /// The books listed before anything is typed.
    pub listed: &'a [BookId],
    /// The books typing can find.
    pub searchable: &'a [BookId],
    /// A book's name in the list.
    pub name: &'a dyn Fn(BookId) -> String,
    /// The menu's width.
    pub width: f32,
}

/// The typed filter and the highlighted row, kept while the menu is open.
#[derive(Clone, Default)]
struct State {
    query: String,
    /// Row index; `None` until the keyboard or pointer moves it (then the current choice's row).
    active: Option<usize>,
}

struct Row {
    pick: Pick,
    label: String,
    selected: bool,
}

/// The rows for what's typed: "All books", this book and the listed books when nothing is (with
/// the index the separator goes before), otherwise the matches, best first.
fn rows(books: &Books<'_>, query: &str) -> (Vec<Row>, Option<usize>) {
    let row =
        |b: BookId, label: String| Row { pick: Pick::Book(b), label, selected: books.current == Some(b) };
    if query.trim().is_empty() {
        let mut out =
            vec![Row { pick: Pick::All, label: "All books".into(), selected: books.current.is_none() }];
        if let Some(b) = books.here {
            out.push(row(b, format!("{} (this book)", (books.name)(b))));
        }
        let separator = out.len();
        out.extend(books.listed.iter().map(|&b| row(b, (books.name)(b))));
        return (out, Some(separator));
    }
    let matches = bible_core::reference::filter_books(query)
        .into_iter()
        .filter(|b| books.searchable.contains(b))
        .map(|b| row(b, (books.name)(b)))
        .collect();
    (matches, None)
}

/// The filter menu opening from `anchor`. Returns the choice the frame it's made.
pub fn show(anchor: &Response, p: &Palette, books: Books<'_>) -> Option<Pick> {
    let id = anchor.id.with("book-filter");
    let ctx = anchor.ctx.clone();
    let mut state: State = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut pick = None;
    let shown =
        menu::popup(anchor, p).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(books.width);
            let field_id = id.with("field");

            // Arrows move the highlight and return chooses; the field keeps the focus throughout.
            let (mut step, enter) = ui.input_mut(|i| {
                let mut step = 0i32;
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    step += 1;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    step -= 1;
                }
                (step, i.key_pressed(Key::Enter))
            });

            let before = state.query.clone();
            let (field, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
            let resp = widgets::text_field(
                ui,
                p,
                field,
                &mut state.query,
                "Type a book",
                Some(icons::search),
                false,
                false,
                field_id,
            );
            resp.request_focus();
            if state.query != before {
                state.active = None;
                step = 0;
            }
            ui.add_space(4.0);

            let (rows, separator) = rows(&books, &state.query);
            // Typing lights the best match; otherwise the current choice starts lit.
            let default =
                if separator.is_some() { rows.iter().position(|r| r.selected).unwrap_or(0) } else { 0 };
            let mut active = state.active.unwrap_or(default).min(rows.len().saturating_sub(1));
            if step != 0 && !rows.is_empty() {
                active = (active as i32 + step).rem_euclid(rows.len() as i32) as usize;
                state.active = Some(active);
            }
            if enter && let Some(r) = rows.get(active) {
                pick = Some(r.pick);
            }

            if rows.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ITEM_HEIGHT), Sense::hover());
                ui.painter().text(
                    egui::pos2(menu::text_left(r), r.center().y),
                    Align2::LEFT_CENTER,
                    "No books match",
                    text::UI.font(),
                    p.ink_3,
                );
            }

            let mut paint_row = |ui: &mut Ui, i: usize, row: &Row, state: &mut State| {
                let (r, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ITEM_HEIGHT), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::Default);
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        row.selected,
                        &row.label,
                    )
                });
                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    state.active = Some(i);
                }
                let lit = i == state.active.unwrap_or(active);
                if lit && step != 0 {
                    resp.scroll_to_me(None);
                }
                menu::paint_cell(
                    ui.painter(),
                    p,
                    r,
                    menu::Cell {
                        icon: None,
                        label: &row.label,
                        centred: false,
                        selected: row.selected,
                        lit,
                        pressed: resp.is_pointer_button_down_on(),
                        muted: false,
                    },
                );
                if resp.clicked() {
                    pick = Some(row.pick);
                }
            };

            let head = separator.unwrap_or(0);
            for (i, row) in rows.iter().enumerate().take(head) {
                paint_row(ui, i, row, &mut state);
            }
            if separator.is_some() {
                menu::separator(ui);
            }
            ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (i, row) in rows.iter().enumerate().skip(head) {
                    paint_row(ui, i, row, &mut state);
                }
            });
            if pick.is_some() {
                ui.close();
            }
        });
    if shown.is_some() && pick.is_none() {
        ctx.data_mut(|d| d.insert_temp(id, state));
    } else {
        // Closed: it opens again with nothing typed.
        ctx.data_mut(|d| d.remove::<State>(id));
    }
    pick
}
