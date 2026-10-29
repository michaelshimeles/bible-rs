//! The latest release as GitHub describes it, and which of its files a Windows update takes: the
//! installer, from this repository's release downloads only, with a SHA-256 to check it against.

use std::fmt;

/// Every file of a release is downloaded from here: `<DOWNLOADS>/<tag>/<file name>`.
pub const DOWNLOADS: &str = "https://github.com/michaelshimeles/bible-rs/releases/download";
/// The latest published release (drafts and pre-releases never are).
pub const LATEST: &str = "https://api.github.com/repos/michaelshimeles/bible-rs/releases/latest";

/// Larger than any installer or checksum list should be; a bigger one isn't downloaded.
const MAX_INSTALLER: u64 = 512 * 1024 * 1024;
pub const MAX_SUMS: u64 = 64 * 1024;

/// The latest release, as much of GitHub's answer as the update reads.
#[derive(serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    /// `sha256:<hex>`, which GitHub works out for every file it is given.
    #[serde(default)]
    pub digest: Option<String>,
}

/// A newer version and the files that install it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    /// "0.3.0".
    pub version: String,
    /// `bible-rs-<version>-windows-x64-setup.exe` (`scripts/package-windows.sh`).
    pub installer: String,
    pub url: String,
    pub size: u64,
    /// The installer's SHA-256 as GitHub reports it.
    pub sha256: Option<[u8; 32]>,
    /// `bible-rs-<version>-windows-x64-SHA256SUMS.txt`, when the release has it.
    pub sums_url: Option<String>,
}

/// Why there is no update to install, in words for the person using the app.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// No connection, or it broke off. The detail is for the log.
    Offline(String),
    /// GitHub answered with this HTTP status.
    Server(u16),
    /// GitHub's answer wasn't a release.
    Unreadable,
    /// The newer version has no Windows installer.
    NoInstaller(String),
    /// The installer isn't at this repository's release downloads, or its size is implausible.
    Unexpected(String),
    /// Neither GitHub nor the release's checksum list gives the installer's SHA-256.
    NoChecksum(String),
    /// The download ended early.
    Incomplete,
    /// The download's size or SHA-256 isn't the expected one.
    Mismatch,
    /// Saving the download failed.
    Disk(String),
    /// The installer wouldn't start.
    Launch(String),
    /// The person using the app left the download.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Offline(_) => {
                write!(
                    f,
                    "Bible couldn’t reach GitHub, where updates come from. Check your internet connection."
                )
            }
            Error::Server(code) => {
                write!(f, "GitHub didn’t answer as expected (error {code}). Try again in a few minutes.")
            }
            Error::Unreadable => write!(f, "GitHub’s answer about the newest version didn’t make sense."),
            Error::NoInstaller(v) => {
                write!(f, "Bible {v} is out, but its Windows installer isn’t ready yet. Try again later.")
            }
            Error::Unexpected(v) => {
                write!(
                    f,
                    "The installer for Bible {v} isn’t where Bible expects it, so it wasn’t downloaded."
                )
            }
            Error::NoChecksum(v) => write!(
                f,
                "The installer for Bible {v} came without a checksum, so Bible can’t check it and won’t \
                 install it."
            ),
            Error::Incomplete => write!(f, "The download stopped before it finished."),
            Error::Mismatch => {
                write!(
                    f,
                    "The download wasn’t the file GitHub listed, so Bible deleted it without installing it."
                )
            }
            Error::Disk(e) => write!(f, "Bible couldn’t save the update: {e}."),
            Error::Launch(e) => write!(f, "Bible couldn’t start the installer: {e}."),
            Error::Cancelled => write!(f, "The update was cancelled."),
        }
    }
}

/// "v1.2.3" or "1.2.3" (a "-beta" suffix ignored) as numbers.
pub fn version(tag: &str) -> Option<(u64, u64, u64)> {
    let core = tag.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(v)
}

