//! Updating the installed app, from inside it: no step opens a web page.
//!
//! On macOS that is Sparkle ([`sparkle`]), loaded once the first frame is on screen so it adds
//! nothing to launch. Its window installs an update with one click.
//!
//! On Windows the app asks GitHub for the latest release, at most once a day and whenever "Check
//! for Updates…" is chosen. A newer one is offered in the app (`crate::update_ui`). Update
//! downloads its installer, checks its size and SHA-256, runs it silently and quits, and the
//! installer opens the new version (WINDOWS.md). Only a copy the installer put on the PC updates
//! itself.
//!
//! A build that can't update itself (no Sparkle, a Windows copy run in place, Linux) says so in
//! the app instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use eframe::egui;

#[cfg(any(windows, test))]
mod download;
#[cfg(target_os = "macos")]
mod old_copy;
pub mod release;
#[cfg(target_os = "macos")]
mod sparkle;
#[cfg(windows)]
mod windows;

/// The project's releases on GitHub (Settings › About › Release Notes).
pub const RELEASES: &str = "https://github.com/michaelshimeles/bible-rs/releases";
/// The running version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Where updating stands, as the prompt shows it.
#[derive(Clone, Debug, Default)]
pub enum Status {
    /// Nothing to show.
    #[default]
    Idle,
    /// Asking GitHub for the latest release.
    Checking,
    /// The running version is the newest.
    UpToDate,
    /// A newer version, waiting for Update or Later.
    Available { version: String },
    /// Downloading the new version's installer and checking it.
    Downloading { version: String, progress: Arc<Progress> },
    /// The installer is running: the app quits, and the installer opens the new version.
    Installing { version: String },
    /// A check or a download failed: what happened, in plain words, and what Retry does.
    Failed { message: String, retry: Retry },
    /// This build can't update itself.
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Check,
    Download,
}

/// A download's progress, in bytes.
#[derive(Debug)]
pub struct Progress {
    done: AtomicU64,
    total: u64,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Progress { done: AtomicU64::new(0), total }
    }

    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    fn set(&self, done: u64) {
        self.done.store(done, Ordering::Relaxed);
    }
}

/// The updater's state, shared by the main window, Settings › About and the background threads.
struct Shared {
    status: Status,
    /// Show the status over the main window too, not only on the About page: it answers the
    /// menu's "Check for Updates…", or the daily check found a newer version.
    sheet: bool,
    /// The installer is running, so the app should quit.
    quit: bool,
    /// Bumped whenever the status is replaced, so the answer of a check or download the user has
    /// left is dropped.
    run: u64,
    /// For repainting when a background answer arrives.
    ctx: Option<egui::Context>,
    /// The update on offer, for Update and Retry.
    #[cfg(any(windows, test))]
    update: Option<Arc<release::Update>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared::new());

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Asks for a repaint, so an answer from a background thread shows.
#[cfg(windows)]
fn repaint() {
    let ctx = shared().ctx.clone();
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
}

impl Shared {
    const fn new() -> Self {
        Shared {
            status: Status::Idle,
            sheet: false,
            quit: false,
            run: 0,
            ctx: None,
            #[cfg(any(windows, test))]
            update: None,
        }
    }

    /// Replaces the status, leaving whatever was under way; returns the new run.
    fn show(&mut self, status: Status, sheet: bool) -> u64 {
        self.run += 1;
        self.status = status;
        self.sheet = sheet;
        self.run
    }

    /// Checking, downloading or installing.
    fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Downloading { .. } | Status::Installing { .. })
    }

    /// Later, OK, Close or Cancel. An installer already running can't be called back.
    fn dismiss(&mut self) {
        if !matches!(self.status, Status::Installing { .. }) {
            self.show(Status::Idle, false);
        }
    }

    /// A check's answer. One the user didn't ask for shows only a newer version.
    #[cfg(any(windows, test))]
    fn checked(&mut self, run: u64, asked: bool, answer: Result<Option<release::Update>, release::Error>) {
        if run != self.run {
            return;
        }
        self.status = match answer {
            Ok(Some(update)) => {
                let version = update.version.clone();
                self.update = Some(Arc::new(update));
                self.sheet |= !asked;
                Status::Available { version }
            }
            Ok(None) if asked => Status::UpToDate,
            Err(e) if asked => Status::Failed { message: e.to_string(), retry: Retry::Check },
            _ => Status::Idle,
        };
    }

    /// A download's end: the installer is running (even if the user has just left the
    /// download, as it can't be stopped then), or what went wrong.
    #[cfg(any(windows, test))]
    fn downloaded(&mut self, run: u64, result: Result<(), release::Error>) {
        let version = self.update.as_ref().map(|u| u.version.clone()).unwrap_or_default();
        match result {
            Ok(()) => {
                self.status = Status::Installing { version };
                self.quit = true;
            }
            Err(e) if run == self.run => {
                self.status = Status::Failed { message: e.to_string(), retry: Retry::Download };
            }
            Err(_) => {}
        }
    }
}

