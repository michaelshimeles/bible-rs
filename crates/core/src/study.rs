//! Local study data: highlights, notes, bookmarks, reading history, plans, memorised passages and
//! settings.
//!
//! Everything is keyed by canonical verse reference (not by translation), so a highlight made
//! while reading the KJV shows up in the WEB too. A verse can hold several highlights, each over
//! part of it or all of it ([`crate::highlight`] places them in a translation's text).
//!
//! The store lives in memory — hundreds of rows, not millions — so the reader can look it up per
//! verse every frame. The native apps back it with
//! one SQLite file and write straight through (WAL mode, a few hundred microseconds each); the web
//! app has no SQLite and saves the store's JSON export instead.
//!
//! Every highlighted verse, note, bookmark, plan, ticked passage and memorised passage also
//! carries what sync needs
//! ([`crate::sync`]): when it last changed and on which device, and a local sequence number, so
//! [`StudyStore::changes_since`] can say what changed after a point. Deleting leaves a tombstone.
//! Importing a JSON export and taking in another copy's changes go through one merge,
//! [`StudyStore::merge`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
#[cfg(feature = "native")]
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::books::{BookId, ChapterIdx, VerseRef};
use crate::highlight;
pub use crate::highlight::{Highlight, TextSpan};
use crate::memory::{Grade, Memorised, Review, Schedule};
use crate::sync::{
    self, BookmarkChange, HighlightPart, HighlightsChange, Hlc, MemoryChange, MemoryReviewChange, NoteChange,
    NotePick, NoteVersion, Pick, PlanChange, PlanReadChange, VerseKey, lww,
};
pub use crate::sync::{Changes, MergeMode, MergeReport, NoteConflict, Stamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightColor {
    Yellow,
    Green,
    Blue,
    Pink,
    Purple,
    Orange,
}

impl HighlightColor {
    /// In key order: 1–6 apply these, 0 clears. The stored value is the index, so new colours
    /// only ever go at the end.
    pub const ALL: [HighlightColor; 6] = [
        HighlightColor::Yellow,
        HighlightColor::Green,
        HighlightColor::Blue,
        HighlightColor::Pink,
        HighlightColor::Purple,
        HighlightColor::Orange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HighlightColor::Yellow => "yellow",
            HighlightColor::Green => "green",
            HighlightColor::Blue => "blue",
            HighlightColor::Pink => "pink",
            HighlightColor::Purple => "purple",
            HighlightColor::Orange => "orange",
        }
    }

    #[cfg(feature = "native")]
    fn from_i64(v: i64) -> Option<Self> {
        Self::ALL.get(usize::try_from(v).ok()?).copied()
    }

    #[cfg(feature = "native")]
    fn to_i64(self) -> i64 {
        Self::ALL.iter().position(|c| *c == self).unwrap() as i64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub body: String,
    pub created: i64,
    pub updated: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub label: String,
    pub created: i64,
}

/// Unix milliseconds, for a store whose clock tests set ([`StudyStore::set_clock`]).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// A record's sync state: its stamp and the local sequence number of its last change here.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rev {
    stamp: Stamp,
    seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadRev {
    rev: Rev,
    /// When it was ticked (Unix seconds).
    at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanRev {
    rev: Rev,
    /// When the plan was last (re)started (ms).
    reset: i64,
}

/// How long a tombstone is kept: long enough for every device in use to have seen it.
pub const TOMBSTONE_DAYS: i64 = 90;

/// A wall clock this far ahead of here is not believed (a device set to the wrong year).
const MAX_DRIFT_MS: i64 = 24 * 3600 * 1000;

pub struct StudyStore {
    /// The SQLite file behind the store, when there is one.
    #[cfg(feature = "native")]
    conn: Option<Connection>,
    /// Each highlighted verse's highlights, in order, not overlapping.
    highlights: HashMap<u32, Vec<Highlight>>,
    notes: HashMap<u32, Note>,
    bookmarks: BTreeMap<u32, Bookmark>,
    /// Reading sessions by when they started.
    reading: BTreeMap<i64, ReadingSession>,
    plans: HashMap<String, PlanProgress>,
    plan_reads: HashMap<String, HashSet<(u16, u8)>>,
    /// Passages being memorised, by their first verse.
    memory: BTreeMap<u32, Memorised>,
    /// Every recall of each passage being memorised, oldest first, by its first verse.
    reviews: BTreeMap<u32, Vec<Review>>,
    settings: HashMap<String, String>,
    generation: u64,
    // Sync state of every record, tombstones (records not in the maps above) included.
    highlight_revs: HashMap<u32, Rev>,
    note_revs: HashMap<u32, (NoteVersion, i64)>,
    bookmark_revs: HashMap<u32, Rev>,
    plan_revs: HashMap<String, PlanRev>,
    read_revs: HashMap<(String, u16, u8), ReadRev>,
    memory_revs: HashMap<u32, Rev>,
    review_revs: HashMap<(u32, i64, Grade), Rev>,
    conflicts: BTreeMap<(u32, Stamp), NoteConflict>,
    device: String,
    hlc: Hlc,
    seq: i64,
    /// Sync bookkeeping kept beside the data (`sync_meta`): the device id, how far a folder
    /// library has been written and read.
    meta: HashMap<String, String>,
    clock: Option<Clock>,
}

#[cfg(feature = "native")]
const SYNC_COLUMNS: [(&str, &str); 4] = [
    ("updated_ms", "INTEGER NOT NULL DEFAULT 0"),
    ("device", "TEXT NOT NULL DEFAULT ''"),
    ("deleted", "INTEGER NOT NULL DEFAULT 0"),
    ("seq", "INTEGER NOT NULL DEFAULT 0"),
];

#[cfg(feature = "native")]
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS highlights(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    char_start INTEGER NOT NULL DEFAULT 0, char_end INTEGER, quote TEXT,
    color INTEGER NOT NULL, created INTEGER NOT NULL, edited INTEGER,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(book, chapter, verse, char_start));
CREATE TABLE IF NOT EXISTS notes(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    body TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    base_updated INTEGER NOT NULL DEFAULT 0, base_device TEXT NOT NULL DEFAULT '',
    seen TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(book, chapter, verse));
CREATE TABLE IF NOT EXISTS bookmarks(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    label TEXT NOT NULL, created INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(book, chapter, verse));
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS reading(
    started INTEGER PRIMARY KEY, book TEXT NOT NULL, chapter INTEGER NOT NULL,
    translation TEXT NOT NULL, seconds INTEGER NOT NULL, furthest INTEGER NOT NULL,
    verses INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS plans(
    id TEXT PRIMARY KEY, started INTEGER NOT NULL, finished INTEGER,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    reset_ms INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS plan_reads(
    plan TEXT NOT NULL, day INTEGER NOT NULL, passage INTEGER NOT NULL, at INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(plan, day, passage));
CREATE TABLE IF NOT EXISTS memory(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL, last_verse INTEGER NOT NULL,
    translation TEXT NOT NULL, created INTEGER NOT NULL,
    due INTEGER NOT NULL, stability REAL NOT NULL, difficulty REAL NOT NULL,
    reps INTEGER NOT NULL, lapses INTEGER NOT NULL, last_review INTEGER,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(book, chapter, verse));
CREATE TABLE IF NOT EXISTS memory_reviews(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    at INTEGER NOT NULL, grade INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL DEFAULT 0, device TEXT NOT NULL DEFAULT '',
    deleted INTEGER NOT NULL DEFAULT 0, seq INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(book, chapter, verse, at, grade));
CREATE TABLE IF NOT EXISTS conflicts(
    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    body TEXT NOT NULL, updated_ms INTEGER NOT NULL, device TEXT NOT NULL, found INTEGER NOT NULL,
    PRIMARY KEY(book, chapter, verse, updated_ms, device));
CREATE TABLE IF NOT EXISTS sync_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
";
/// A stretch of reading one chapter: when it started, for how long, and how far it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadingSession {
    /// Unix seconds; also the session's identity.
    pub started: i64,
    pub book: BookId,
    pub chapter: u8,
    pub translation: String,
    /// Time actually spent reading (idle time is not counted).
    pub seconds: u32,
    /// The furthest verse reached, and how many verses the chapter has.
    pub furthest: u8,
    pub verses: u8,
}

impl ReadingSession {
    pub fn chapter_idx(&self) -> Option<ChapterIdx> {
        ChapterIdx::new(self.book, self.chapter)
    }

    /// Reached the chapter's last verse.
    pub fn finished(&self) -> bool {
        self.verses > 0 && self.furthest >= self.verses
    }

    /// How much of the chapter was read, 0–1.
    pub fn progress(&self) -> f32 {
        if self.verses == 0 { 0.0 } else { (self.furthest as f32 / self.verses as f32).min(1.0) }
    }
}

/// A reading plan under way (or finished): which, and when it began (the start of that day).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProgress {
    pub id: String,
    pub started: i64,
    pub finished: Option<i64>,
}

/// Unix seconds.
pub fn now() -> i64 {
    // The browser has no system clock for std to read: asking panics.
    #[cfg(target_arch = "wasm32")]
    return (js_sys::Date::now() / 1000.0) as i64;
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(feature = "native")]
fn verse_from_row(book: &str, chapter: i64, verse: i64) -> Option<VerseRef> {
    let book = BookId::from_code(book)?;
    Some(VerseRef::new(book, u8::try_from(chapter).ok()?, u8::try_from(verse).ok()?))
}

#[cfg(feature = "native")]
fn query<T>(
    conn: &Connection,
    sql: &str,
    row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    Ok(conn.prepare(sql)?.query_map([], row)?.collect::<rusqlite::Result<_>>()?)
}

fn verse_of(k: &VerseKey) -> Option<VerseRef> {
    Some(VerseRef::new(BookId::from_code(&k.book)?, k.chapter, k.verse))
}

fn key_of(v: VerseRef) -> VerseKey {
    VerseKey { book: v.book.info().code.to_string(), chapter: v.chapter, verse: v.verse }
}

fn schedule_from_reviews(created: i64, reviews: &[Review]) -> Schedule {
    let mut s = Schedule::new(created);
    for r in reviews {
        s = s.next(r.grade, r.at);
    }
    s
}

fn part_of(h: &Highlight) -> HighlightPart {
    HighlightPart {
        color: h.color,
        start: h.start,
        end: h.end,
        quote: h.quote.clone(),
        created: h.created,
        edited: h.edited,
    }
}

fn highlight_of(p: &HighlightPart) -> Highlight {
    Highlight {
        color: p.color,
        start: p.start,
        end: p.end,
        quote: p.quote.clone().filter(|_| p.start > 0 || p.end.is_some()),
        created: p.created,
        edited: p.edited,
    }
}

#[cfg(feature = "native")]
fn join_seen(seen: &[Stamp]) -> String {
    if seen.is_empty() { String::new() } else { serde_json::to_string(seen).unwrap_or_default() }
}

/// A note's superseded versions: JSON stamps (or, as first written, times alone).
#[cfg(feature = "native")]
fn split_seen(s: &str) -> Vec<Stamp> {
    if s.trim_start().starts_with('[') {
        return serde_json::from_str(s).unwrap_or_default();
    }
    s.split(',').filter_map(|t| t.trim().parse().ok()).map(|t| Stamp::new(t, "")).collect()
}

/// This device's id, made the first time the file is opened.
#[cfg(feature = "native")]
fn device_id(conn: &Connection) -> Result<String> {
    use rusqlite::OptionalExtension;
    let id: Option<String> =
        conn.query_row("SELECT value FROM sync_meta WHERE key = 'device'", [], |r| r.get(0)).optional()?;
    match id {
        Some(id) if !id.is_empty() => Ok(id),
        _ => {
            let id = sync::new_device_id();
            conn.execute("INSERT OR REPLACE INTO sync_meta(key, value) VALUES('device', ?1)", [&id])?;
            Ok(id)
        }
    }
}

#[cfg(feature = "native")]
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
        [column],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// The version `migrate` brings a file to.
#[cfg(feature = "native")]
const VERSION: i64 = 2;

/// Brings a file made by an older version up to date, recording its version in `user_version`.
///
/// Version 1 (2026-10): a verse holds several highlights, each over part of it or all of it. The
/// highlights table gains char offsets, the words covered and when each was last coloured (empty:
/// when it was made), and the start offset joins its key. Every highlight before was a whole
/// verse, and stays one.
///
/// Version 2 (2026-11): sync. Highlights, notes, bookmarks, plans and ticked passages gain when
/// they last changed (`updated_ms`, Unix ms: their newest time before), the device that changed
/// them (this one), a tombstone flag and a local sequence number; notes the version they were
/// edited from, plans when they were last restarted. Nothing was deleted before, so nothing is.
#[cfg(feature = "native")]
fn migrate(conn: &Connection, device: &str) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= VERSION {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    if version < 1 && !has_column(&tx, "highlights", "char_start")? {
        // The schema makes the new table once the old one is out of its way.
        tx.execute_batch(&format!(
            "ALTER TABLE highlights RENAME TO highlights_v0;
             {SCHEMA}
             INSERT INTO highlights(book, chapter, verse, char_start, char_end, quote, color, created, edited)
                 SELECT book, chapter, verse, 0, NULL, NULL, color, created, NULL FROM highlights_v0;
             DROP TABLE highlights_v0;"
        ))
        .context("upgrading the highlights table")?;
    }
    let add = |table: &str, column: &str, decl: &str| -> Result<()> {
        if !has_column(&tx, table, column)? {
            tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))
                .with_context(|| format!("adding {table}.{column}"))?;
        }
        Ok(())
    };
    for table in ["highlights", "notes", "bookmarks", "plans", "plan_reads"] {
        for (column, decl) in SYNC_COLUMNS {
            add(table, column, decl)?;
        }
    }
    add("notes", "base_updated", "INTEGER NOT NULL DEFAULT 0")?;
    add("notes", "base_device", "TEXT NOT NULL DEFAULT ''")?;
    add("notes", "seen", "TEXT NOT NULL DEFAULT ''")?;
    add("plans", "reset_ms", "INTEGER NOT NULL DEFAULT 0")?;
    for (table, newest) in [
        ("highlights", "MAX(created, COALESCE(edited, created))"),
        ("notes", "MAX(created, updated)"),
        ("bookmarks", "created"),
        ("plans", "MAX(started, COALESCE(finished, started))"),
        ("plan_reads", "at"),
    ] {
        tx.execute(
            &format!(
                "UPDATE {table} SET updated_ms = {newest} * 1000, device = ?1, seq = 1 WHERE updated_ms = 0"
            ),
            [device],
        )
        .with_context(|| format!("stamping {table}"))?;
    }
    tx.pragma_update(None, "user_version", VERSION)?;
    tx.commit()?;
    Ok(())
}

/// Adds sync columns to memorised-passage tables written before they joined the sync model
/// (the tables in this file's first form had none). Files that already have the columns are left
/// as they are. Idempotent, so it can run after [`migrate`] even when that returns early.
#[cfg(feature = "native")]
fn ensure_memory_sync(conn: &Connection, device: &str) -> Result<()> {
    if !has_column(conn, "memory", "book")? {
        return Ok(());
    }
    let add = |table: &str, column: &str, decl: &str| -> Result<()> {
        if !has_column(conn, table, column)? {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))
                .with_context(|| format!("adding {table}.{column}"))?;
        }
        Ok(())
    };
    for table in ["memory", "memory_reviews"] {
        for (column, decl) in SYNC_COLUMNS {
            add(table, column, decl)?;
        }
    }
    conn.execute(
        "UPDATE memory SET updated_ms = MAX(created, COALESCE(last_review, created)) * 1000, device = ?1, seq = 1
         WHERE updated_ms = 0",
        [device],
    )?;
    conn.execute(
        "UPDATE memory_reviews SET updated_ms = at * 1000, device = ?1, seq = 1 WHERE updated_ms = 0",
        [device],
    )?;
    Ok(())
}