/// The update `release` offers a copy running `current`: none when it isn't newer, else its
/// installer, which must be in the release under its usual name at this repository's downloads,
/// with a SHA-256 from GitHub or the release's checksum list.
pub fn pick(release: &Release, current: &str) -> Result<Option<Update>, Error> {
    let tag = release.tag_name.as_str();
    let (Some(latest), Some(running)) = (version(tag), version(current)) else { return Ok(None) };
    if latest <= running {
        return Ok(None);
    }
    let v = tag.trim().trim_start_matches('v').to_string();
    // The version goes into a file name.
    if !v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')) {
        return Err(Error::Unexpected(v));
    }
    let find = |name: String| release.assets.iter().find(|a| a.name == name);
    let ours = |a: &Asset, max: u64| {
        a.browser_download_url == format!("{DOWNLOADS}/{tag}/{}", a.name) && (1..=max).contains(&a.size)
    };
    let Some(installer) = find(format!("bible-rs-{v}-windows-x64-setup.exe")) else {
        return Err(Error::NoInstaller(v));
    };
    let sums = find(format!("bible-rs-{v}-windows-x64-SHA256SUMS.txt"));
    if !ours(installer, MAX_INSTALLER) || sums.is_some_and(|s| !ours(s, MAX_SUMS)) {
        return Err(Error::Unexpected(v));
    }
    let sha256 = installer.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).and_then(sha256_hex);
    if sha256.is_none() && sums.is_none() {
        return Err(Error::NoChecksum(v));
    }
    Ok(Some(Update {
        version: v,
        installer: installer.name.clone(),
        url: installer.browser_download_url.clone(),
        size: installer.size,
        sha256,
        sums_url: sums.map(|s| s.browser_download_url.clone()),
    }))
}

/// The SHA-256 that a `sha256sum`/`shasum -a 256` listing gives `file`.
pub fn sum_for(list: &str, file: &str) -> Option<[u8; 32]> {
    list.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        // `*` marks a file read in binary mode.
        (name.trim_start().trim_start_matches('*') == file).then(|| sha256_hex(hash)).flatten()
    })
}

