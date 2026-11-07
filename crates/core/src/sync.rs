//! What two copies of a study need to agree: who wrote each record and when, and the rules that
//! pick one when they differ.
//!
//! Every highlighted verse, note, bookmark, plan, ticked plan passage and memorised passage
//! carries a [`Stamp`]: the
//! time it last changed (Unix milliseconds from a hybrid logical clock, so a device whose clock is
//! behind still moves forward) and the id of the device that changed it. Deleting keeps the record
//! as a tombstone with a newer stamp, so the delete travels like any other change.
//!
//! One merge decides between a record here and the same record from elsewhere (an imported
//! export, another device's changes in a study library):
//!
//! - The newer stamp wins, ties broken by device id ([`lww`]). A verse's highlights travel together.
//! - A tombstone wins over an older edit. Importing a file never deletes, and brings back what it
//!   has that was deleted here ([`MergeMode::Import`]).
//! - Notes keep both versions when they were edited on both sides since the version they last
//!   shared ([`merge_note`]): the newer one stays the note and the other becomes a conflict copy.
//!   Text is never dropped silently, even by a delete.

use serde::{Deserialize, Serialize};

/// When a record last changed (hybrid logical clock, Unix ms) and on which device. Ordered by
/// time, then device id, so every device picks the same winner.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    #[serde(rename = "updated_ms")]
    pub at: i64,
    #[serde(default)]
    pub device: String,
}

impl Stamp {
    pub fn new(at: i64, device: impl Into<String>) -> Self {
        Stamp { at, device: device.into() }
    }
}

/// A hybrid logical clock: the wall clock, but never behind (or equal to) a time it has already
/// given out or seen, so later changes always get later stamps even when clocks disagree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hlc {
    last: i64,
}

impl Hlc {
    /// The next stamp's time, for a change made now (`wall`, Unix ms).
    pub fn tick(&mut self, wall: i64) -> i64 {
        self.last = wall.max(self.last + 1);
        self.last
    }

    /// Takes in a time seen in a record from elsewhere.
    pub fn observe(&mut self, at: i64) {
        self.last = self.last.max(at);
    }

    pub fn last(&self) -> i64 {
        self.last
    }
}

/// Unix milliseconds.
pub fn now_ms() -> i64 {
    #[cfg(target_arch = "wasm32")]
    return js_sys::Date::now() as i64;
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A new device id: 16 random hex digits.
pub fn new_device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_i64(now_ms());
    #[cfg(not(target_arch = "wasm32"))]
    {
        h.write_u32(std::process::id());
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        h.write_u32(nanos);
    }
    // The browser's std has no random keys to hash with.
    #[cfg(target_arch = "wasm32")]
    h.write_u64((js_sys::Math::random() * u64::MAX as f64) as u64);
    format!("{:016x}", h.finish())
}

/// How a merge treats what comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// Another copy of the same study (a study library, a saved snapshot): newer wins, deletes
    /// included.
    Sync,
    /// A file the reader chose to bring in: it never deletes anything here, and what it has that
    /// was deleted here comes back.
    Import,
}

/// What wins between a record here and the same one from elsewhere, for records merged by stamp
/// alone (highlights of a verse, bookmarks, plans, ticked passages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Keep this side's.
    Local,
    /// Take the other side's, with its stamp.
    Remote,
    /// Take the other side's, but as a change made here now (an import bringing back something
    /// deleted here more recently than the file's copy).
    Revive,
}

/// Last writer wins. `local` is this side's stamp and whether it is live (not a tombstone), when
/// this side has the record at all.
pub fn lww(local: Option<(&Stamp, bool)>, remote: (&Stamp, bool), mode: MergeMode) -> Pick {
    let (r, r_live) = remote;
    if mode == MergeMode::Import && !r_live {
        return Pick::Local;
    }
    match local {
        None => Pick::Remote,
        Some((l, _)) if r > l => Pick::Remote,
        Some((_, false)) if mode == MergeMode::Import => Pick::Revive,
        Some(_) => Pick::Local,
    }
}

