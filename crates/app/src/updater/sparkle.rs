//! Sparkle (macOS): `scripts/bundle-macos.sh` embeds Sparkle.framework, and the bundle's Info.plist
//! names the update feed and the key every update must be signed with (RELEASING.md).

use std::cell::OnceCell;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send};
use objc2_foundation::{NSBundle, NSString};

thread_local! {
    /// The `SPUStandardUpdaterController`, kept for the life of the app; `None` without Sparkle.
    static CONTROLLER: OnceCell<Option<Retained<AnyObject>>> = const { OnceCell::new() };
}

/// Runs `f` with the updater controller, loading Sparkle on first use. Main thread only.
fn with<R>(f: impl FnOnce(&AnyObject) -> R) -> Option<R> {
    MainThreadMarker::new()?;
    CONTROLLER.with(|c| c.get_or_init(load).as_deref().map(f))
}

pub fn start() {
    with(|_| ());
}

pub fn check() -> bool {
    // SAFETY: `-[SPUStandardUpdaterController checkForUpdates:]` takes a nullable sender and
    // returns nothing.
    with(|controller| unsafe {
        let () = msg_send![controller, checkForUpdates: None::<&AnyObject>];
    })
    .is_some()
}

fn load() -> Option<Retained<AnyObject>> {
    let frameworks = NSBundle::mainBundle().privateFrameworksPath()?;
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&format!("{frameworks}/Sparkle.framework")))?;
    // SAFETY: the framework is sealed in this app's signature; loading it registers its
    // classes. The hardened runtime refuses it unless it is signed by the app's team, so an
    // ad-hoc signed build says in the app that it can't update itself.
    if !unsafe { bundle.load() } {
        return None;
    }
    let class = AnyClass::get(c"SPUStandardUpdaterController")?;
    // SAFETY: `initWithStartingUpdater:updaterDelegate:userDriverDelegate:` takes a BOOL and two
    // nullable delegates, and returns the initialised controller.
    unsafe {
        let controller: Allocated<AnyObject> = msg_send![class, alloc];
        msg_send![
            controller,
            initWithStartingUpdater: Bool::YES,
            updaterDelegate: None::<&AnyObject>,
            userDriverDelegate: None::<&AnyObject>
        ]
    }
}
