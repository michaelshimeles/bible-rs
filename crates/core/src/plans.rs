//! Reading plans: a catalogue of day-by-day readings, generated from the canon's chapter lists.
//! Progress (which plans are under way, which passages are read) lives in the study store.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::books::{BookId, ChapterIdx};
use crate::reference;

/// One reading: a whole chapter, or verses of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Passage {
    pub chapter: ChapterIdx,
    pub verses: Option<(u8, u8)>,
}

impl Passage {
    pub fn whole(chapter: ChapterIdx) -> Self {
        Self { chapter, verses: None }
    }

    /// Verses `start` through `end` of a chapter, inclusive.
    pub fn verses(chapter: ChapterIdx, start: u8, end: u8) -> Self {
        let (a, b) = if end < start { (end, start) } else { (start, end) };
        Self { chapter, verses: Some((a, b)) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanIcon {
    Calendar,
    Book,
    Quote,
    History,
    Flame,
    Note,
}

#[derive(Debug)]
pub struct Plan {
    pub id: &'static str,
    pub title: &'static str,
    /// Length and pace ("90 days · ~3 chapters a day").
    pub meta: &'static str,
    pub description: &'static str,
    pub icon: PlanIcon,
    pub days: Vec<Vec<Passage>>,
}

impl Plan {
    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// The passages for day `n` (1-based).
    pub fn day(&self, n: usize) -> &[Passage] {
        n.checked_sub(1).and_then(|i| self.days.get(i)).map_or(&[], Vec::as_slice)
    }
}

fn book_chapters(books: impl IntoIterator<Item = u8>) -> Vec<ChapterIdx> {
    books
        .into_iter()
        .flat_map(|b| {
            let b = BookId(b);
            let first = b.first_chapter();
            (0..b.chapters() as u16).map(move |i| ChapterIdx(first.0 + i))
        })
        .collect()
}

/// Chapters spread evenly over `days` days, in order.
fn spread(chapters: &[ChapterIdx], days: usize) -> Vec<Vec<Passage>> {
    let n = chapters.len();
    (0..days)
        .map(|d| chapters[d * n / days..(d + 1) * n / days].iter().map(|c| Passage::whole(*c)).collect())
        .collect()
}

/// Several streams read side by side, each spread over the same days.
fn streams(parts: &[Vec<ChapterIdx>], days: usize) -> Vec<Vec<Passage>> {
    let spread: Vec<_> = parts.iter().map(|p| spread(p, days)).collect();
    (0..days).map(|d| spread.iter().flat_map(|s| s[d].iter().copied()).collect()).collect()
}

const PSALMS: u8 = 18;
const PROVERBS: u8 = 19;
const MATTHEW: u8 = 39;
const JOHN: u8 = 42;

fn build() -> Vec<Plan> {
    let gospels = book_chapters(MATTHEW..=JOHN);
    let nt = book_chapters(MATTHEW..66);
    let ot_without_psalms = book_chapters((0..MATTHEW).filter(|b| *b != PSALMS));
    let psalms = book_chapters([PSALMS]);
    let twice = |v: &Vec<ChapterIdx>| v.iter().chain(v.iter()).copied().collect::<Vec<_>>();
    let ps = |n: u16| Passage::whole(ChapterIdx(BookId(PSALMS).first_chapter().0 + n - 1));
    let mt = |c: u8, a: u8, b: u8| Passage {
        chapter: ChapterIdx::new(BookId(MATTHEW), c).expect("Matthew"),
        verses: Some((a, b)),
    };
    vec![
        Plan {
            id: "gospels-30",
            title: "Gospels in 30 Days",
            meta: "30 days · ~3 chapters a day",
            description: "Matthew, Mark, Luke and John — about 3 chapters a day.",
            icon: PlanIcon::Book,
            days: spread(&gospels, 30),
        },
        Plan {
            id: "year-streams",
            title: "Through the Bible in a Year",
            meta: "365 days · ~4 chapters a day",
            description: "Old Testament once, New Testament and Psalms twice.",
            icon: PlanIcon::Calendar,
            days: streams(&[ot_without_psalms, twice(&nt), twice(&psalms)], 365),
        },
        Plan {
            id: "nt-90",
            title: "New Testament in 90 Days",
            meta: "90 days · ~3 chapters a day",
            description: "Matthew to Revelation at a steady pace.",
            icon: PlanIcon::Book,
            days: spread(&nt, 90),
        },
        Plan {
            id: "psalms-proverbs",
            title: "Psalms & Proverbs",
            meta: "31 days · 5 psalms + 1 proverb",
            description: "A month of prayer and wisdom, every evening.",
            icon: PlanIcon::Quote,
            days: (1..=31u16)
                .map(|d| {
                    let mut day: Vec<Passage> =
                        (0..5).map(|k| d + 30 * k).filter(|n| *n <= 150).map(ps).collect();
                    day.push(Passage::whole(ChapterIdx::new(BookId(PROVERBS), d as u8).expect("Proverbs")));
                    day
                })
                .collect(),
        },
        Plan {
            id: "bible-year",
            title: "Bible in a Year",
            meta: "365 days · ~3 chapters a day",
            description: "Genesis to Revelation, in order.",
            icon: PlanIcon::History,
            days: spread(&ChapterIdx::all().collect::<Vec<_>>(), 365),
        },
        Plan {
            id: "sermon-mount",
            title: "Sermon on the Mount",
            meta: "7 days · Matthew 5–7",
            description: "A week with the longest teaching of Jesus.",
            icon: PlanIcon::Flame,
            days: vec![
                vec![mt(5, 1, 16)],
                vec![mt(5, 17, 32)],
                vec![mt(5, 33, 48)],
                vec![mt(6, 1, 18)],
                vec![mt(6, 19, 34)],
                vec![mt(7, 1, 14)],
                vec![mt(7, 15, 29)],
            ],
        },
        Plan {
            id: "john-21",
            title: "Gospel of John",
            meta: "21 days · a chapter a day",
            description: "Slow reading, one chapter at a time.",
            icon: PlanIcon::Note,
            days: spread(&book_chapters([JOHN]), 21),
        },
    ]
}

/// Every plan, in the order they're offered.
pub fn catalogue() -> &'static [Plan] {
    static PLANS: OnceLock<Vec<Plan>> = OnceLock::new();
    PLANS.get_or_init(build)
}

pub fn get(id: &str) -> Option<&'static Plan> {
    catalogue().iter().find(|p| p.id == id)
}

/// A day's passages as a short label: "Luke 5–6", "Psalms 1, 31", "Matthew 5:1–16".
pub fn label(passages: &[Passage], name: impl Fn(BookId) -> String) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < passages.len() {
        let p = passages[i];
        let book = p.chapter.book();
        if let Some((a, b)) = p.verses {
            parts.push(format!("{} {}:{a}–{b}", name(book), p.chapter.chapter()));
            i += 1;
            continue;
        }
        // A run of whole, consecutive chapters of one book.
        let mut j = i;
        while j + 1 < passages.len()
            && passages[j + 1].verses.is_none()
            && passages[j + 1].chapter.book() == book
            && passages[j + 1].chapter.0 == passages[j].chapter.0 + 1
        {
            j += 1;
        }
        let first = p.chapter.chapter();
        let last = passages[j].chapter.chapter();
        let range = if j > i { format!("{first}–{last}") } else { first.to_string() };
        match parts.last_mut() {
            // Further runs of the same book join the previous part: "Psalms 1, 31".
            Some(prev)
                if i > 0 && passages[i - 1].chapter.book() == book && passages[i - 1].verses.is_none() =>
            {
                prev.push_str(&format!(", {range}"));
            }
            _ => parts.push(format!("{} {range}", name(book))),
        }
        i = j + 1;
    }
    parts.join(" · ")
}

