//! A study library kept in a folder the reader picks (iCloud Drive, Dropbox, Syncthing, a USB
//! stick), so their devices share highlights, notes, bookmarks and plans with no account.
//!
//! The SQLite file never goes in the folder. Each device writes only its own files, so the
//! service syncing the folder never sees two writers on one file:
//!
//! ```text
//! <folder>/library.json                     what this is (format, the library's id)
//! <folder>/changes/<device>/<n>.json        what that device changed, numbered 1, 2, 3…
//! <folder>/snapshots/<device>.json          everything it changed up to change n, after which
//!                                           its older change files are removed
//! ```
//!
//! A sync reads the other devices' new files into the store ([`MergeMode::Sync`]: newer wins,
//! deletes included, a note edited on two devices keeps a conflict copy), then writes what
//! changed here since the last one. How far it has read and written is kept in the store's sync
//! state, per library.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::study::StudyStore;
use crate::sync::{Changes, MergeMode, MergeReport};

/// The folder format this build writes and reads.
pub const FORMAT: u32 = 1;
const MARKER: &str = "library.json";
const APP: &str = "bible-rs study library";
/// A device's change files before it folds them into its snapshot.
const COMPACT_AFTER: usize = 64;

#[derive(Serialize, Deserialize)]
struct Marker {
    app: String,
    format: u32,
    id: String,
}

/// A change file or a snapshot.
#[derive(Serialize, Deserialize)]
struct Batch {
    format: u32,
    device: String,
    /// The change's number; for a snapshot, the last change it holds.
    seq: u64,
    #[serde(default)]
    changes: Changes,
}

/// A device's files, as found in the folder.
#[derive(Default)]
struct DeviceFiles {
    /// Change files here in full.
    changes: BTreeSet<u64>,
    /// Change files the sync service hasn't downloaded yet (iCloud's `.<name>.icloud`).
    pending: BTreeSet<u64>,
    snapshot: bool,
    snapshot_pending: bool,
}

/// Where a store has read each other device's files up to (from [`FolderLibrary::plan`]).
#[derive(Debug, Clone, Default)]
pub struct Plan {
    me: String,
    read: std::collections::HashMap<String, u64>,
}

/// The other devices' files read by [`FolderLibrary::fetch`], for [`FolderLibrary::apply`].
#[derive(Default)]
pub struct Fetched {
    devices: Vec<(String, Vec<Batch>)>,
    waiting: usize,
    problems: Vec<String>,
}

impl Fetched {
    /// Files read.
    pub fn files(&self) -> usize {
        self.devices.iter().map(|(_, b)| b.len()).sum()
    }
}

/// What one [`FolderLibrary::sync`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Records written for the other devices.
    pub written: usize,
    /// Files read from them.
    pub read: usize,
    /// What reading them changed here.
    pub merged: MergeReport,
    /// Files there that couldn't be read yet (still downloading, or unreadable).
    pub waiting: usize,
    /// Why files were left, for the log.
    pub problems: Vec<String>,
}

/// How the library stands, cheaply (nothing is read or written).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStatus {
    /// The folder is missing or can't be read (a drive not plugged in, a revoked permission).
    pub offline: bool,
    /// Records changed here not yet written to the folder.
    pub outgoing: usize,
    /// Files from other devices not yet read.
    pub incoming: usize,
    /// Note conflict copies waiting for the reader.
    pub conflicts: usize,
    /// Other devices writing to the folder.
    pub devices: usize,
}

impl SyncStatus {
    pub fn waiting(&self) -> usize {
        self.outgoing + self.incoming
    }

    /// "Synced", "3 waiting", "Offline", "2 conflicts", or two of them joined by " · ".
    pub fn label(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        if self.offline {
            parts.push("Offline".to_string());
        } else if self.waiting() > 0 {
            parts.push(format!("{} waiting", self.waiting()));
        }
        if self.conflicts > 0 {
            parts.push(plural(self.conflicts, "conflict", "conflicts"));
        }
        if parts.is_empty() { "Synced".into() } else { parts.join(" · ") }
    }
}