/// A note's version: its stamp, whether it is live, and the versions it supersedes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteVersion {
    pub stamp: Stamp,
    pub live: bool,
    /// The version this one was edited from (time 0 for a new note).
    pub base: Stamp,
    /// Older versions it also supersedes: the base's own history, and versions merged into it.
    /// Versions are told apart by their whole stamps: two devices can write in the same
    /// millisecond.
    pub seen: Vec<Stamp>,
}

/// How many superseded versions a note remembers.
pub const NOTE_HISTORY: usize = 32;

impl NoteVersion {
    /// Whether this version already took `version` into account (is it, or came after it).
    pub fn has_seen(&self, version: &Stamp) -> bool {
        *version == self.stamp || (version.at != 0 && *version == self.base) || self.seen.contains(version)
    }

    /// Every version it supersedes, newest first.
    pub fn ancestors(&self) -> Vec<Stamp> {
        let mut v: Vec<Stamp> =
            std::iter::once(&self.base).chain(&self.seen).filter(|t| t.at != 0).cloned().collect();
        v.sort_unstable_by(|a, b| b.cmp(a));
        v.dedup();
        v.truncate(NOTE_HISTORY);
        v
    }

    /// Also supersedes `others`.
    pub fn absorb(&mut self, others: impl IntoIterator<Item = Stamp>) {
        let mut seen = self.seen.clone();
        seen.extend(others.into_iter().filter(|t| t.at != 0 && *t != self.stamp && *t != self.base));
        seen.sort_unstable_by(|a, b| b.cmp(a));
        seen.dedup();
        seen.truncate(NOTE_HISTORY);
        self.seen = seen;
    }

    /// The version an edit of this one makes, stamped `stamp`.
    pub fn edited(&self, stamp: Stamp, live: bool) -> NoteVersion {
        let mut v = NoteVersion { stamp, live, base: self.stamp.clone(), seen: Vec::new() };
        v.absorb(self.ancestors());
        v
    }
}

/// What a note merge decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotePick {
    /// Keep this side's note as it is.
    Local,
    /// Keep this side's note, now also superseding these versions (the other side's).
    LocalAbsorbing(NoteVersion),
    /// Take the other side's note, with this version (its own, plus this side's history).
    Remote(NoteVersion),
    /// Take the other side's body as a change made here now, with this version.
    Revive(NoteVersion),
}

/// The outcome of merging one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMerge {
    pub pick: NotePick,
    /// The losing side's body is kept as a conflict copy: `true` for this side's, `false` for the
    /// other's.
    pub conflict_copy: Option<bool>,
}

/// Merges a note. Bodies matter only to tell whether two concurrent versions say the same thing
/// (`same_body`); a tombstone's body is empty. `revive` stamps a change made here now.
pub fn merge_note(
    local: Option<&NoteVersion>,
    remote: &NoteVersion,
    same_body: bool,
    mode: MergeMode,
    revive: impl FnOnce() -> Stamp,
) -> NoteMerge {
    let keep = |pick| NoteMerge { pick, conflict_copy: None };
    if mode == MergeMode::Import && !remote.live {
        return keep(NotePick::Local);
    }
    let Some(l) = local else { return keep(NotePick::Remote(remote.clone())) };
    if mode == MergeMode::Import && !l.live {
        // Deleted here, and the file has it: bring it back, superseding both.
        let mut v = l.edited(revive(), true);
        v.absorb(remote.ancestors().into_iter().chain([remote.stamp.clone()]));
        return keep(NotePick::Revive(v));
    }
    if l.stamp == remote.stamp {
        // The same version; one side may know more of its history.
        let mut v = l.clone();
        v.absorb(remote.ancestors());
        return keep(if v == *l { NotePick::Local } else { NotePick::LocalAbsorbing(v) });
    }
    if l.has_seen(&remote.stamp) {
        return keep(NotePick::Local);
    }
    if remote.has_seen(&l.stamp) {
        let mut v = remote.clone();
        v.absorb(l.ancestors());
        return keep(NotePick::Remote(v));
    }
    // Changed on both sides since they last agreed.
    let both = |winner: &NoteVersion, loser: &NoteVersion| {
        let mut v = winner.clone();
        v.absorb(loser.ancestors().into_iter().chain([loser.stamp.clone()]));
        v
    };
    let remote_wins = remote.stamp > l.stamp;
    let (winner, loser) = if remote_wins { (remote, l) } else { (l, remote) };
    let merged = both(winner, loser);
    let conflict_copy = (loser.live && !(winner.live && same_body)).then_some(remote_wins);
    let pick = if remote_wins { NotePick::Remote(merged) } else { NotePick::LocalAbsorbing(merged) };
    NoteMerge { pick, conflict_copy }
}

