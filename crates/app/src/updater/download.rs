//! Asking GitHub for the latest release, and downloading an update's installer and checking it
//! before anything runs it.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use super::Progress;
use super::release::{Error, LATEST, MAX_SUMS, Release, Update, sum_for};

/// GitHub's answer about a release is a few kilobytes per file it lists.
const MAX_ANSWER: u64 = 1024 * 1024;

/// Numbers each download's temporary file. A download the user has left can still be waiting
/// for GitHub when the next one starts, and must not write over or delete that one's file.
static DOWNLOADS: AtomicU64 = AtomicU64::new(0);

/// HTTPS requests, behind a trait so the tests can answer them.
pub trait Net {
    /// The body of `url` as text, refusing one longer than `limit` bytes.
    fn text(&self, url: &str, limit: u64) -> Result<String, Error>;
    /// The body of `url`, to read as it arrives.
    fn body(&self, url: &str) -> Result<Box<dyn Read>, Error>;
}

/// The latest release, from GitHub's API.
pub fn latest(net: &dyn Net) -> Result<Release, Error> {
    serde_json::from_str(&net.text(LATEST, MAX_ANSWER)?).map_err(|_| Error::Unreadable)
}

/// Downloads `update`'s installer into `dir` and returns its path once its size and SHA-256 are
/// the ones the release gives; otherwise it leaves no file behind. `stop` is asked between reads,
/// and ends the download with [`Error::Cancelled`] when it says so.
pub fn fetch(
    net: &dyn Net,
    update: &Update,
    dir: &Path,
    progress: &Progress,
    stop: &dyn Fn() -> bool,
) -> Result<PathBuf, Error> {
    let sha256 = expected_sha256(net, update)?;
    fs::create_dir_all(dir).map_err(disk)?;
    let path = dir.join(&update.installer);
    let n = DOWNLOADS.fetch_add(1, Ordering::Relaxed);
    let part = dir.join(format!("{}.{}-{n}.part", update.installer, std::process::id()));
    let saved = save(net, update, &part, sha256, progress, stop)
        .and_then(|()| fs::rename(&part, &path).map_err(disk));
    if saved.is_err() {
        let _ = fs::remove_file(&part);
    }
    saved.map(|()| path)
}

/// The installer's SHA-256: GitHub's, and the release's checksum list's, which must agree when
/// there are both.
fn expected_sha256(net: &dyn Net, update: &Update) -> Result<[u8; 32], Error> {
    let listed = match &update.sums_url {
        Some(url) => Some(
            sum_for(&net.text(url, MAX_SUMS)?, &update.installer)
                .ok_or_else(|| Error::NoChecksum(update.version.clone()))?,
        ),
        None => None,
    };
    match (update.sha256, listed) {
        (Some(github), Some(listed)) if github != listed => Err(Error::Mismatch),
        (Some(sha256), _) | (None, Some(sha256)) => Ok(sha256),
        (None, None) => Err(Error::NoChecksum(update.version.clone())),
    }
}

/// Writes the installer to `part`, hashing it as it arrives.
fn save(
    net: &dyn Net,
    update: &Update,
    part: &Path,
    sha256: [u8; 32],
    progress: &Progress,
    stop: &dyn Fn() -> bool,
) -> Result<(), Error> {
    let mut body = net.body(&update.url)?;
    if stop() {
        return Err(Error::Cancelled);
    }
    let mut file = File::create(part).map_err(disk)?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    let mut done = 0;
    loop {
        if stop() {
            return Err(Error::Cancelled);
        }
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("update download: {e}");
                return Err(Error::Incomplete);
            }
        };
        done += n as u64;
        if done > update.size {
            return Err(Error::Mismatch);
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(disk)?;
        progress.set(done);
    }
    if done < update.size {
        return Err(Error::Incomplete);
    }
    if hash.finalize()[..] != sha256 {
        return Err(Error::Mismatch);
    }
    file.sync_all().map_err(disk)
}