/// A day's passages as typed references that parse back: each passage keeps its book name
/// ("Psalms 1 · Psalms 3"), so an editor can save the label `label` would show as "Psalms 1, 3".
pub fn edit_label(passages: &[Passage], name: impl Fn(BookId) -> String + Copy) -> String {
    passages.iter().map(|p| label(std::slice::from_ref(p), name)).collect::<Vec<_>>().join(" · ")
}

/// How many days a plan you create may run (about five and a half years).
pub const MAX_DAYS: usize = 2000;
/// How many passages one day may hold.
pub const MAX_PASSAGES_PER_DAY: usize = 32;
pub const MAX_TITLE: usize = 120;
pub const MAX_DESCRIPTION: usize = 2000;

/// A book, chapter, and optional verse range, named so it can be stored and synced without tying
/// the data to this build's chapter-index numbering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassageRef {
    pub book: String,
    pub chapter: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verses: Option<(u8, u8)>,
}

impl PassageRef {
    pub fn from_passage(p: Passage) -> Self {
        Self {
            book: p.chapter.book().info().code.to_string(),
            chapter: p.chapter.chapter(),
            verses: p.verses,
        }
    }

    pub fn to_passage(&self) -> Option<Passage> {
        let book = BookId::from_code(&self.book)?;
        let chapter = ChapterIdx::new(book, self.chapter)?;
        Some(match self.verses {
            Some((a, b)) => Passage::verses(chapter, a, b),
            None => Passage::whole(chapter),
        })
    }
}

