//! The study library folder (Settings → Data, beta): when one is set, the study is synced with
//! it in the background, a little after each change and every half minute.
//!
//! Opening the folder and reading the other devices' files happen on a thread (iCloud can hold a
//! read until it has downloaded the file); merging them and writing this device's changes happen on the UI thread,
//! where the store lives. See [`bible_core::folder`].

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use anyhow::Result;
use bible_core::folder::{Fetched, FolderLibrary, SyncReport, SyncStatus};
use bible_core::study::StudyStore;
use eframe::egui;

/// How long after a change here it is written out (more changes in that time go together).
const AFTER_CHANGE: f64 = 2.0;
/// How often the folder is read for other devices' changes.
const EVERY: f64 = 30.0;
/// How often the status is refreshed between syncs (it only lists the folder).
const STATUS_EVERY: f64 = 5.0;

#[derive(Default)]
pub struct StudySync {
    /// The folder set in Settings, and the library opened in it.
    folder: Option<PathBuf>,
    library: Option<FolderLibrary>,
    /// An open or a fetch running on its thread.
    fetching: Option<Receiver<Fetch>>,
    /// When the last sync started, and the study's generation then (egui time).
    last_sync: Option<f64>,
    synced_gen: u64,
    /// When the study was first seen changed since the last sync.
    changed_at: Option<f64>,
    /// When the status was last read.
    status_at: Option<f64>,
    pub status: Option<SyncStatus>,
    /// Why the last sync failed, until one succeeds.
    pub error: Option<String>,
    /// When the last one finished (Unix seconds).
    pub synced_at: Option<i64>,
    /// Asked for from Settings.
    now: bool,
}

/// What the sync thread sends back.
enum Fetch {
    /// The library opened (its marker read or made); the fetch follows.
    Opened(Result<FolderLibrary>),
    Fetched(Result<Fetched>),
}

/// What the Data tab shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncView {
    pub folder: Option<PathBuf>,
    pub label: String,
    pub detail: Option<String>,
    pub syncing: bool,
}

impl StudySync {
    /// Syncs now (the Sync Now button), or as soon as the running one is done.
    pub fn sync_soon(&mut self) {
        self.now = true;
    }

    /// Once a frame: starts, finishes and schedules syncs. Returns what a finished one did.
    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        folder: Option<&str>,
        study: &mut StudyStore,
    ) -> Option<SyncReport> {
        let now = ctx.input(|i| i.time);
        let folder = folder.map(PathBuf::from);
        if folder != self.folder {
            *self = StudySync { folder, now: true, ..Default::default() };
        }
        let folder = self.folder.clone()?;

        let mut report = None;
        if let Some(rx) = &self.fetching {
            match rx.try_recv() {
                Ok(Fetch::Fetched(fetched)) => {
                    self.fetching = None;
                    report = self.finish(fetched, study);
                    self.status_at = None;
                }
                Ok(Fetch::Opened(opened)) => {
                    self.fetching = None;
                    match opened {
                        Ok(lib) => {
                            self.library = Some(lib);
                            // Fetch straight away.
                            self.now = true;
                        }
                        Err(e) => self.error = Some(format!("{e:#}")),
                    }
                    self.status_at = None;
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    return None;
                }
                Err(TryRecvError::Disconnected) => self.fetching = None,
            }
        }

        if study.generation() != self.synced_gen && self.changed_at.is_none() {
            self.changed_at = Some(now);
        }
        let due = self.now
            || self.last_sync.is_none_or(|t| now - t >= EVERY)
            || self.changed_at.is_some_and(|t| now - t >= AFTER_CHANGE);
        if due {
            self.start(&folder, study, now);
        }
        if self.status_at.is_none_or(|t| now - t >= STATUS_EVERY) {
            self.status_at = Some(now);
            self.status = Some(match &self.library {
                Some(lib) if folder.is_dir() => lib.status(study),
                _ => SyncStatus { offline: true, conflicts: study.conflict_count(), ..Default::default() },
            });
        }
        let next = [
            self.last_sync.map(|t| t + EVERY),
            self.changed_at.map(|t| t + AFTER_CHANGE),
            self.status_at.map(|t| t + STATUS_EVERY),
        ];
        if let Some(at) = next.into_iter().flatten().reduce(f64::min) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.05)));
        }
        report
    }

    fn start(&mut self, folder: &std::path::Path, study: &StudyStore, now: f64) {
        self.now = false;
        self.last_sync = Some(now);
        self.changed_at = None;
        self.synced_gen = study.generation();
        let (tx, rx) = channel();
        let job: Box<dyn FnOnce() -> Fetch + Send> = match self.library.clone() {
            Some(lib) => {
                let plan = lib.plan(study);
                Box::new(move || Fetch::Fetched(lib.fetch(&plan)))
            }
            None => {
                let folder = folder.to_path_buf();
                Box::new(move || Fetch::Opened(FolderLibrary::open(&folder)))
            }
        };
        let spawned = std::thread::Builder::new().name("study-sync".into()).spawn(move || {
            let _ = tx.send(job());
        });
        match spawned {
            Ok(_) => self.fetching = Some(rx),
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn finish(&mut self, fetched: Result<Fetched>, study: &mut StudyStore) -> Option<SyncReport> {
        let lib = self.library.as_ref()?;
        match fetched.and_then(|f| lib.apply(study, f)) {
            Ok(report) => {
                for p in &report.problems {
                    eprintln!("study library: {p}");
                }
                self.error = None;
                self.synced_at = Some(bible_core::study::now());
                // What it merged in and wrote out isn't a change to write again.
                self.synced_gen = study.generation();
                Some(report)
            }
            Err(e) => {
                // The folder went away: open it again next time.
                self.library = None;
                self.error = Some(format!("{e:#}"));
                None
            }
        }
    }

    pub fn view(&self) -> SyncView {
        let Some(folder) = self.folder.clone() else { return SyncView::default() };
        let label = match (&self.status, &self.error) {
            (Some(s), _) if s.offline => s.label(),
            (_, Some(_)) => "Couldn't sync".into(),
            (Some(s), None) => s.label(),
            (None, None) => "Syncing…".into(),
        };
        let detail = self.error.clone().or_else(|| {
            let devices = self.status.as_ref().map_or(0, |s| s.devices);
            let others = match devices {
                0 => "no other devices yet".to_string(),
                1 => "1 other device".to_string(),
                n => format!("{n} other devices"),
            };
            Some(match self.synced_at {
                Some(at) => format!("Last synced {} · {others}", crate::time::clock(at)),
                None => others,
            })
        });
        SyncView { folder: Some(folder), label, detail, syncing: self.fetching.is_some() }
    }
}
