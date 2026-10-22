//! `bible://` deep links: `bible://jhn.3.16`, `bible://jhn.3.16-18`, `bible://rom.8`.
//!
//! The path is a passage in the format route.bible uses for its URLs: the book's lower-case
//! USFM code, chapter and verse joined by dots, and a same-chapter range as `-<verse>`. So
//! `https://route.bible/jhn.3.16-18` and `bible://jhn.3.16-18` name the same passage. OSIS
//! book names (`John.3.16`, `1Cor.13.4-7`) are accepted too, and a path without dots goes to
//! the ⌘K parser, so `bible://jn3:16` and `bible://John%203:16` work as well.
//!
//! A range that ends in another chapter (`jhn.3.16-jhn.4.2`) opens at its first verse: the
//! reader's selection stays within one chapter.

use crate::books::{BookId, ChapterIdx};
use crate::reference::{self, Reference};

pub const SCHEME: &str = "bible";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLink {
    pub reference: Reference,
    /// `?translation=web`: a translation id, lower-case. The app ignores ids it doesn't have.
    pub translation: Option<String>,
}

/// Parses `bible://<passage>[?translation=<id>]` (also `bible:<passage>`). The scheme is
/// optional, so a bare `jhn.3.16` parses the same way.
pub fn parse(input: &str) -> Option<DeepLink> {
    let s = input.trim();
    let s = match s.split_once(':') {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case(SCHEME) => rest.trim_start_matches('/'),
        _ => s,
    };
    let s = s.split('#').next().unwrap_or_default();
    let (path, query) = s.split_once('?').unwrap_or((s, ""));
    let path = percent_decode(path);
    let path = path.trim().trim_end_matches('/');
    // A dotted path is route.bible's format and nothing else; other paths are typed references.
    let dotted = path.contains('.') && !path.contains([' ', ':']);
    let reference = if dotted { passage(path)? } else { reference::parse(path)? };
    let translation = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == "translation")
        .map(|(_, v)| percent_decode(v).trim().to_lowercase())
        .filter(|v| !v.is_empty());
    Some(DeepLink { reference, translation })
}

/// The canonical link for a reference: `bible://jhn.3.16-18`.
pub fn format(r: &Reference) -> String {
    let mut s = format!("{SCHEME}://{}.{}", r.book.info().code.to_lowercase(), r.chapter);
    if let Some(v) = r.verse {
        s.push_str(&format!(".{v}"));
        if let Some(e) = r.verse_end.filter(|&e| e > v) {
            s.push_str(&format!("-{e}"));
        }
    }
    s
}

/// `<book>[.<chapter>[.<verse>[-<verse>|-<book>.<chapter>.<verse>]]]`, book as a USFM code or
/// an OSIS name, any case.
fn passage(path: &str) -> Option<Reference> {
    let (start, end) = match path.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (path, None),
    };
    let mut parts = start.split('.');
    let book = book(parts.next()?)?;
    let nums: Vec<u8> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let (chapter, verse) = match nums[..] {
        [] => (1, None),
        // Single-chapter books take a verse: `jud.3` is Jude 3 (as `jude 3` is in ⌘K).
        [n] if book.chapters() == 1 && n > 1 => (1, Some(n)),
        [c] => (c, None),
        [c, v] => (c, Some(v)),
        _ => return None,
    };
    let chapter_idx = ChapterIdx::new(book, chapter)?;
    if verse == Some(0) {
        return None;
    }
    let verse_end = match (verse, end) {
        (Some(v), Some(end)) => {
            range_end(book, chapter, end)?.map(|e| e.min(chapter_idx.kjv_verses())).filter(|&e| e > v)
        }
        (None, Some(_)) => return None,
        (_, None) => None,
    };
    Some(Reference { book, chapter, verse, verse_end })
}

/// The end of a range: `18` or `jhn.3.18` in the same chapter gives `Some(18)`; another chapter
/// gives `None` (opened at the first verse).
fn range_end(book: BookId, chapter: u8, end: &str) -> Option<Option<u8>> {
    if let Ok(v) = end.parse::<u8>() {
        return Some(Some(v));
    }
    let mut parts = end.split('.');
    let end_book = self::book(parts.next()?)?;
    let nums: Vec<u8> = parts.map(|p| p.parse().ok()).collect::<Option<_>>()?;
    match nums[..] {
        [c, v] if end_book == book && c == chapter => Some(Some(v)),
        [_, _] => Some(None),
        _ => None,
    }
}

fn book(s: &str) -> Option<BookId> {
    BookId::from_code(s).or_else(|| BookId::from_osis(s))
}

