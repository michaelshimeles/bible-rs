//! Windows: asking GitHub over HTTPS, and handing the update to its installer.

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::download::{self, Net};
use super::release::{self, Error};
use super::{CURRENT, Progress, Status, repaint, shared};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// The installer puts its uninstaller beside the app (packaging/windows/installer.nsi).
const UNINSTALLER: &str = "Uninstall Bible.exe";

/// The folder the installer put this copy in. `None` for a copy it didn't (a build run in place),
/// which can't update itself.
pub fn install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(UNINSTALLER).is_file().then(|| dir.to_path_buf())
}

/// Where installers are downloaded: `%LOCALAPPDATA%\bible-rs\updates`.
fn downloads() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("bible-rs").join("updates")
}

/// At launch: deletes the installer the last update left, then checks GitHub if the last check
/// was a day ago or more.
pub fn start() {
    let _ = std::fs::remove_dir_all(downloads());
    if install_dir().is_none() {
        return;
    }
    let stamp = crate::data_dir().join("update-check");
    let due = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .map_or(true, |t| t.elapsed().map_or(true, |age| age >= DAY));
    if due {
        check(false, false);
    }
}

/// Asks GitHub for the latest release on a background thread. `asked`: the user chose "Check for
/// Updates…", so every answer shows; otherwise only a newer version does.
pub fn check(asked: bool, sheet: bool) {
    let run = {
        let mut s = shared();
        if asked { s.show(Status::Checking, sheet) } else { s.run }
    };
    spawn("update-check", move || {
        let latest = download::latest(&Github::new());
        if latest.is_ok() {
            let _ = std::fs::write(crate::data_dir().join("update-check"), CURRENT);
        }
        let answer = latest.and_then(|r| release::pick(&r, CURRENT));
        if let Err(e) = &answer {
            eprintln!("update check: {e:?}");
        }
        shared().checked(run, asked, answer);
        repaint();
    });
}

/// Update: downloads the installer on offer, checks it, starts it and lets the app quit.
pub fn download() {
    let mut s = shared();
    let Some(update) = s.update.clone() else { return };
    if s.busy() {
        return;
    }
    let sheet = s.sheet;
    let Some(dir) = install_dir() else {
        s.show(Status::Unsupported, sheet);
        return;
    };
    let progress = Arc::new(Progress::new(update.size));
    let run =
        s.show(Status::Downloading { version: update.version.clone(), progress: progress.clone() }, sheet);
    drop(s);
    spawn("update-download", move || {
        let stop = || shared().run != run;
        let result = download::fetch(&Github::new(), &update, &downloads(), &progress, &stop)
            .and_then(|installer| if stop() { Err(Error::Cancelled) } else { launch(&installer, &dir) });
        if let Err(e) = &result {
            eprintln!("update: {e:?}");
        }
        shared().downloaded(run, result);
        repaint();
    });
}

/// Starts the installer: silently (`/S`), into this copy's folder (`/D=`, which NSIS wants last
/// and unquoted), opening the app again when done (`/RELAUNCH`). It waits for the app to quit
/// before replacing it.
fn launch(installer: &Path, dir: &Path) -> Result<(), Error> {
    Command::new(installer)
        .args(["/S", "/RELAUNCH"])
        .raw_arg(format!("/D={}", dir.display()))
        .spawn()
        .map(drop)
        .map_err(|e| Error::Launch(e.to_string()))
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(f);
}

/// GitHub over HTTPS only, redirects included. A download may take minutes; connecting and the
/// wait for an answer may not.
struct Github(ureq::Agent);

impl Github {
    fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(30 * 60)))
            .user_agent(concat!("bible-rs/", env!("CARGO_PKG_VERSION")))
            .build();
        Github(config.into())
    }

    fn get(&self, url: &str) -> Result<ureq::Body, Error> {
        let mut request = self.0.get(url);
        if url == release::LATEST {
            request = request.header("Accept", "application/vnd.github+json");
        }
        let response = request.call().map_err(offline)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Server(status.as_u16()));
        }
        Ok(response.into_body())
    }
}

fn offline(e: ureq::Error) -> Error {
    Error::Offline(e.to_string())
}

impl Net for Github {
    fn text(&self, url: &str, limit: u64) -> Result<String, Error> {
        self.get(url)?.into_with_config().limit(limit).read_to_string().map_err(offline)
    }

    fn body(&self, url: &str) -> Result<Box<dyn Read>, Error> {
        Ok(Box::new(self.get(url)?.into_reader()))
    }
}