/// How to turn a list of passages into days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// This many passages (usually whole chapters) each day.
    ChaptersPerDay(u16),
    /// Spread the passages over this many days.
    InDays(u16),
}

/// A plan the reader wrote: title, optional description, and an ordered list of days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomPlan {
    pub id: String,
    pub title: String,
    pub description: String,
    pub days: Vec<Vec<Passage>>,
    /// Unix seconds, when it was first saved.
    pub created: i64,
}

impl CustomPlan {
    pub fn meta(&self) -> String {
        pace_label(&self.days)
    }

    pub fn resolved(&self) -> ResolvedPlan {
        ResolvedPlan {
            id: self.id.clone(),
            title: self.title.clone(),
            meta: self.meta(),
            description: self.description.clone(),
            icon: PlanIcon::Note,
            days: self.days.clone(),
            custom: true,
        }
    }

    pub fn days_as_refs(&self) -> Vec<Vec<PassageRef>> {
        self.days.iter().map(|d| d.iter().copied().map(PassageRef::from_passage).collect()).collect()
    }

    pub fn from_refs(
        id: String,
        title: String,
        description: String,
        days: &[Vec<PassageRef>],
        created: i64,
    ) -> Option<Self> {
        let days: Option<Vec<Vec<Passage>>> =
            days.iter().map(|d| d.iter().map(PassageRef::to_passage).collect()).collect();
        let days = days?;
        if days.iter().any(|d| d.is_empty()) {
            return None;
        }
        Some(Self { id, title, description, days, created })
    }
}

/// A built-in or custom plan, owned, so the same UI can show both.
#[derive(Debug, Clone)]
pub struct ResolvedPlan {
    pub id: String,
    pub title: String,
    pub meta: String,
    pub description: String,
    pub icon: PlanIcon,
    pub days: Vec<Vec<Passage>>,
    pub custom: bool,
}

impl ResolvedPlan {
    pub fn from_builtin(p: &Plan) -> Self {
        Self {
            id: p.id.to_string(),
            title: p.title.to_string(),
            meta: p.meta.to_string(),
            description: p.description.to_string(),
            icon: p.icon,
            days: p.days.clone(),
            custom: false,
        }
    }

    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    /// The passages for day `n` (1-based).
    pub fn day(&self, n: usize) -> &[Passage] {
        n.checked_sub(1).and_then(|i| self.days.get(i)).map_or(&[], Vec::as_slice)
    }
}

/// An id reserved for a plan the reader created (`custom-` and a unique suffix).
pub fn is_custom_id(id: &str) -> bool {
    id.starts_with("custom-") && id.len() > "custom-".len()
}

/// A fresh id that will not collide with the catalogue.
pub fn new_custom_id() -> String {
    format!("custom-{}", crate::sync::new_device_id())
}

/// Length and pace, e.g. "12 days · ~1 chapter a day".
pub fn pace_label(days: &[Vec<Passage>]) -> String {
    let n = days.len();
    if n == 0 {
        return "No days".into();
    }
    let passages = days.iter().map(Vec::len).sum::<usize>();
    let avg = (passages as f32 / n as f32).round().max(1.0) as usize;
    let whole = days.iter().all(|d| d.iter().all(|p| p.verses.is_none()));
    let unit = match (whole, avg) {
        (true, 1) => "chapter",
        (true, _) => "chapters",
        (false, 1) => "passage",
        (false, _) => "passages",
    };
    format!("{n} days · ~{avg} {unit} a day")
}

/// Split `items` into `days` groups, in order, the way built-in plans spread chapters.
pub fn spread_items<T: Clone>(items: &[T], days: usize) -> Vec<Vec<T>> {
    let n = items.len();
    if n == 0 || days == 0 {
        return Vec::new();
    }
    let days = days.min(n);
    (0..days)
        .map(|d| items[d * n / days..(d + 1) * n / days].to_vec())
        .filter(|day| !day.is_empty())
        .collect()
}

