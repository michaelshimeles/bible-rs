//! Opening `bible://` links ([`bible_core::link`]) in the running app.
//!
//! Links arrive from the command line (`bible-rs bible://jhn.3.16`, which is how Linux desktops
//! pass them) and, on macOS, from the system as a `GURL` Apple Event, both while the app is
//! running and when a link launches it. They queue here until the reader can take them: a link
//! that arrives before the first frame, or while the first-launch screens show, opens once the
//! reader is up.

use std::sync::{Mutex, OnceLock};

use bible_core::VerseRef;
use bible_core::link::DeepLink;
use eframe::egui;

static PENDING: Mutex<Vec<DeepLink>> = Mutex::new(Vec::new());
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Queues a link and wakes the app. Returns `false` when `url` isn't a passage.
pub fn open(url: &str) -> bool {
    let Some(link) = bible_core::link::parse(url) else { return false };
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(link);
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
    true
}

/// Lets links that arrive later wake the event loop.
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// The links that arrived since the last call, oldest first.
pub fn take() -> Vec<DeepLink> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The verses a link selects once the reader is there: the first verse alone, or the
/// same-chapter range. A chapter link selects nothing.
pub fn selection(link: &DeepLink) -> Option<(VerseRef, VerseRef)> {
    let r = link.reference;
    let from = VerseRef::new(r.book, r.chapter, r.verse?);
    let to = r.verse_end.map_or(from, |e| VerseRef::new(r.book, r.chapter, e));
    Some((from, to))
}

/// macOS: AppKit delivers `bible://` URLs (registered in `CFBundleURLTypes`, see
/// `scripts/bundle-macos.sh`) as the Internet `GURL` Apple Event. winit has no hook for it, so
/// a small handler object is registered with `NSAppleEventManager`. Install it before the event
/// loop starts, so the event that launched the app isn't missed.
#[cfg(target_os = "macos")]
pub fn install() {
    macos::install();
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AllocAnyThread, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSString};

    /// `kInternetEventClass` and `kAEGetURL` are both 'GURL'; the URL is the direct object.
    const GURL: u32 = u32::from_be_bytes(*b"GURL");
    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "BibleRsUrlHandler"]
        struct UrlHandler;

        impl UrlHandler {
            #[unsafe(method(handleGetURL:withReplyEvent:))]
            fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                // `paramDescriptorForKeyword:` is called directly: its typed binding needs the
                // objc2-core-services crate for one integer type.
                let param: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };
                let url: Option<Retained<NSString>> = param.and_then(|p| p.stringValue());
                if let Some(url) = url {
                    super::open(&url.to_string());
                }
            }
        }
    );

    pub fn install() {
        static INSTALLED: OnceLock<()> = OnceLock::new();
        INSTALLED.get_or_init(|| {
            let handler: Retained<UrlHandler> = unsafe { msg_send![UrlHandler::alloc(), init] };
            let manager = NSAppleEventManager::sharedAppleEventManager();
            unsafe {
                let _: () = msg_send![
                    &manager,
                    setEventHandler: &*handler,
                    andSelector: sel!(handleGetURL:withReplyEvent:),
                    forEventClass: GURL,
                    andEventID: GURL
                ];
            }
            // The event manager doesn't retain its handlers; this one lives as long as the app.
            std::mem::forget(handler);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(url: &str) -> Option<(String, String)> {
        selection(&bible_core::link::parse(url).unwrap()).map(|(a, b)| (a.to_string(), b.to_string()))
    }

    #[test]
    fn selection_covers_the_range() {
        assert_eq!(sel("bible://jhn.3.16"), Some(("John 3:16".into(), "John 3:16".into())));
        assert_eq!(sel("bible://jhn.3.16-18"), Some(("John 3:16".into(), "John 3:18".into())));
        assert_eq!(sel("bible://jhn.3.16-jhn.4.2"), Some(("John 3:16".into(), "John 3:16".into())));
        assert_eq!(sel("bible://rom.8"), None);
    }

    #[test]
    fn queue_takes_only_passages() {
        assert!(open("bible://psa.23.1-3"));
        assert!(!open("bible://nowhere"));
        let links = take();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].reference.display(), "Psalm 23:1–3");
        assert!(take().is_empty());
    }
}
