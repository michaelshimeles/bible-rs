//! Keyboard shortcuts as the design lists them (frames 26 and 29): every shortcut as a row in a
//! group, each shown as keycaps. Rows that do one thing can be given another chord in Settings ›
//! Shortcuts; fixed rows (previous / next, the 1–6 colours, the ⌘ chords…) keep theirs.

use std::collections::BTreeMap;

use eframe::egui::{Key, Modifiers};

use crate::actions::Action;

/// A key with modifiers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub key: Key,
}

impl Chord {
    const fn new(key: Key) -> Self {
        Self { ctrl: false, alt: false, shift: false, cmd: false, key }
    }

    const fn cmd(key: Key) -> Self {
        Self { cmd: true, ..Self::new(key) }
    }

    const fn shift_cmd(key: Key) -> Self {
        Self { shift: true, cmd: true, ..Self::new(key) }
    }

    const fn ctrl_cmd(key: Key) -> Self {
        Self { ctrl: true, cmd: true, ..Self::new(key) }
    }

    pub fn modifiers(&self) -> Modifiers {
        let mut m = Modifiers::NONE;
        if self.ctrl {
            m |= Modifiers::CTRL;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.cmd {
            m |= Modifiers::COMMAND;
        }
        m
    }

    /// With a command or control key it works while typing in a field.
    pub fn is_command(&self) -> bool {
        self.cmd || self.ctrl
    }

    fn modifier_count(&self) -> usize {
        [self.ctrl, self.alt, self.shift, self.cmd].iter().filter(|b| **b).count()
    }

    /// Keycaps in the macOS order: ⌃ ⌥ ⇧ ⌘, then the key.
    pub fn keycaps(&self) -> Vec<String> {
        let mut caps = Vec::new();
        for (on, glyph) in [(self.ctrl, "⌃"), (self.alt, "⌥"), (self.shift, "⇧"), (self.cmd, "⌘")] {
            if on {
                caps.push(glyph.to_string());
            }
        }
        caps.push(key_glyph(self.key));
        caps
    }

    /// "ctrl+cmd+F", as stored in the settings.
    pub fn spec(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for (on, name) in [(self.ctrl, "ctrl"), (self.alt, "alt"), (self.shift, "shift"), (self.cmd, "cmd")] {
            if on {
                parts.push(name);
            }
        }
        parts.push(self.key.name());
        parts.join("+")
    }

    pub fn parse(spec: &str) -> Option<Chord> {
        let mut c = Chord::new(Key::A);
        let mut key = None;
        for part in spec.split('+') {
            match part {
                "ctrl" => c.ctrl = true,
                "alt" => c.alt = true,
                "shift" => c.shift = true,
                "cmd" => c.cmd = true,
                k => key = Key::from_name(k),
            }
        }
        c.key = key?;
        Some(c)
    }

    /// A chord from a key press, if it's one a shortcut can use.
    pub fn from_press(key: Key, m: Modifiers) -> Option<Chord> {
        if matches!(key, Key::Escape | Key::Tab | Key::Enter | Key::Backspace | Key::Delete) {
            return None;
        }
        Some(Chord { ctrl: m.ctrl, alt: m.alt, shift: m.shift, cmd: m.command || m.mac_cmd, key })
    }
}

fn key_glyph(k: Key) -> String {
    match k {
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::Plus | Key::Equals => "+".into(),
        Key::Minus => "−".into(),
        Key::OpenBracket => "[".into(),
        Key::CloseBracket => "]".into(),
        Key::Comma => ",".into(),
        Key::Period => ".".into(),
        Key::Slash => "/".into(),
        Key::Space => "Space".into(),
        k => k.symbol_or_name().to_uppercase(),
    }
}

/// One row of the list.
pub struct Shortcut {
    pub id: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    /// The default chord of a row that can be changed; `None` for a fixed row.
    pub default: Option<Chord>,
    /// A fixed row's keycaps.
    pub fixed: &'static [&'static str],
}

const fn bindable(id: &'static str, group: &'static str, label: &'static str, chord: Chord) -> Shortcut {
    Shortcut { id, group, label, default: Some(chord), fixed: &[] }
}

const fn fixed(
    id: &'static str,
    group: &'static str,
    label: &'static str,
    caps: &'static [&'static str],
) -> Shortcut {
    Shortcut { id, group, label, default: None, fixed: caps }
}

pub const GROUPS: [&str; 6] = ["Navigate", "Search", "Study", "Translations", "View", "App"];

/// How the shortcuts overlay stacks the groups in its three columns.
pub const COLUMNS: [&[&str]; 3] = [&["Navigate", "Search"], &["Study", "Translations"], &["View", "App"]];