/// Turn a flat list of passages into days at the given pace.
pub fn schedule(items: &[Passage], pace: Pace) -> Result<Vec<Vec<Passage>>, String> {
    if items.is_empty() {
        return Err("Add some books or passages first".into());
    }
    let days = match pace {
        Pace::ChaptersPerDay(0) => return Err("Pace has to be at least one passage a day".into()),
        Pace::ChaptersPerDay(n) => items.chunks(n as usize).map(|c| c.to_vec()).collect(),
        Pace::InDays(0) => return Err("A plan needs at least one day".into()),
        Pace::InDays(d) => spread_items(items, d as usize),
    };
    if days.is_empty() {
        return Err("That schedule has no days".into());
    }
    if days.len() > MAX_DAYS {
        return Err("That would be more than 2,000 days".into());
    }
    if days.iter().any(|d| d.len() > MAX_PASSAGES_PER_DAY) {
        return Err("A day can hold at most 32 passages".into());
    }
    Ok(days)
}

/// One or more passages from typed references: "John 1", "John 1–3", "Matthew 5:1–16",
/// "Proverbs 3, James 1:2–8", or a book name for every chapter of that book. Does not invent
/// verse text — only the references.
pub fn parse_passages(input: &str) -> Result<Vec<Passage>, String> {
    let parts: Vec<&str> =
        input.split([',', ';', '\n', '·']).map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Err("Add a book, chapter, or verse range".into());
    }
    let mut out = Vec::new();
    let mut last_book: Option<BookId> = None;
    for part in parts {
        let carried;
        let token = if !part.chars().any(|c| c.is_ascii_alphabetic()) {
            if let Some(book) = last_book {
                carried = format!("{} {part}", book.name());
                carried.as_str()
            } else {
                part
            }
        } else {
            part
        };
        let added = parse_one(token)?;
        if let Some(p) = added.last() {
            last_book = Some(p.chapter.book());
        }
        out.extend(added);
    }
    Ok(out)
}

fn parse_one(token: &str) -> Result<Vec<Passage>, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Empty passage".into());
    }
    if let Some(book) = reference::parse_book(token) {
        let chapters = book_chapters([book.0]);
        if chapters.is_empty() {
            return Err(format!("Couldn't read “{token}” as a book"));
        }
        return Ok(chapters.into_iter().map(Passage::whole).collect());
    }
    if !token.contains(':')
        && !token.contains('.')
        && let Some((book, start, end)) = chapter_span(token)
    {
        if start < 1 || end < start {
            return Err(format!("“{token}” is not a chapter range"));
        }
        let mut out = Vec::new();
        for c in start..=end {
            let ch = ChapterIdx::new(book, c).ok_or_else(|| format!("{} has no chapter {c}", book.name()))?;
            out.push(Passage::whole(ch));
        }
        return Ok(out);
    }
    let r = reference::parse(token).ok_or_else(|| format!("Couldn't read “{token}” as a passage"))?;
    let verses = match (r.verse, r.verse_end) {
        (Some(a), Some(b)) => Some(if b >= a { (a, b) } else { (b, a) }),
        (Some(a), None) => Some((a, a)),
        (None, _) => None,
    };
    Ok(vec![Passage { chapter: r.chapter_idx(), verses }])
}

