# iOS and iPadOS development plan

This fork keeps the upstream Rust engine and develops the native Apple app incrementally.

## Guardrails

- Keep the Rust core and UniFFI boundary unless a concrete limitation requires changing them.
- Keep changes on a feature branch; do not modify upstream.
- Preserve existing reading, search, highlighting, notes, bookmarks, and saved-position behavior.
- Build and test with the repository's scripts on a Mac before treating a change as verified.
- Avoid adding network services or accounts unless a feature genuinely requires them.

## Current baseline

The repository already includes a SwiftUI app targeting iOS 17+, with iPhone and iPad device families enabled. The app includes a custom UIKit/TextKit reader, local Bible translations, reference navigation, search, themes, verse selection, highlights, notes, and bookmarks. The README identifies parallel/compare translation views, importing translations, and online translations as Mac-only.

## Primary iPad workflow

The main target workflow is iPadOS Split View / windowed multitasking: Bible reading in this app on one side, with the user's preferred note-taking app alongside it. The Bible app should remain useful at half-width without requiring a dedicated in-app notes workspace.

Current implementation notes:
- The reader already recalculates its text column from the actual scroll-view width and safe-area insets.
- When the reader is re-typeset after a width or text-style change, it records the verse at the reading line and its relative line offset, then restores that position. Keep this behavior intact when changing layout code.
- The iPad toolbar uses a centered maximum width in regular layouts and consolidates its tools into an overflow menu below 700 pt of window width.
- The generated app configuration explicitly opts out of full-screen-only behavior.

Acceptance checks for this workflow:
- The app can participate in iPad multitasking and resize without restarting.
- At narrow iPad widths, reading controls collapse into an overflow menu while reference and translation remain directly accessible.
- The Bible text stays in a comfortable measure and adapts to width changes without losing the current verse (reader position restoration exists in code; verify visually on device).
- Search, settings, bookmarks, highlights and verse notes remain available in the compact layout.
- The UI test checks that Search remains reachable from either the compact overflow menu or the regular toolbar.
- Validate with common note-taking apps in Split View, portrait/landscape, and Stage Manager where available; these device-level checks are still outstanding.

## Proposed milestones

### 0. Establish a working baseline
- [ ] Build and launch the current app in an iOS simulator.
- [ ] Run the existing UI tests.
- [ ] Install on a physical iPhone and iPad.
- [ ] Record any setup/build issues and baseline screenshots.

### 1. Audit the iPad experience
- [ ] Review compact and regular-width layouts across portrait, landscape, and multitasking window sizes.
- [ ] Check reader line length, margins, navigation, sheets, and toolbar behavior.
- [ ] Test hardware keyboard, pointer, VoiceOver labels, and Dynamic Type.
- [ ] Identify the highest-value changes before editing the reader's layout code.

### 2. Improve native iPad navigation and reading
- [ ] Introduce an adaptive regular-width navigation pattern if it improves discoverability.
- [ ] Keep the reading column comfortable and responsive rather than stretching text across the whole display.
- [ ] Ensure study, search, book navigation, and settings work well in both compact and regular widths.
- [ ] Add or update UI tests for the adaptive layout.

### 3. Bring selected Mac capabilities to mobile
- [ ] Evaluate the existing FFI for translation comparison and parallel reading.
- [ ] Design mobile-appropriate comparison UI before implementing it.
- [ ] Investigate safe, platform-appropriate translation import and online translation support, including key storage and each translation's license/terms.

## Definition of done for each change

- The change is isolated and documented in a commit.
- Existing reading and study workflows remain intact.
- Relevant tests are updated and run where possible.
- Any build or device testing that could not be performed in this environment is clearly marked as unverified.