impl Default for StudyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StudyStore {
    /// An empty store that keeps nothing on disk.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "native")]
            conn: None,
            highlights: HashMap::new(),
            notes: HashMap::new(),
            bookmarks: BTreeMap::new(),
            reading: BTreeMap::new(),
            plans: HashMap::new(),
            plan_reads: HashMap::new(),
            memory: BTreeMap::new(),
            reviews: BTreeMap::new(),
            settings: HashMap::new(),
            meta: HashMap::new(),
            generation: 1,
            highlight_revs: HashMap::new(),
            note_revs: HashMap::new(),
            bookmark_revs: HashMap::new(),
            plan_revs: HashMap::new(),
            read_revs: HashMap::new(),
            memory_revs: HashMap::new(),
            review_revs: HashMap::new(),
            conflicts: BTreeMap::new(),
            device: sync::new_device_id(),
            hlc: Hlc::default(),
            seq: 0,
            clock: None,
        }
    }

    /// Opens (or makes) the store's file. A file from before sync is first copied to
    /// `<name>.pre-sync` beside it ([`pre_sync_backup`](Self::pre_sync_backup)), then brought up
    /// to date.
    #[cfg(feature = "native")]
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::back_up_before_sync(&conn, path)?;
        let mut s = Self::with_connection(conn)?;
        s.prune_tombstones(s.wall_ms() - TOMBSTONE_DAYS * 86_400_000)?;
        Ok(s)
    }

    /// Where [`open`](Self::open) keeps the copy of a file from before sync.
    pub fn pre_sync_backup(path: &std::path::Path) -> std::path::PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".pre-sync");
        path.with_file_name(name)
    }

    #[cfg(feature = "native")]
    fn back_up_before_sync(conn: &Connection, path: &std::path::Path) -> Result<()> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let tables: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))?;
        let backup = Self::pre_sync_backup(path);
        if version >= VERSION || tables == 0 || backup.exists() {
            return Ok(());
        }
        // A consistent copy, what is still in the WAL included.
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])
            .with_context(|| format!("backing up to {}", backup.display()))?;
        Ok(())
    }

    #[cfg(feature = "native")]
    pub fn open_in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    #[cfg(feature = "native")]
    fn with_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        let device = device_id(&conn)?;
        migrate(&conn, &device)?;
        ensure_memory_sync(&conn, &device)?;
        let mut s = Self { conn: Some(conn), device, ..Self::new() };
        s.load()?;
        Ok(s)
    }

    /// Reads every table into memory.
    #[cfg(feature = "native")]
    fn load(&mut self) -> Result<()> {
        let Some(conn) = &self.conn else { return Ok(()) };
        let verse = |r: &rusqlite::Row<'_>| -> rusqlite::Result<_> {
            Ok(verse_from_row(&r.get::<_, String>(0)?, r.get(1)?, r.get(2)?))
        };
        // The sync columns from column `i` on: the stamp, whether it is a tombstone, the sequence.
        fn meta(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<(Rev, bool)> {
            let stamp = Stamp::new(r.get(i)?, r.get::<_, String>(i + 1)?);
            Ok((Rev { stamp, seq: r.get(i + 3)? }, r.get::<_, i64>(i + 2)? != 0))
        }
        const META: &str = "updated_ms, device, deleted, seq";
        let highlights = query(
            conn,
            &format!(
                "SELECT book, chapter, verse, char_start, char_end, quote, color, created, edited, {META}
                 FROM highlights ORDER BY book, chapter, verse, char_start"
            ),
            |r| {
                let (start, end, quote, created) = (r.get(3)?, r.get(4)?, r.get(5)?, r.get(7)?);
                let edited = r.get::<_, Option<i64>>(8)?.unwrap_or(created);
                let h = HighlightColor::from_i64(r.get(6)?).map(|color| Highlight {
                    color,
                    start,
                    end,
                    quote,
                    created,
                    edited,
                });
                Ok((verse(r)?, h, meta(r, 9)?))
            },
        )?;
        let (mut newest, mut last_seq) = (0, 0);
        // As when merging: a stamp more than a day ahead (a device set to the wrong year) doesn't
        // move the clock.
        let limit = self.wall_ms() + MAX_DRIFT_MS;
        let mut saw = |rev: &Rev| {
            if rev.stamp.at <= limit {
                newest = newest.max(rev.stamp.at);
            }
            last_seq = last_seq.max(rev.seq);
        };
        self.highlights.clear();
        self.highlight_revs.clear();
        for (v, h, (rev, deleted)) in highlights {
            saw(&rev);
            let Some(v) = v else { continue };
            // A verse's rows change together; its stamp is the newest.
            let r = self.highlight_revs.entry(v.key()).or_insert_with(|| rev.clone());
            if (&rev.stamp, rev.seq) > (&r.stamp, r.seq) {
                *r = rev;
            }
            if let (Some(h), false) = (h, deleted) {
                self.highlights.entry(v.key()).or_default().push(h);
            }
        }
        let notes = query(
            conn,
            &format!(
                "SELECT book, chapter, verse, body, created, updated, {META}, base_updated, base_device, seen FROM notes"
            ),
            |r| {
                let (rev, deleted) = meta(r, 6)?;
                let version = NoteVersion {
                    stamp: rev.stamp,
                    live: !deleted,
                    base: Stamp::new(r.get(10)?, r.get::<_, String>(11)?),
                    seen: split_seen(&r.get::<_, String>(12)?),
                };
                let note = Note { body: r.get(3)?, created: r.get(4)?, updated: r.get(5)? };
                Ok((verse(r)?, note, version, rev.seq))
            },
        )?;
        self.notes.clear();
        self.note_revs.clear();
        for (v, n, version, seq) in notes {
            saw(&Rev { stamp: version.stamp.clone(), seq });
            let Some(v) = v else { continue };
            if version.live {
                self.notes.insert(v.key(), n);
            }
            self.note_revs.insert(v.key(), (version, seq));
        }
        let bookmarks = query(
            conn,
            &format!("SELECT book, chapter, verse, label, created, {META} FROM bookmarks"),
            |r| Ok((verse(r)?, Bookmark { label: r.get(3)?, created: r.get(4)? }, meta(r, 5)?)),
        )?;
        self.bookmarks.clear();
        self.bookmark_revs.clear();
        for (v, b, (rev, deleted)) in bookmarks {
            saw(&rev);
            let Some(v) = v else { continue };
            if !deleted {
                self.bookmarks.insert(v.key(), b);
            }
            self.bookmark_revs.insert(v.key(), rev);
        }
        let reading = query(
            conn,
            "SELECT started, book, chapter, translation, seconds, furthest, verses FROM reading",
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )?;
        self.reading = reading
            .into_iter()
            .filter_map(|(started, book, chapter, translation, seconds, furthest, verses)| {
                let session = ReadingSession {
                    started,
                    book: BookId::from_code(&book)?,
                    chapter: u8::try_from(chapter).ok()?,
                    translation,
                    seconds: u32::try_from(seconds).ok()?,
                    furthest: u8::try_from(furthest).ok()?,
                    verses: u8::try_from(verses).ok()?,
                };
                Some((started, session))
            })
            .collect();
        let plans =
            query(conn, &format!("SELECT id, started, finished, {META}, reset_ms FROM plans"), |r| {
                let (rev, deleted) = meta(r, 3)?;
                let p = PlanProgress { id: r.get(0)?, started: r.get(1)?, finished: r.get(2)? };
                Ok((p, rev, deleted, r.get(7)?))
            })?;
        self.plans.clear();
        self.plan_revs.clear();
        for (p, rev, deleted, reset) in plans {
            saw(&rev);
            self.plan_revs.insert(p.id.clone(), PlanRev { rev, reset });
            if !deleted {
                self.plans.insert(p.id.clone(), p);
            }
        }
        let reads = query(conn, &format!("SELECT plan, day, passage, at, {META} FROM plan_reads"), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                meta(r, 4)?,
            ))
        })?;
        self.plan_reads.clear();
        self.read_revs.clear();
        for (plan, day, passage, at, (rev, deleted)) in reads {
            saw(&rev);
            if let (Ok(day), Ok(passage)) = (u16::try_from(day), u8::try_from(passage)) {
                if !deleted {
                    self.plan_reads.entry(plan.clone()).or_default().insert((day, passage));
                }
                self.read_revs.insert((plan, day, passage), ReadRev { rev, at });
            }
        }
        let conflicts = query(
            conn,
            "SELECT book, chapter, verse, body, updated_ms, device, found FROM conflicts",
            |r| {
                let stamp = Stamp::new(r.get(4)?, r.get::<_, String>(5)?);
                Ok((verse(r)?, NoteConflict { body: r.get(3)?, stamp, found: r.get(6)? }))
            },
        )?;
        self.conflicts =
            conflicts.into_iter().filter_map(|(v, c)| Some(((v?.key(), c.stamp.clone()), c))).collect();
        let memory = query(
            conn,
            &format!(
                "SELECT book, chapter, verse, last_verse, translation, created, due, stability, difficulty,
                        reps, lapses, last_review, {META} FROM memory"
            ),
            |r| {
                let first = verse(r)?;
                let last_verse = u8::try_from(r.get::<_, i64>(3)?).ok();
                let last = first.zip(last_verse).map(|(f, n)| VerseRef::new(f.book, f.chapter, n));
                let schedule = Schedule {
                    due: r.get(6)?,
                    stability: r.get(7)?,
                    difficulty: r.get(8)?,
                    reps: r.get(9)?,
                    lapses: r.get(10)?,
                    last_review: r.get(11)?,
                };
                Ok((first, last, r.get::<_, String>(4)?, r.get::<_, i64>(5)?, schedule, meta(r, 12)?))
            },
        )?;
        self.memory.clear();
        self.memory_revs.clear();
        for (first, last, translation, created, schedule, (rev, deleted)) in memory {
            saw(&rev);
            let Some((first, last)) = first.zip(last) else { continue };
            if !deleted {
                self.memory.insert(first.key(), Memorised { first, last, translation, created, schedule });
            }
            self.memory_revs.insert(first.key(), rev);
        }
        let reviews = query(
            conn,
            &format!("SELECT book, chapter, verse, at, grade, {META} FROM memory_reviews ORDER BY at, rowid"),
            |r| {
                let n: i64 = r.get(4)?;
                let grade = Grade::ALL.into_iter().find(|g| *g as i64 == n);
                Ok((verse(r)?, r.get::<_, i64>(3)?, grade, meta(r, 5)?))
            },
        )?;
        self.reviews.clear();
        self.review_revs.clear();
        for (first, at, grade, (rev, deleted)) in reviews {
            saw(&rev);
            if let (Some(first), Some(grade)) = (first, grade) {
                if !deleted {
                    self.reviews.entry(first.key()).or_default().push(Review { at, grade });
                }
                self.review_revs.insert((first.key(), at, grade), rev);
            }
        }
        self.settings = query(conn, "SELECT key, value FROM settings", |r| Ok((r.get(0)?, r.get(1)?)))?
            .into_iter()
            .collect();
        self.meta = query(conn, "SELECT key, value FROM sync_meta", |r| Ok((r.get(0)?, r.get(1)?)))?
            .into_iter()
            .collect();
        // Pruned tombstones can take the newest sequence number with them: it's kept here too.
        let floor = self.meta.get("seq").and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        self.hlc.observe(newest);
        self.seq = self.seq.max(last_seq).max(floor);
        self.generation += 1;
        Ok(())
    }

    /// Runs a statement on the SQLite file, when the store has one; how many rows it changed.
    #[cfg(feature = "native")]
    fn exec(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        match &self.conn {
            Some(conn) => Ok(conn.execute(sql, params)?),
            None => Ok(0),
        }
    }

    /// Runs `f` as one transaction: when it fails, what it wrote is undone on disk and in memory.
    fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        #[cfg(feature = "native")]
        self.exec("BEGIN", [])?;
        let result = f(self);
        #[cfg(feature = "native")]
        if result.is_ok() {
            self.exec("COMMIT", [])?;
        } else {
            self.exec("ROLLBACK", [])?;
            self.load()?;
        }
        result
    }

    /// Increments whenever study data changes (the reader uses it to invalidate layouts).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    // ---------------------------------------------------------------- sync state

    /// This device's id (kept in the file; a store without one gets a new id each time).
    pub fn device_id(&self) -> &str {
        &self.device
    }

    /// Replaces the wall clock (Unix ms) the store stamps changes with: for tests.
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = Some(clock);
    }

    /// The local sequence number of the latest change here (made here or taken in): what
    /// [`changes_since`](Self::changes_since) counts from.
    pub fn seq(&self) -> i64 {
        self.seq
    }

    /// A value kept with the sync state (not exported, not synced).
    pub fn sync_value(&self, key: &str) -> Option<&str> {
        self.meta.get(key).map(String::as_str)
    }

    pub fn set_sync_value(&mut self, key: &str, value: &str) -> Result<()> {
        #[cfg(feature = "native")]
        self.exec("INSERT OR REPLACE INTO sync_meta(key, value) VALUES(?1, ?2)", params![key, value])?;
        self.meta.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn wall_ms(&self) -> i64 {
        self.clock.as_ref().map_or_else(sync::now_ms, |c| c())
    }

    /// Unix seconds, from the store's clock.
    fn now_s(&self) -> i64 {
        self.wall_ms().div_euclid(1000)
    }

    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    /// The stamp of a change made here now.
    fn next_rev(&mut self) -> Rev {
        let at = self.hlc.tick(self.wall_ms());
        Rev { stamp: Stamp::new(at, self.device.clone()), seq: self.next_seq() }
    }

    /// A verse's highlight colour: the newest highlight's, when parts of it have different ones.
    pub fn highlight(&self, v: VerseRef) -> Option<HighlightColor> {
        self.highlight_parts(v).iter().max_by_key(|h| h.created).map(|h| h.color)
    }

    /// A verse's highlights as stored (offsets in the text each was made in).
    pub fn highlight_parts(&self, v: VerseRef) -> &[Highlight] {
        self.highlights.get(&v.key()).map_or(&[], Vec::as_slice)
    }

    /// What a verse whose text is `text` shows: its coloured char ranges, in order.
    pub fn highlight_ranges(&self, v: VerseRef, text: &str) -> Vec<(std::ops::Range<usize>, HighlightColor)> {
        highlight::ranges(self.highlight_parts(v), text)
    }

    /// The colour that covers every letter of `span`, if one does. `text` gives the verse texts
    /// the span counts in; a verse it hasn't counts as its whole-verse highlight.
    pub fn span_color(
        &self,
        span: &TextSpan,
        text: impl Fn(VerseRef) -> Option<String>,
    ) -> Option<HighlightColor> {
        let mut color = None;
        for v in span.refs() {
            let c = match text(v) {
                Some(t) => highlight::color_of(self.highlight_parts(v), &t, span.range_of(v))?,
                None => self.highlight_parts(v).iter().find(|h| h.is_whole())?.color,
            };
            if color.is_some_and(|k| k != c) {
                return None;
            }
            color = Some(c);
        }
        color
    }

    pub fn note(&self, v: VerseRef) -> Option<&Note> {
        self.notes.get(&v.key())
    }

    pub fn bookmark(&self, v: VerseRef) -> Option<&Bookmark> {
        self.bookmarks.get(&v.key())
    }

    pub fn is_bookmarked(&self, v: VerseRef) -> bool {
        self.bookmarks.contains_key(&v.key())
    }

    /// Highlights a whole verse in `color`, replacing what it had, or (with `None`) clears all of
    /// it. A verse already highlighted keeps when it was first highlighted.
    pub fn set_highlight(&mut self, v: VerseRef, color: Option<HighlightColor>) -> Result<()> {
        let parts = match color {
            Some(c) => {
                let t = self.now_s();
                let created = self.highlight_parts(v).iter().map(|h| h.created).min().unwrap_or(t);
                vec![Highlight { edited: t, ..Highlight::whole(c, created) }]
            }
            None => Vec::new(),
        };
        self.transaction(|s| s.put_highlights(v, parts))?;
        self.generation += 1;
        Ok(())
    }

    /// Colours a stretch of text (`None` clears it): over the highlights there, which it replaces
    /// where they overlap. `text` gives the verse texts the span counts in; a verse it hasn't is
    /// coloured (or cleared) whole.
    pub fn highlight_span(
        &mut self,
        span: &TextSpan,
        color: Option<HighlightColor>,
        text: impl Fn(VerseRef) -> Option<String>,
    ) -> Result<()> {
        let t = self.now_s();
        self.transaction(|s| {
            for v in span.refs() {
                let parts = match text(v) {
                    Some(text) => highlight::paint(s.highlight_parts(v), &text, span.range_of(v), color, t),
                    None => color.map(|c| Highlight::whole(c, t)).into_iter().collect(),
                };
                s.put_highlights(v, parts)?;
            }
            Ok(())
        })?;
        self.generation += 1;
        Ok(())
    }

    /// Removes one of a verse's highlights: the one stored from char `start`.
    pub fn remove_highlight(&mut self, v: VerseRef, start: u32) -> Result<()> {
        let parts: Vec<Highlight> =
            self.highlight_parts(v).iter().filter(|h| h.start != start).cloned().collect();
        self.transaction(|s| s.put_highlights(v, parts))?;
        self.generation += 1;
        Ok(())
    }

    /// Replaces a verse's highlights, as a change made here (unless nothing changes).
    fn put_highlights(&mut self, v: VerseRef, mut parts: Vec<Highlight>) -> Result<()> {
        parts.sort_by_key(|h| h.start);
        parts.dedup_by_key(|h| h.start);
        if parts.as_slice() == self.highlight_parts(v) {
            return Ok(());
        }
        let rev = self.next_rev();
        self.store_highlights(v, parts, rev)
    }

    /// Writes a verse's highlights with their stamp. Rows no longer there stay, as tombstones; a
    /// verse with none gets one.
    fn store_highlights(&mut self, v: VerseRef, parts: Vec<Highlight>, rev: Rev) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let code = v.book.info().code;
            let (at, device, seq) = (rev.stamp.at, rev.stamp.device.as_str(), rev.seq);
            let marked = self.exec(
                "UPDATE highlights SET deleted = 1, updated_ms = ?4, device = ?5, seq = ?6
                 WHERE book = ?1 AND chapter = ?2 AND verse = ?3",
                params![code, v.chapter, v.verse, at, device, seq],
            )?;
            for h in &parts {
                self.exec(
                    "INSERT INTO highlights(book, chapter, verse, char_start, char_end, quote, color, created, edited,
                         updated_ms, device, deleted, seq)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12)
                     ON CONFLICT(book, chapter, verse, char_start) DO UPDATE SET char_end = excluded.char_end,
                         quote = excluded.quote, color = excluded.color, created = excluded.created,
                         edited = excluded.edited, updated_ms = excluded.updated_ms, device = excluded.device,
                         deleted = 0, seq = excluded.seq",
                    params![
                        code,
                        v.chapter,
                        v.verse,
                        h.start,
                        h.end,
                        h.quote,
                        h.color.to_i64(),
                        h.created,
                        (h.edited != h.created).then_some(h.edited),
                        at,
                        device,
                        seq
                    ],
                )?;
            }
            if parts.is_empty() && marked == 0 && self.conn.is_some() {
                self.exec(
                    "INSERT INTO highlights(book, chapter, verse, char_start, color, created, updated_ms, device, deleted, seq)
                     VALUES(?1, ?2, ?3, 0, 0, 0, ?4, ?5, 1, ?6)",
                    params![code, v.chapter, v.verse, at, device, seq],
                )?;
            }
        }
        if parts.is_empty() {
            self.highlights.remove(&v.key());
        } else {
            self.highlights.insert(v.key(), parts);
        }
        self.highlight_revs.insert(v.key(), rev);
        Ok(())
    }

    /// Saves a note; an empty (whitespace-only) body deletes it.
    pub fn set_note(&mut self, v: VerseRef, body: &str) -> Result<()> {
        let old = self.notes.get(&v.key());
        let note = if body.trim().is_empty() {
            if old.is_none() {
                return Ok(());
            }
            None
        } else {
            if old.is_some_and(|n| n.body == body) {
                return Ok(());
            }
            let t = self.now_s();
            Some(Note { body: body.to_string(), created: old.map_or(t, |n| n.created), updated: t })
        };
        let rev = self.next_rev();
        let version = self.note_version(v).edited(rev.stamp, note.is_some());
        self.store_note(v, note, version, rev.seq)?;
        self.generation += 1;
        Ok(())
    }

    /// Saves a note written in an editor opened on `base` (the text it started from). If the
    /// note changed underneath it (a sync brought another device's edit), that text is kept as a
    /// conflict copy rather than overwritten. Whether one was kept.
    pub fn set_note_over(&mut self, v: VerseRef, body: &str, base: &str) -> Result<bool> {
        let theirs = self
            .notes
            .get(&v.key())
            .filter(|n| n.body != base && n.body != body && !n.body.trim().is_empty())
            .map(|n| n.body.clone());
        let Some(theirs) = theirs else {
            self.set_note(v, body)?;
            return Ok(false);
        };
        let stamp = self.note_version(v).stamp;
        let found = self.now_s();
        let mut copies: Vec<NoteConflict> = self.note_conflicts_at(v).into_iter().cloned().collect();
        if !copies.iter().any(|k| k.stamp == stamp) {
            copies.push(NoteConflict { body: theirs, stamp, found });
        }
        self.transaction(|s| {
            s.set_note(v, body)?;
            s.put_conflicts(v, copies)
        })?;
        self.generation += 1;
        Ok(true)
    }

    fn note_version(&self, v: VerseRef) -> NoteVersion {
        self.note_revs.get(&v.key()).map(|(n, _)| n.clone()).unwrap_or_default()
    }

    /// Writes a note (`None`: a tombstone) with its version.
    fn store_note(&mut self, v: VerseRef, note: Option<Note>, version: NoteVersion, seq: i64) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let (body, created, updated) =
                note.as_ref().map_or(("", 0, 0), |n| (n.body.as_str(), n.created, n.updated));
            self.exec(
                "INSERT INTO notes(book, chapter, verse, body, created, updated, updated_ms, device, deleted, seq,
                     base_updated, base_device, seen)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(book, chapter, verse) DO UPDATE SET body = excluded.body, created = excluded.created,
                     updated = excluded.updated, updated_ms = excluded.updated_ms, device = excluded.device,
                     deleted = excluded.deleted, seq = excluded.seq, base_updated = excluded.base_updated,
                     base_device = excluded.base_device, seen = excluded.seen",
                params![
                    v.book.info().code,
                    v.chapter,
                    v.verse,
                    body,
                    created,
                    updated,
                    version.stamp.at,
                    version.stamp.device,
                    !version.live,
                    seq,
                    version.base.at,
                    version.base.device,
                    join_seen(&version.seen)
                ],
            )?;
        }
        match note {
            Some(n) => self.notes.insert(v.key(), n),
            None => self.notes.remove(&v.key()),
        };
        self.note_revs.insert(v.key(), (version, seq));
        Ok(())
    }

    pub fn set_bookmark(&mut self, v: VerseRef, label: Option<&str>) -> Result<()> {
        let old = self.bookmarks.get(&v.key());
        let bookmark = match label {
            Some(label) if old.is_some_and(|b| b.label == label) => return Ok(()),
            Some(label) => {
                let created = old.map_or_else(|| self.now_s(), |b| b.created);
                Some(Bookmark { label: label.to_string(), created })
            }
            None if old.is_none() => return Ok(()),
            None => None,
        };
        let rev = self.next_rev();
        self.store_bookmark(v, bookmark, rev)?;
        self.generation += 1;
        Ok(())
    }

    /// Writes a bookmark (`None`: a tombstone) with its stamp.
    fn store_bookmark(&mut self, v: VerseRef, bookmark: Option<Bookmark>, rev: Rev) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let (label, created) = bookmark.as_ref().map_or(("", 0), |b| (b.label.as_str(), b.created));
            self.exec(
                "INSERT INTO bookmarks(book, chapter, verse, label, created, updated_ms, device, deleted, seq)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(book, chapter, verse) DO UPDATE SET label = excluded.label, created = excluded.created,
                     updated_ms = excluded.updated_ms, device = excluded.device, deleted = excluded.deleted,
                     seq = excluded.seq",
                params![
                    v.book.info().code,
                    v.chapter,
                    v.verse,
                    label,
                    created,
                    rev.stamp.at,
                    rev.stamp.device,
                    bookmark.is_none(),
                    rev.seq
                ],
            )?;
        }
        match bookmark {
            Some(b) => self.bookmarks.insert(v.key(), b),
            None => self.bookmarks.remove(&v.key()),
        };
        self.bookmark_revs.insert(v.key(), rev);
        Ok(())
    }

    pub fn toggle_bookmark(&mut self, v: VerseRef) -> Result<bool> {
        let on = !self.is_bookmarked(v);
        self.set_bookmark(v, on.then_some(""))?;
        Ok(on)
    }

    /// Every highlight in canonical order (a verse's in the order they lie in it).
    pub fn highlights(&self) -> Vec<(VerseRef, &Highlight)> {
        let mut v: Vec<_> = self
            .highlights
            .iter()
            .filter_map(|(k, hs)| Some((VerseRef::from_key(*k)?, hs)))
            .flat_map(|(r, hs)| hs.iter().map(move |h| (r, h)))
            .collect();
        v.sort_by_key(|(r, h)| (*r, h.start));
        v
    }

    /// Every highlight, the most recently made first.
    pub fn highlights_dated(&self) -> Vec<(VerseRef, &Highlight)> {
        let mut v = self.highlights();
        v.sort_by_key(|(r, h)| (std::cmp::Reverse(h.created), *r, h.start));
        v
    }

    pub fn notes(&self) -> Vec<(VerseRef, &Note)> {
        let mut v: Vec<_> =
            self.notes.iter().filter_map(|(k, n)| Some((VerseRef::from_key(*k)?, n))).collect();
        v.sort_by_key(|(r, _)| *r);
        v
    }

    pub fn bookmarks(&self) -> Vec<(VerseRef, &Bookmark)> {
        self.bookmarks.iter().filter_map(|(k, b)| Some((VerseRef::from_key(*k)?, b))).collect()
    }

    /// How many highlights (a verse's parts counting one each), notes and bookmarks there are.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.highlights.values().map(Vec::len).sum(), self.notes.len(), self.bookmarks.len())
    }

    /// Saves a reading session, replacing an earlier save of the same session.
    pub fn record_reading(&mut self, s: &ReadingSession) -> Result<()> {
        #[cfg(feature = "native")]
        self.exec(
            "INSERT INTO reading(started, book, chapter, translation, seconds, furthest, verses)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(started) DO UPDATE SET seconds = excluded.seconds, furthest = excluded.furthest,
               verses = excluded.verses, translation = excluded.translation",
            params![s.started, s.book.info().code, s.chapter, s.translation, s.seconds, s.furthest, s.verses],
        )?;
        self.reading
            .entry(s.started)
            .and_modify(|e| {
                e.seconds = s.seconds;
                e.furthest = s.furthest;
                e.verses = s.verses;
                e.translation.clone_from(&s.translation);
            })
            .or_insert_with(|| s.clone());
        Ok(())
    }

    /// Reading sessions that started at or after `since` (Unix seconds), newest first.
    pub fn reading_since(&self, since: i64) -> Vec<ReadingSession> {
        self.reading.range(since..).rev().map(|(_, s)| s.clone()).collect()
    }

    /// Forgets reading history from `since` on (all of it for 0).
    pub fn clear_reading(&mut self, since: i64) -> Result<()> {
        #[cfg(feature = "native")]
        self.exec("DELETE FROM reading WHERE started >= ?1", [since])?;
        self.reading.retain(|&started, _| started < since);
        Ok(())
    }

    /// Plans started, newest first.
    pub fn plans(&self) -> Vec<PlanProgress> {
        let mut v: Vec<_> = self.plans.values().cloned().collect();
        v.sort_by(|a, b| b.started.cmp(&a.started).then_with(|| a.id.cmp(&b.id)));
        v
    }

    /// Starts (or restarts) a plan on the day containing `started`, forgetting earlier progress.
    pub fn start_plan(&mut self, id: &str, started: i64) -> Result<()> {
        self.transaction(|s| {
            let rev = s.next_rev();
            let reset = rev.stamp.at;
            s.unread_all(id, &rev.stamp)?;
            s.store_plan(id, Some((started, None)), PlanRev { rev, reset })
        })?;
        self.generation += 1;
        Ok(())
    }

    /// Writes a plan (`None`: a tombstone) with its stamp.
    fn store_plan(&mut self, id: &str, plan: Option<(i64, Option<i64>)>, rev: PlanRev) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let (started, finished) = plan.unwrap_or((0, None));
            self.exec(
                "INSERT OR REPLACE INTO plans(id, started, finished, updated_ms, device, deleted, seq, reset_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    started,
                    finished,
                    rev.rev.stamp.at,
                    rev.rev.stamp.device,
                    plan.is_none(),
                    rev.rev.seq,
                    rev.reset
                ],
            )?;
        }
        match plan {
            Some((started, finished)) => {
                self.plans.insert(id.to_string(), PlanProgress { id: id.to_string(), started, finished });
            }
            None => {
                self.plans.remove(id);
            }
        }
        self.plan_revs.insert(id.to_string(), rev);
        Ok(())
    }

    /// Unticks every passage ticked in a plan, stamped `stamp`.
    fn unread_all(&mut self, id: &str, stamp: &Stamp) -> Result<()> {
        let mut ticked: Vec<(u16, u8)> = self.plan_reads(id).into_iter().collect();
        ticked.sort_unstable();
        for (day, passage) in ticked {
            let at = self.read_revs.get(&(id.to_string(), day, passage)).map_or(0, |r| r.at);
            let rev = Rev { stamp: stamp.clone(), seq: self.next_seq() };
            self.store_read(id, day, passage, false, ReadRev { rev, at })?;
        }
        Ok(())
    }

    pub fn finish_plan(&mut self, id: &str, at: Option<i64>) -> Result<()> {
        let Some(p) = self.plans.get(id) else { return Ok(()) };
        if p.finished == at {
            return Ok(());
        }
        let started = p.started;
        let reset = self.plan_revs.get(id).map_or(0, |r| r.reset);
        let rev = self.next_rev();
        self.store_plan(id, Some((started, at)), PlanRev { rev, reset })?;
        self.generation += 1;
        Ok(())
    }

    pub fn stop_plan(&mut self, id: &str) -> Result<()> {
        if !self.plans.contains_key(id) {
            return Ok(());
        }
        self.transaction(|s| s.drop_plan(id))?;
        self.generation += 1;
        Ok(())
    }

    /// The passages ticked off in a plan, as (day, passage index) pairs.
    pub fn plan_reads(&self, id: &str) -> HashSet<(u16, u8)> {
        self.plan_reads.get(id).cloned().unwrap_or_default()
    }

    pub fn set_plan_read(&mut self, id: &str, day: u16, passage: u8, read: bool) -> Result<()> {
        if self.plan_reads.get(id).is_some_and(|r| r.contains(&(day, passage))) == read {
            return Ok(());
        }
        let at = if read {
            self.now_s()
        } else {
            self.read_revs.get(&(id.to_string(), day, passage)).map_or(0, |r| r.at)
        };
        let rev = self.next_rev();
        self.store_read(id, day, passage, read, ReadRev { rev, at })?;
        self.generation += 1;
        Ok(())
    }

    /// Writes whether a passage is ticked, with its stamp.
    fn store_read(&mut self, id: &str, day: u16, passage: u8, read: bool, rev: ReadRev) -> Result<()> {
        #[cfg(feature = "native")]
        self.exec(
            "INSERT OR REPLACE INTO plan_reads(plan, day, passage, at, updated_ms, device, deleted, seq)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, day, passage, rev.at, rev.rev.stamp.at, rev.rev.stamp.device, !read, rev.rev.seq],
        )?;
        if read {
            self.plan_reads.entry(id.to_string()).or_default().insert((day, passage));
        } else if let Some(reads) = self.plan_reads.get_mut(id) {
            reads.remove(&(day, passage));
            if reads.is_empty() {
                self.plan_reads.remove(id);
            }
        }
        self.read_revs.insert((id.to_string(), day, passage), rev);
        Ok(())
    }

    /// The passage being memorised that verse `v` is part of.
    pub fn memorised_at(&self, v: VerseRef) -> Option<&Memorised> {
        self.memory.range(..=v.key()).next_back().map(|(_, m)| m).filter(|m| m.contains(v))
    }

    /// Every passage being memorised, the soonest due first.
    pub fn memorised(&self) -> Vec<&Memorised> {
        let mut v: Vec<&Memorised> = self.memory.values().collect();
        v.sort_by_key(|m| (m.schedule.due, m.first));
        v
    }

    /// The passages that share a verse with `first..=last`.
    pub fn memorised_in(&self, first: VerseRef, last: VerseRef) -> Vec<&Memorised> {
        self.memory.values().filter(|m| m.overlaps(first, last)).collect()
    }

    /// The passages due at `now`, the longest overdue first.
    pub fn due(&self, now: i64) -> Vec<&Memorised> {
        self.memorised().into_iter().filter(|m| m.schedule.is_due(now)).collect()
    }

    /// Starts memorising verses `first..=last` (one chapter) as `translation` has them, due now.
    /// Passages it overlaps are replaced.
    pub fn memorise(&mut self, first: VerseRef, last: VerseRef, translation: &str) -> Result<()> {
        if first.chapter_idx() != last.chapter_idx() || last < first {
            bail!("a passage to memorise is verses of one chapter");
        }
        let overlapping: Vec<VerseRef> =
            self.memory.values().filter(|m| m.overlaps(first, last)).map(|m| m.first).collect();
        let t = self.now_s();
        let m = Memorised {
            first,
            last,
            translation: translation.to_string(),
            created: t,
            schedule: Schedule::new(t),
        };
        self.transaction(|s| {
            for f in overlapping {
                s.forget_passage(f)?;
            }
            s.put_memorised(&m)
        })?;
        self.generation += 1;
        Ok(())
    }

    /// Stops memorising the passage that starts at `first`.
    pub fn forget(&mut self, first: VerseRef) -> Result<()> {
        self.transaction(|s| s.forget_passage(first))?;
        self.generation += 1;
        Ok(())
    }

    fn forget_passage(&mut self, first: VerseRef) -> Result<()> {
        let rev = self.next_rev();
        self.store_memorised(first, None, rev)?;
        self.forget_reviews(first)
    }

    /// Tombstones every live recall of the passage that starts at `first`.
    fn forget_reviews(&mut self, first: VerseRef) -> Result<()> {
        let reviews = self.reviews.get(&first.key()).cloned().unwrap_or_default();
        for review in reviews {
            let rev = self.next_rev();
            self.store_review(first, review, false, rev)?;
        }
        Ok(())
    }

    /// Every recall of the passage that starts at `first`, oldest first.
    pub fn reviews(&self, first: VerseRef) -> &[Review] {
        self.reviews.get(&first.key()).map_or(&[], Vec::as_slice)
    }

    fn put_review(&mut self, first: VerseRef, review: Review) -> Result<()> {
        let rev = self.next_rev();
        self.store_review(first, review, true, rev)
    }

    /// Writes a recall (`live`: it happened; otherwise a tombstone) with its stamp.
    fn store_review(&mut self, first: VerseRef, review: Review, live: bool, rev: Rev) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let code = first.book.info().code;
            let n = self.exec(
                "UPDATE memory_reviews SET updated_ms = ?1, device = ?2, deleted = ?3, seq = ?4
                 WHERE book = ?5 AND chapter = ?6 AND verse = ?7 AND at = ?8 AND grade = ?9",
                params![
                    rev.stamp.at,
                    rev.stamp.device,
                    !live,
                    rev.seq,
                    code,
                    first.chapter,
                    first.verse,
                    review.at,
                    review.grade as u8
                ],
            )?;
            if n == 0 {
                self.exec(
                    "INSERT INTO memory_reviews(book, chapter, verse, at, grade, updated_ms, device, deleted, seq)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        code,
                        first.chapter,
                        first.verse,
                        review.at,
                        review.grade as u8,
                        rev.stamp.at,
                        rev.stamp.device,
                        !live,
                        rev.seq
                    ],
                )?;
            }
        }
        let key = first.key();
        let list = self.reviews.entry(key).or_default();
        list.retain(|r| !(r.at == review.at && r.grade == review.grade));
        if live {
            list.push(review);
            list.sort_by_key(|r| (r.at, r.grade as u8));
        } else if list.is_empty() {
            self.reviews.remove(&key);
        }
        self.review_revs.insert((key, review.at, review.grade), rev);
        Ok(())
    }

    /// Records a recall of the passage that starts at `first`, scheduling it again.
    pub fn review(&mut self, first: VerseRef, grade: Grade, at: i64) -> Result<()> {
        let Some(mut m) = self.memory.get(&first.key()).cloned() else {
            bail!("{first} isn't being memorised")
        };
        m.schedule = m.schedule.next(grade, at);
        self.transaction(|s| {
            s.put_review(first, Review { at, grade })?;
            s.put_memorised(&m)
        })?;
        self.generation += 1;
        Ok(())
    }

    fn put_memorised(&mut self, m: &Memorised) -> Result<()> {
        let rev = self.next_rev();
        self.store_memorised(m.first, Some(m.clone()), rev)
    }

    /// Writes a memorised passage (`None`: a tombstone) with its stamp.
    fn store_memorised(&mut self, first: VerseRef, passage: Option<Memorised>, rev: Rev) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let (last_verse, translation, created, due, stability, difficulty, reps, lapses, last_review) =
                passage.as_ref().map_or((0, "", 0, 0, 0.0, 0.0, 0, 0, None), |m| {
                    (
                        m.last.verse,
                        m.translation.as_str(),
                        m.created,
                        m.schedule.due,
                        m.schedule.stability,
                        m.schedule.difficulty,
                        m.schedule.reps,
                        m.schedule.lapses,
                        m.schedule.last_review,
                    )
                });
            self.exec(
                "INSERT INTO memory(book, chapter, verse, last_verse, translation, created, due,
                     stability, difficulty, reps, lapses, last_review, updated_ms, device, deleted, seq)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(book, chapter, verse) DO UPDATE SET
                     last_verse = excluded.last_verse, translation = excluded.translation,
                     created = excluded.created, due = excluded.due, stability = excluded.stability,
                     difficulty = excluded.difficulty, reps = excluded.reps, lapses = excluded.lapses,
                     last_review = excluded.last_review, updated_ms = excluded.updated_ms,
                     device = excluded.device, deleted = excluded.deleted, seq = excluded.seq",
                params![
                    first.book.info().code,
                    first.chapter,
                    first.verse,
                    last_verse,
                    translation,
                    created,
                    due,
                    stability,
                    difficulty,
                    reps,
                    lapses,
                    last_review,
                    rev.stamp.at,
                    rev.stamp.device,
                    passage.is_none(),
                    rev.seq
                ],
            )?;
        }
        match passage {
            Some(m) => {
                self.memory.insert(first.key(), m);
            }
            None => {
                self.memory.remove(&first.key());
            }
        }
        self.memory_revs.insert(first.key(), rev);
        Ok(())
    }

    /// Forgets live passages that overlap `first..=last`, other than the one that starts at `keep`.
    fn retire_overlapping(&mut self, first: VerseRef, last: VerseRef, keep: VerseRef) -> Result<()> {
        let overlapping: Vec<VerseRef> = self
            .memory
            .values()
            .filter(|o| o.first != keep && o.overlaps(first, last))
            .map(|o| o.first)
            .collect();
        for f in overlapping {
            self.forget_passage(f)?;
        }
        Ok(())
    }

    /// Among live passages that share a verse, keeps the one with the newer stamp so two devices
    /// that memorised overlapping ranges agree. Returns how many were forgotten.
    fn resolve_memory_overlaps(&mut self) -> Result<usize> {
        let mut n = 0;
        loop {
            let live: Vec<Memorised> = self.memory.values().cloned().collect();
            let mut loser = None;
            'pairs: for a in &live {
                for b in &live {
                    if a.first >= b.first || !a.overlaps(b.first, b.last) {
                        continue;
                    }
                    let sa = self.memory_revs.get(&a.first.key()).map(|r| &r.stamp);
                    let sb = self.memory_revs.get(&b.first.key()).map(|r| &r.stamp);
                    loser = Some(match (sa, sb) {
                        (Some(sa), Some(sb)) if sa < sb => a.first,
                        (Some(sa), Some(sb)) if sb < sa => b.first,
                        _ if a.first > b.first => a.first,
                        _ => b.first,
                    });
                    break 'pairs;
                }
            }
            let Some(first) = loser else { break };
            self.forget_passage(first)?;
            n += 1;
        }
        Ok(n)
    }

    /// Rebuilds a live passage's schedule from its recalls, so two devices that each reviewed it
    /// agree after they have both logs.
    fn rebuild_memory_from_reviews(&mut self, first: VerseRef) -> Result<()> {
        let Some(mut m) = self.memory.get(&first.key()).cloned() else { return Ok(()) };
        let reviews = self.reviews.get(&first.key()).cloned().unwrap_or_default();
        if reviews.is_empty() {
            return Ok(());
        }
        let rebuilt = schedule_from_reviews(m.created, &reviews);
        if rebuilt == m.schedule {
            return Ok(());
        }
        m.schedule = rebuilt;
        let Some(rev) = self.memory_revs.get(&first.key()).cloned() else { return Ok(()) };
        self.store_memorised(m.first, Some(m), rev)
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.settings.get(key).cloned()
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        #[cfg(feature = "native")]
        self.exec(
            "INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        self.settings.insert(key.to_string(), value.to_string());
        Ok(())
    }

    /// JSON export of all study data. `text` supplies verse text for context (may return `None`).
    pub fn export_json(
        &self,
        translation: &str,
        text: impl Fn(VerseRef) -> Option<String>,
    ) -> Result<String> {
        self.export_json_with(translation, text, Include::STUDY)
    }

    /// The study data as JSON (re-importable), with what `include` asks for.
    pub fn export_json_with(
        &self,
        translation: &str,
        text: impl Fn(VerseRef) -> Option<String>,
        include: Include,
    ) -> Result<String> {
        let entry = |v: VerseRef| ExportVerse {
            reference: v.to_string(),
            book: v.book.info().code.to_string(),
            chapter: v.chapter,
            verse: v.verse,
            text: text(v),
        };
        let stamp_of = |r: Option<&Rev>| r.map(|r| r.stamp.clone()).unwrap_or_default();
        let mut doc = ExportDoc {
            app: "bible-rs".into(),
            format: EXPORT_FORMAT,
            exported_at: rfc3339(self.now_s()),
            translation: translation.to_string(),
            history: Vec::new(),
            plans: Vec::new(),
            memory: Vec::new(),
            highlights: self
                .highlights()
                .into_iter()
                .map(|(v, h)| {
                    let stamp = stamp_of(self.highlight_revs.get(&v.key()));
                    ExportHighlight {
                        verse: entry(v),
                        color: h.color,
                        created: rfc3339(h.created),
                        start: h.start,
                        end: h.end,
                        quote: h.quote.clone(),
                        edited: (h.edited != h.created).then(|| rfc3339(h.edited)),
                        updated_ms: Some(stamp.at),
                        device: Some(stamp.device),
                    }
                })
                .collect(),
            notes: self
                .notes()
                .into_iter()
                .map(|(v, n)| {
                    let version = self.note_version(v);
                    ExportNote {
                        verse: entry(v),
                        note: n.body.clone(),
                        created: rfc3339(n.created),
                        updated: rfc3339(n.updated),
                        updated_ms: Some(version.stamp.at),
                        device: Some(version.stamp.device),
                        base_updated: version.base.at,
                        base_device: version.base.device,
                        seen: version.seen,
                        conflicts: self.note_conflicts_at(v).into_iter().cloned().collect(),
                    }
                })
                .collect(),
            bookmarks: self
                .bookmarks()
                .into_iter()
                .map(|(v, b)| {
                    let stamp = stamp_of(self.bookmark_revs.get(&v.key()));
                    ExportBookmark {
                        verse: entry(v),
                        label: b.label.clone(),
                        created: rfc3339(b.created),
                        updated_ms: Some(stamp.at),
                        device: Some(stamp.device),
                    }
                })
                .collect(),
            deleted: Changes::default(),
        };
        let mut deleted = self.changes_since(0, false);
        deleted.highlights.retain(|c| c.deleted && include.highlights);
        deleted.notes.retain(|c| c.deleted && include.notes);
        deleted.bookmarks.retain(|c| c.deleted && include.bookmarks);
        deleted.plans.retain(|c| c.deleted && include.history);
        deleted.plan_reads.retain(|c| c.deleted && include.history);
        deleted.memory.retain(|c| c.deleted && include.history);
        deleted.memory_reviews.retain(|c| c.deleted && include.history);
        doc.deleted = deleted;
        if !include.highlights {
            doc.highlights.clear();
        }
        if !include.notes {
            doc.notes.clear();
        }
        if !include.bookmarks {
            doc.bookmarks.clear();
        }
        if include.history {
            doc.history = self
                .reading_since(0)
                .into_iter()
                .map(|s| ExportReading {
                    started: rfc3339(s.started),
                    book: s.book.info().code.to_string(),
                    chapter: s.chapter,
                    translation: s.translation,
                    seconds: s.seconds,
                    furthest: s.furthest,
                    verses: s.verses,
                })
                .collect();
            doc.plans = self
                .plans()
                .into_iter()
                .map(|p| {
                    let mut read: Vec<(u16, u8)> = self.plan_reads(&p.id).into_iter().collect();
                    read.sort_unstable();
                    let reads = read
                        .iter()
                        .map(|&(day, passage)| {
                            let r = self.read_revs.get(&(p.id.clone(), day, passage));
                            ExportRead {
                                day,
                                passage,
                                at: r.map_or(0, |r| r.at),
                                stamp: stamp_of(r.map(|r| &r.rev)),
                            }
                        })
                        .collect();
                    let rev = self.plan_revs.get(&p.id);
                    let stamp = stamp_of(rev.map(|r| &r.rev));
                    ExportPlan {
                        read,
                        reads,
                        started: rfc3339(p.started),
                        finished: p.finished.map(rfc3339),
                        updated_ms: Some(stamp.at),
                        device: Some(stamp.device),
                        reset_ms: rev.map_or(0, |r| r.reset),
                        id: p.id,
                    }
                })
                .collect();
            doc.memory = self
                .memorised()
                .into_iter()
                .map(|m| {
                    let stamp = stamp_of(self.memory_revs.get(&m.first.key()));
                    ExportMemorised {
                        verse: entry(m.first),
                        last_verse: m.last.verse,
                        translation: m.translation.clone(),
                        created: rfc3339(m.created),
                        due: rfc3339(m.schedule.due),
                        stability: m.schedule.stability,
                        difficulty: m.schedule.difficulty,
                        reps: m.schedule.reps,
                        lapses: m.schedule.lapses,
                        last_review: m.schedule.last_review.map(rfc3339),
                        reviews: self
                            .reviews(m.first)
                            .iter()
                            .map(|r| {
                                let rs = stamp_of(self.review_revs.get(&(m.first.key(), r.at, r.grade)));
                                ExportReview {
                                    at: rfc3339(r.at),
                                    grade: r.grade.name().to_string(),
                                    updated_ms: Some(rs.at),
                                    device: Some(rs.device),
                                }
                            })
                            .collect(),
                        updated_ms: Some(stamp.at),
                        device: Some(stamp.device),
                    }
                })
                .collect();
        }
        Ok(serde_json::to_string_pretty(&doc)?)
    }

    pub fn export_markdown(&self, translation: &str, text: impl Fn(VerseRef) -> Option<String>) -> String {
        self.export_markdown_with(translation, text, Include::STUDY)
    }

    /// The study data as readable Markdown, with what `include` asks for.
    pub fn export_markdown_with(
        &self,
        translation: &str,
        text: impl Fn(VerseRef) -> Option<String>,
        include: Include,
    ) -> String {
        use std::fmt::Write;
        let mut md = String::new();
        let _ = writeln!(md, "# Bible study notes\n");
        let _ = writeln!(
            md,
            "Exported from Bible on {} · verse text: {translation}\n",
            &rfc3339(self.now_s())[..10]
        );
        let bookmarks = if include.bookmarks { self.bookmarks() } else { Vec::new() };
        if !bookmarks.is_empty() {
            let _ = writeln!(md, "## Bookmarks\n");
            for (v, b) in bookmarks {
                if b.label.is_empty() {
                    let _ = writeln!(md, "- **{v}**");
                } else {
                    let _ = writeln!(md, "- **{v}** — {}", b.label);
                }
            }
            md.push('\n');
        }
        let highlights = if include.highlights { self.highlights() } else { Vec::new() };
        if !highlights.is_empty() {
            let _ = writeln!(md, "## Highlights\n");
            let mut book = None;
            for (v, h) in highlights {
                if book != Some(v.book) {
                    book = Some(v.book);
                    let _ = writeln!(md, "### {}\n", v.book.name());
                }
                // Part of a verse: the words it covers.
                let t = match text(v) {
                    Some(t) if !h.is_whole() => h.excerpt(&t),
                    Some(t) => t,
                    None => h.quote.clone().unwrap_or_default(),
                };
                let _ = writeln!(md, "- **{v}** ({}) {t}", h.color.name());
            }
            md.push('\n');
        }
        let notes = if include.notes { self.notes() } else { Vec::new() };
        if !notes.is_empty() {
            let _ = writeln!(md, "## Notes\n");
            for (v, n) in notes {
                let _ = writeln!(md, "### {v}\n");
                if let Some(t) = text(v) {
                    let _ = writeln!(md, "> {t}\n");
                }
                let _ = writeln!(md, "{}\n", n.body.trim_end());
            }
        }
        let memory = if include.history { self.memorised() } else { Vec::new() };
        if !memory.is_empty() {
            let _ = writeln!(md, "## Memorising\n");
            for m in memory {
                let _ = writeln!(
                    md,
                    "- **{}** ({}) next due {}",
                    m.reference(),
                    m.translation,
                    &rfc3339(m.schedule.due)[..10]
                );
            }
            md.push('\n');
        }
        let history = if include.history { self.reading_since(0) } else { Vec::new() };
        if !history.is_empty() {
            let _ = writeln!(md, "## Reading history\n");
            for s in history {
                let minutes = (s.seconds + 30) / 60;
                let done = if s.finished() {
                    "finished".to_string()
                } else {
                    format!("{:.0} %", s.progress() * 100.0)
                };
                let _ = writeln!(
                    md,
                    "- {} — {} {} ({}), {minutes} min, {done}",
                    &rfc3339(s.started)[..10],
                    s.book.name(),
                    s.chapter,
                    s.translation
                );
            }
            md.push('\n');
        }
        md
    }

    /// Merges a JSON export in, as an import: what is newer here stays, a note edited on both
    /// sides keeps the losing side's text as a conflict copy, and nothing here is deleted (what
    /// the file has that was deleted here comes back). Returns how many highlights, notes and
    /// bookmarks the file has.
    pub fn import_json(&mut self, json: &str) -> Result<(usize, usize, usize)> {
        let (counts, _) = self.merge_json(json, MergeMode::Import)?;
        Ok(counts)
    }

    /// Takes in a JSON export of another copy of this study (a saved snapshot of it), deletes
    /// included: newer wins.
    pub fn restore_json(&mut self, json: &str) -> Result<MergeReport> {
        let (_, report) = self.merge_json(json, MergeMode::Sync)?;
        Ok(report)
    }

    fn merge_json(&mut self, json: &str, mode: MergeMode) -> Result<((usize, usize, usize), MergeReport)> {
        let doc: ExportDoc = serde_json::from_str(json).context("not a bible-rs export")?;
        if doc.app != "bible-rs" {
            bail!("not a bible-rs export");
        }
        let changes = self.doc_changes(&doc)?;
        let now = self.now_s();
        let date = |s: &str| parse_rfc3339(s).unwrap_or(now);
        self.observe(&changes);
        let report = self.transaction(|s| {
            let report = s.merge_records(&changes, mode)?;
            for r in &doc.history {
                let Some(book) = BookId::from_code(&r.book) else { continue };
                s.record_reading(&ReadingSession {
                    started: date(&r.started),
                    book,
                    chapter: r.chapter,
                    translation: r.translation.clone(),
                    seconds: r.seconds,
                    furthest: r.furthest,
                    verses: r.verses,
                })?;
            }
            Ok(report)
        })?;
        self.generation += 1;
        Ok(((doc.highlights.len(), doc.notes.len(), doc.bookmarks.len()), report))
    }

    /// An export's records as changes. An export from before sync (format 1) has no stamps: a
    /// record's is its newest time, from no device in particular.
    fn doc_changes(&self, doc: &ExportDoc) -> Result<Changes> {
        let verse = |e: &ExportVerse| -> Result<VerseKey> {
            BookId::from_code(&e.book).with_context(|| format!("unknown book {}", e.book))?;
            Ok(VerseKey { book: e.book.clone(), chapter: e.chapter, verse: e.verse })
        };
        let now = self.now_s();
        let date = |s: &str| parse_rfc3339(s).unwrap_or(now);
        let stamp = |ms: Option<i64>, device: &Option<String>, newest: i64| {
            Stamp::new(ms.unwrap_or(newest * 1000), device.clone().unwrap_or_default())
        };
        let mut changes = Changes::default();
        // A verse's highlights travel together, stamped by the newest of them.
        let mut by_verse: BTreeMap<(String, u8, u8), HighlightsChange> = BTreeMap::new();
        for h in &doc.highlights {
            let key = verse(&h.verse)?;
            let created = date(&h.created);
            let edited = h.edited.as_deref().and_then(parse_rfc3339).unwrap_or(created);
            let s = stamp(h.updated_ms, &h.device, created.max(edited));
            let c = by_verse.entry((key.book.clone(), key.chapter, key.verse)).or_insert_with(|| {
                HighlightsChange { verse: key, stamp: s.clone(), deleted: false, parts: Vec::new() }
            });
            c.stamp = c.stamp.clone().max(s);
            c.parts.push(HighlightPart {
                color: h.color,
                start: h.start,
                end: h.end,
                quote: h.quote.clone(),
                created,
                edited,
            });
        }
        changes.highlights = by_verse.into_values().collect();
        for n in &doc.notes {
            let (created, updated) = (date(&n.created), date(&n.updated));
            changes.notes.push(NoteChange {
                verse: verse(&n.verse)?,
                stamp: stamp(n.updated_ms, &n.device, created.max(updated)),
                deleted: false,
                body: n.note.clone(),
                created,
                updated,
                base_updated: n.base_updated,
                base_device: n.base_device.clone(),
                seen: n.seen.clone(),
                conflicts: n.conflicts.clone(),
            });
        }
        for b in &doc.bookmarks {
            let created = date(&b.created);
            changes.bookmarks.push(BookmarkChange {
                verse: verse(&b.verse)?,
                stamp: stamp(b.updated_ms, &b.device, created),
                deleted: false,
                label: b.label.clone(),
                created,
            });
        }
        for p in &doc.plans {
            let started = date(&p.started);
            let finished = p.finished.as_deref().and_then(parse_rfc3339);
            changes.plans.push(PlanChange {
                id: p.id.clone(),
                stamp: stamp(p.updated_ms, &p.device, started.max(finished.unwrap_or(started))),
                deleted: false,
                started,
                finished,
                reset_ms: p.reset_ms,
            });
            if p.reads.is_empty() {
                for &(day, passage) in &p.read {
                    changes.plan_reads.push(PlanReadChange {
                        plan: p.id.clone(),
                        day,
                        passage,
                        stamp: Stamp::new(started * 1000, ""),
                        deleted: false,
                        at: started,
                    });
                }
            }
            for r in &p.reads {
                changes.plan_reads.push(PlanReadChange {
                    plan: p.id.clone(),
                    day: r.day,
                    passage: r.passage,
                    stamp: r.stamp.clone(),
                    deleted: false,
                    at: r.at,
                });
            }
        }
        for m in &doc.memory {
            let first = verse(&m.verse)?;
            let created = date(&m.created);
            let last_review = m.last_review.as_deref().and_then(parse_rfc3339);
            let newest = created.max(last_review.unwrap_or(created));
            changes.memory.push(MemoryChange {
                verse: first.clone(),
                last_verse: m.last_verse,
                translation: m.translation.clone(),
                created,
                due: parse_rfc3339(&m.due).unwrap_or(created),
                stability: m.stability,
                difficulty: m.difficulty,
                reps: m.reps,
                lapses: m.lapses,
                last_review,
                stamp: stamp(m.updated_ms, &m.device, newest),
                deleted: false,
            });
            for r in &m.reviews {
                let Some(at) = parse_rfc3339(&r.at) else { continue };
                if Grade::from_name(&r.grade).is_none() {
                    continue;
                }
                changes.memory_reviews.push(MemoryReviewChange {
                    verse: first.clone(),
                    at,
                    grade: r.grade.clone(),
                    stamp: stamp(r.updated_ms, &r.device, at),
                    deleted: false,
                });
            }
        }
        let d = &doc.deleted;
        changes.highlights.extend(d.highlights.iter().filter(|c| c.deleted).cloned());
        changes.notes.extend(d.notes.iter().filter(|c| c.deleted).cloned());
        changes.bookmarks.extend(d.bookmarks.iter().filter(|c| c.deleted).cloned());
        changes.plans.extend(d.plans.iter().filter(|c| c.deleted).cloned());
        changes.plan_reads.extend(d.plan_reads.iter().filter(|c| c.deleted).cloned());
        changes.memory.extend(d.memory.iter().filter(|c| c.deleted).cloned());
        changes.memory_reviews.extend(d.memory_reviews.iter().filter(|c| c.deleted).cloned());
        Ok(changes)
    }

    /// Erases every highlight, note, bookmark, plan, memorised passage and the reading history.
    /// The erasure is a change like any other: a study library passes it on to the reader's other
    /// devices.
    pub fn erase_all(&mut self) -> Result<()> {
        self.transaction(|s| {
            let verses: HashSet<u32> =
                s.highlights.keys().chain(s.notes.keys()).chain(s.bookmarks.keys()).copied().collect();
            let mut verses: Vec<u32> = verses.into_iter().collect();
            verses.sort_unstable();
            for v in verses.into_iter().filter_map(VerseRef::from_key) {
                s.put_highlights(v, Vec::new())?;
                s.set_note(v, "")?;
                s.set_bookmark(v, None)?;
            }
            let mut plans: Vec<String> = s.plans.keys().cloned().collect();
            plans.sort_unstable();
            for id in plans {
                s.drop_plan(&id)?;
            }
            let mut passages: Vec<VerseRef> = s.memory.values().map(|m| m.first).collect();
            passages.sort();
            for first in passages {
                s.forget_passage(first)?;
            }
            let mut with_copies: Vec<u32> = s.conflicts.keys().map(|(k, _)| *k).collect();
            with_copies.dedup();
            for v in with_copies.into_iter().filter_map(VerseRef::from_key) {
                s.put_conflicts(v, Vec::new())?;
            }
            s.clear_reading(0)
        })?;
        self.generation += 1;
        Ok(())
    }

    // ---------------------------------------------------------------- conflicts

    /// Every conflict copy of a note, by verse, the oldest first.
    pub fn note_conflicts(&self) -> Vec<(VerseRef, &NoteConflict)> {
        let mut v: Vec<_> =
            self.conflicts.iter().filter_map(|((k, _), c)| Some((VerseRef::from_key(*k)?, c))).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.stamp.cmp(&b.1.stamp)));
        v
    }

    /// A verse's conflict copies, the oldest first.
    pub fn note_conflicts_at(&self, v: VerseRef) -> Vec<&NoteConflict> {
        self.conflicts.iter().filter(|((k, _), _)| *k == v.key()).map(|(_, c)| c).collect()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    /// Drops a conflict copy (an edit of its note, so other devices drop it too). Whether there
    /// was one.
    pub fn dismiss_conflict(&mut self, v: VerseRef, stamp: &Stamp) -> Result<bool> {
        let mut copies: Vec<NoteConflict> = self.note_conflicts_at(v).into_iter().cloned().collect();
        let before = copies.len();
        copies.retain(|c| c.stamp != *stamp);
        if copies.len() == before {
            return Ok(false);
        }
        self.transaction(|s| s.put_conflicts(v, copies))?;
        self.generation += 1;
        Ok(true)
    }

    /// Makes a conflict copy the verse's note; the note it had becomes a conflict copy in turn.
    pub fn use_conflict(&mut self, v: VerseRef, stamp: &Stamp) -> Result<bool> {
        let Some(c) = self.conflicts.get(&(v.key(), stamp.clone())).cloned() else { return Ok(false) };
        let mut copies: Vec<NoteConflict> =
            self.note_conflicts_at(v).into_iter().filter(|k| k.stamp != *stamp).cloned().collect();
        if let Some(n) = self.notes.get(&v.key()).filter(|n| n.body != c.body) {
            let current = self.note_version(v).stamp;
            copies.push(NoteConflict { body: n.body.clone(), stamp: current, found: self.now_s() });
        }
        self.transaction(|s| {
            s.set_note(v, &c.body)?;
            s.put_conflicts(v, copies)
        })?;
        self.generation += 1;
        Ok(true)
    }

    /// Replaces a verse's conflict copies, as an edit of its note made here.
    fn put_conflicts(&mut self, v: VerseRef, copies: Vec<NoteConflict>) -> Result<()> {
        let rev = self.next_rev();
        let note = self.notes.get(&v.key()).cloned();
        let version = self.note_version(v).edited(rev.stamp, note.is_some());
        self.set_conflicts(v, copies)?;
        self.store_note(v, note, version, rev.seq)
    }

    /// Writes a verse's conflict copies (only these).
    fn set_conflicts(&mut self, v: VerseRef, copies: Vec<NoteConflict>) -> Result<()> {
        #[cfg(feature = "native")]
        {
            let code = v.book.info().code;
            self.exec(
                "DELETE FROM conflicts WHERE book = ?1 AND chapter = ?2 AND verse = ?3",
                params![code, v.chapter, v.verse],
            )?;
            for c in &copies {
                self.exec(
                    "INSERT OR REPLACE INTO conflicts(book, chapter, verse, body, updated_ms, device, found)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![code, v.chapter, v.verse, c.body, c.stamp.at, c.stamp.device, c.found],
                )?;
            }
        }
        self.conflicts.retain(|(k, _), _| *k != v.key());
        for c in copies {
            self.conflicts.insert((v.key(), c.stamp.clone()), c);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- sync

    /// Stops a plan and unticks its passages, as one change.
    fn drop_plan(&mut self, id: &str) -> Result<()> {
        if !self.plans.contains_key(id) {
            return Ok(());
        }
        let rev = self.next_rev();
        let reset = self.plan_revs.get(id).map_or(0, |r| r.reset);
        self.unread_all(id, &rev.stamp)?;
        self.store_plan(id, None, PlanRev { rev, reset })
    }

    /// What changed here after local sequence number `since` (everything for 0), tombstones
    /// included; with `mine_only`, only what was changed on this device (not taken in from
    /// elsewhere).
    pub fn changes_since(&self, since: i64, mine_only: bool) -> Changes {
        let want = |r: &Rev| r.seq > since && (!mine_only || r.stamp.device == self.device);
        let mut c = Changes::default();
        for (&k, rev) in &self.highlight_revs {
            let Some(v) = VerseRef::from_key(k).filter(|_| want(rev)) else { continue };
            let parts: Vec<HighlightPart> = self.highlight_parts(v).iter().map(part_of).collect();
            c.highlights.push(HighlightsChange {
                verse: key_of(v),
                stamp: rev.stamp.clone(),
                deleted: parts.is_empty(),
                parts,
            });
        }
        for (&k, (version, seq)) in &self.note_revs {
            let rev = Rev { stamp: version.stamp.clone(), seq: *seq };
            let Some(v) = VerseRef::from_key(k).filter(|_| want(&rev)) else { continue };
            let note = self.notes.get(&k);
            c.notes.push(NoteChange {
                verse: key_of(v),
                stamp: version.stamp.clone(),
                deleted: note.is_none(),
                body: note.map(|n| n.body.clone()).unwrap_or_default(),
                created: note.map_or(0, |n| n.created),
                updated: note.map_or(0, |n| n.updated),
                base_updated: version.base.at,
                base_device: version.base.device.clone(),
                seen: version.seen.clone(),
                conflicts: self.note_conflicts_at(v).into_iter().cloned().collect(),
            });
        }
        for (&k, rev) in &self.bookmark_revs {
            let Some(v) = VerseRef::from_key(k).filter(|_| want(rev)) else { continue };
            let b = self.bookmarks.get(&k);
            c.bookmarks.push(BookmarkChange {
                verse: key_of(v),
                stamp: rev.stamp.clone(),
                deleted: b.is_none(),
                label: b.map(|b| b.label.clone()).unwrap_or_default(),
                created: b.map_or(0, |b| b.created),
            });
        }
        for (id, rev) in &self.plan_revs {
            if !want(&rev.rev) {
                continue;
            }
            let p = self.plans.get(id);
            c.plans.push(PlanChange {
                id: id.clone(),
                stamp: rev.rev.stamp.clone(),
                deleted: p.is_none(),
                started: p.map_or(0, |p| p.started),
                finished: p.and_then(|p| p.finished),
                reset_ms: rev.reset,
            });
        }
        for ((plan, day, passage), rev) in &self.read_revs {
            if !want(&rev.rev) {
                continue;
            }
            let read = self.plan_reads.get(plan).is_some_and(|r| r.contains(&(*day, *passage)));
            c.plan_reads.push(PlanReadChange {
                plan: plan.clone(),
                day: *day,
                passage: *passage,
                stamp: rev.rev.stamp.clone(),
                deleted: !read,
                at: rev.at,
            });
        }
        for (&k, rev) in &self.memory_revs {
            let Some(v) = VerseRef::from_key(k).filter(|_| want(rev)) else { continue };
            let m = self.memory.get(&k);
            let s = m.map(|m| m.schedule);
            c.memory.push(MemoryChange {
                verse: key_of(v),
                last_verse: m.map(|m| m.last.verse).unwrap_or(0),
                translation: m.map(|m| m.translation.clone()).unwrap_or_default(),
                created: m.map_or(0, |m| m.created),
                due: s.map_or(0, |s| s.due),
                stability: s.map_or(0.0, |s| s.stability),
                difficulty: s.map_or(0.0, |s| s.difficulty),
                reps: s.map_or(0, |s| s.reps),
                lapses: s.map_or(0, |s| s.lapses),
                last_review: s.and_then(|s| s.last_review),
                stamp: rev.stamp.clone(),
                deleted: m.is_none(),
            });
        }
        for (&(k, at, grade), rev) in &self.review_revs {
            if !want(rev) {
                continue;
            }
            let Some(v) = VerseRef::from_key(k) else { continue };
            let live =
                self.reviews.get(&k).is_some_and(|rs| rs.iter().any(|r| r.at == at && r.grade == grade));
            c.memory_reviews.push(MemoryReviewChange {
                verse: key_of(v),
                at,
                grade: grade.name().to_string(),
                stamp: rev.stamp.clone(),
                deleted: !live,
            });
        }
        // In the order they were made, so a reader of them sees the same thing every time.
        let verse = |v: &VerseKey| (v.book.clone(), v.chapter, v.verse);
        c.highlights.sort_by_key(|h| (h.stamp.clone(), verse(&h.verse)));
        c.notes.sort_by_key(|n| (n.stamp.clone(), verse(&n.verse)));
        c.bookmarks.sort_by_key(|b| (b.stamp.clone(), verse(&b.verse)));
        c.plans.sort_by(|a, b| (&a.stamp, &a.id).cmp(&(&b.stamp, &b.id)));
        c.plan_reads.sort_by(|a, b| {
            (&a.stamp, &a.plan, a.day, a.passage).cmp(&(&b.stamp, &b.plan, b.day, b.passage))
        });
        c.memory.sort_by(|a, b| (&a.stamp, verse(&a.verse)).cmp(&(&b.stamp, verse(&b.verse))));
        c.memory_reviews.sort_by(|a, b| {
            (&a.stamp, verse(&a.verse), a.at, &a.grade).cmp(&(&b.stamp, verse(&b.verse), b.at, &b.grade))
        });
        c
    }

    /// Takes in changes from another copy of the study ([`MergeMode::Sync`]) or from a file the
    /// reader chose ([`MergeMode::Import`]). Each record: newer wins; a note edited on both sides
    /// since they last agreed keeps the losing side's text as a conflict copy. Records for books
    /// this build doesn't know are skipped.
    pub fn merge(&mut self, changes: &Changes, mode: MergeMode) -> Result<MergeReport> {
        self.observe(changes);
        let report = self.transaction(|s| s.merge_records(changes, mode))?;
        if report.changed() + report.conflicts > 0 {
            self.generation += 1;
        }
        Ok(report)
    }

    /// Moves the clock past every stamp in `changes`, but not past a day ahead of the wall clock
    /// (a device set to the wrong year shouldn't drag every later change with it).
    fn observe(&mut self, changes: &Changes) {
        let limit = self.wall_ms() + MAX_DRIFT_MS;
        for s in changes.stamps() {
            if s.at <= limit {
                self.hlc.observe(s.at);
            }
        }
    }

    fn merge_records(&mut self, changes: &Changes, mode: MergeMode) -> Result<MergeReport> {
        let mut report = MergeReport::default();
        for c in &changes.highlights {
            let Some(v) = verse_of(&c.verse) else { continue };
            let k = v.key();
            let local =
                self.highlight_revs.get(&k).map(|r| (r.stamp.clone(), self.highlights.contains_key(&k)));
            let live = !c.deleted && !c.parts.is_empty();
            let mut parts: Vec<Highlight> =
                if live { c.parts.iter().map(highlight_of).collect() } else { Vec::new() };
            parts.sort_by_key(|h| h.start);
            parts.dedup_by_key(|h| h.start);
            let rev = match lww(local.as_ref().map(|(s, l)| (s, *l)), (&c.stamp, live), mode) {
                Pick::Local => continue,
                Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                Pick::Revive => self.next_rev(),
            };
            self.store_highlights(v, parts, rev)?;
            report.highlights += 1;
        }
        for c in &changes.notes {
            let Some(v) = verse_of(&c.verse) else { continue };
            let k = v.key();
            let local = self.note_revs.get(&k).map(|(n, _)| n.clone());
            let local_note = self.notes.get(&k).cloned();
            let same_body = local_note.as_ref().map_or("", |n| n.body.as_str()) == c.body;
            let (wall, device) = (self.wall_ms(), self.device.clone());
            let hlc = &mut self.hlc;
            let merged = sync::merge_note(local.as_ref(), &c.version(), same_body, mode, || {
                Stamp::new(hlc.tick(wall), device)
            });
            let found = self.now_s();
            // The note's conflict copies: the descendant's when one side simply follows on from
            // the other (it dismissed the ones it no longer has), with any of the other side's
            // its version never took in; both sides' otherwise; and the losing side's text.
            let here = self.note_conflicts_at(v).into_iter().cloned().collect::<Vec<_>>();
            let incoming = c.version();
            let follows_on = local.as_ref().is_none_or(|l| incoming.has_seen(&l.stamp));
            let unseen = |by: &NoteVersion, copies: &[NoteConflict]| -> Vec<NoteConflict> {
                copies.iter().filter(|k| !by.has_seen(&k.stamp)).cloned().collect()
            };
            let mut copies = match (&merged.pick, &local) {
                (NotePick::Local, Some(l)) => here.iter().cloned().chain(unseen(l, &c.conflicts)).collect(),
                (NotePick::Local, None) => here.clone(),
                (NotePick::Remote(_), _) if follows_on && mode == MergeMode::Sync => {
                    c.conflicts.iter().cloned().chain(unseen(&incoming, &here)).collect()
                }
                _ => here.iter().chain(&c.conflicts).cloned().collect(),
            };
            let lost = match merged.conflict_copy {
                Some(true) => local_note.as_ref().zip(local.as_ref()).map(|(n, l)| NoteConflict {
                    body: n.body.clone(),
                    stamp: l.stamp.clone(),
                    found,
                }),
                Some(false) => Some(NoteConflict { body: c.body.clone(), stamp: c.stamp.clone(), found }),
                None => None,
            };
            if let Some(copy) = lost {
                if !copies.iter().any(|k| k.stamp == copy.stamp) {
                    report.conflicts += 1;
                }
                copies.push(copy);
            }
            copies.sort_by(|a, b| a.stamp.cmp(&b.stamp));
            copies.dedup_by(|a, b| a.stamp == b.stamp);
            // The note's version accounts for every copy it holds, so dismissing one later tells
            // the other copies of the study it was seen and dropped, not merely never received.
            let held = copies.iter().map(|k| k.stamp.clone()).collect::<Vec<_>>();
            let copies_changed = copies != here;
            if copies_changed {
                self.set_conflicts(v, copies)?;
            }
            let pick = match merged.pick {
                NotePick::Local => match &local {
                    Some(l) if copies_changed || held.iter().any(|t| !l.has_seen(t)) => {
                        // Copies arrived for a note kept as it is: a change here all the same.
                        if copies_changed {
                            report.notes += 1;
                        }
                        let mut version = l.clone();
                        version.absorb(held.iter().cloned());
                        NotePick::LocalAbsorbing(version)
                    }
                    _ => NotePick::Local,
                },
                NotePick::LocalAbsorbing(mut version) => {
                    version.absorb(held.iter().cloned());
                    NotePick::LocalAbsorbing(version)
                }
                NotePick::Remote(mut version) => {
                    version.absorb(held.iter().cloned());
                    NotePick::Remote(version)
                }
                NotePick::Revive(mut version) => {
                    version.absorb(held.iter().cloned());
                    NotePick::Revive(version)
                }
            };
            let created = |remote: i64| match (local_note.as_ref().map(|n| n.created), remote) {
                (Some(l), r) if l > 0 && (r == 0 || l < r) => l,
                (_, r) if r > 0 => r,
                _ => found,
            };
            match pick {
                NotePick::Local => {}
                NotePick::LocalAbsorbing(version) => {
                    let seq = self.next_seq();
                    self.store_note(v, local_note, version, seq)?;
                }
                NotePick::Remote(version) => {
                    let note = version.live.then(|| Note {
                        body: c.body.clone(),
                        created: created(c.created),
                        updated: if c.updated > 0 { c.updated } else { c.stamp.at.div_euclid(1000) },
                    });
                    let seq = self.next_seq();
                    self.store_note(v, note, version, seq)?;
                    report.notes += 1;
                }
                NotePick::Revive(version) => {
                    let note = Note { body: c.body.clone(), created: created(c.created), updated: found };
                    let seq = self.next_seq();
                    self.store_note(v, Some(note), version, seq)?;
                    report.notes += 1;
                }
            }
        }
        for c in &changes.bookmarks {
            let Some(v) = verse_of(&c.verse) else { continue };
            let k = v.key();
            let local =
                self.bookmark_revs.get(&k).map(|r| (r.stamp.clone(), self.bookmarks.contains_key(&k)));
            let rev = match lww(local.as_ref().map(|(s, l)| (s, *l)), (&c.stamp, !c.deleted), mode) {
                Pick::Local => continue,
                Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                Pick::Revive => self.next_rev(),
            };
            let created = match self.bookmarks.get(&k).map(|b| b.created) {
                Some(l) if l > 0 && (c.created == 0 || l < c.created) => l,
                _ => c.created,
            };
            let bookmark = (!c.deleted).then(|| Bookmark { label: c.label.clone(), created });
            self.store_bookmark(v, bookmark, rev)?;
            report.bookmarks += 1;
        }
        for c in &changes.plans {
            let local = self.plan_revs.get(&c.id).cloned();
            let live_here = self.plans.contains_key(&c.id);
            let local_reset = local.as_ref().map_or(0, |r| r.reset);
            let reset = local_reset.max(c.reset_ms);
            let rev =
                match lww(local.as_ref().map(|r| (&r.rev.stamp, live_here)), (&c.stamp, !c.deleted), mode) {
                    Pick::Local => {
                        // A restart elsewhere still forgets the progress before it; but an import
                        // never takes anything away, so its restarts count only when its plan wins.
                        let sync = mode == MergeMode::Sync;
                        if let Some(mut r) = local.filter(|_| sync && reset > local_reset) {
                            r.reset = reset;
                            r.rev.seq = self.next_seq();
                            let plan = self.plans.get(&c.id).map(|p| (p.started, p.finished));
                            self.store_plan(&c.id, plan, r)?;
                            self.forget_reads_before(&c.id, reset)?;
                            report.plans += 1;
                        }
                        continue;
                    }
                    Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                    Pick::Revive => self.next_rev(),
                };
            let plan = (!c.deleted).then_some((c.started, c.finished));
            if c.deleted {
                self.unread_all(&c.id, &rev.stamp)?;
            }
            self.store_plan(&c.id, plan, PlanRev { rev, reset })?;
            self.forget_reads_before(&c.id, reset)?;
            report.plans += 1;
        }
        for c in &changes.plan_reads {
            let reset = self.plan_revs.get(&c.plan).map_or(0, |r| r.reset);
            if c.stamp.at < reset {
                // Ticked in an earlier run of the plan.
                continue;
            }
            let key = (c.plan.clone(), c.day, c.passage);
            let local = self.read_revs.get(&key).map(|r| r.rev.stamp.clone());
            let read_here = self.plan_reads.get(&c.plan).is_some_and(|r| r.contains(&(c.day, c.passage)));
            let rev = match lww(local.as_ref().map(|s| (s, read_here)), (&c.stamp, !c.deleted), mode) {
                Pick::Local => continue,
                Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                Pick::Revive => self.next_rev(),
            };
            self.store_read(&c.plan, c.day, c.passage, !c.deleted, ReadRev { rev, at: c.at })?;
            report.plan_reads += 1;
        }
        let mut rebuild = Vec::new();
        for c in &changes.memory {
            let Some(first) = verse_of(&c.verse) else { continue };
            let k = first.key();
            let local = self.memory_revs.get(&k).map(|r| (r.stamp.clone(), self.memory.contains_key(&k)));
            let rev = match lww(local.as_ref().map(|(s, l)| (s, *l)), (&c.stamp, !c.deleted), mode) {
                Pick::Local => continue,
                Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                Pick::Revive => self.next_rev(),
            };
            let created = match self.memory.get(&k).map(|m| m.created) {
                Some(l) if l > 0 && (c.created == 0 || l < c.created) => l,
                _ => c.created,
            };
            let passage = if c.deleted {
                None
            } else {
                let last = VerseRef::new(first.book, first.chapter, c.last_verse);
                if last < first {
                    continue;
                }
                Some(Memorised {
                    first,
                    last,
                    translation: c.translation.clone(),
                    created,
                    schedule: Schedule {
                        due: c.due,
                        stability: c.stability,
                        difficulty: c.difficulty,
                        reps: c.reps,
                        lapses: c.lapses,
                        last_review: c.last_review,
                    },
                })
            };
            if let Some(m) = &passage {
                if mode == MergeMode::Import {
                    self.retire_overlapping(m.first, m.last, m.first)?;
                }
            } else {
                self.forget_reviews(first)?;
            }
            self.store_memorised(first, passage, rev)?;
            report.memory += 1;
            rebuild.push(first);
        }
        for c in &changes.memory_reviews {
            let Some(first) = verse_of(&c.verse) else { continue };
            let Some(grade) = Grade::from_name(&c.grade) else { continue };
            let key = (first.key(), c.at, grade);
            let local = self.review_revs.get(&key).map(|r| r.stamp.clone());
            let live_here = self
                .reviews
                .get(&first.key())
                .is_some_and(|rs| rs.iter().any(|r| r.at == c.at && r.grade == grade));
            let rev = match lww(local.as_ref().map(|s| (s, live_here)), (&c.stamp, !c.deleted), mode) {
                Pick::Local => continue,
                Pick::Remote => Rev { stamp: c.stamp.clone(), seq: self.next_seq() },
                Pick::Revive => self.next_rev(),
            };
            self.store_review(first, Review { at: c.at, grade }, !c.deleted, rev)?;
            report.memory_reviews += 1;
            rebuild.push(first);
        }
        if mode == MergeMode::Sync {
            let forgotten = self.resolve_memory_overlaps()?;
            report.memory += forgotten;
        }
        rebuild.sort();
        rebuild.dedup();
        for first in rebuild {
            if self.memory.contains_key(&first.key()) {
                self.rebuild_memory_from_reviews(first)?;
            } else {
                // A newer forget beat a concurrent review: drop the review so it cannot linger.
                self.forget_reviews(first)?;
            }
        }
        Ok(report)
    }

    /// Unticks a plan's passages ticked before it was (re)started at `reset` (ms).
    fn forget_reads_before(&mut self, id: &str, reset: i64) -> Result<()> {
        let mut stale: Vec<((u16, u8), i64)> = self
            .plan_reads(id)
            .into_iter()
            .filter_map(|(day, passage)| {
                let r = self.read_revs.get(&(id.to_string(), day, passage))?;
                (r.rev.stamp.at < reset).then_some(((day, passage), r.at))
            })
            .collect();
        stale.sort_unstable();
        for ((day, passage), at) in stale {
            let rev = Rev { stamp: Stamp::new(reset, self.device.clone()), seq: self.next_seq() };
            self.store_read(id, day, passage, false, ReadRev { rev, at })?;
        }
        Ok(())
    }

    /// Forgets tombstones older than `before_ms` (Unix ms). Returns how many.
    pub fn prune_tombstones(&mut self, before_ms: i64) -> Result<usize> {
        let mut n = 0;
        #[cfg(feature = "native")]
        for table in ["highlights", "bookmarks", "plans", "plan_reads", "memory", "memory_reviews"] {
            self.exec(&format!("DELETE FROM {table} WHERE deleted = 1 AND updated_ms < ?1"), [before_ms])?;
        }
        // A deleted note's conflict copies keep it.
        #[cfg(feature = "native")]
        self.exec(
            "DELETE FROM notes WHERE deleted = 1 AND updated_ms < ?1 AND NOT EXISTS (SELECT 1 FROM conflicts c
                 WHERE c.book = notes.book AND c.chapter = notes.chapter AND c.verse = notes.verse)",
            [before_ms],
        )?;
        let old = |r: &Rev| r.stamp.at < before_ms;
        let before = self.highlight_revs.len();
        let live = &self.highlights;
        self.highlight_revs.retain(|k, r| live.contains_key(k) || !old(r));
        n += before - self.highlight_revs.len();
        let before = self.note_revs.len();
        let copies: HashSet<u32> = self.conflicts.keys().map(|(k, _)| *k).collect();
        self.note_revs.retain(|k, (v, _)| v.live || v.stamp.at >= before_ms || copies.contains(k));
        n += before - self.note_revs.len();
        let before = self.bookmark_revs.len();
        let live = &self.bookmarks;
        self.bookmark_revs.retain(|k, r| live.contains_key(k) || !old(r));
        n += before - self.bookmark_revs.len();
        let before = self.plan_revs.len();
        let live = &self.plans;
        self.plan_revs.retain(|k, r| live.contains_key(k) || !old(&r.rev));
        n += before - self.plan_revs.len();
        let before = self.read_revs.len();
        let live = &self.plan_reads;
        self.read_revs
            .retain(|(p, d, i), r| live.get(p).is_some_and(|s| s.contains(&(*d, *i))) || !old(&r.rev));
        n += before - self.read_revs.len();
        let before = self.memory_revs.len();
        let live = &self.memory;
        self.memory_revs.retain(|k, r| live.contains_key(k) || !old(r));
        n += before - self.memory_revs.len();
        let before = self.review_revs.len();
        let live = &self.reviews;
        self.review_revs.retain(|(k, at, grade), r| {
            live.get(k).is_some_and(|rs| rs.iter().any(|x| x.at == *at && x.grade == *grade)) || !old(r)
        });
        n += before - self.review_revs.len();
        if n > 0 {
            let seq = self.seq.to_string();
            self.set_sync_value("seq", &seq)?;
        }
        Ok(n)
    }
}

#[derive(Serialize, Deserialize)]
struct ExportDoc {
    app: String,
    format: u32,
    exported_at: String,
    translation: String,
    #[serde(default)]
    highlights: Vec<ExportHighlight>,
    #[serde(default)]
    notes: Vec<ExportNote>,
    #[serde(default)]
    bookmarks: Vec<ExportBookmark>,
    /// Reading sessions and plans, when the export includes history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    history: Vec<ExportReading>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    plans: Vec<ExportPlan>,
    /// Passages being memorised and their schedules, when the export includes history.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    memory: Vec<ExportMemorised>,
    /// Tombstones: what was deleted, so another copy of the study deletes it too (an import
    /// ignores them).
    #[serde(default, skip_serializing_if = "Changes::is_empty")]
    deleted: Changes,
}

/// A passage being memorised: verses `verse..=last_verse` of one chapter.
#[derive(Serialize, Deserialize)]
struct ExportMemorised {
    #[serde(flatten)]
    verse: ExportVerse,
    last_verse: u8,
    translation: String,
    created: String,
    due: String,
    stability: f64,
    difficulty: f64,
    reps: u32,
    lapses: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_review: Option<String>,
    /// Every recall, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reviews: Vec<ExportReview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
}

/// One recall of a passage: when, and its grade ("Again", "Hard", "Good" or "Easy").
#[derive(Serialize, Deserialize)]
struct ExportReview {
    at: String,
    grade: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
}

/// The export format [`StudyStore::export_json_with`] writes. 2 (2026-11) adds each record's
/// stamp (`updated_ms`, `device`), notes' history, plans' ticked passages with theirs, and
/// tombstones; apps that read format 1 ignore them, and format 1 still imports.
pub const EXPORT_FORMAT: u32 = 2;

#[derive(Serialize, Deserialize)]
struct ExportReading {
    started: String,
    book: String,
    chapter: u8,
    translation: String,
    seconds: u32,
    furthest: u8,
    verses: u8,
}

#[derive(Serialize, Deserialize)]
struct ExportPlan {
    id: String,
    started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished: Option<String>,
    /// Passages ticked, as (day, index).
    read: Vec<(u16, u8)>,
    /// The same, with when each was ticked and its stamp (format 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reads: Vec<ExportRead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    /// When it was last (re)started (ms).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    reset_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct ExportRead {
    day: u16,
    passage: u8,
    /// Unix seconds.
    #[serde(default)]
    at: i64,
    #[serde(flatten)]
    stamp: Stamp,
}

/// What an export carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Include {
    pub highlights: bool,
    pub notes: bool,
    pub bookmarks: bool,
    /// Reading history, plans and passages being memorised.
    pub history: bool,
}