/// 64 hex digits as 32 bytes.
fn sha256_hex(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0; 32];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "8e3e68a6337087aefc3bd875bfd34266524b674f292c5b8bd575eb8679902f58";

    fn asset(tag: &str, name: &str, digest: bool) -> Asset {
        Asset {
            name: name.into(),
            browser_download_url: format!("{DOWNLOADS}/{tag}/{name}"),
            size: if name.ends_with(".txt") { 279 } else { 9_000_000 },
            digest: digest.then(|| format!("sha256:{HASH}")),
        }
    }

    fn release(tag: &str, assets: Vec<Asset>) -> Release {
        Release { tag_name: tag.into(), assets }
    }

    fn hash() -> [u8; 32] {
        let mut h = [0; 32];
        for (i, b) in h.iter_mut().enumerate() {
            *b = u8::from_str_radix(&HASH[2 * i..2 * i + 2], 16).unwrap();
        }
        h
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(version("v0.10.0"), Some((0, 10, 0)));
        assert_eq!(version("1.2"), Some((1, 2, 0)));
        assert_eq!(version("v1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(version("nightly"), None);
        assert_eq!(version("1.2.3.4"), None);
        assert!(version("v0.10.0") > version("0.9.9"));
    }

    #[test]
    fn a_newer_release_offers_its_installer_and_checksums() {
        let r = release(
            "v0.3.0",
            vec![
                asset("v0.3.0", "bible-rs-0.3.0-universal.dmg", true),
                asset("v0.3.0", "bible-rs-0.3.0-windows-x64-setup.exe", true),
                asset("v0.3.0", "bible-rs-0.3.0-windows-x64-SHA256SUMS.txt", true),
            ],
        );
        assert_eq!(
            pick(&r, "0.2.0"),
            Ok(Some(Update {
                version: "0.3.0".into(),
                installer: "bible-rs-0.3.0-windows-x64-setup.exe".into(),
                url: format!("{DOWNLOADS}/v0.3.0/bible-rs-0.3.0-windows-x64-setup.exe"),
                size: 9_000_000,
                sha256: Some(hash()),
                sums_url: Some(format!("{DOWNLOADS}/v0.3.0/bible-rs-0.3.0-windows-x64-SHA256SUMS.txt")),
            }))
        );
    }

    #[test]
    fn the_same_or_an_older_release_is_no_update() {
        let r = release("v0.3.0", vec![asset("v0.3.0", "bible-rs-0.3.0-windows-x64-setup.exe", true)]);
        assert_eq!(pick(&r, "0.3.0"), Ok(None));
        assert_eq!(pick(&r, "0.4.1"), Ok(None));
        assert_eq!(pick(&release("latest", vec![]), "0.2.0"), Ok(None));
    }

    #[test]
    fn a_release_without_a_windows_installer_is_never_offered_as_a_web_page() {
        let mac_only = release("v0.3.0", vec![asset("v0.3.0", "bible-rs-0.3.0-universal.dmg", true)]);
        assert_eq!(pick(&mac_only, "0.2.0"), Err(Error::NoInstaller("0.3.0".into())));
        // Another version's installer doesn't count.
        let stale = release("v0.3.0", vec![asset("v0.3.0", "bible-rs-0.2.9-windows-x64-setup.exe", true)]);
        assert_eq!(pick(&stale, "0.2.0"), Err(Error::NoInstaller("0.3.0".into())));
    }

    #[test]
    fn an_installer_from_anywhere_else_is_refused() {
        let name = "bible-rs-0.3.0-windows-x64-setup.exe";
        for url in [
            format!("http://github.com/michaelshimeles/bible-rs/releases/download/v0.3.0/{name}"),
            format!("https://github.com/someone-else/bible-rs/releases/download/v0.3.0/{name}"),
            format!("https://example.com/michaelshimeles/bible-rs/releases/download/v0.3.0/{name}"),
            format!("{DOWNLOADS}/v0.2.0/{name}"),
            format!("{DOWNLOADS}/v0.3.0/other.exe"),
        ] {
            let mut a = asset("v0.3.0", name, true);
            a.browser_download_url = url.clone();
            assert_eq!(
                pick(&release("v0.3.0", vec![a]), "0.2.0"),
                Err(Error::Unexpected("0.3.0".into())),
                "{url}"
            );
        }
        for size in [0, MAX_INSTALLER + 1] {
            let mut a = asset("v0.3.0", name, true);
            a.size = size;
            assert_eq!(pick(&release("v0.3.0", vec![a]), "0.2.0"), Err(Error::Unexpected("0.3.0".into())));
        }
        // A tag that would make an odd file name.
        let odd = release(
            "v0.3.0-a/b",
            vec![asset("v0.3.0-a/b", "bible-rs-0.3.0-a/b-windows-x64-setup.exe", true)],
        );
        assert!(matches!(pick(&odd, "0.2.0"), Err(Error::Unexpected(_))));
    }

    #[test]
    fn an_installer_needs_a_checksum_from_github_or_the_release() {
        let exe = "bible-rs-0.3.0-windows-x64-setup.exe";
        let sums = "bible-rs-0.3.0-windows-x64-SHA256SUMS.txt";
        let none = release("v0.3.0", vec![asset("v0.3.0", exe, false)]);
        assert_eq!(pick(&none, "0.2.0"), Err(Error::NoChecksum("0.3.0".into())));
        // GitHub's digest alone, or the checksum list alone, will do.
        let digest = pick(&release("v0.3.0", vec![asset("v0.3.0", exe, true)]), "0.2.0").unwrap().unwrap();
        assert_eq!((digest.sha256, digest.sums_url), (Some(hash()), None));
        let list = pick(
            &release("v0.3.0", vec![asset("v0.3.0", exe, false), asset("v0.3.0", sums, false)]),
            "0.2.0",
        )
        .unwrap()
        .unwrap();
        assert_eq!((list.sha256, list.sums_url), (None, Some(format!("{DOWNLOADS}/v0.3.0/{sums}"))));
        // Only a SHA-256 counts, and the checksum list must come from the same place.
        let mut md5 = asset("v0.3.0", exe, false);
        md5.digest = Some("md5:0123456789abcdef0123456789abcdef".into());
        assert_eq!(pick(&release("v0.3.0", vec![md5]), "0.2.0"), Err(Error::NoChecksum("0.3.0".into())));
        let mut elsewhere = asset("v0.3.0", sums, false);
        elsewhere.browser_download_url = format!("https://example.com/{sums}");
        assert_eq!(
            pick(&release("v0.3.0", vec![asset("v0.3.0", exe, true), elsewhere]), "0.2.0"),
            Err(Error::Unexpected("0.3.0".into()))
        );
    }

    #[test]
    fn checksum_lists_from_sha256sum_and_shasum_are_read() {
        let file = "bible-rs-0.3.0-windows-x64-setup.exe";
        let other = "0000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(sum_for(&format!("{HASH}  {file}\n"), file), Some(hash()));
        assert_eq!(sum_for(&format!("{HASH} *{file}\r\n"), file), Some(hash()));
        assert_eq!(
            sum_for(&format!("{other}  a.dmg\n{}  {file}\n", HASH.to_uppercase()), file),
            Some(hash())
        );
        assert_eq!(sum_for(&format!("{other}  a.dmg\n"), file), None);
        assert_eq!(sum_for(&format!("{HASH}  {file}.old\n"), file), None);
        assert_eq!(sum_for(&format!("{}  {file}\n", &HASH[1..]), file), None);
        assert_eq!(sum_for(&format!("{}g  {file}\n", &HASH[1..]), file), None);
        assert_eq!(sum_for("", file), None);
    }
}
