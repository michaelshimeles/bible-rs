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