/// Decodes `%XX` escapes; `+` stays as it is (it is only a space in form data).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::books::BOOKS;

    fn p(s: &str) -> String {
        parse(s).map(|l| l.reference.display()).unwrap_or_else(|| "-".into())
    }

    #[test]
    fn route_bible_paths() {
        assert_eq!(p("bible://jhn.3.16"), "John 3:16");
        assert_eq!(p("bible://jhn.3.16-18"), "John 3:16–18");
        assert_eq!(p("bible://jhn.3"), "John 3");
        assert_eq!(p("bible://psa.23"), "Psalm 23");
        assert_eq!(p("bible://php.4.11-13"), "Philippians 4:11–13");
        assert_eq!(p("bible://1co.13.4-7"), "1 Corinthians 13:4–7");
        assert_eq!(p("bible://jhn.3.16-jhn.3.18"), "John 3:16–18");
        assert_eq!(p("bible://jhn"), "John 1");
        assert_eq!(p("bible:jhn.3.16"), "John 3:16");
        assert_eq!(p("BIBLE://JHN.3.16"), "John 3:16");
        assert_eq!(p("bible://jhn.3.16/"), "John 3:16");
        assert_eq!(p("jhn.3.16"), "John 3:16");
    }

    #[test]
    fn osis_and_free_form() {
        assert_eq!(p("bible://John.3.16"), "John 3:16");
        assert_eq!(p("bible://John.3.16-18"), "John 3:16–18");
        assert_eq!(p("bible://John.3.16-John.3.18"), "John 3:16–18");
        assert_eq!(p("bible://Rom.8"), "Romans 8");
        assert_eq!(p("bible://1Cor.13.4-7"), "1 Corinthians 13:4–7");
        assert_eq!(p("bible://Phlm.1.6"), "Philemon 1:6");
        assert_eq!(p("bible://jn3:16"), "John 3:16");
        assert_eq!(p("bible://jn3:16-18"), "John 3:16–18");
        assert_eq!(p("bible://rom8"), "Romans 8");
    }

    #[test]
    fn single_chapter_books() {
        assert_eq!(p("bible://jud.1.3"), "Jude 1:3");
        assert_eq!(p("bible://jud.3"), "Jude 1:3");
        assert_eq!(p("bible://jud.1"), "Jude 1");
        assert_eq!(p("bible://3jn.1.14"), "3 John 1:14");
    }

    #[test]
    fn cross_chapter_ranges_open_at_the_first_verse() {
        let l = parse("bible://jhn.3.16-jhn.4.2").unwrap();
        assert_eq!(l.reference.display(), "John 3:16");
        assert_eq!(l.reference.verse_end, None);
        assert_eq!(p("bible://php.3.20-php.4.1"), "Philippians 3:20");
    }

    #[test]
    fn ranges_are_kept_inside_the_chapter() {
        // John 3 has 36 verses (KJV).
        assert_eq!(p("bible://jhn.3.30-99"), "John 3:30–36");
        // A backwards range is the first verse alone.
        assert_eq!(p("bible://jhn.3.18-16"), "John 3:18");
    }

    #[test]
    fn percent_encoding_and_spaces() {
        assert_eq!(p("bible://John%203:16"), "John 3:16");
        assert_eq!(p("bible://John%203%3A16-18"), "John 3:16–18");
        assert_eq!(p("bible://1%20Cor%2013"), "1 Corinthians 13");
        assert_eq!(p("bible://John 3:16"), "John 3:16");
        assert_eq!(p("  bible://jhn.3.16\n"), "John 3:16");
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
    }

    #[test]
    fn translation_query() {
        let l = parse("bible://jhn.3.16?translation=WEB").unwrap();
        assert_eq!(l.translation.as_deref(), Some("web"));
        assert_eq!(l.reference.display(), "John 3:16");
        let l = parse("bible://rom.8?src=x&translation=bsb#top").unwrap();
        assert_eq!(l.translation.as_deref(), Some("bsb"));
        assert_eq!(parse("bible://rom.8?translation=").unwrap().translation, None);
        assert_eq!(parse("bible://rom.8").unwrap().translation, None);
    }

    #[test]
    fn bad_input() {
        for s in [
            "",
            "bible://",
            "bible://xyzzy",
            "bible://jhn.22",
            "bible://jhn.3.0",
            "bible://jhn.3.16.1",
            "bible://jhn.3-5",
            "bible://psa.151",
            "bible://3:16",
        ] {
            assert!(parse(s).is_none(), "{s:?} parsed as {:?}", parse(s));
        }
    }

    /// Every book code route.bible uses (grab-bcv's `OSIS_BOOK_CODES`, in canon order).
    const ROUTE_BIBLE_CODES: [&str; 66] = [
        "gen", "exo", "lev", "num", "deu", "jos", "jdg", "rut", "1sa", "2sa", "1ki", "2ki", "1ch", "2ch",
        "ezr", "neh", "est", "job", "psa", "pro", "ecc", "sng", "isa", "jer", "lam", "ezk", "dan", "hos",
        "jol", "amo", "oba", "jon", "mic", "nam", "hab", "zep", "hag", "zec", "mal", "mat", "mrk", "luk",
        "jhn", "act", "rom", "1co", "2co", "gal", "eph", "php", "col", "1th", "2th", "1ti", "2ti", "tit",
        "phm", "heb", "jas", "1pe", "2pe", "1jn", "2jn", "3jn", "jud", "rev",
    ];

    #[test]
    fn every_route_bible_book_code() {
        for (i, code) in ROUTE_BIBLE_CODES.iter().enumerate() {
            let book = BookId(i as u8);
            let l = parse(&format!("bible://{code}.1.1")).unwrap_or_else(|| panic!("{code}"));
            assert_eq!(l.reference.book, book, "{code}");
            assert_eq!(l.reference.verse, Some(1));
            let last = book.chapters();
            assert_eq!(parse(&format!("bible://{code}.{last}")).unwrap().reference.chapter, last, "{code}");
        }
    }

    #[test]
    fn every_osis_book_name() {
        for (i, b) in BOOKS.iter().enumerate() {
            let l = parse(&format!("bible://{}.1.1", b.osis)).unwrap_or_else(|| panic!("{}", b.osis));
            assert_eq!(l.reference.book, BookId(i as u8), "{}", b.osis);
        }
    }

    #[test]
    fn format_round_trips() {
        for s in ["bible://jhn.3.16", "bible://jhn.3.16-18", "bible://rom.8", "bible://1co.13.4-7"] {
            assert_eq!(format(&parse(s).unwrap().reference), s);
        }
        for (i, code) in ROUTE_BIBLE_CODES.iter().enumerate() {
            let r = Reference { book: BookId(i as u8), chapter: 1, verse: Some(2), verse_end: Some(3) };
            assert_eq!(format(&r), format!("bible://{code}.1.2-3"));
        }
    }
}