/// A study library folder.
#[derive(Debug, Clone)]
pub struct FolderLibrary {
    root: PathBuf,
    id: String,
}

impl FolderLibrary {
    /// Opens the library in `root`, making it there if the folder is empty of one. Fails when the
    /// folder can't be reached or was made by a newer version.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!("{} isn't reachable", root.display());
        }
        let marker = root.join(MARKER);
        let id = match fs::read_to_string(&marker) {
            Ok(s) => {
                let m: Marker =
                    serde_json::from_str(&s).with_context(|| format!("reading {}", marker.display()))?;
                if m.format > FORMAT {
                    bail!("this study library was made by a newer version of the app; update to use it");
                }
                m.id
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let m = Marker { app: APP.into(), format: FORMAT, id: crate::sync::new_device_id() };
                write_atomic(&marker, &serde_json::to_vec_pretty(&m)?)?;
                m.id
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", marker.display())),
        };
        fs::create_dir_all(root.join("changes"))?;
        fs::create_dir_all(root.join("snapshots"))?;
        Ok(Self { root: root.to_path_buf(), id })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The library's id (from `library.json`): the store keeps its place in each library apart.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Reads the other devices' new files in, then writes what changed here.
    pub fn sync(&self, store: &mut StudyStore) -> Result<SyncReport> {
        let fetched = self.fetch(&self.plan(store))?;
        self.apply(store, fetched)
    }

    /// Where it stands, from file names alone.
    pub fn status(&self, store: &StudyStore) -> SyncStatus {
        let mut s = SyncStatus { conflicts: store.conflict_count(), ..Default::default() };
        let Ok(devices) = self.devices(store.device_id()) else {
            s.offline = true;
            s.outgoing = store.changes_since(self.pushed(store), true).len();
            return s;
        };
        s.outgoing = self.outgoing(store, self.pushed(store), &devices).len();
        s.devices = devices.len();
        for (device, files) in &devices {
            let read = self.read_upto(store, device);
            s.incoming += files.changes.range(read + 1..).count() + files.pending.range(read + 1..).count();
            if files.snapshot_pending && read == 0 && !files.snapshot {
                s.incoming += 1;
            }
        }
        s
    }

    fn key(&self, what: &str) -> String {
        format!("library.{}.{what}", self.id)
    }

    /// The store's sequence number this device's files are written up to.
    fn pushed(&self, store: &StudyStore) -> i64 {
        let n = store.sync_value(&self.key("pushed")).and_then(|v| v.parse().ok()).unwrap_or(0);
        // A store that went back (restored from an older copy) writes all its records again.
        if n > store.seq() { 0 } else { n }
    }

    /// What this device writes: what changed here since `since`, but for the records the other
    /// devices here wrote themselves. So what came in from elsewhere (an imported backup, an
    /// older version's records) goes out too, and what was read from this folder doesn't go back.
    fn outgoing(&self, store: &StudyStore, since: i64, others: &[(String, DeviceFiles)]) -> Changes {
        let mut c = store.changes_since(since, false);
        c.retain_stamps(|s| !others.iter().any(|(d, _)| *d == s.device));
        c
    }

    fn read_upto(&self, store: &StudyStore, device: &str) -> u64 {
        store.sync_value(&self.key(&format!("read.{device}"))).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    fn changes_dir(&self, device: &str) -> PathBuf {
        self.root.join("changes").join(device)
    }

    fn snapshot_path(&self, device: &str) -> PathBuf {
        self.root.join("snapshots").join(format!("{device}.json"))
    }

    /// Every other device with files here.
    fn devices(&self, me: &str) -> Result<Vec<(String, DeviceFiles)>> {
        let mut out: std::collections::BTreeMap<String, DeviceFiles> = Default::default();
        for entry in fs::read_dir(self.root.join("changes"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == me || !valid_device(&name) || !entry.path().is_dir() {
                continue;
            }
            out.insert(name.clone(), list_changes(&entry.path())?);
        }
        for entry in fs::read_dir(self.root.join("snapshots"))? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            let (stem, pending) = match placeholder(&name) {
                Some(inner) => (inner.to_string(), true),
                None => (name, false),
            };
            let Some(device) = stem.strip_suffix(".json").filter(|d| *d != me && valid_device(d)) else {
                continue;
            };
            let files = out.entry(device.to_string()).or_default();
            if pending {
                files.snapshot_pending = true;
            } else {
                files.snapshot = true;
            }
        }
        Ok(out.into_iter().collect())
    }

    /// Where this store has read each device up to: what [`fetch`](Self::fetch) starts from.
    pub fn plan(&self, store: &StudyStore) -> Plan {
        let me = store.device_id().to_string();
        let read = match self.devices(&me) {
            Ok(devices) => devices
                .into_iter()
                .map(|(d, _)| {
                    let n = self.read_upto(store, &d);
                    (d, n)
                })
                .collect(),
            Err(_) => Default::default(),
        };
        Plan { me, read }
    }

    /// Reads the other devices' new files, without the store: this is the part that can wait on
    /// the sync service (a file iCloud hasn't downloaded yet blocks the read until it has), so
    /// an app runs it off its UI thread and gives the result to [`apply`](Self::apply).
    pub fn fetch(&self, plan: &Plan) -> Result<Fetched> {
        let mut out = Fetched::default();
        for (device, files) in self.devices(&plan.me)? {
            let mut read = plan.read.get(&device).copied().unwrap_or(0);
            let mut batches = Vec::new();
            // The next change file is gone (folded into the snapshot): start from the snapshot.
            if files.changes.first().is_none_or(|n| *n > read + 1)
                && (files.snapshot || files.snapshot_pending)
            {
                let path = self.snapshot_path(&device);
                match files.snapshot.then(|| read_batch(&path, &device)) {
                    Some(Ok(batch)) => {
                        if batch.seq > read {
                            read = batch.seq;
                            batches.push(batch);
                        }
                    }
                    Some(Err(e)) => out.problems.push(format!("{}: {e:#}", path.display())),
                    None => {}
                }
                if read == 0 && !files.snapshot {
                    out.waiting += 1;
                }
            }
            // In order, one after another: a gap is a file still on its way.
            while files.changes.contains(&(read + 1)) {
                let path = self.changes_dir(&device).join(change_name(read + 1));
                match read_batch(&path, &device) {
                    Ok(batch) => {
                        read += 1;
                        batches.push(Batch { seq: read, ..batch });
                    }
                    Err(e) => {
                        out.problems.push(format!("{}: {e:#}", path.display()));
                        break;
                    }
                }
            }
            out.waiting += files.changes.range(read + 1..).count() + files.pending.range(read + 1..).count();
            if !batches.is_empty() {
                out.devices.push((device, batches));
            }
        }
        Ok(out)
    }

    /// Merges what [`fetch`](Self::fetch) read into the store, then writes what changed here.
    pub fn apply(&self, store: &mut StudyStore, fetched: Fetched) -> Result<SyncReport> {
        let mut report =
            SyncReport { waiting: fetched.waiting, problems: fetched.problems, ..Default::default() };
        for (device, batches) in fetched.devices {
            for batch in batches {
                // Read already (another sync got there first): nothing new in it.
                if batch.seq <= self.read_upto(store, &device) {
                    continue;
                }
                let r = store.merge(&batch.changes, MergeMode::Sync)?;
                add(&mut report.merged, &r);
                report.read += 1;
                store.set_sync_value(&self.key(&format!("read.{device}")), &batch.seq.to_string())?;
            }
        }
        report.written = self.push(store)?;
        Ok(report)
    }

    /// Writes this device's changes since the last time; how many records.
    fn push(&self, store: &mut StudyStore) -> Result<usize> {
        let me = store.device_id().to_string();
        let pushed = self.pushed(store);
        let others = self.devices(&me)?;
        let changes = self.outgoing(store, pushed, &others);
        let seq = store.seq();
        let count = changes.len();
        if changes.is_empty() {
            if pushed != seq {
                store.set_sync_value(&self.key("pushed"), &seq.to_string())?;
            }
            return Ok(0);
        }
        let dir = self.changes_dir(&me);
        fs::create_dir_all(&dir)?;
        let mut mine = list_changes(&dir)?;
        let snapshot = self.snapshot_path(&me);
        let written: u64 = store.sync_value(&self.key("written")).and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut last = mine.changes.last().copied().unwrap_or(0).max(written);
        if last == 0 && snapshot.exists() {
            last = read_batch(&snapshot, &me).map_or(0, |b| b.seq);
        }
        let n = last + 1;
        if n > 1 && mine.changes.is_empty() && !snapshot.exists() {
            // This device's files were removed from the folder: write all of it again, as a
            // snapshot, so a device starting from nothing doesn't wait for changes 1 to n - 1.
            let all = Batch { format: FORMAT, device: me, seq: n, changes: self.outgoing(store, 0, &others) };
            write_atomic(&snapshot, &serde_json::to_vec(&all)?)?;
            store.set_sync_value(&self.key("written"), &n.to_string())?;
            store.set_sync_value(&self.key("pushed"), &seq.to_string())?;
            return Ok(all.changes.len());
        }
        let batch = Batch { format: FORMAT, device: me.clone(), seq: n, changes };
        write_atomic(&dir.join(change_name(n)), &serde_json::to_vec(&batch)?)?;
        mine.changes.insert(n);
        store.set_sync_value(&self.key("written"), &n.to_string())?;
        store.set_sync_value(&self.key("pushed"), &seq.to_string())?;
        if mine.changes.len() > COMPACT_AFTER {
            // Everything this device's records say now, as of change n; then its files up to n go.
            let all = Batch { format: FORMAT, device: me, seq: n, changes: self.outgoing(store, 0, &others) };
            write_atomic(&snapshot, &serde_json::to_vec(&all)?)?;
            for old in mine.changes.range(..=n) {
                let _ = fs::remove_file(dir.join(change_name(*old)));
            }
        }
        Ok(count)
    }
}

fn add(total: &mut MergeReport, r: &MergeReport) {
    total.highlights += r.highlights;
    total.notes += r.notes;
    total.bookmarks += r.bookmarks;
    total.plans += r.plans;
    total.plan_reads += r.plan_reads;
    total.conflicts += r.conflicts;
}

fn change_name(n: u64) -> String {
    format!("{n:016}.json")
}

/// The number in a change file's name.
fn change_number(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".json")?;
    if digits.len() != 16 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The real name of a file iCloud hasn't downloaded yet (`.<name>.icloud`).
fn placeholder(name: &str) -> Option<&str> {
    name.strip_prefix('.')?.strip_suffix(".icloud")
}

/// A device id as this app makes them: nothing that could step out of the folder.
fn valid_device(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn list_changes(dir: &Path) -> Result<DeviceFiles> {
    let mut files = DeviceFiles::default();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some(n) = change_number(&name) {
            files.changes.insert(n);
        } else if let Some(n) = placeholder(&name).and_then(change_number) {
            files.pending.insert(n);
        }
    }
    // Downloaded since it was listed both ways.
    files.pending.retain(|n| !files.changes.contains(n));
    Ok(files)
}

fn read_batch(path: &Path, device: &str) -> Result<Batch> {
    let bytes = fs::read(path)?;
    let batch: Batch = serde_json::from_slice(&bytes).context("not a whole change file yet")?;
    if batch.format > FORMAT {
        bail!("written by a newer version of the app");
    }
    if batch.device != device {
        bail!("written by another device ({})", batch.device);
    }
    Ok(batch)
}

/// Writes beside the file, then renames over it, so another device never reads half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;
    use crate::study::HighlightColor;
    use crate::{BookId, VerseRef};

    const T0: i64 = 1_790_000_000_000;

    /// An empty folder of its own under the system's temp folder, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "bible-folder-{name}-{}-{}",
                std::process::id(),
                crate::sync::new_device_id()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn device(start_ms: i64) -> (StudyStore, Arc<AtomicI64>) {
        let mut s = StudyStore::open_in_memory().unwrap();
        let now = Arc::new(AtomicI64::new(start_ms));
        let clock = now.clone();
        s.set_clock(Arc::new(move || clock.load(Ordering::SeqCst)));
        (s, now)
    }

    fn later(clock: &AtomicI64, ms: i64) {
        clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn v(n: u8) -> VerseRef {
        VerseRef::new(BookId(42), 3, n)
    }

    fn body(s: &StudyStore, n: u8) -> Option<&str> {
        s.note(v(n)).map(|n| n.body.as_str())
    }

    /// Every file under `dir`, relative, sorted.
    fn files(dir: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn what_came_from_elsewhere_goes_out_too() {
        let dir = TempDir::new("elsewhere");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, _) = device(T0);
        let (mut b, _) = device(T0 + 5);
        // A backup from before sync (no device), and a record from a device not in this folder.
        a.import_json(
            r#"{"app":"bible-rs","format":1,"exported_at":"2026-10-14T12:00:00Z","translation":"KJV",
                "notes":[{"reference":"John 3:16","book":"JHN","chapter":3,"verse":16,
                "note":"old","created":"2026-10-14T12:00:00Z","updated":"2026-10-14T12:00:00Z"}]}"#,
        )
        .unwrap();
        let mut other = Changes::default();
        other.notes.push(crate::sync::NoteChange {
            verse: crate::sync::VerseKey { book: "JHN".into(), chapter: 3, verse: 17 },
            stamp: crate::sync::Stamp::new(T0 - 1_000, "elsewhere"),
            deleted: false,
            body: "from elsewhere".into(),
            created: 1_790_000_000,
            updated: 1_790_000_000,
            base_updated: 0,
            seen: vec![],
            conflicts: vec![],
        });
        a.merge(&other, MergeMode::Import).unwrap();
        assert_eq!(lib.status(&a).outgoing, 2);
        assert_eq!(lib.sync(&mut a).unwrap().written, 2);
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.merged.changed()), (1, 2));
        assert_eq!((body(&b, 16), body(&b, 17)), (Some("old"), Some("from elsewhere")));
        // b passes those on once too (it can't tell them from its own imports); a takes nothing
        // new from that, and neither writes what it read from the other.
        lib.sync(&mut b).unwrap();
        let r = lib.sync(&mut a).unwrap();
        assert_eq!((r.merged.changed(), r.written), (0, 0));
        assert_eq!(lib.sync(&mut b).unwrap().written, 0);
        assert_eq!(lib.status(&a).label(), "Synced");
        assert_eq!(lib.status(&b).label(), "Synced");
    }

    #[test]
    fn two_devices_share_through_the_folder() {
        let dir = TempDir::new("share");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, clock_a) = device(T0);
        let (mut b, clock_b) = device(T0 + 5);
        a.set_note(v(16), "love").unwrap();
        a.set_highlight(v(16), Some(HighlightColor::Green)).unwrap();
        a.set_bookmark(v(17), Some("")).unwrap();
        a.start_plan("bible-in-a-year", 1_790_000_000).unwrap();
        a.set_plan_read("bible-in-a-year", 1, 0, true).unwrap();
        let r = lib.sync(&mut a).unwrap();
        assert_eq!((r.read, r.written), (0, 5));
        assert_eq!(lib.status(&a).label(), "Synced");

        assert_eq!(lib.status(&b).label(), "1 waiting");
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.written, r.merged.changed()), (1, 0, 5));
        assert_eq!(body(&b, 16), Some("love"));
        assert_eq!(b.highlight(v(16)), Some(HighlightColor::Green));
        assert!(b.is_bookmarked(v(17)));
        assert!(b.plan_reads("bible-in-a-year").contains(&(1, 0)));
        assert_eq!(lib.status(&b).label(), "Synced");

        // A delete there comes back here.
        later(&clock_b, 1000);
        b.set_bookmark(v(17), None).unwrap();
        b.set_note(v(16), "love one another").unwrap();
        assert_eq!(lib.status(&b).label(), "2 waiting");
        lib.sync(&mut b).unwrap();
        later(&clock_a, 2000);
        lib.sync(&mut a).unwrap();
        assert!(!a.is_bookmarked(v(17)));
        assert_eq!(body(&a, 16), Some("love one another"));
        assert_eq!(a.conflict_count(), 0);
        // Nothing left to do either way; syncing again reads and writes nothing.
        let r = lib.sync(&mut a).unwrap();
        assert_eq!((r.read, r.written), (0, 0));
        assert_eq!(lib.status(&a), SyncStatus { devices: 1, ..Default::default() });

        // Each device wrote only its own files, and no database went in.
        let (da, db) = (a.device_id().to_string(), b.device_id().to_string());
        for f in files(&dir.0) {
            assert!(
                f == MARKER
                    || f.starts_with(&format!("changes/{da}/"))
                    || f.starts_with(&format!("changes/{db}/")),
                "{f}"
            );
            assert!(!f.contains("sqlite"), "{f}");
        }
    }

    #[test]
    fn a_note_edited_on_both_keeps_a_conflict_copy_on_both() {
        let dir = TempDir::new("conflict");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, clock_a) = device(T0);
        let (mut b, clock_b) = device(T0);
        a.set_note(v(16), "love").unwrap();
        lib.sync(&mut a).unwrap();
        lib.sync(&mut b).unwrap();
        // Both edit it before either syncs.
        later(&clock_a, 1000);
        a.set_note(v(16), "from a").unwrap();
        later(&clock_b, 2000);
        b.set_note(v(16), "from b").unwrap();
        lib.sync(&mut a).unwrap();
        let r = lib.sync(&mut b).unwrap();
        assert_eq!(r.merged.conflicts, 1);
        lib.sync(&mut a).unwrap();
        // b's edit is newer; a's text is kept on both as a copy.
        for s in [&a, &b] {
            assert_eq!(body(s, 16), Some("from b"));
            let copies = s.note_conflicts_at(v(16));
            assert_eq!(copies.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(), ["from a"]);
        }
        assert_eq!(lib.status(&a).label(), "1 conflict");
        // Dropping it on one drops it on the other.
        let stamp = a.note_conflicts_at(v(16))[0].stamp.clone();
        later(&clock_a, 1000);
        assert!(a.dismiss_conflict(v(16), &stamp).unwrap());
        lib.sync(&mut a).unwrap();
        lib.sync(&mut b).unwrap();
        assert_eq!(b.conflict_count(), 0);
        assert_eq!(body(&b, 16), Some("from b"));
        assert_eq!(lib.status(&b).label(), "Synced");
    }

    #[test]
    fn many_changes_fold_into_a_snapshot_that_a_new_device_starts_from() {
        let dir = TempDir::new("compact");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, clock) = device(T0);
        let (mut b, _) = device(T0);
        let mut b_read_some = false;
        for n in 1..=(COMPACT_AFTER as u8 + 10) {
            later(&clock, 1000);
            a.set_note(v(n % 30 + 1), &format!("note {n}")).unwrap();
            lib.sync(&mut a).unwrap();
            if n == 20 {
                lib.sync(&mut b).unwrap();
                b_read_some = true;
            }
        }
        assert!(b_read_some);
        let me = a.device_id().to_string();
        let left = fs::read_dir(lib.changes_dir(&me)).unwrap().count();
        assert!(left < COMPACT_AFTER, "{left} change files left");
        assert!(lib.snapshot_path(&me).exists());

        // One that read some before, and one that read none, both end up with all of it.
        let (mut c, _) = device(T0);
        for s in [&mut b, &mut c] {
            lib.sync(s).unwrap();
            assert_eq!(s.notes().len(), 30);
            for n in 1..=30 {
                assert_eq!(body(s, n), a.note(v(n)).map(|n| n.body.as_str()));
            }
            assert_eq!(lib.status(s).label(), "Synced");
        }
    }

    #[test]
    fn files_still_downloading_are_waited_for_in_order() {
        let dir = TempDir::new("pending");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, clock) = device(T0);
        let (mut b, _) = device(T0);
        a.set_note(v(1), "one").unwrap();
        lib.sync(&mut a).unwrap();
        later(&clock, 1000);
        a.set_note(v(2), "two").unwrap();
        lib.sync(&mut a).unwrap();
        // iCloud hasn't downloaded the first one yet.
        let changes = lib.changes_dir(a.device_id());
        let first = changes.join(change_name(1));
        let placeholder = changes.join(format!(".{}.icloud", change_name(1)));
        fs::rename(&first, &placeholder).unwrap();
        assert_eq!(lib.status(&b).label(), "2 waiting");
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.waiting), (0, 2));
        assert_eq!(b.notes().len(), 0);
        // Half of it so far: still waiting, nothing breaks.
        fs::write(&first, b"{\"format\":1,\"dev").unwrap();
        fs::remove_file(&placeholder).unwrap();
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.waiting, r.problems.len()), (0, 2, 1));
        // Here now.
        let whole =
            Batch { format: FORMAT, device: a.device_id().into(), seq: 1, changes: a.changes_since(0, true) };
        fs::write(&first, serde_json::to_vec(&whole).unwrap()).unwrap();
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.waiting), (2, 0));
        assert_eq!((body(&b, 1), body(&b, 2)), (Some("one"), Some("two")));
    }

    #[test]
    fn a_missing_folder_is_offline() {
        let dir = TempDir::new("offline");
        let root = dir.0.join("library");
        assert!(FolderLibrary::open(&root).is_err());
        fs::create_dir_all(&root).unwrap();
        let lib = FolderLibrary::open(&root).unwrap();
        let (mut a, _) = device(T0);
        a.set_note(v(1), "one").unwrap();
        fs::remove_dir_all(&root).unwrap();
        let s = lib.status(&a);
        assert!(s.offline);
        assert_eq!(s.label(), "Offline");
        assert!(lib.sync(&mut a).is_err());
        // Back: what waited goes out.
        fs::create_dir_all(&root).unwrap();
        let lib = FolderLibrary::open(&root).unwrap();
        assert_eq!(lib.sync(&mut a).unwrap().written, 1);
    }

    #[test]
    fn a_library_from_a_newer_version_is_refused() {
        let dir = TempDir::new("newer");
        fs::write(dir.0.join(MARKER), r#"{"app":"bible-rs study library","format":99,"id":"x"}"#).unwrap();
        let e = FolderLibrary::open(&dir.0).unwrap_err();
        assert!(e.to_string().contains("newer version"), "{e}");
    }

    #[test]
    fn each_library_keeps_its_own_place() {
        let (one, two) = (TempDir::new("one"), TempDir::new("two"));
        let (lib1, lib2) = (FolderLibrary::open(&one.0).unwrap(), FolderLibrary::open(&two.0).unwrap());
        assert_ne!(lib1.id(), lib2.id());
        assert_eq!(FolderLibrary::open(&one.0).unwrap().id(), lib1.id());
        let (mut a, _) = device(T0);
        a.set_note(v(1), "one").unwrap();
        assert_eq!(lib1.sync(&mut a).unwrap().written, 1);
        // Switching to another folder writes everything there too.
        assert_eq!(lib2.sync(&mut a).unwrap().written, 1);
        assert_eq!(lib1.sync(&mut a).unwrap().written, 0);
    }

    #[test]
    fn a_device_whose_files_were_removed_writes_a_snapshot() {
        let dir = TempDir::new("removed");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, clock) = device(T0);
        a.set_note(v(1), "one").unwrap();
        lib.sync(&mut a).unwrap();
        fs::remove_dir_all(lib.changes_dir(a.device_id())).unwrap();
        later(&clock, 1000);
        a.set_note(v(2), "two").unwrap();
        assert_eq!(lib.sync(&mut a).unwrap().written, 2);
        let (mut b, _) = device(T0);
        lib.sync(&mut b).unwrap();
        assert_eq!((body(&b, 1), body(&b, 2)), (Some("one"), Some("two")));
    }

    #[test]
    fn stray_files_are_ignored() {
        let dir = TempDir::new("stray");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, _) = device(T0);
        a.set_note(v(1), "one").unwrap();
        lib.sync(&mut a).unwrap();
        let changes = lib.changes_dir(a.device_id());
        // A sync service's conflicted copy, an editor's leftovers, a folder that isn't a device.
        fs::write(changes.join("0000000000000001 2.json"), b"junk").unwrap();
        fs::write(changes.join(".DS_Store"), b"junk").unwrap();
        fs::create_dir_all(dir.0.join("changes").join("..evil")).unwrap();
        fs::write(dir.0.join("changes").join("notes.txt"), b"junk").unwrap();
        let (mut b, _) = device(T0);
        let r = lib.sync(&mut b).unwrap();
        assert_eq!((r.read, r.waiting, r.problems.len()), (1, 0, 0));
        assert_eq!(body(&b, 1), Some("one"));
    }

    #[test]
    fn its_place_is_kept_in_the_file() {
        let dir = TempDir::new("reopen");
        let root = dir.0.join("lib");
        fs::create_dir_all(&root).unwrap();
        let lib = FolderLibrary::open(&root).unwrap();
        let path = dir.0.join("study.sqlite");
        let (mut b, _) = device(T0);
        {
            let mut a = StudyStore::open(&path).unwrap();
            a.set_note(v(1), "one").unwrap();
            lib.sync(&mut a).unwrap();
            b.set_note(v(2), "two").unwrap();
            lib.sync(&mut b).unwrap();
            lib.sync(&mut a).unwrap();
        }
        let mut a = StudyStore::open(&path).unwrap();
        assert_eq!(lib.status(&a).label(), "Synced");
        let r = lib.sync(&mut a).unwrap();
        assert_eq!((r.read, r.written), (0, 0));
        assert_eq!(body(&a, 2), Some("two"));
    }

    #[test]
    fn reads_fetched_twice_are_taken_once() {
        let dir = TempDir::new("twice");
        let lib = FolderLibrary::open(&dir.0).unwrap();
        let (mut a, _) = device(T0);
        let (mut b, _) = device(T0);
        a.set_note(v(1), "one").unwrap();
        lib.sync(&mut a).unwrap();
        // Two fetches started before either was applied (off the UI thread, say).
        let (one, two) = (lib.fetch(&lib.plan(&b)).unwrap(), lib.fetch(&lib.plan(&b)).unwrap());
        assert_eq!((one.files(), two.files()), (1, 1));
        assert_eq!(lib.apply(&mut b, one).unwrap().read, 1);
        assert_eq!(lib.apply(&mut b, two).unwrap().read, 0);
        assert_eq!(body(&b, 1), Some("one"));
    }

    #[test]
    fn labels() {
        let s = |offline, outgoing, incoming, conflicts| SyncStatus {
            offline,
            outgoing,
            incoming,
            conflicts,
            devices: 1,
        };
        assert_eq!(s(false, 0, 0, 0).label(), "Synced");
        assert_eq!(s(false, 2, 1, 0).label(), "3 waiting");
        assert_eq!(s(true, 2, 0, 0).label(), "Offline");
        assert_eq!(s(false, 0, 0, 1).label(), "1 conflict");
        assert_eq!(s(false, 1, 0, 2).label(), "1 waiting · 2 conflicts");
    }
}