impl Include {
    /// Highlights, notes and bookmarks.
    pub const STUDY: Include = Include { highlights: true, notes: true, bookmarks: true, history: false };
    pub const EVERYTHING: Include = Include { highlights: true, notes: true, bookmarks: true, history: true };

    pub const fn default_none() -> Include {
        Include { highlights: false, notes: false, bookmarks: false, history: false }
    }
}

#[derive(Serialize, Deserialize)]
struct ExportVerse {
    reference: String,
    book: String,
    chapter: u8,
    verse: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ExportHighlight {
    #[serde(flatten)]
    verse: ExportVerse,
    color: HighlightColor,
    created: String,
    /// Part of a verse: from this char of its text (exports before 2026-10 have whole verses).
    #[serde(default, skip_serializing_if = "is_zero")]
    start: u32,
    /// To before this char; left out, to the verse's end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end: Option<u32>,
    /// The words covered, to place them in another translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quote: Option<String>,
    /// When it was last coloured, if after it was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edited: Option<String>,
    /// When the verse's highlights last changed (ms) and where (format 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize)]
struct ExportNote {
    #[serde(flatten)]
    verse: ExportVerse,
    note: String,
    created: String,
    updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    /// The version it was edited from (ms and device) and older ones it supersedes.
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    base_updated: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    base_device: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    seen: Vec<Stamp>,
    /// Versions that lost to it, edited at the same time elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<NoteConflict>,
}

#[derive(Serialize, Deserialize)]
struct ExportBookmark {
    #[serde(flatten)]
    verse: ExportVerse,
    label: String,
    created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
}

/// Unix seconds → `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian, UTC).
pub fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let days = days_from_civil(num(0..4)?, num(5..7)? as u32, num(8..10)? as u32);
    Some(days * 86_400 + num(11..13)? * 3600 + num(14..16)? * 60 + num(17..19)?)
}

// Howard Hinnant's date algorithms.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    #[test]
    fn plans_start_tick_and_stop() {
        let mut s = StudyStore::open_in_memory().unwrap();
        s.start_plan("john-21", 1_000).unwrap();
        s.set_plan_read("john-21", 1, 0, true).unwrap();
        s.set_plan_read("john-21", 2, 0, true).unwrap();
        s.set_plan_read("john-21", 2, 0, false).unwrap();
        assert_eq!(s.plans()[0].id, "john-21");
        assert_eq!(s.plan_reads("john-21").into_iter().collect::<Vec<_>>(), vec![(1, 0)]);
        // Restarting forgets progress.
        s.start_plan("john-21", 2_000).unwrap();
        assert!(s.plan_reads("john-21").is_empty());
        s.finish_plan("john-21", Some(3_000)).unwrap();
        assert_eq!(s.plans()[0].finished, Some(3_000));
        s.stop_plan("john-21").unwrap();
        assert!(s.plans().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jn316() -> VerseRef {
        VerseRef::new(BookId(42), 3, 16)
    }

    #[test]
    fn reading_sessions() {
        let mut s = StudyStore::open_in_memory().unwrap();
        let mut session = ReadingSession {
            started: 1_000,
            book: BookId(42),
            chapter: 3,
            translation: "kjv".into(),
            seconds: 30,
            furthest: 10,
            verses: 36,
        };
        s.record_reading(&session).unwrap();
        session.seconds = 400;
        session.furthest = 36;
        s.record_reading(&session).unwrap();
        s.record_reading(&ReadingSession { started: 2_000, chapter: 4, furthest: 3, ..session.clone() })
            .unwrap();
        let all = s.reading_since(0);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].chapter, 4, "newest first");
        assert_eq!(all[1].seconds, 400, "a session is saved in place");
        assert!(all[1].finished() && !all[0].finished());
        assert_eq!(s.reading_since(1_500).len(), 1);
        s.clear_reading(1_500).unwrap();
        assert_eq!(s.reading_since(0).len(), 1);
    }