/// The book, chapter and verse a record belongs to, as written in exports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerseKey {
    pub book: String,
    pub chapter: u8,
    pub verse: u8,
}

/// One coloured stretch of a verse, as it travels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightPart {
    pub color: crate::study::HighlightColor,
    #[serde(default)]
    pub start: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// Unix seconds.
    pub created: i64,
    pub edited: i64,
}

/// A verse's highlights (all of them: they change together).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightsChange {
    #[serde(flatten)]
    pub verse: VerseKey,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<HighlightPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteChange {
    #[serde(flatten)]
    pub verse: VerseKey,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    #[serde(default)]
    pub body: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
    /// The version this one was edited from (ms and device; 0 for a new note).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub base_updated: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_device: String,
    /// Older versions it supersedes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<Stamp>,
    /// Its conflict copies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<NoteConflict>,
}

impl NoteChange {
    pub fn version(&self) -> NoteVersion {
        NoteVersion {
            stamp: self.stamp.clone(),
            live: !self.deleted,
            base: Stamp::new(self.base_updated, self.base_device.clone()),
            seen: self.seen.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkChange {
    #[serde(flatten)]
    pub verse: VerseKey,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanChange {
    pub id: String,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    /// Unix seconds: the start of the plan's first day.
    #[serde(default)]
    pub started: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<i64>,
    /// When it was last (re)started (ms): passages ticked before belong to an earlier run.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub reset_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanReadChange {
    pub plan: String,
    pub day: u16,
    pub passage: u8,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    /// Unix seconds.
    #[serde(default)]
    pub at: i64,
}

/// A passage being memorised (its first verse is the key), as it travels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryChange {
    #[serde(flatten)]
    pub verse: VerseKey,
    #[serde(default)]
    pub last_verse: u8,
    #[serde(default)]
    pub translation: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub due: i64,
    #[serde(default)]
    pub stability: f64,
    #[serde(default)]
    pub difficulty: f64,
    #[serde(default)]
    pub reps: u32,
    #[serde(default)]
    pub lapses: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_review: Option<i64>,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
}

/// One recall of a memorised passage, as it travels. The passage is identified by its first verse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryReviewChange {
    #[serde(flatten)]
    pub verse: VerseKey,
    /// Unix seconds.
    pub at: i64,
    /// "Again", "Hard", "Good" or "Easy".
    pub grade: String,
    /// Stable identity of this recall, so two devices grading the same way in the same second
    /// both keep their log entries.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// Last verse of the passage this recall was of (0 in older files).
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub last_verse: u8,
    /// Translation the passage was learnt in when recalled (empty in older files).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub translation: String,
    #[serde(flatten)]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
}

/// Records with their stamps: what one copy of a study sends another.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Changes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<HighlightsChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<NoteChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookmarks: Vec<BookmarkChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<PlanChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_reads: Vec<PlanReadChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory: Vec<MemoryChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_reviews: Vec<MemoryReviewChange>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.highlights.len()
            + self.notes.len()
            + self.bookmarks.len()
            + self.plans.len()
            + self.plan_reads.len()
            + self.memory.len()
            + self.memory_reviews.len()
    }

    /// Keeps the records whose stamps `keep` says to.
    pub fn retain_stamps(&mut self, mut keep: impl FnMut(&Stamp) -> bool) {
        self.highlights.retain(|c| keep(&c.stamp));
        self.notes.retain(|c| keep(&c.stamp));
        self.bookmarks.retain(|c| keep(&c.stamp));
        self.plans.retain(|c| keep(&c.stamp));
        self.plan_reads.retain(|c| keep(&c.stamp));
        self.memory.retain(|c| keep(&c.stamp));
        self.memory_reviews.retain(|c| keep(&c.stamp));
    }