/// Starts updating in the background. macOS: Sparkle, which checks once a day without asking
/// first (`SUEnableAutomaticChecks`), and moves a copy of the app left under its old name to the
/// Trash. Windows: a check of GitHub if the last one was a day ago or more. Main thread; later
/// calls do nothing.
pub fn start(ctx: &egui::Context) {
    shared().ctx = Some(ctx.clone());
    #[cfg(target_os = "macos")]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            sparkle::start();
            let _ = std::thread::Builder::new().name("old-copy".into()).spawn(old_copy::retire);
        });
    }
    #[cfg(windows)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(windows::start);
    }
}

/// "Check for Updates…": Sparkle's window where Sparkle runs. Otherwise the answer shows in the
/// app: over the main window when `sheet` (the menu), else on the About page only. While a check
/// or a download is under way it carries on, shown over the main window too when `sheet`.
pub fn check_for_updates(sheet: bool) {
    {
        let mut s = shared();
        if s.busy() {
            s.sheet |= sheet;
            return;
        }
    }
    #[cfg(target_os = "macos")]
    if sparkle::check() {
        return;
    }
    #[cfg(windows)]
    if windows::install_dir().is_some() {
        windows::check(true, sheet);
        return;
    }
    shared().show(Status::Unsupported, sheet);
}

/// What the prompt shows.
pub fn status() -> Status {
    shared().status.clone()
}

/// Whether a check or a download is under way.
pub fn busy() -> bool {
    shared().busy()
}

/// Whether the main window shows the prompt as a sheet.
pub fn sheet_open() -> bool {
    let s = shared();
    s.sheet && !matches!(s.status, Status::Idle)
}

/// Update: downloads the new version's installer, checks it, starts it and quits (Windows).
pub fn update() {
    #[cfg(windows)]
    windows::download();
}

/// Retry: the check or the download again.
pub fn retry() {
    let (retry, sheet) = {
        let s = shared();
        match s.status {
            Status::Failed { retry, .. } => (retry, s.sheet),
            _ => return,
        }
    };
    match retry {
        Retry::Check => check_for_updates(sheet),
        Retry::Download => update(),
    }
}

/// Later, OK, Close or Cancel: hides the prompt and drops what was under way.
pub fn dismiss() {
    shared().dismiss();
}

/// Whether the app should quit now, for the installer to replace it. Asks once.
pub fn take_quit() -> bool {
    std::mem::take(&mut shared().quit)
}

/// Screenshots (`BIBLE_RS_DEMO`): puts the prompt in `state` (`checking`, `current`, `available`,
/// `downloading`, `installing`, `failed-check`, `failed-download` or `unsupported`), over the
/// main window when `sheet`, else on the About page only.
pub fn demo(state: &str, sheet: bool) {
    let version = "0.3.0".to_string();
    let status = match state {
        "checking" => Status::Checking,
        "current" => Status::UpToDate,
        "available" => Status::Available { version },
        "downloading" => {
            let progress = Progress::new(11_800_000);
            progress.set(4_700_000);
            Status::Downloading { version, progress: Arc::new(progress) }
        }
        "installing" => Status::Installing { version },
        "failed-check" => Status::Failed {
            message: release::Error::Offline(String::new()).to_string(),
            retry: Retry::Check,
        },
        "failed-download" => {
            Status::Failed { message: release::Error::Mismatch.to_string(), retry: Retry::Download }
        }
        "unsupported" => Status::Unsupported,
        _ => {
            eprintln!("BIBLE_RS_DEMO: unknown update state {state:?}");
            return;
        }
    };
    shared().show(status, sheet);
}

#[cfg(test)]
mod tests {
    use super::release::{Error, Update};
    use super::*;