/// "John 1-3" / "John 1–3" as chapters 1 through 3 of John (not John 1:3).
fn chapter_span(token: &str) -> Option<(BookId, u8, u8)> {
    let pos = token.rfind(['-', '–', '—'])?;
    let (left, right) = (token[..pos].trim(), token[pos + token[pos..].chars().next()?.len_utf8()..].trim());
    let end: u8 = right.parse().ok()?;
    let r = reference::parse(left)?;
    if r.verse.is_some() {
        return None;
    }
    Some((r.book, r.chapter, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_cover_what_they_say() {
        let gospels = get("gospels-30").unwrap();
        assert_eq!(gospels.len(), 30);
        assert_eq!(gospels.days.iter().map(Vec::len).sum::<usize>(), 89);
        let year = get("year-streams").unwrap();
        assert_eq!(year.len(), 365);
        assert_eq!(year.days.iter().map(Vec::len).sum::<usize>(), 779 + 2 * 260 + 2 * 150);
        assert_eq!(get("bible-year").unwrap().days.iter().map(Vec::len).sum::<usize>(), 1189);
        let pp = get("psalms-proverbs").unwrap();
        assert_eq!(pp.day(1).len(), 6);
        assert_eq!(pp.day(31).len(), 5);
        let name = |b: BookId| b.name().to_string();
        assert_eq!(label(pp.day(1), name), "Psalms 1, 31, 61, 91, 121 · Proverbs 1");
        assert_eq!(label(get("sermon-mount").unwrap().day(1), name), "Matthew 5:1–16");
        let luke5 = ChapterIdx::new(BookId(41), 5).unwrap();
        let two = [Passage::whole(luke5), Passage::whole(ChapterIdx(luke5.0 + 1))];
        assert_eq!(label(&two, name), "Luke 5–6");
    }

    #[test]
    fn typed_passages_become_days() {
        let john = parse_passages("John").unwrap();
        assert_eq!(john.len(), 21);
        assert!(john.iter().all(|p| p.verses.is_none() && p.chapter.book().name() == "John"));
        let range = parse_passages("John 1–3").unwrap();
        assert_eq!(range.len(), 3);
        assert_eq!(range[0].chapter.chapter(), 1);
        assert_eq!(range[2].chapter.chapter(), 3);
        let mixed = parse_passages("Matthew 5:1–16, Proverbs 3").unwrap();
        assert_eq!(mixed.len(), 2);
        assert_eq!(mixed[0].verses, Some((1, 16)));
        assert!(mixed[1].verses.is_none());
        let many = parse_passages("John 1 · Prov 3:5–6").unwrap();
        assert_eq!(many.len(), 2);
        assert!(parse_passages("xyzzy").is_err());
        assert!(parse_passages("").is_err());
    }

    #[test]
    fn pace_spreads_chapters_without_empty_days() {
        let john = parse_passages("John").unwrap();
        let one = schedule(&john, Pace::ChaptersPerDay(1)).unwrap();
        assert_eq!(one.len(), 21);
        assert!(one.iter().all(|d| d.len() == 1));
        let three = schedule(&john, Pace::ChaptersPerDay(3)).unwrap();
        assert_eq!(three.len(), 7);
        assert_eq!(three.last().unwrap().len(), 3);
        let month = schedule(&john, Pace::InDays(10)).unwrap();
        assert_eq!(month.len(), 10);
        assert_eq!(month.iter().map(Vec::len).sum::<usize>(), 21);
        // More days than chapters: one chapter a day, no empty days.
        let stretched = schedule(&john, Pace::InDays(40)).unwrap();
        assert_eq!(stretched.len(), 21);
        assert!(pace_label(&one).starts_with("21 days"));
    }

    #[test]
    fn compact_labels_parse_back() {
        let a = parse_passages("Psalms 1, 3").unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].chapter.book().name(), "Psalms");
        assert_eq!(a[0].chapter.chapter(), 1);
        assert_eq!(a[1].chapter.chapter(), 3);
        let label = label(&a, |b| b.ref_name().to_string());
        assert!(label.contains('1') && label.contains('3'), "{label}");
        let again = parse_passages(&label).unwrap();
        assert_eq!(again.len(), 2);
        let edit = edit_label(&a, |b| b.ref_name().to_string());
        assert!(edit.contains("Psalm"), "{edit}");
        assert_eq!(parse_passages(&edit).unwrap().len(), 2);
    }

    #[test]
    fn numbered_books_cover_every_chapter() {
        let john = parse_passages("John").unwrap();
        assert_eq!(john.len(), 21);
        let first = parse_passages("1 John").unwrap();
        assert_eq!(first.len(), 5, "1 John has five chapters, not only chapter 1");
        assert!(first.iter().all(|p| p.chapter.book().name() == "1 John"));
        let only = parse_passages("1 John 1").unwrap();
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].chapter.chapter(), 1);
    }

    #[test]
    fn passage_refs_round_trip() {
        let p = parse_passages("Matthew 5:1–16").unwrap()[0];
        let r = PassageRef::from_passage(p);
        assert_eq!(r.book, "MAT");
        assert_eq!(r.chapter, 5);
        assert_eq!(r.verses, Some((1, 16)));
        assert_eq!(r.to_passage(), Some(p));
        assert!(is_custom_id(&new_custom_id()));
        assert!(!is_custom_id("john-21"));
        assert!(!is_custom_id("custom-"));
    }
}