    /// Every stamp in it.
    pub fn stamps(&self) -> impl Iterator<Item = &Stamp> {
        self.highlights
            .iter()
            .map(|c| &c.stamp)
            .chain(self.notes.iter().map(|c| &c.stamp))
            .chain(self.bookmarks.iter().map(|c| &c.stamp))
            .chain(self.plans.iter().map(|c| &c.stamp))
            .chain(self.plan_reads.iter().map(|c| &c.stamp))
            .chain(self.memory.iter().map(|c| &c.stamp))
            .chain(self.memory_reviews.iter().map(|c| &c.stamp))
    }
}

/// What a merge changed here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub highlights: usize,
    pub notes: usize,
    pub bookmarks: usize,
    pub plans: usize,
    pub plan_reads: usize,
    pub memory: usize,
    pub memory_reviews: usize,
    /// New conflict copies of notes.
    pub conflicts: usize,
}

impl MergeReport {
    pub fn changed(&self) -> usize {
        self.highlights
            + self.notes
            + self.bookmarks
            + self.plans
            + self.plan_reads
            + self.memory
            + self.memory_reviews
    }
}

/// A version of a note that lost to another edited at the same time elsewhere, kept for the reader
/// to look at, copy from, or drop. A note's copies travel with it: dropping one is an edit of the
/// note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteConflict {
    pub body: String,
    /// The losing version's stamp.
    #[serde(flatten)]
    pub stamp: Stamp,
    /// When it was found (Unix seconds).
    #[serde(default)]
    pub found: i64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(at: i64, device: &str) -> Stamp {
        Stamp::new(at, device)
    }

    #[test]
    fn hlc_never_goes_back() {
        let mut c = Hlc::default();
        assert_eq!(c.tick(1_000), 1_000);
        assert_eq!(c.tick(1_000), 1_001, "same millisecond");
        assert_eq!(c.tick(500), 1_002, "the wall clock went back");
        c.observe(5_000);
        assert_eq!(c.tick(1_100), 5_001, "a device ahead of this one was seen");
        c.observe(10);
        assert_eq!(c.tick(6_000), 6_000);
    }

    #[test]
    fn device_ids_differ() {
        let a = new_device_id();
        let b = new_device_id();
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
    }

    #[test]
    fn last_writer_wins() {
        use MergeMode::*;
        let (old, new) = (s(1, "a"), s(2, "a"));
        assert_eq!(lww(None, (&old, true), Sync), Pick::Remote);
        assert_eq!(lww(Some((&old, true)), (&new, true), Sync), Pick::Remote);
        assert_eq!(lww(Some((&new, true)), (&old, true), Sync), Pick::Local);
        assert_eq!(lww(Some((&new, true)), (&new, true), Sync), Pick::Local, "the same change");
        // Ties go to the larger device id, the same on every device.
        assert_eq!(lww(Some((&s(5, "a"), true)), (&s(5, "b"), true), Sync), Pick::Remote);
        assert_eq!(lww(Some((&s(5, "b"), true)), (&s(5, "a"), true), Sync), Pick::Local);
        // Tombstones are changes like any other.
        assert_eq!(lww(Some((&old, true)), (&new, false), Sync), Pick::Remote, "a newer delete wins");
        assert_eq!(
            lww(Some((&new, false)), (&old, true), Sync),
            Pick::Local,
            "an older edit loses to a delete"
        );
        assert_eq!(lww(None, (&old, false), Sync), Pick::Remote, "a delete of something never seen is kept");
        assert_eq!(lww(Some((&old, false)), (&new, true), Sync), Pick::Remote, "an edit after a delete wins");
        // Importing never deletes, and brings back.
        assert_eq!(lww(Some((&old, true)), (&new, false), Import), Pick::Local);
        assert_eq!(lww(None, (&old, false), Import), Pick::Local);
        assert_eq!(lww(Some((&new, false)), (&old, true), Import), Pick::Revive);
        assert_eq!(lww(Some((&old, false)), (&new, true), Import), Pick::Remote);
        assert_eq!(
            lww(Some((&new, true)), (&old, true), Import),
            Pick::Local,
            "an older copy doesn't overwrite"
        );
    }

    /// A version; ancestors as (time, device).
    fn v(at: i64, device: &str, base: (i64, &str), seen: &[(i64, &str)]) -> NoteVersion {
        NoteVersion {
            stamp: s(at, device),
            live: true,
            base: s(base.0, base.1),
            seen: seen.iter().map(|&(t, d)| s(t, d)).collect(),
        }
    }

    const NEW: (i64, &str) = (0, "");

    fn here() -> Stamp {
        s(99, "here")
    }

    #[test]
    fn notes_fast_forward() {
        use MergeMode::*;
        // b edited the version a still has: b's wins without a copy.
        let a = v(10, "a", NEW, &[]);
        let b = a.edited(s(20, "b"), true);
        assert_eq!(b.base, s(10, "a"));
        let m = merge_note(Some(&a), &b, false, Sync, here);
        assert_eq!(m.conflict_copy, None);
        assert!(matches!(m.pick, NotePick::Remote(ref x) if x.stamp == b.stamp));
        // And the other way round, a keeps its own.
        let m = merge_note(Some(&b), &a, false, Sync, here);
        assert_eq!(m, NoteMerge { pick: NotePick::Local, conflict_copy: None });
        // Several edits in a row still descend from the first.
        let c = b.edited(s(30, "b"), true).edited(s(40, "b"), true);
        assert!(c.has_seen(&s(10, "a")) && c.has_seen(&s(20, "b")) && c.has_seen(&s(30, "b")));
        assert_eq!(merge_note(Some(&a), &c, false, Sync, here).conflict_copy, None);
        // A new note arrives as it is.
        assert!(matches!(merge_note(None, &a, false, Sync, here).pick, NotePick::Remote(_)));
    }

    #[test]
    fn notes_edited_on_both_sides_keep_both() {
        use MergeMode::*;
        let base = v(10, "a", NEW, &[]);
        let mine = base.edited(s(20, "a"), true);
        let theirs = base.edited(s(30, "b"), true);
        // Theirs is newer: it becomes the note, mine the copy.
        let m = merge_note(Some(&mine), &theirs, false, Sync, here);
        assert_eq!(m.conflict_copy, Some(true));
        let NotePick::Remote(won) = &m.pick else { panic!("{m:?}") };
        assert_eq!(won.stamp, theirs.stamp);
        assert!(won.has_seen(&s(20, "a")), "the merged note supersedes the copy's version");
        // The other device decides the same: its own stays, mine becomes the copy.
        let m2 = merge_note(Some(&theirs), &mine, false, Sync, here);
        assert_eq!(m2.conflict_copy, Some(false));
        let NotePick::LocalAbsorbing(won2) = &m2.pick else { panic!("{m2:?}") };
        assert_eq!(won, won2);
        // Once both have the merged version, merging again changes nothing.
        assert_eq!(merge_note(Some(won), won2, true, Sync, here).pick, NotePick::Local);
        // The same words written on both sides need no copy.
        assert_eq!(merge_note(Some(&mine), &theirs, true, Sync, here).conflict_copy, None);
        // Two notes started apart on the same verse are concurrent too.
        let m = merge_note(Some(&v(5, "a", NEW, &[])), &v(6, "b", NEW, &[]), false, Sync, here);
        assert_eq!(m.conflict_copy, Some(true));
    }

    #[test]
    fn note_deletes() {
        use MergeMode::*;
        let base = v(10, "a", NEW, &[]);
        let deleted = base.edited(s(20, "a"), false);
        // A delete of the version the other side has wins quietly.
        assert!(matches!(merge_note(Some(&base), &deleted, true, Sync, here).pick, NotePick::Remote(_)));
        assert_eq!(merge_note(Some(&base), &deleted, false, Sync, here).conflict_copy, None);
        // A newer delete over an edit it never saw: the delete wins, the edit is kept as a copy.
        let edited = base.edited(s(15, "b"), true);
        let m = merge_note(Some(&edited), &deleted, false, Sync, here);
        assert!(matches!(m.pick, NotePick::Remote(ref x) if !x.live));
        assert_eq!(m.conflict_copy, Some(true));
        // An edit after a delete it never saw wins; the tombstone leaves no copy.
        let late = base.edited(s(30, "b"), true);
        let m = merge_note(Some(&deleted), &late, false, Sync, here);
        assert_eq!(
            m,
            NoteMerge {
                pick: NotePick::Remote({
                    let mut x = late.clone();
                    x.absorb([s(20, "a")]);
                    x
                }),
                conflict_copy: None
            }
        );
        // Importing a note deleted here brings it back as a new change, superseding both.
        let m = merge_note(Some(&deleted), &edited, false, Import, here);
        let NotePick::Revive(back) = &m.pick else { panic!("{m:?}") };
        assert_eq!(back.stamp, here());
        assert!(back.live && back.has_seen(&s(20, "a")) && back.has_seen(&s(15, "b")));
        // Even when the file has the very version that was deleted.
        let m = merge_note(Some(&deleted), &base, false, Import, here);
        assert!(matches!(m.pick, NotePick::Revive(ref x) if x.live && x.has_seen(&s(20, "a"))));
        // Importing never deletes.
        assert_eq!(merge_note(Some(&base), &deleted, false, Import, here).pick, NotePick::Local);
    }

    #[test]
    fn versions_written_in_the_same_millisecond_are_told_apart() {
        use MergeMode::*;
        // Two new notes, and two edits of one shared version, at the same time on two devices.
        let base = v(10, "a", NEW, &[]);
        for (a, b) in [
            (v(20, "a", NEW, &[]), v(20, "b", NEW, &[])),
            (base.edited(s(20, "a"), true), base.edited(s(20, "b"), true)),
        ] {
            assert!(!a.has_seen(&b.stamp) && !b.has_seen(&a.stamp));
            let on_a = merge_note(Some(&a), &b, false, Sync, here);
            let on_b = merge_note(Some(&b), &a, false, Sync, here);
            // b's device id is the larger: its text stays the note on both, a's is the copy.
            assert_eq!((on_a.conflict_copy, on_b.conflict_copy), (Some(true), Some(false)));
            let (NotePick::Remote(x), NotePick::LocalAbsorbing(y)) = (&on_a.pick, &on_b.pick) else {
                panic!("{on_a:?} {on_b:?}")
            };
            assert_eq!(x, y);
            assert_eq!(merge_note(Some(x), y, true, Sync, here).pick, NotePick::Local);
        }
    }

    #[test]
    fn note_history_is_bounded() {
        let mut n = v(1, "a", NEW, &[]);
        for t in 2..100 {
            n = n.edited(s(t, "a"), true);
        }
        assert!(n.seen.len() <= NOTE_HISTORY);
        assert!(n.has_seen(&s(98, "a")) && n.has_seen(&s(70, "a")) && !n.has_seen(&s(2, "a")));
    }

    #[test]
    fn changes_round_trip_as_json() {
        let c = Changes {
            notes: vec![NoteChange {
                verse: VerseKey { book: "JHN".into(), chapter: 3, verse: 16 },
                stamp: s(1_700_000_000_000, "abc"),
                deleted: false,
                body: "love".into(),
                created: 1,
                updated: 2,
                base_updated: 5,
                base_device: "abc".into(),
                seen: vec![s(3, "def")],
                conflicts: vec![NoteConflict { body: "old".into(), stamp: s(4, "def"), found: 9 }],
            }],
            plan_reads: vec![PlanReadChange {
                plan: "john-21".into(),
                day: 1,
                passage: 0,
                stamp: s(3, "abc"),
                deleted: true,
                at: 0,
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"updated_ms\":1700000000000"), "{json}");
        assert_eq!(serde_json::from_str::<Changes>(&json).unwrap(), c);
        assert_eq!(c.len(), 2);
    }
}