    #[test]
    fn memorised_passages_are_scheduled_and_kept() {
        let dir = std::env::temp_dir().join(format!("bible-rs-memory-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        let v = |n| VerseRef::new(BookId(42), 3, n);
        {
            let mut s = StudyStore::open(&path).unwrap();
            s.memorise(v(16), v(17), "kjv").unwrap();
            assert_eq!(s.memorised_at(v(17)).map(|m| m.first), Some(v(16)));
            assert!(s.memorised_at(v(18)).is_none());
            assert_eq!(s.due(now()).len(), 1, "a new passage is due at once");
            // One that overlaps takes its place.
            s.memorise(v(17), v(18), "kjv").unwrap();
            assert!(s.memorised_at(v(16)).is_none());
            assert_eq!(s.memorised().len(), 1);
            s.review(v(17), Grade::Good, now()).unwrap();
            assert!(s.due(now()).is_empty());
            assert!(s.memorise(v(18), VerseRef::new(BookId(42), 4, 1), "kjv").is_err());
        }
        let mut s = StudyStore::open(&path).unwrap();
        let m = s.memorised_at(v(18)).cloned().unwrap();
        assert_eq!((m.first, m.last, m.translation.as_str(), m.schedule.reps), (v(17), v(18), "kjv", 1));
        s.forget(v(17)).unwrap();
        assert!(StudyStore::open(&path).unwrap().memorised().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memorised_passages_survive_a_backup() {
        let v = |n| VerseRef::new(BookId(42), 3, n);
        let mut s = StudyStore::open_in_memory().unwrap();
        s.memorise(v(16), v(17), "bsb").unwrap();
        s.review(v(16), Grade::Good, now()).unwrap();
        let saved = s.memorised_at(v(16)).cloned().unwrap();

        let study_only = s.export_json_with("BSB", |_| None, Include::STUDY).unwrap();
        assert!(!study_only.contains("\"memory\""), "a notes export leaves memorising out");
        let backup = s.export_json_with("BSB", |_| None, Include::EVERYTHING).unwrap();
        assert!(s.export_markdown_with("BSB", |_| None, Include::EVERYTHING).contains("John 3:16–17"));

        let mut restored = StudyStore::open_in_memory().unwrap();
        // An overlapping passage already there gives way to the backup's.
        restored.memorise(v(17), v(18), "kjv").unwrap();
        restored.import_json(&backup).unwrap();
        let m = restored.memorised_at(v(17)).cloned().unwrap();
        assert_eq!((m.first, m.last, m.translation.as_str()), (v(16), v(17), "bsb"));
        assert_eq!(m.schedule.reps, 1);
        assert_eq!(m.schedule.due, saved.schedule.due, "the schedule carries over");
        assert_eq!(m.schedule.stability, saved.schedule.stability);
        assert!(restored.memorised_at(v(18)).is_none());
        assert_eq!(restored.reviews(v(16)), s.reviews(v(16)), "and so does every recall");
    }

    #[test]
    fn every_recall_is_logged_and_kept() {
        let dir = std::env::temp_dir().join(format!("bible-rs-reviews-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        let v = |n| VerseRef::new(BookId(42), 3, n);
        let t = 1_790_000_000;
        {
            let mut s = StudyStore::open(&path).unwrap();
            s.memorise(v(16), v(17), "bsb").unwrap();
            s.review(v(16), Grade::Again, t).unwrap();
            s.review(v(16), Grade::Good, t + 600).unwrap();
        }
        let mut s = StudyStore::open(&path).unwrap();
        let log = [Review { at: t, grade: Grade::Again }, Review { at: t + 600, grade: Grade::Good }];
        assert_eq!(s.reviews(v(16)), log);
        let replayed = log.iter().fold(Schedule::new(t), |sch, r| sch.next(r.grade, r.at));
        assert_eq!(
            s.memorised_at(v(16)).unwrap().schedule.due,
            replayed.due,
            "the log rebuilds the schedule"
        );
        s.forget(v(16)).unwrap();
        assert!(s.reviews(v(16)).is_empty());
        let s = StudyStore::open(&path).unwrap();
        assert!(s.reviews(v(16)).is_empty(), "forgetting a passage forgets its recalls");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crud_and_persistence() {
        let dir = std::env::temp_dir().join(format!("bible-rs-study-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = StudyStore::open(&path).unwrap();
            let g = s.generation();
            s.set_highlight(jn316(), Some(HighlightColor::Yellow)).unwrap();
            s.set_highlight(jn316(), Some(HighlightColor::Green)).unwrap();
            s.set_note(jn316(), "God's love").unwrap();
            assert!(s.toggle_bookmark(VerseRef::new(BookId(0), 1, 1)).unwrap());
            s.set_setting("theme", "dark").unwrap();
            s.record_reading(&ReadingSession {
                started: 1_000,
                book: BookId(42),
                chapter: 3,
                translation: "kjv".into(),
                seconds: 90,
                furthest: 20,
                verses: 36,
            })
            .unwrap();
            s.start_plan("john-21", 900).unwrap();
            s.set_plan_read("john-21", 1, 0, true).unwrap();
            assert!(s.generation() > g);
        }
        let mut s = StudyStore::open(&path).unwrap();
        assert_eq!(s.highlight(jn316()), Some(HighlightColor::Green));
        assert_eq!(s.note(jn316()).unwrap().body, "God's love");
        assert!(s.is_bookmarked(VerseRef::new(BookId(0), 1, 1)));
        assert_eq!(s.setting("theme").as_deref(), Some("dark"));
        assert_eq!(s.counts(), (1, 1, 1));
        assert_eq!(s.reading_since(0)[0].furthest, 20);
        assert_eq!((s.plans()[0].started, s.plan_reads("john-21").len()), (900, 1));
        s.set_note(jn316(), "  ").unwrap();
        s.set_highlight(jn316(), None).unwrap();
        assert!(!s.toggle_bookmark(VerseRef::new(BookId(0), 1, 1)).unwrap());
        assert_eq!(s.counts(), (0, 0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_round_trip_and_markdown() {
        let mut a = StudyStore::open_in_memory().unwrap();
        a.set_highlight(jn316(), Some(HighlightColor::Pink)).unwrap();
        a.set_note(jn316(), "line one\nline two").unwrap();
        a.set_bookmark(VerseRef::new(BookId(18), 23, 1), Some("Shepherd")).unwrap();
        let text = |v: VerseRef| Some(format!("text of {v}"));
        let json = a.export_json("KJV", text).unwrap();
        assert!(json.contains("\"reference\": \"John 3:16\""));
        assert!(json.contains("\"color\": \"pink\""));

        let mut b = StudyStore::open_in_memory().unwrap();
        assert_eq!(b.import_json(&json).unwrap(), (1, 1, 1));
        assert_eq!(b.highlight(jn316()), Some(HighlightColor::Pink));
        assert_eq!(b.note(jn316()).unwrap().body, "line one\nline two");
        assert_eq!(b.bookmark(VerseRef::new(BookId(18), 23, 1)).unwrap().label, "Shepherd");
        assert_eq!(b.note(jn316()).unwrap().created, a.note(jn316()).unwrap().created);

        let md = b.export_markdown("KJV", text);
        assert!(md.contains("## Bookmarks"));
        assert!(md.contains("- **Psalm 23:1** — Shepherd"));
        assert!(md.contains("- **John 3:16** (pink) text of John 3:16"));
        assert!(md.contains("### John 3:16\n\n> text of John 3:16\n\nline one\nline two"));
        assert!(b.import_json("{}").is_err());
        // History and plans travel when asked for, and erasing clears everything.
        a.record_reading(&ReadingSession {
            started: 1_000,
            book: BookId(42),
            chapter: 3,
            translation: "kjv".into(),
            seconds: 300,
            furthest: 36,
            verses: 36,
        })
        .unwrap();
        a.start_plan("john-21", 900).unwrap();
        a.set_plan_read("john-21", 1, 0, true).unwrap();
        let all = a.export_json_with("KJV", text, Include::EVERYTHING).unwrap();
        let only_notes =
            a.export_json_with("KJV", text, Include { notes: true, ..Include::default_none() }).unwrap();
        assert!(!only_notes.contains("\"highlights\": [\n"), "{only_notes}");
        let mut c = StudyStore::open_in_memory().unwrap();
        c.import_json(&all).unwrap();
        assert_eq!(c.reading_since(0).len(), 1);
        assert_eq!(c.plan_reads("john-21").len(), 1);
        c.erase_all().unwrap();
        assert_eq!((c.counts(), c.reading_since(0).len(), c.plans().len()), ((0, 0, 0), 0, 0));
    }

    /// The web app's store: no SQLite, saved and restored through its JSON export.
    #[test]
    fn without_a_database() {
        let mut a = StudyStore::new();
        a.set_highlight(jn316(), Some(HighlightColor::Blue)).unwrap();
        a.set_note(jn316(), "first").unwrap();
        a.set_bookmark(VerseRef::new(BookId(18), 23, 1), Some("Shepherd")).unwrap();
        a.set_setting("web", "{}").unwrap();
        a.record_reading(&ReadingSession {
            started: 1_000,
            book: BookId(42),
            chapter: 3,
            translation: "kjv".into(),
            seconds: 300,
            furthest: 36,
            verses: 36,
        })
        .unwrap();
        a.start_plan("john-21", 900).unwrap();
        a.set_plan_read("john-21", 1, 0, true).unwrap();
        a.finish_plan("john-21", Some(2_000)).unwrap();
        assert_eq!(a.setting("web").as_deref(), Some("{}"));

        let saved = a.export_json_with("KJV", |_| None, Include::EVERYTHING).unwrap();
        let mut b = StudyStore::new();
        b.import_json(&saved).unwrap();
        assert_eq!(b.highlight(jn316()), Some(HighlightColor::Blue));
        assert_eq!(b.note(jn316()), a.note(jn316()));
        assert_eq!(b.bookmarks()[0].1, a.bookmarks()[0].1);
        assert_eq!(b.reading_since(0), a.reading_since(0));
        assert_eq!(b.plans(), a.plans());
        assert_eq!(b.plan_reads("john-21"), a.plan_reads("john-21"));

        b.clear_reading(0).unwrap();
        b.stop_plan("john-21").unwrap();
        assert!(b.reading_since(0).is_empty() && b.plans().is_empty() && b.plan_reads("john-21").is_empty());
        b.erase_all().unwrap();
        assert_eq!(b.counts(), (0, 0, 0));
    }

    fn text(v: VerseRef) -> Option<String> {
        Some(
            match v.verse {
                16 => "For God so loved the world, that he gave his only begotten Son.",
                17 => "For God sent not his Son into the world to condemn the world.",
                18 => "He that believeth on him is not condemned.",
                _ => return None,
            }
            .to_string(),
        )
    }

    fn jn3(n: u8) -> VerseRef {
        VerseRef::new(BookId(42), 3, n)
    }

    fn shown(s: &StudyStore, v: VerseRef) -> Vec<(String, HighlightColor)> {
        let t = text(v).unwrap();
        s.highlight_ranges(v, &t)
            .into_iter()
            .map(|(r, c)| (t.chars().skip(r.start).take(r.len()).collect(), c))
            .collect()
    }

    /// A file written before highlights could cover part of a verse: one row per verse.
    #[test]
    fn migrates_whole_verse_highlights() {
        let dir = std::env::temp_dir().join(format!("bible-rs-migrate-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE highlights(
                     book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                     color INTEGER NOT NULL, created INTEGER NOT NULL,
                     PRIMARY KEY(book, chapter, verse));
                 INSERT INTO highlights VALUES('JHN', 3, 16, 0, 1000), ('JHN', 3, 17, 3, 2000),
                     ('PSA', 23, 1, 5, 3000), ('XXX', 1, 1, 0, 4000);
                 CREATE TABLE notes(
                     book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                     body TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
                     PRIMARY KEY(book, chapter, verse));
                 INSERT INTO notes VALUES('JHN', 3, 16, 'kept', 1, 1);",
            )
            .unwrap();
        }
        let mut s = StudyStore::open(&path).unwrap();
        let all: Vec<(VerseRef, Highlight)> =
            s.highlights().into_iter().map(|(v, h)| (v, h.clone())).collect();
        assert_eq!(
            all,
            vec![
                (VerseRef::new(BookId::PSALMS, 23, 1), Highlight::whole(HighlightColor::Orange, 3000)),
                (jn3(16), Highlight::whole(HighlightColor::Yellow, 1000)),
                (jn3(17), Highlight::whole(HighlightColor::Pink, 2000)),
            ]
        );
        assert_eq!(s.note(jn3(16)).unwrap().body, "kept");
        // The unknown book's row is carried over too, though nothing shows it.
        let conn = Connection::open(&path).unwrap();
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM highlights", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 4);
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 2);
        drop(conn);
        // Parts of verses save in the new table, and opening again changes nothing.
        s.highlight_span(
            &TextSpan { first: jn3(18), start: 0, last: jn3(18), end: 14 },
            Some(HighlightColor::Blue),
            text,
        )
        .unwrap();
        drop(s);
        let s = StudyStore::open(&path).unwrap();
        assert_eq!(s.counts().0, 4);
        assert_eq!(shown(&s, jn3(18)), vec![("He that believ".into(), HighlightColor::Blue)]);
        assert_eq!(s.highlight(jn3(17)), Some(HighlightColor::Pink));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spans_across_verses_persist() {
        let dir = std::env::temp_dir().join(format!("bible-rs-spans-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = StudyStore::open(&path).unwrap();
            // From "the world," in 16 to "his Son" in 17.
            let span = TextSpan { first: jn3(16), start: 17, last: jn3(17), end: 24 };
            s.highlight_span(&span, Some(HighlightColor::Yellow), text).unwrap();
            assert_eq!(
                shown(&s, jn3(16)),
                vec![("the world, that he gave his only begotten Son.".into(), HighlightColor::Yellow)]
            );
            assert_eq!(shown(&s, jn3(17)), vec![("For God sent not his Son".into(), HighlightColor::Yellow)]);
            assert_eq!(s.span_color(&span, text), Some(HighlightColor::Yellow));
            assert_eq!(s.span_color(&TextSpan::verses(jn3(16), jn3(17)), text), None, "not all of them");
            // Clearing the middle leaves both ends.
            s.highlight_span(&TextSpan { first: jn3(16), start: 28, last: jn3(17), end: 3 }, None, text)
                .unwrap();
            assert_eq!(shown(&s, jn3(16)), vec![("the world,".into(), HighlightColor::Yellow)]);
            assert_eq!(shown(&s, jn3(17)), vec![("God sent not his Son".into(), HighlightColor::Yellow)]);
        }
        let mut s = StudyStore::open(&path).unwrap();
        assert_eq!(shown(&s, jn3(16)), vec![("the world,".into(), HighlightColor::Yellow)]);
        assert_eq!(s.counts().0, 2);
        // Whole verses over parts replace them; removing one highlight leaves the verse's others.
        let made = s.highlight_parts(jn3(16))[0].created;
        s.set_highlight(jn3(16), Some(HighlightColor::Green)).unwrap();
        let parts = s.highlight_parts(jn3(16));
        assert!(parts.len() == 1 && parts[0].is_whole() && parts[0].color == HighlightColor::Green);
        assert_eq!(parts[0].created, made, "a verse already highlighted keeps when it was first highlighted");
        s.highlight_span(
            &TextSpan { first: jn3(18), start: 0, last: jn3(18), end: 7 },
            Some(HighlightColor::Blue),
            text,
        )
        .unwrap();
        s.highlight_span(
            &TextSpan { first: jn3(18), start: 28, last: jn3(18), end: 31 },
            Some(HighlightColor::Pink),
            text,
        )
        .unwrap();
        assert_eq!(s.highlight_parts(jn3(18)).len(), 2);
        s.remove_highlight(jn3(18), 28).unwrap();
        assert_eq!(shown(&s, jn3(18)), vec![("He that".into(), HighlightColor::Blue)]);
        // A verse the translation hasn't is coloured whole.
        s.highlight_span(
            &TextSpan { first: jn3(19), start: 3, last: jn3(19), end: 9 },
            Some(HighlightColor::Blue),
            text,
        )
        .unwrap();
        assert!(s.highlight_parts(jn3(19))[0].is_whole());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parts_of_verses_round_trip_through_json() {
        let mut a = StudyStore::new();
        a.highlight_span(
            &TextSpan { first: jn3(16), start: 4, last: jn3(16), end: 26 },
            Some(HighlightColor::Pink),
            text,
        )
        .unwrap();
        a.highlight_span(
            &TextSpan { first: jn3(17), start: 21, last: jn3(17), end: usize::MAX },
            Some(HighlightColor::Blue),
            text,
        )
        .unwrap();
        a.set_highlight(jn3(18), Some(HighlightColor::Green)).unwrap();
        let json = a.export_json("KJV", text).unwrap();
        assert!(json.contains("\"quote\": \"God so loved the world\""), "{json}");
        assert!(json.contains("\"start\": 4"));
        let md = a.export_markdown("KJV", text);
        assert!(md.contains("- **John 3:16** (pink) …God so loved the world…"), "{md}");
        assert!(md.contains("- **John 3:17** (blue) …Son into the world to condemn the world."), "{md}");
        assert!(md.contains("- **John 3:18** (green) He that believeth on him is not condemned."), "{md}");

        // The web app's snapshot path: no database.
        let mut b = StudyStore::new();
        assert_eq!(b.import_json(&json).unwrap().0, 3);
        for v in [jn3(16), jn3(17), jn3(18)] {
            assert_eq!(b.highlight_parts(v), a.highlight_parts(v), "{v}");
        }
        // And into a file.
        let mut c = StudyStore::open_in_memory().unwrap();
        c.import_json(&json).unwrap();
        assert_eq!(shown(&c, jn3(16)), vec![("God so loved the world".into(), HighlightColor::Pink)]);
    }

    /// When a highlight was last coloured travels with it, through JSON and the file.
    #[test]
    fn colouring_times_persist() {
        let json = r#"{"app":"bible-rs","format":1,"exported_at":"2026-10-16T12:00:00Z","translation":"KJV",
            "highlights":[{"reference":"John 3:16","book":"JHN","chapter":3,"verse":16,"color":"blue",
              "created":"2026-10-14T12:00:00Z","edited":"2026-10-15T12:00:00Z"}]}"#;
        let dir = std::env::temp_dir().join(format!("bible-rs-edited-{}", std::process::id()));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = StudyStore::open(&path).unwrap();
        a.import_json(json).unwrap();
        let h = a.highlight_parts(jn3(16))[0].clone();
        assert_eq!((h.created, h.edited), (1_791_979_200, 1_792_065_600));
        let out = a.export_json("KJV", |_| None).unwrap();
        assert!(out.contains("\"edited\": \"2026-10-15T12:00:00Z\""), "{out}");
        drop(a);
        assert_eq!(StudyStore::open(&path).unwrap().highlight_parts(jn3(16)), std::slice::from_ref(&h));
        let mut b = StudyStore::new();
        b.import_json(&out).unwrap();
        assert_eq!(b.highlight_parts(jn3(16)), &[h]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exports made before parts of verses could be highlighted.
    #[test]
    fn old_exports_import_as_whole_verses() {
        let json = r#"{"app":"bible-rs","format":1,"exported_at":"2026-10-14T12:00:00Z","translation":"KJV",
            "highlights":[
              {"reference":"John 3:16","book":"JHN","chapter":3,"verse":16,"color":"yellow","created":"2026-10-14T12:00:00Z"},
              {"reference":"John 3:17","book":"JHN","chapter":3,"verse":17,"text":"For God sent","color":"green","created":"2026-10-14T12:00:00Z"}
            ]}"#;
        let mut s = StudyStore::new();
        assert_eq!(s.import_json(json).unwrap(), (2, 0, 0));
        assert_eq!(s.highlight_parts(jn3(16)), &[Highlight::whole(HighlightColor::Yellow, 1_791_979_200)]);
        assert!(s.highlight_parts(jn3(17))[0].is_whole());
        // Importing the file again doesn't undo what was coloured here since.
        s.highlight_span(
            &TextSpan { first: jn3(16), start: 4, last: jn3(16), end: 7 },
            Some(HighlightColor::Blue),
            text,
        )
        .unwrap();
        let after = shown(&s, jn3(16));
        assert_eq!(after.len(), 3);
        assert_eq!(s.import_json(json).unwrap(), (2, 0, 0));
        assert_eq!(shown(&s, jn3(16)), after);
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_791_158_400), "2026-10-05T00:00:00Z");
        for t in [0, 951_782_400, 1_791_158_399, 4_102_444_800] {
            assert_eq!(parse_rfc3339(&rfc3339(t)), Some(t));
        }
    }
}

#[cfg(test)]
mod sync_tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    /// A store on device `device` whose clock reads what the returned handle says (Unix ms).
    fn device(device: &str, start_ms: i64) -> (StudyStore, Arc<AtomicI64>) {
        let mut s = StudyStore::open_in_memory().unwrap();
        s.device = device.to_string();
        let now = Arc::new(AtomicI64::new(start_ms));
        let clock = now.clone();
        s.set_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
        (s, now)
    }

    const T0: i64 = 1_790_000_000_000;

    fn v(n: u8) -> VerseRef {
        VerseRef::new(BookId(42), 3, n)
    }

    fn later(clock: &AtomicI64, ms: i64) {
        clock.fetch_add(ms, Ordering::SeqCst);
    }

    /// Everything `from` has (or changed after `since`) taken in by `to`.
    fn sync(from: &StudyStore, to: &mut StudyStore) -> MergeReport {
        to.merge(&from.changes_since(0, false), MergeMode::Sync).unwrap()
    }

    fn body(s: &StudyStore, n: u8) -> Option<&str> {
        s.note(v(n)).map(|n| n.body.as_str())
    }

    #[test]
    fn notes_written_in_the_same_millisecond_agree_after_syncing() {
        let (mut a, _) = device("aaaa", T0);
        let (mut b, _) = device("bbbb", T0);
        a.set_note(v(16), "from a").unwrap();
        b.set_note(v(16), "from b").unwrap();
        assert_eq!(a.note_version(v(16)).stamp.at, b.note_version(v(16)).stamp.at);
        let (a0, b0) = (a.changes_since(0, false), b.changes_since(0, false));
        assert_eq!(a.merge(&b0, MergeMode::Sync).unwrap().conflicts, 1);
        assert_eq!(b.merge(&a0, MergeMode::Sync).unwrap().conflicts, 1);
        // bbbb's id is the larger, so its text is the note on both, and a's the copy.
        assert_eq!((body(&a, 16), body(&b, 16)), (Some("from b"), Some("from b")));
        for s in [&a, &b] {
            let copies: Vec<&str> = s.note_conflicts_at(v(16)).iter().map(|c| c.body.as_str()).collect();
            assert_eq!(copies, ["from a"]);
        }
        assert_eq!(a.note_version(v(16)), b.note_version(v(16)));
    }

    #[test]
    fn a_later_edit_keeps_conflict_copies_its_writer_never_saw() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        let (mut c, clock_c) = device("cccc", T0);
        a.set_note(v(16), "base").unwrap();
        sync(&a, &mut b);
        sync(&a, &mut c);
        // b edits first, a later without knowing: a's is the note, b's the copy, on a.
        later(&clock_b, 10);
        b.set_note(v(16), "from b").unwrap();
        later(&clock_a, 20);
        a.set_note(v(16), "from a").unwrap();
        sync(&a, &mut c);
        assert_eq!(sync(&b, &mut a).conflicts, 1);
        // c edits a's note, never having seen b's: b's text stays a copy on a.
        later(&clock_c, 30);
        c.set_note(v(16), "from c").unwrap();
        sync(&c, &mut a);
        assert_eq!(body(&a, 16), Some("from c"));
        let copies = |s: &StudyStore| -> Vec<String> {
            s.note_conflicts_at(v(16)).iter().map(|k| k.body.clone()).collect()
        };
        assert_eq!(copies(&a), ["from b"]);
        // Dismissed on a, it goes everywhere, even where it arrived late.
        sync(&b, &mut c);
        assert_eq!(copies(&c), ["from b"]);
        let stamp = a.note_conflicts_at(v(16))[0].stamp.clone();
        assert!(a.dismiss_conflict(v(16), &stamp).unwrap());
        sync(&a, &mut c);
        sync(&a, &mut b);
        for s in [&a, &b, &c] {
            assert_eq!((body(s, 16), copies(s).len()), (Some("from c"), 0));
        }
    }

    #[test]
    fn a_conflict_copy_dismissed_after_arriving_stays_dismissed() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        let (mut c, clock_c) = device("cccc", T0);
        a.set_note(v(3), "base").unwrap();
        sync(&a, &mut b);
        // b edits first, a later without knowing; c has a's edit before b's text reaches a.
        later(&clock_b, 10);
        b.set_note(v(3), "from b").unwrap();
        later(&clock_a, 20);
        a.set_note(v(3), "from a").unwrap();
        sync(&a, &mut c);
        assert_eq!(sync(&b, &mut a).conflicts, 1);
        // c edits on, then hears of b's copy: it is reported and counted as a change since.
        later(&clock_c, 30);
        c.set_note(v(3), "from c").unwrap();
        let since = c.seq();
        let generation = c.generation();
        assert_eq!(sync(&a, &mut c).notes, 1);
        assert!(c.generation() > generation);
        let news = c.changes_since(since, false);
        assert_eq!(news.notes.len(), 1);
        assert_eq!(news.notes[0].conflicts.len(), 1);
        // Dismissed on c, it leaves a, and a's older copy of the note doesn't bring it back.
        let copies = |s: &StudyStore| s.note_conflicts_at(v(3)).len();
        let stamp = c.note_conflicts_at(v(3))[0].stamp.clone();
        assert!(c.dismiss_conflict(v(3), &stamp).unwrap());
        let a_before = a.changes_since(0, false);
        sync(&c, &mut a);
        c.merge(&a_before, MergeMode::Sync).unwrap();
        sync(&a, &mut c);
        sync(&b, &mut c);
        sync(&c, &mut b);
        for s in [&a, &b, &c] {
            assert_eq!((body(s, 3), copies(s)), (Some("from c"), 0));
        }
    }

    #[test]
    fn an_editor_saving_over_a_synced_note_keeps_it_as_a_copy() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        a.set_note(v(5), "base").unwrap();
        sync(&a, &mut b);
        // b's editor opens on "base"; a's edit arrives while it is open.
        later(&clock_a, 10);
        a.set_note(v(5), "from a").unwrap();
        sync(&a, &mut b);
        later(&clock_b, 20);
        assert!(b.set_note_over(v(5), "typed on b", "base").unwrap());
        let copies: Vec<_> = b.note_conflicts_at(v(5)).iter().map(|k| k.body.clone()).collect();
        assert_eq!((body(&b, 5), copies), (Some("typed on b"), vec!["from a".to_string()]));
        // Saving again over its own text keeps nothing more.
        assert!(!b.set_note_over(v(5), "typed more", "typed on b").unwrap());
        assert_eq!(b.note_conflicts_at(v(5)).len(), 1);
        sync(&b, &mut a);
        assert_eq!((body(&a, 5), a.note_conflicts_at(v(5)).len()), (Some("typed more"), 1));
    }

    #[test]
    fn reopening_ignores_stamps_from_far_ahead() {
        let dir = std::env::temp_dir().join(format!(
            "bible-ahead-{}-{}",
            std::process::id(),
            sync::new_device_id()
        ));
        let path = dir.join("study.sqlite");
        let year = 365 * 24 * 3600 * 1000;
        {
            let mut s = StudyStore::open(&path).unwrap();
            let (mut wrong, _) = device("wrong-year", sync::now_ms() + year);
            wrong.set_note(v(1), "next year").unwrap();
            s.merge(&wrong.changes_since(0, false), MergeMode::Import).unwrap();
            assert_eq!(body(&s, 1), Some("next year"));
        }
        let mut s = StudyStore::open(&path).unwrap();
        s.set_note(v(2), "today").unwrap();
        assert!(s.note_version(v(2)).stamp.at < sync::now_ms() + MAX_DRIFT_MS);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_a_stopped_restart_keeps_progress_here() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        a.start_plan("john-21", 1_790_000_000).unwrap();
        later(&clock_a, 10);
        a.set_plan_read("john-21", 1, 0, true).unwrap();
        a.set_plan_read("john-21", 2, 0, true).unwrap();
        // The file's copy restarted the plan later, then stopped it.
        later(&clock_b, 100);
        b.start_plan("john-21", 1_790_000_000).unwrap();
        b.stop_plan("john-21").unwrap();
        a.merge(&b.changes_since(0, false), MergeMode::Import).unwrap();
        assert!(a.plans().iter().any(|p| p.id == "john-21"));
        assert_eq!(a.plan_reads("john-21").len(), 2);
        // Syncing, the later stop wins.
        sync(&b, &mut a);
        assert!(a.plans().is_empty());
    }

    #[test]
    fn the_sequence_survives_pruning_its_newest_tombstone() {
        let dir =
            std::env::temp_dir().join(format!("bible-seq-{}-{}", std::process::id(), sync::new_device_id()));
        let path = dir.join("study.sqlite");
        {
            let mut s = StudyStore::open(&path).unwrap();
            s.set_note(v(1), "one").unwrap();
            s.set_note(v(1), "").unwrap();
            assert_eq!(s.seq(), 2);
            assert_eq!(s.prune_tombstones(i64::MAX).unwrap(), 1);
            assert_eq!(s.seq(), 2);
        }
        // What's written after counts on from 2, so a reader that got to 2 doesn't miss it.
        let s = StudyStore::open(&path).unwrap();
        assert_eq!(s.seq(), 2);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_changes_are_stamped_and_counted() {
        let (mut a, clock) = device("aaaa", T0);
        assert_eq!(a.seq(), 0);
        a.set_note(v(16), "love").unwrap();
        a.set_bookmark(v(16), Some("")).unwrap();
        a.set_highlight(v(16), Some(HighlightColor::Green)).unwrap();
        assert_eq!(a.seq(), 3);
        let c = a.changes_since(0, true);
        assert_eq!((c.notes.len(), c.bookmarks.len(), c.highlights.len()), (1, 1, 1));
        assert_eq!(c.notes[0].stamp, Stamp::new(T0, "aaaa"));
        // Same wall time: the clock still moves forward.
        assert_eq!(c.bookmarks[0].stamp, Stamp::new(T0 + 1, "aaaa"));
        assert_eq!(c.highlights[0].stamp, Stamp::new(T0 + 2, "aaaa"));
        // A change that changes nothing is no change.
        a.set_note(v(16), "love").unwrap();
        a.set_bookmark(v(17), None).unwrap();
        a.set_highlight(v(18), None).unwrap();
        assert_eq!(a.seq(), 3);
        later(&clock, 1000);
        a.set_note(v(16), "").unwrap();
        let c = a.changes_since(3, true);
        assert_eq!(c.len(), 1);
        assert!(c.notes[0].deleted);
        assert_eq!(c.notes[0].base_updated, T0);
    }

    #[test]
    fn two_devices_converge() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0 + 5);
        a.set_note(v(16), "from a").unwrap();
        a.set_bookmark(v(16), Some("start")).unwrap();
        a.set_highlight(v(17), Some(HighlightColor::Blue)).unwrap();
        b.set_note(v(18), "from b").unwrap();
        let r = sync(&a, &mut b);
        assert_eq!((r.notes, r.bookmarks, r.highlights, r.conflicts), (1, 1, 1, 0));
        sync(&b, &mut a);
        for s in [&a, &b] {
            assert_eq!(body(s, 16), Some("from a"));
            assert_eq!(body(s, 18), Some("from b"));
            assert_eq!(s.bookmark(v(16)).unwrap().label, "start");
            assert_eq!(s.highlight(v(17)), Some(HighlightColor::Blue));
        }
        // Taking the same changes in again changes nothing.
        assert_eq!(sync(&a, &mut b).changed(), 0);
        assert_eq!(sync(&b, &mut a).changed(), 0);
        // Deletes travel too.
        later(&cb, 1000);
        b.set_bookmark(v(16), None).unwrap();
        b.set_highlight(v(17), None).unwrap();
        sync(&b, &mut a);
        assert!(!a.is_bookmarked(v(16)));
        assert_eq!(a.highlight(v(17)), None);
        // An older edit doesn't bring back what was deleted after it.
        let mut stale = StudyStore::new();
        stale.device = "cccc".into();
        stale.set_clock(Arc::new(|| T0 + 10));
        stale.set_bookmark(v(16), Some("old")).unwrap();
        sync(&stale, &mut a);
        assert!(!a.is_bookmarked(v(16)));
        // A newer one does.
        later(&ca, 5000);
        let mut fresh = StudyStore::new();
        fresh.device = "cccc".into();
        fresh.set_clock(Arc::new(|| T0 + 10_000));
        fresh.set_bookmark(v(16), Some("again")).unwrap();
        sync(&fresh, &mut a);
        assert_eq!(a.bookmark(v(16)).unwrap().label, "again");
    }

    #[test]
    fn equal_times_break_ties_by_device() {
        let (mut a, _) = device("aaaa", T0);
        let (mut b, _) = device("bbbb", T0);
        a.set_highlight(v(16), Some(HighlightColor::Yellow)).unwrap();
        b.set_highlight(v(16), Some(HighlightColor::Pink)).unwrap();
        let (ca, cb) = (a.changes_since(0, false), b.changes_since(0, false));
        a.merge(&cb, MergeMode::Sync).unwrap();
        b.merge(&ca, MergeMode::Sync).unwrap();
        assert_eq!(a.highlight(v(16)), Some(HighlightColor::Pink));
        assert_eq!(b.highlight(v(16)), Some(HighlightColor::Pink));
    }

    #[test]
    fn a_verse_highlights_merge_as_a_whole() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        let span = |s, e| TextSpan { first: v(16), start: s, last: v(16), end: e };
        let text = |_| Some("For God so loved the world.".to_string());
        a.highlight_span(&span(0, 3), Some(HighlightColor::Yellow), text).unwrap();
        sync(&a, &mut b);
        later(&ca, 10);
        a.highlight_span(&span(4, 7), Some(HighlightColor::Blue), text).unwrap();
        later(&cb, 20);
        b.highlight_span(&span(8, 10), Some(HighlightColor::Green), text).unwrap();
        sync(&a, &mut b);
        sync(&b, &mut a);
        // The later edit of the verse wins whole: no mix of the two.
        assert_eq!(a.highlight_parts(v(16)), b.highlight_parts(v(16)));
        let colors: Vec<_> = a.highlight_parts(v(16)).iter().map(|h| h.color).collect();
        assert_eq!(colors, vec![HighlightColor::Yellow, HighlightColor::Green]);
    }

    #[test]
    fn notes_edited_on_both_sides_keep_a_conflict_copy() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.set_note(v(16), "first").unwrap();
        sync(&a, &mut b);
        later(&ca, 100);
        a.set_note(v(16), "a's edit").unwrap();
        later(&cb, 200);
        b.set_note(v(16), "b's edit").unwrap();
        let r = sync(&a, &mut b);
        assert_eq!((r.notes, r.conflicts), (0, 1));
        // A takes B's version, and the copy B made of A's with it.
        let r = sync(&b, &mut a);
        assert_eq!((r.notes, r.conflicts), (1, 0));
        for s in [&a, &b] {
            assert_eq!(body(s, 16), Some("b's edit"));
            let copies = s.note_conflicts_at(v(16));
            assert_eq!(copies.len(), 1);
            assert_eq!(copies[0].body, "a's edit");
            assert_eq!(copies[0].stamp, Stamp::new(T0 + 100, "aaaa"));
        }
        // Once merged, both know of both versions: no new conflicts.
        assert_eq!(sync(&a, &mut b).conflicts, 0);
        assert_eq!(sync(&b, &mut a).conflicts, 0);
        // An edit after the merge simply wins.
        later(&ca, 1000);
        a.set_note(v(16), "settled").unwrap();
        let r = sync(&a, &mut b);
        assert_eq!((r.notes, r.conflicts), (1, 0));
        assert_eq!(body(&b, 16), Some("settled"));
    }

    #[test]
    fn the_same_text_written_twice_is_no_conflict() {
        let (mut a, _) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.set_note(v(16), "same").unwrap();
        later(&cb, 10);
        b.set_note(v(16), "same").unwrap();
        assert_eq!(sync(&a, &mut b).conflicts, 0);
        assert_eq!(sync(&b, &mut a).conflicts, 0);
        assert_eq!(a.conflict_count() + b.conflict_count(), 0);
    }

    #[test]
    fn a_delete_against_an_edit_keeps_the_edit_as_a_copy() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.set_note(v(16), "draft").unwrap();
        sync(&a, &mut b);
        later(&cb, 10);
        b.set_note(v(16), "b kept writing").unwrap();
        later(&ca, 20);
        a.set_note(v(16), "").unwrap();
        sync(&a, &mut b);
        sync(&b, &mut a);
        for s in [&a, &b] {
            assert_eq!(body(s, 16), None);
            assert_eq!(s.note_conflicts_at(v(16))[0].body, "b kept writing");
        }
        // The reader takes it back.
        let stamp = b.note_conflicts_at(v(16))[0].stamp.clone();
        later(&cb, 1000);
        assert!(b.use_conflict(v(16), &stamp).unwrap());
        assert_eq!(body(&b, 16), Some("b kept writing"));
        assert_eq!(b.conflict_count(), 0);
        // Taking it back is an edit of the note: the copy goes everywhere.
        sync(&b, &mut a);
        assert_eq!(body(&a, 16), Some("b kept writing"));
        assert_eq!(a.conflict_count(), 0);
        assert!(!a.dismiss_conflict(v(16), &stamp).unwrap());
    }

    #[test]
    fn using_a_copy_keeps_the_note_it_replaces() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.set_note(v(16), "base").unwrap();
        sync(&a, &mut b);
        later(&ca, 10);
        a.set_note(v(16), "a").unwrap();
        later(&cb, 20);
        b.set_note(v(16), "b").unwrap();
        sync(&b, &mut a);
        let stamp = a.note_conflicts_at(v(16))[0].stamp.clone();
        a.use_conflict(v(16), &stamp).unwrap();
        assert_eq!(body(&a, 16), Some("a"));
        let copies: Vec<_> = a.note_conflicts_at(v(16)).iter().map(|c| c.body.clone()).collect();
        assert_eq!(copies, vec!["b".to_string()]);
    }

    #[test]
    fn imports_never_delete_and_bring_back_what_was_deleted() {
        let (mut a, ca) = device("aaaa", T0);
        a.set_note(v(16), "keep me").unwrap();
        a.set_bookmark(v(17), Some("mark")).unwrap();
        let file = a.export_json("KJV", |_| None).unwrap();
        later(&ca, 1000);
        a.set_note(v(16), "").unwrap();
        a.set_bookmark(v(17), None).unwrap();
        a.set_note(v(18), "gone later").unwrap();
        later(&ca, 1000);
        a.set_note(v(18), "").unwrap();
        let with_tombstones = a.export_json("KJV", |_| None).unwrap();
        assert!(with_tombstones.contains("\"deleted\""));
        // The deleted things come back, as changes made here now.
        assert_eq!(a.import_json(&file).unwrap(), (0, 1, 1));
        assert_eq!(body(&a, 16), Some("keep me"));
        assert_eq!(a.bookmark(v(17)).unwrap().label, "mark");
        let c = a.changes_since(0, true);
        assert!(c.notes.iter().any(|n| n.verse.verse == 16 && n.stamp.at > T0 + 2000));
        // A file's tombstones delete nothing on import...
        let (mut b, _) = device("bbbb", T0 + 5000);
        b.set_note(v(18), "b's").unwrap();
        b.import_json(&with_tombstones).unwrap();
        assert_eq!(body(&b, 18), Some("b's"));
        // ...but restoring a copy of the study does take its deletes.
        let (mut c, _) = device("cccc", T0);
        c.restore_json(&file).unwrap();
        assert_eq!(body(&c, 16), Some("keep me"));
        c.restore_json(&with_tombstones).unwrap();
        assert_eq!(body(&c, 16), None);
        assert!(!c.is_bookmarked(v(17)));
    }

    #[test]
    fn importing_an_older_note_keeps_both_versions() {
        let (mut a, ca) = device("aaaa", T0);
        a.set_note(v(16), "old text").unwrap();
        let file = a.export_json("KJV", |_| None).unwrap();
        let (mut b, _) = device("bbbb", T0 + 500);
        b.set_note(v(16), "b's text").unwrap();
        b.import_json(&file).unwrap();
        assert_eq!(body(&b, 16), Some("b's text"));
        assert_eq!(b.note_conflicts_at(v(16))[0].body, "old text");
        // A's own later edit of the note it exported is no conflict.
        later(&ca, 100);
        a.set_note(v(16), "new text").unwrap();
        a.import_json(&file).unwrap();
        assert_eq!(body(&a, 16), Some("new text"));
        assert_eq!(a.conflict_count(), 0);
    }

    #[test]
    fn format_1_exports_still_import() {
        let json = r#"{"app":"bible-rs","format":1,"exported_at":"2026-10-14T12:00:00Z","translation":"KJV",
            "highlights":[{"reference":"John 3:16","book":"JHN","chapter":3,"verse":16,"color":"yellow","created":"2026-10-14T12:00:00Z"}],
            "notes":[{"reference":"John 3:16","book":"JHN","chapter":3,"verse":16,"note":"n","created":"2026-10-14T12:00:00Z","updated":"2026-10-15T12:00:00Z"}],
            "bookmarks":[{"reference":"John 3:17","book":"JHN","chapter":3,"verse":17,"label":"b","created":"2026-10-14T12:00:00Z"}],
            "plans":[{"id":"nt","started":"2026-10-01T00:00:00Z","read":[[1,0],[2,0]]}]}"#;
        let (mut s, _) = device("aaaa", T0);
        assert_eq!(s.import_json(json).unwrap(), (1, 1, 1));
        assert_eq!(body(&s, 16), Some("n"));
        assert_eq!(s.note_version(v(16)).stamp, Stamp::new(1_792_065_600_000, ""));
        assert_eq!(s.plan_reads("nt").len(), 2);
        // What it writes, an app that only knows format 1 reads (it ignores the rest).
        let out = s.export_json_with("KJV", |_| None, Include::EVERYTHING).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["format"], 2);
        assert_eq!(doc["plans"][0]["read"].as_array().unwrap().len(), 2);
        assert_eq!(doc["notes"][0]["note"], "n");
        let (mut t, _) = device("bbbb", T0);
        t.import_json(&out).unwrap();
        assert_eq!(t.note_version(v(16)), s.note_version(v(16)));
        assert_eq!(t.plan_reads("nt"), s.plan_reads("nt"));
    }

    #[test]
    fn restarting_a_plan_forgets_progress_everywhere() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.start_plan("nt", 1_790_000_000).unwrap();
        a.set_plan_read("nt", 1, 0, true).unwrap();
        sync(&a, &mut b);
        later(&cb, 10);
        b.set_plan_read("nt", 2, 0, true).unwrap();
        let before_restart = b.changes_since(0, false);
        later(&ca, 100);
        a.start_plan("nt", 1_790_086_400).unwrap();
        a.set_plan_read("nt", 1, 1, true).unwrap();
        sync(&a, &mut b);
        // B's tick from the earlier run arrives late: it stays forgotten.
        a.merge(&before_restart, MergeMode::Sync).unwrap();
        for s in [&a, &b] {
            assert_eq!(s.plans()[0].started, 1_790_086_400);
            assert_eq!(s.plan_reads("nt"), HashSet::from([(1, 1)]));
        }
        // Stopping travels too.
        later(&ca, 100);
        a.stop_plan("nt").unwrap();
        sync(&a, &mut b);
        assert!(b.plans().is_empty());
        assert!(b.plan_reads("nt").is_empty());
    }

    #[test]
    fn a_plan_restarted_elsewhere_resets_even_when_finished_here_later() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, cb) = device("bbbb", T0);
        a.start_plan("nt", 1).unwrap();
        a.set_plan_read("nt", 1, 0, true).unwrap();
        sync(&a, &mut b);
        later(&ca, 10);
        a.start_plan("nt", 2).unwrap();
        later(&cb, 20);
        b.finish_plan("nt", Some(3)).unwrap();
        sync(&a, &mut b);
        sync(&b, &mut a);
        for s in [&a, &b] {
            assert_eq!(s.plans()[0].finished, Some(3));
            assert!(s.plan_reads("nt").is_empty());
        }
    }

    #[test]
    fn changes_since_counts_from_a_point_and_tells_mine_from_theirs() {
        let (mut a, _) = device("aaaa", T0);
        let (mut b, _) = device("bbbb", T0);
        a.set_note(v(16), "a").unwrap();
        let mark = a.seq();
        b.set_note(v(17), "b").unwrap();
        sync(&b, &mut a);
        a.set_bookmark(v(18), Some("")).unwrap();
        let since = a.changes_since(mark, false);
        assert_eq!((since.notes.len(), since.bookmarks.len()), (1, 1));
        let mine = a.changes_since(mark, true);
        assert_eq!((mine.notes.len(), mine.bookmarks.len()), (0, 1));
        assert_eq!(a.changes_since(a.seq(), false).len(), 0);
    }

    #[test]
    fn far_future_stamps_dont_drag_the_clock() {
        let (mut a, _) = device("aaaa", T0);
        let mut wrong = StudyStore::new();
        wrong.device = "ffff".into();
        let year = 365 * 86_400_000;
        wrong.set_clock(Arc::new(move || T0 + year));
        wrong.set_note(v(16), "from the future").unwrap();
        sync(&wrong, &mut a);
        assert_eq!(body(&a, 16), Some("from the future"));
        a.set_note(v(17), "now").unwrap();
        assert!(a.note_version(v(17)).stamp.at < T0 + MAX_DRIFT_MS);
        // A day or so ahead is believed.
        let mut ahead = StudyStore::new();
        ahead.device = "eeee".into();
        ahead.set_clock(Arc::new(|| T0 + 3_600_000));
        ahead.set_note(v(18), "an hour ahead").unwrap();
        sync(&ahead, &mut a);
        a.set_note(v(17), "later").unwrap();
        assert!(a.note_version(v(17)).stamp.at > T0 + 3_600_000);
    }

    #[test]
    fn tombstones_are_pruned() {
        let (mut a, ca) = device("aaaa", T0);
        a.set_note(v(16), "x").unwrap();
        a.set_bookmark(v(17), Some("")).unwrap();
        a.set_highlight(v(18), Some(HighlightColor::Blue)).unwrap();
        a.start_plan("nt", 0).unwrap();
        a.set_plan_read("nt", 1, 0, true).unwrap();
        later(&ca, 1000);
        a.set_note(v(16), "").unwrap();
        a.set_bookmark(v(17), None).unwrap();
        a.set_highlight(v(18), None).unwrap();
        a.stop_plan("nt").unwrap();
        a.set_note(v(19), "live").unwrap();
        assert_eq!(a.changes_since(0, false).len(), 6);
        assert_eq!(a.prune_tombstones(T0).unwrap(), 0);
        assert_eq!(a.prune_tombstones(T0 + 2000).unwrap(), 5);
        let left = a.changes_since(0, false);
        assert_eq!(left.len(), 1);
        assert_eq!(left.notes[0].body, "live");
    }

    #[test]
    fn erasing_everything_syncs() {
        let (mut a, ca) = device("aaaa", T0);
        let (mut b, _) = device("bbbb", T0);
        a.set_note(v(16), "x").unwrap();
        a.set_bookmark(v(17), Some("")).unwrap();
        a.set_highlight(v(18), Some(HighlightColor::Blue)).unwrap();
        a.start_plan("nt", 0).unwrap();
        sync(&a, &mut b);
        later(&ca, 1000);
        a.erase_all().unwrap();
        assert_eq!(a.counts(), (0, 0, 0));
        sync(&a, &mut b);
        assert_eq!(b.counts(), (0, 0, 0));
        assert!(b.plans().is_empty());
    }

    #[test]
    fn unknown_books_are_skipped() {
        let (mut a, _) = device("aaaa", T0);
        let mut c = Changes::default();
        c.notes.push(NoteChange {
            verse: VerseKey { book: "XXX".into(), chapter: 1, verse: 1 },
            stamp: Stamp::new(T0, "bbbb"),
            deleted: false,
            body: "?".into(),
            created: 1,
            updated: 1,
            base_updated: 0,
            base_device: String::new(),
            seen: Vec::new(),
            conflicts: Vec::new(),
        });
        assert_eq!(a.merge(&c, MergeMode::Sync).unwrap().changed(), 0);
    }

    #[test]
    fn sync_state_survives_reopening() {
        let dir = std::env::temp_dir().join(format!("bible-rs-sync-reopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("study.sqlite");
        let (mut b, cb) = device("bbbb", T0);
        let device;
        {
            let mut a = StudyStore::open(&path).unwrap();
            a.set_clock(Arc::new(|| T0));
            device = a.device_id().to_string();
            a.set_note(v(16), "base").unwrap();
            a.set_highlight(v(17), Some(HighlightColor::Pink)).unwrap();
            a.set_highlight(v(18), Some(HighlightColor::Pink)).unwrap();
            a.set_highlight(v(18), None).unwrap();
            a.start_plan("nt", 0).unwrap();
            a.set_plan_read("nt", 1, 0, true).unwrap();
            sync(&a, &mut b);
            later(&cb, 10);
            b.set_note(v(16), "b").unwrap();
            a.set_clock(Arc::new(|| T0 + 5));
            a.set_note(v(16), "a").unwrap();
            sync(&b, &mut a);
            assert_eq!(a.conflict_count(), 1);
        }
        let a = StudyStore::open(&path).unwrap();
        assert_eq!(a.device_id(), device);
        assert!(!StudyStore::pre_sync_backup(&path).exists(), "a new file needs no backup");
        assert_eq!(body(&a, 16), Some("b"));
        assert_eq!(a.conflict_count(), 1);
        sync(&a, &mut b);
        let (here, there) = (a.changes_since(0, false), b.changes_since(0, false));
        assert_eq!(here.highlights, there.highlights);
        assert_eq!(here.plans, there.plans);
        assert_eq!(here.plan_reads, there.plan_reads);
        assert_eq!(here.notes[0].conflicts, there.notes[0].conflicts);
        assert!(a.hlc.last() >= T0 + 10);
        assert_eq!(a.note_version(v(16)).stamp, b.note_version(v(16)).stamp);
        drop(a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrating_to_sync_backs_up_and_stamps_what_is_there() {
        let dir = std::env::temp_dir().join(format!("bible-rs-sync-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("study.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE highlights(
                     book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                     char_start INTEGER NOT NULL DEFAULT 0, char_end INTEGER, quote TEXT,
                     color INTEGER NOT NULL, created INTEGER NOT NULL, edited INTEGER,
                     PRIMARY KEY(book, chapter, verse, char_start));
                 CREATE TABLE notes(
                     book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                     body TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL,
                     PRIMARY KEY(book, chapter, verse));
                 CREATE TABLE bookmarks(
                     book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                     label TEXT NOT NULL, created INTEGER NOT NULL, PRIMARY KEY(book, chapter, verse));
                 CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE plans(id TEXT PRIMARY KEY, started INTEGER NOT NULL, finished INTEGER);
                 CREATE TABLE plan_reads(
                     plan TEXT NOT NULL, day INTEGER NOT NULL, passage INTEGER NOT NULL, at INTEGER NOT NULL,
                     PRIMARY KEY(plan, day, passage));
                 INSERT INTO highlights VALUES('JHN', 3, 16, 0, NULL, NULL, 1, 100, 300);
                 INSERT INTO notes VALUES('JHN', 3, 16, 'kept', 100, 200);
                 INSERT INTO bookmarks VALUES('JHN', 3, 17, 'b', 400);
                 INSERT INTO plans VALUES('nt', 500, 900);
                 INSERT INTO plan_reads VALUES('nt', 1, 0, 600);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let s = StudyStore::open(&path).unwrap();
        let device = s.device_id().to_string();
        assert_eq!(device.len(), 16);
        let c = s.changes_since(0, false);
        assert_eq!(c.highlights[0].stamp, Stamp::new(300_000, device.clone()));
        assert_eq!(c.notes[0].stamp, Stamp::new(200_000, device.clone()));
        assert_eq!(c.bookmarks[0].stamp, Stamp::new(400_000, device.clone()));
        assert_eq!(c.plans[0].stamp, Stamp::new(900_000, device.clone()));
        assert_eq!(c.plan_reads[0].stamp, Stamp::new(600_000, device.clone()));
        assert!(c.highlights.iter().chain([].iter()).all(|h| !h.deleted));
        assert_eq!(s.note(v(16)).unwrap().body, "kept");
        assert_eq!(s.plan_reads("nt"), HashSet::from([(1, 0)]));
        drop(s);
        // The file as it was is kept beside it, readable by the version before.
        let backup = StudyStore::pre_sync_backup(&path);
        assert_eq!(backup, dir.join("study.sqlite.pre-sync"));
        let old = Connection::open(&backup).unwrap();
        let version: i64 = old.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 1);
        let body: String = old.query_row("SELECT body FROM notes", [], |r| r.get(0)).unwrap();
        assert_eq!(body, "kept");
        assert!(!has_column(&old, "notes", "updated_ms").unwrap());
        drop(old);
        // Opening again neither re-stamps nor makes a new backup.
        std::fs::write(&backup, b"untouched").unwrap();
        let s = StudyStore::open(&path).unwrap();
        assert_eq!(s.device_id(), device);
        assert_eq!(s.changes_since(0, false).notes[0].stamp, Stamp::new(200_000, device));
        assert_eq!(std::fs::read(&backup).unwrap(), b"untouched");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memorised_passages_sync_between_devices() {
        let (mut a, clock) = device("aaaa", T0);
        let (mut b, _) = device("bbbb", T0);
        a.memorise(v(16), v(17), "kjv").unwrap();
        assert_eq!(sync(&a, &mut b).memory, 1);
        let m = b.memorised_at(v(16)).cloned().unwrap();
        assert_eq!((m.first, m.last, m.translation.as_str()), (v(16), v(17), "kjv"));
        later(&clock, 10);
        a.review(v(16), Grade::Good, a.now_s()).unwrap();
        assert_eq!(sync(&a, &mut b).memory_reviews, 1);
        assert_eq!(b.reviews(v(16)).len(), 1);
        assert_eq!(b.memorised_at(v(16)).unwrap().schedule, a.memorised_at(v(16)).unwrap().schedule);
        later(&clock, 10);
        a.forget(v(16)).unwrap();
        sync(&a, &mut b);
        assert!(b.memorised_at(v(16)).is_none());
        assert!(b.reviews(v(16)).is_empty());
    }

    #[test]
    fn a_newer_forget_drops_a_concurrent_review() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        a.memorise(v(16), v(16), "kjv").unwrap();
        sync(&a, &mut b);
        later(&clock_b, 10);
        b.review(v(16), Grade::Good, b.now_s()).unwrap();
        later(&clock_a, 20);
        a.forget(v(16)).unwrap();
        sync(&b, &mut a);
        sync(&a, &mut b);
        for s in [&a, &b] {
            assert!(s.memorised_at(v(16)).is_none());
            assert!(s.reviews(v(16)).is_empty(), "a review must not outlive its forgotten passage");
        }
    }

    #[test]
    fn concurrent_reviews_rebuild_the_same_schedule() {
        let (mut a, clock_a) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        a.memorise(v(16), v(16), "web").unwrap();
        sync(&a, &mut b);
        let created = a.memorised_at(v(16)).unwrap().created;
        later(&clock_a, 10_000);
        a.review(v(16), Grade::Good, a.now_s()).unwrap();
        later(&clock_b, 20_000);
        b.review(v(16), Grade::Hard, b.now_s()).unwrap();
        sync(&a, &mut b);
        sync(&b, &mut a);
        let reviews_a = a.reviews(v(16)).to_vec();
        let reviews_b = b.reviews(v(16)).to_vec();
        assert_eq!(reviews_a, reviews_b);
        assert_eq!(reviews_a.len(), 2);
        let expected = schedule_from_reviews(created, &reviews_a);
        assert_eq!(a.memorised_at(v(16)).unwrap().schedule, expected);
        assert_eq!(b.memorised_at(v(16)).unwrap().schedule, expected);
    }

    #[test]
    fn overlapping_passages_keep_the_newer_one() {
        let (mut a, _) = device("aaaa", T0);
        let (mut b, clock_b) = device("bbbb", T0);
        a.memorise(v(16), v(18), "kjv").unwrap();
        later(&clock_b, 10);
        b.memorise(v(17), v(19), "kjv").unwrap();
        sync(&a, &mut b);
        sync(&b, &mut a);
        for s in [&a, &b] {
            assert!(s.memorised_at(v(16)).is_none(), "the older overlapping passage is gone");
            let m = s.memorised_at(v(17)).cloned().unwrap();
            assert_eq!((m.first, m.last), (v(17), v(19)));
        }
    }

    #[test]
    fn a_forgotten_passage_stays_gone_after_reopen() {
        let dir = std::env::temp_dir().join(format!(
            "bible-memory-tombstone-{}-{}",
            std::process::id(),
            crate::sync::new_device_id()
        ));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = StudyStore::open(&path).unwrap();
            s.memorise(v(16), v(17), "kjv").unwrap();
            s.review(v(16), Grade::Good, s.now_s()).unwrap();
            s.forget(v(16)).unwrap();
            assert!(s.changes_since(0, false).memory.iter().any(|c| c.deleted));
        }
        let s = StudyStore::open(&path).unwrap();
        assert!(s.memorised().is_empty());
        assert!(s.reviews(v(16)).is_empty());
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn memorised_rows_from_before_sync_are_stamped() {
        let dir = std::env::temp_dir().join(format!(
            "bible-memory-presync-{}-{}",
            std::process::id(),
            crate::sync::new_device_id()
        ));
        let path = dir.join("study.sqlite");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE memory(
                    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL, last_verse INTEGER NOT NULL,
                    translation TEXT NOT NULL, created INTEGER NOT NULL,
                    due INTEGER NOT NULL, stability REAL NOT NULL, difficulty REAL NOT NULL,
                    reps INTEGER NOT NULL, lapses INTEGER NOT NULL, last_review INTEGER,
                    PRIMARY KEY(book, chapter, verse));
                 CREATE TABLE memory_reviews(
                    book TEXT NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
                    at INTEGER NOT NULL, grade INTEGER NOT NULL);
                 INSERT INTO memory(book, chapter, verse, last_verse, translation, created, due,
                     stability, difficulty, reps, lapses, last_review)
                 VALUES('JHN', 3, 16, 17, 'kjv', 100, 200, 1.5, 2.5, 1, 0, 150);
                 INSERT INTO memory_reviews(book, chapter, verse, at, grade) VALUES('JHN', 3, 16, 150, 3);",
            )
            .unwrap();
        }
        let s = StudyStore::open(&path).unwrap();
        let m = s.memorised_at(v(16)).cloned().unwrap();
        assert_eq!((m.first, m.last, m.translation.as_str(), m.schedule.reps), (v(16), v(17), "kjv", 1));
        assert_eq!(s.reviews(v(16)), [Review { at: 150, grade: Grade::Good }]);
        let c = s.changes_since(0, false);
        assert_eq!(c.memory.len(), 1);
        assert!(!c.memory[0].deleted);
        assert_eq!(c.memory_reviews.len(), 1);
        assert_eq!(c.memory[0].stamp.at, 150_000, "stamped from the newest of created and last_review");
        assert_eq!(c.memory_reviews[0].stamp.at, 150_000);
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