/// Every shortcut, as the shortcuts overlay (`?`) and Settings › Shortcuts list them.
/// `BibleApp::shortcuts` acts on them: a changeable row through [`action`], a fixed row by its own
/// line there. A new shortcut needs a row here to be listed.
pub const ALL: &[Shortcut] = &[
    bindable("jump", "Navigate", "Jump to a reference", Chord::cmd(Key::K)),
    fixed("chapter", "Navigate", "Previous / next chapter", &["←", "→"]),
    fixed("history", "Navigate", "Back / forward", &["⌘", "[", "⌘", "]"]),
    fixed("history-arrows", "Navigate", "Back / forward (arrows)", &["⌥", "←", "⌥", "→"]),
    fixed("scroll", "Navigate", "Scroll", &["↑", "↓"]),
    fixed("page", "Navigate", "Page down / up", &["Space", "⇧", "Space"]),
    bindable("sidebar", "Navigate", "Books & study sidebar", Chord::cmd(Key::B)),
    bindable("search", "Search", "Search", Chord::cmd(Key::F)),
    fixed("search-slash", "Search", "Search (while reading)", &["/"]),
    fixed("search-step", "Search", "Previous / next result", &["↑", "↓"]),
    fixed("search-open", "Search", "Go to the result", &["⏎"]),
    fixed("search-clear", "Search", "Clear the search", &["Esc"]),
    fixed("range", "Study", "Select a verse range", &["⇧", "Click"]),
    fixed("verse", "Study", "Next / previous verse", &["J", "K"]),
    fixed("deselect", "Study", "Clear the selection", &["Esc"]),
    fixed("highlight", "Study", "Highlight colour · clear", &["1–6", "0"]),
    bindable("note", "Study", "Note", Chord::new(Key::N)),
    bindable("bookmark", "Study", "Bookmark", Chord::new(Key::B)),
    fixed("bookmark-cmd", "Study", "Bookmark (anywhere)", &["⌘", "D"]),
    fixed("copy-text", "Study", "Copy", &["⌘", "C"]),
    bindable("copy", "Study", "Copy with reference", Chord::shift_cmd(Key::C)),
    bindable("compare", "Study", "Compare translations", Chord::new(Key::C)),
    fixed("translation", "Translations", "Switch translation", &["⌘", "1…9"]),
    bindable("parallel", "Translations", "Parallel translations", Chord::shift_cmd(Key::P)),
    fixed("import", "Translations", "Import a translation", &["⌘", "I"]),
    fixed("size", "View", "Text size", &["⌘", "+", "⌘", "−"]),
    fixed("size-reset", "View", "Default text size", &["⌘", "0"]),
    fixed("text", "View", "Text & layout", &["⌘", "T"]),
    bindable("mode", "View", "Paragraphs / verse per line", Chord::cmd(Key::L)),
    bindable("red", "View", "Red letters", Chord::cmd(Key::R)),
    bindable("theme", "View", "Theme", Chord::shift_cmd(Key::L)),
    bindable("focus", "View", "Focus mode", Chord::ctrl_cmd(Key::F)),
    fixed("help", "App", "Keyboard shortcuts", &["?"]),
    fixed("settings", "App", "Settings", &["⌘", ","]),
    fixed("export", "App", "Export as Markdown", &["⌘", "E"]),
    fixed("export-json", "App", "Export as JSON", &["⇧", "⌘", "E"]),
];

/// What a changeable row does.
pub fn action(id: &str) -> Option<Action> {
    Some(match id {
        "jump" => Action::OpenPalette,
        "search" => Action::OpenSearch(None),
        "sidebar" => Action::ToggleSidebar,
        "note" => Action::EditSelectedNote,
        "bookmark" => Action::ToggleBookmark,
        "copy" => Action::Copy,
        "compare" => Action::Compare,
        "parallel" => Action::ToggleParallel,
        "mode" => Action::ToggleMode,
        "red" => Action::ToggleRedLetters,
        "theme" => Action::CycleTheme,
        "focus" => Action::ToggleFocus,
        _ => return None,
    })
}

/// A row's chord: the one chosen in Settings, else its default.
pub fn chord(s: &Shortcut, overrides: &BTreeMap<String, String>) -> Option<Chord> {
    match overrides.get(s.id) {
        Some(spec) => Chord::parse(spec),
        None => s.default,
    }
}

/// A row's keycaps as shown.
pub fn keycaps(s: &Shortcut, overrides: &BTreeMap<String, String>) -> Vec<String> {
    match s.default {
        Some(_) => chord(s, overrides).map(|c| c.keycaps()).unwrap_or_default(),
        None => s.fixed.iter().map(|c| c.to_string()).collect(),
    }
}

/// The changeable rows' chords and actions, those with more modifiers first (egui matches
/// shift loosely, so ⌘L would otherwise also take ⌘⇧L).
pub fn bindings(overrides: &BTreeMap<String, String>) -> Vec<(Chord, Action)> {
    let mut out: Vec<(Chord, Action)> =
        ALL.iter().filter_map(|s| Some((chord(s, overrides)?, action(s.id)?))).collect();
    out.sort_by_key(|(c, _)| std::cmp::Reverse(c.modifier_count()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_and_read_as_keycaps() {
        let c = Chord::ctrl_cmd(Key::F);
        assert_eq!(c.spec(), "ctrl+cmd+F");
        assert_eq!(Chord::parse(&c.spec()), Some(c));
        assert_eq!(c.keycaps(), ["⌃", "⌘", "F"]);
        assert_eq!(Chord::shift_cmd(Key::L).keycaps(), ["⇧", "⌘", "L"]);
        let mut overrides = BTreeMap::new();
        overrides.insert("note".to_string(), "shift+N".to_string());
        let note = ALL.iter().find(|s| s.id == "note").unwrap();
        assert_eq!(keycaps(note, &overrides), ["⇧", "N"]);
        // More modifiers first.
        let b = bindings(&BTreeMap::new());
        assert!(b.first().unwrap().0.modifier_count() >= b.last().unwrap().0.modifier_count());
    }

    #[test]
    fn every_row_is_in_a_listed_group_and_column_once() {
        let mut ids = std::collections::BTreeSet::new();
        for s in ALL {
            assert!(ids.insert(s.id), "{} is listed twice", s.id);
            assert!(GROUPS.contains(&s.group), "{} has no group", s.id);
            assert!(s.default.is_some() || !s.fixed.is_empty(), "{} has no keys", s.id);
        }
        let stacked: Vec<&str> = COLUMNS.iter().flat_map(|c| c.iter().copied()).collect();
        assert_eq!(stacked.len(), GROUPS.len());
        assert!(GROUPS.iter().all(|g| stacked.contains(g)));
        // Every row Settings can change does something.
        assert!(ALL.iter().filter(|s| s.default.is_some()).all(|s| action(s.id).is_some()));
    }
}