fn disk(e: io::Error) -> Error {
    Error::Disk(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const EXE: &str = "bible-rs-0.3.0-windows-x64-setup.exe";
    const EXE_URL: &str = "https://github.com/michaelshimeles/bible-rs/releases/download/v0.3.0/bible-rs-0.3.0-windows-x64-setup.exe";
    const SUMS_URL: &str = "https://github.com/michaelshimeles/bible-rs/releases/download/v0.3.0/bible-rs-0.3.0-windows-x64-SHA256SUMS.txt";

    /// Answers from memory, `chunk` bytes per read, and can break the connection after `fail_at`
    /// bytes of a body.
    struct Mock {
        files: HashMap<&'static str, Vec<u8>>,
        chunk: usize,
        fail_at: Option<usize>,
        asked: RefCell<Vec<String>>,
    }

    impl Mock {
        fn new(files: &[(&'static str, Vec<u8>)]) -> Self {
            Mock {
                files: files.iter().cloned().collect(),
                chunk: 4096,
                fail_at: None,
                asked: RefCell::default(),
            }
        }

        fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
            self.asked.borrow_mut().push(url.to_string());
            self.files.get(url).cloned().ok_or(Error::Server(404))
        }
    }

    impl Net for Mock {
        fn text(&self, url: &str, limit: u64) -> Result<String, Error> {
            let bytes = self.get(url)?;
            if bytes.len() as u64 > limit {
                return Err(Error::Unreadable);
            }
            String::from_utf8(bytes).map_err(|_| Error::Unreadable)
        }

        fn body(&self, url: &str) -> Result<Box<dyn Read>, Error> {
            Ok(Box::new(Chunks { data: self.get(url)?, at: 0, chunk: self.chunk, fail_at: self.fail_at }))
        }
    }

    struct Chunks {
        data: Vec<u8>,
        at: usize,
        chunk: usize,
        fail_at: Option<usize>,
    }

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.fail_at.is_some_and(|f| self.at >= f) {
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset"));
            }
            let n = self.chunk.min(buf.len()).min(self.data.len() - self.at);
            buf[..n].copy_from_slice(&self.data[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    type Hook = Box<dyn FnOnce()>;

    /// Answers like `net`, and runs `hook` once: before sending the body, or after its first
    /// chunk.
    struct Hooked {
        net: Mock,
        before_body: bool,
        hook: RefCell<Option<Hook>>,
    }

    impl Net for Hooked {
        fn text(&self, url: &str, limit: u64) -> Result<String, Error> {
            self.net.text(url, limit)
        }

        fn body(&self, url: &str) -> Result<Box<dyn Read>, Error> {
            let hook = self.hook.borrow_mut().take();
            if self.before_body {
                hook.into_iter().for_each(|h| h());
                return self.net.body(url);
            }
            Ok(Box::new(AfterFirstRead { inner: self.net.body(url)?, hook }))
        }
    }

    struct AfterFirstRead {
        inner: Box<dyn Read>,
        hook: Option<Hook>,
    }

    impl Read for AfterFirstRead {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.hook.take().into_iter().for_each(|h| h());
            Ok(n)
        }
    }

    fn installer() -> Vec<u8> {
        (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn sha256(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn hex(h: [u8; 32]) -> String {
        h.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn update(sha256: Option<[u8; 32]>, sums: bool) -> Update {
        Update {
            version: "0.3.0".into(),
            installer: EXE.into(),
            url: EXE_URL.into(),
            size: installer().len() as u64,
            sha256,
            sums_url: sums.then(|| SUMS_URL.into()),
        }
    }

    fn sums(h: [u8; 32]) -> Vec<u8> {
        format!("{}  bible-rs-0.3.0-universal.dmg\n{}  {EXE}\n", hex([1; 32]), hex(h)).into_bytes()
    }

    /// A fresh, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bible-rs-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn the_latest_release_is_read_from_githubs_answer() {
        let answer = format!(
            r#"{{"tag_name":"v0.3.0","html_url":"https://github.com/x","draft":false,
                "assets":[{{"name":"{EXE}","browser_download_url":"{EXE_URL}","size":9,
                "digest":"sha256:00","content_type":"application/octet-stream"}}]}}"#
        );
        let r = latest(&Mock::new(&[(LATEST, answer.into_bytes())])).unwrap();
        assert_eq!((r.tag_name.as_str(), r.assets.len()), ("v0.3.0", 1));
        assert_eq!((r.assets[0].name.as_str(), r.assets[0].size), (EXE, 9));
        assert_eq!(
            latest(&Mock::new(&[(LATEST, b"<html>Service unavailable</html>".to_vec())])).err(),
            Some(Error::Unreadable)
        );
        assert_eq!(latest(&Mock::new(&[])).err(), Some(Error::Server(404)));
    }

    #[test]
    fn a_download_that_matches_its_checksums_is_kept() {
        let data = installer();
        let net = Mock::new(&[(EXE_URL, data.clone()), (SUMS_URL, sums(sha256(&data)))]);
        let dir = folder("ok");
        let progress = Progress::new(data.len() as u64);
        let path = fetch(&net, &update(Some(sha256(&data)), true), &dir, &progress, &|| false).unwrap();
        assert_eq!(path, dir.join(EXE));
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(files_in(&dir), [EXE]);
        assert_eq!((progress.done(), progress.total()), (data.len() as u64, data.len() as u64));
        // The checksum list is read before the installer.
        assert_eq!(*net.asked.borrow(), [SUMS_URL, EXE_URL]);
        // Either checksum alone will do.
        for (sha, list) in [(Some(sha256(&data)), false), (None, true)] {
            let dir = folder("one-checksum");
            fetch(&net, &update(sha, list), &dir, &Progress::new(0), &|| false).unwrap();
            assert_eq!(files_in(&dir), [EXE]);
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_download_with_another_checksum_is_deleted() {
        let data = installer();
        let mut tampered = data.clone();
        tampered[1234] ^= 1;
        let net = Mock::new(&[(EXE_URL, tampered)]);
        let dir = folder("tampered");
        let r = fetch(&net, &update(Some(sha256(&data)), false), &dir, &Progress::new(0), &|| false);
        assert_eq!(r, Err(Error::Mismatch));
        assert!(files_in(&dir).is_empty());
    }

    #[test]
    fn a_download_shorter_or_longer_than_listed_is_deleted() {
        let data = installer();
        let net = Mock::new(&[(EXE_URL, data.clone())]);
        for (size, error) in
            [(data.len() as u64 + 1, Error::Incomplete), (data.len() as u64 - 1, Error::Mismatch)]
        {
            let dir = folder("size");
            let u = Update { size, ..update(Some(sha256(&data)), false) };
            assert_eq!(fetch(&net, &u, &dir, &Progress::new(size), &|| false), Err(error));
            assert!(files_in(&dir).is_empty());
        }
    }

    #[test]
    fn the_checksum_list_must_agree_with_github() {
        let data = installer();
        let dir = folder("disagree");
        let net = Mock::new(&[(EXE_URL, data.clone()), (SUMS_URL, sums([9; 32]))]);
        let r = fetch(&net, &update(Some(sha256(&data)), true), &dir, &Progress::new(0), &|| false);
        assert_eq!(r, Err(Error::Mismatch));
        // Nothing was downloaded.
        assert_eq!(*net.asked.borrow(), [SUMS_URL]);
        assert!(files_in(&dir).is_empty());

        let unlisted = Mock::new(&[(EXE_URL, data.clone()), (SUMS_URL, b"abc  other.exe\n".to_vec())]);
        let r = fetch(&unlisted, &update(None, true), &dir, &Progress::new(0), &|| false);
        assert_eq!(r, Err(Error::NoChecksum("0.3.0".into())));
        let r = fetch(&unlisted, &update(None, false), &dir, &Progress::new(0), &|| false);
        assert_eq!(r, Err(Error::NoChecksum("0.3.0".into())));
        assert!(files_in(&dir).is_empty());
    }

    #[test]
    fn a_broken_connection_leaves_nothing_behind() {
        let data = installer();
        let mut net = Mock::new(&[(EXE_URL, data.clone())]);
        net.fail_at = Some(24 * 4096);
        let dir = folder("broken");
        let progress = Progress::new(data.len() as u64);
        let r = fetch(&net, &update(Some(sha256(&data)), false), &dir, &progress, &|| false);
        assert_eq!(r, Err(Error::Incomplete));
        assert!(files_in(&dir).is_empty());
        assert_eq!(progress.done(), 24 * 4096);

        let missing = Mock::new(&[]);
        let r = fetch(&missing, &update(Some(sha256(&data)), false), &dir, &Progress::new(0), &|| false);
        assert_eq!(r, Err(Error::Server(404)));
        assert!(files_in(&dir).is_empty());
    }

    #[test]
    fn leaving_the_download_stops_it() {
        let data = installer();
        let net = Mock::new(&[(EXE_URL, data.clone())]);
        let dir = folder("cancel");
        let reads = std::cell::Cell::new(0);
        let stop = || {
            reads.set(reads.get() + 1);
            reads.get() > 3
        };
        let r = fetch(&net, &update(Some(sha256(&data)), false), &dir, &Progress::new(0), &stop);
        assert_eq!(r, Err(Error::Cancelled));
        assert!(files_in(&dir).is_empty());
    }

    #[test]
    fn a_download_the_user_left_cannot_spoil_the_next_one() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, mpsc};

        let data = installer();
        let dir = folder("overlap");
        let left = Arc::new(AtomicBool::new(false));
        let (go_on, wait) = mpsc::channel();
        let (ended, end) = mpsc::channel();
        // GitHub is slow to answer the first download…
        let first = {
            let (data, dir, left) = (data.clone(), dir.clone(), left.clone());
            std::thread::spawn(move || {
                let hook: Hook = Box::new(move || wait.recv().unwrap());
                let net = Hooked {
                    net: Mock::new(&[(EXE_URL, data.clone())]),
                    before_body: true,
                    hook: RefCell::new(Some(hook)),
                };
                let stop = || left.load(Ordering::SeqCst);
                let r = fetch(&net, &update(Some(sha256(&data)), false), &dir, &Progress::new(0), &stop);
                ended.send(()).unwrap();
                r
            })
        };
        // …so the user leaves it and starts another, and the first answer arrives while the second
        // is downloading.
        left.store(true, Ordering::SeqCst);
        let hook: Hook = Box::new(move || {
            go_on.send(()).unwrap();
            end.recv().unwrap();
        });
        let net = Hooked {
            net: Mock::new(&[(EXE_URL, data.clone())]),
            before_body: false,
            hook: RefCell::new(Some(hook)),
        };
        let second = fetch(&net, &update(Some(sha256(&data)), false), &dir, &Progress::new(0), &|| false);
        assert_eq!(first.join().unwrap(), Err(Error::Cancelled));
        assert_eq!(second, Ok(dir.join(EXE)));
        assert_eq!(std::fs::read(dir.join(EXE)).unwrap(), data);
        assert_eq!(files_in(&dir), [EXE]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