    fn update() -> Update {
        Update {
            version: "0.3.0".into(),
            installer: "bible-rs-0.3.0-windows-x64-setup.exe".into(),
            url: format!("{}/v0.3.0/bible-rs-0.3.0-windows-x64-setup.exe", release::DOWNLOADS),
            size: 9_000_000,
            sha256: Some([7; 32]),
            sums_url: None,
        }
    }

    #[test]
    fn the_daily_check_shows_only_a_newer_version() {
        let mut s = Shared::new();
        for quiet in
            [Ok(None), Err(Error::Offline("timed out".into())), Err(Error::NoInstaller("0.3.0".into()))]
        {
            s.checked(s.run, false, quiet);
            assert!(matches!(s.status, Status::Idle) && !s.sheet);
        }
        s.checked(s.run, false, Ok(Some(update())));
        assert!(matches!(&s.status, Status::Available { version } if version == "0.3.0"));
        assert!(s.sheet, "a newer version shows over the main window");
        assert_eq!(s.update.as_deref(), Some(&update()));
    }

    #[test]
    fn asking_shows_every_answer_where_it_was_asked() {
        let mut s = Shared::new();
        let run = s.show(Status::Checking, false);
        s.checked(run, true, Ok(None));
        assert!(matches!(s.status, Status::UpToDate) && !s.sheet);
        let run = s.show(Status::Checking, true);
        s.checked(run, true, Err(Error::Server(503)));
        assert!(
            matches!(&s.status, Status::Failed { message, retry: Retry::Check } if message.contains("503")),
            "{:?}",
            s.status
        );
        assert!(s.sheet);
    }

    #[test]
    fn an_answer_after_later_is_dropped() {
        let mut s = Shared::new();
        let run = s.show(Status::Checking, true);
        s.dismiss();
        s.checked(run, true, Ok(Some(update())));
        assert!(matches!(s.status, Status::Idle) && !s.sheet);
    }

    #[test]
    fn a_failed_download_offers_retry_and_a_finished_one_quits() {
        let mut s = Shared::new();
        s.update = Some(Arc::new(update()));
        let downloading =
            || Status::Downloading { version: "0.3.0".into(), progress: Arc::new(Progress::new(9)) };
        let run = s.show(downloading(), true);
        s.downloaded(run, Err(Error::Mismatch));
        assert!(matches!(s.status, Status::Failed { retry: Retry::Download, .. }));
        assert!(!s.quit);
        // Cancelled: the failure of a download the user left isn't shown.
        let run = s.show(downloading(), true);
        s.dismiss();
        s.downloaded(run, Err(Error::Cancelled));
        assert!(matches!(s.status, Status::Idle));
        let run = s.show(downloading(), true);
        s.downloaded(run, Ok(()));
        assert!(matches!(&s.status, Status::Installing { version } if version == "0.3.0"));
        assert!(s.quit);
        // The installer is running; the prompt stays until the app quits.
        s.dismiss();
        assert!(matches!(s.status, Status::Installing { .. }));
    }

    #[test]
    fn what_went_wrong_is_said_in_plain_words_without_links() {
        let errors = [
            Error::Offline("tls handshake eof".into()),
            Error::Server(502),
            Error::Unreadable,
            Error::NoInstaller("0.3.0".into()),
            Error::Unexpected("0.3.0".into()),
            Error::NoChecksum("0.3.0".into()),
            Error::Incomplete,
            Error::Mismatch,
            Error::Disk("There is not enough space on the disk. (os error 112)".into()),
            Error::Launch("Access is denied. (os error 5)".into()),
        ];
        for e in errors {
            let words = e.to_string();
            assert!(!words.contains("http") && !words.contains("github.com"), "{words}");
            assert!(!words.contains("tls"), "{words}");
            assert!(words.ends_with('.'), "{words}");
        }
    }

    /// Before, this opened the latest release's page in the browser.
    #[cfg(not(windows))]
    #[test]
    fn a_build_that_cant_update_itself_says_so_in_the_app() {
        check_for_updates(true);
        assert!(matches!(status(), Status::Unsupported));
        assert!(sheet_open());
        dismiss();
        assert!(!sheet_open());

        // Checking again from the menu while a download runs shows it rather than stopping it.
        demo("downloading", false);
        check_for_updates(true);
        assert!(matches!(status(), Status::Downloading { .. }) && busy() && sheet_open());
        dismiss();
        assert!(matches!(status(), Status::Idle));
    }
}
