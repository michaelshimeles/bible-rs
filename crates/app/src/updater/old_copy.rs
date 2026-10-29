//! Up to 0.1.0 the bundle was `bible-rs.app`. Dragging `Bible.app` from the disk image into
//! Applications leaves that copy beside it, so the new one moves it to the Trash.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use objc2_foundation::{NSBundle, NSFileManager, NSString, NSURL};

const OLD_NAME: &str = "bible-rs.app";

pub fn retire() {
    objc2::rc::autoreleasepool(|_| {
        let Some(old) = find() else { return };
        let url = NSURL::fileURLWithPath(&NSString::from_str(&old.to_string_lossy()));
        let _ = NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None);
    });
}

/// The old copy, when this app runs from an Applications folder that also holds one with the
/// same bundle identifier.
fn find() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // …/Bible.app/Contents/MacOS/Bible
    let app = exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|e| e == "app"))?;
    let dir = app.parent()?;
    if dir != Path::new("/Applications") && dir != dirs::home_dir()?.join("Applications") {
        return None;
    }
    let old = dir.join(OLD_NAME);
    let (ours, theirs) = (std::fs::metadata(app).ok()?, std::fs::metadata(&old).ok()?);
    if (ours.dev(), ours.ino()) == (theirs.dev(), theirs.ino()) {
        return None;
    }
    let id = |path: &Path| NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?.bundleIdentifier();
    let running = NSBundle::mainBundle().bundleIdentifier()?;
    (id(&old)? == running).then_some(old)
}
