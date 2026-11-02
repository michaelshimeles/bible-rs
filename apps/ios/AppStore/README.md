# The App Store listing

What to paste into App Store Connect for Bible on iPhone and iPad. The text lives in
`metadata/`, laid out the way `fastlane deliver` reads it, so it can be uploaded with a tool later
or pasted by hand now. `scripts/check-app-store-metadata.sh` (run in CI) checks every field
against App Store Connect's limits. [../APP_STORE.md](../APP_STORE.md) covers the one-time setup,
signing and each release.

| Field | File | Limit |
|---|---|---|
| Name | `metadata/en-US/name.txt` | 30 |
| Subtitle | `metadata/en-US/subtitle.txt` | 30 |
| Promotional text (changes without a new version) | `metadata/en-US/promotional_text.txt` | 170 |
| Description | `metadata/en-US/description.txt` | 4000 |
| Keywords | `metadata/en-US/keywords.txt` | 100 |
| What's New | `metadata/en-US/release_notes.txt` | 4000 |
| Support URL | `metadata/en-US/support_url.txt` | |
| Marketing URL | `metadata/en-US/marketing_url.txt` | |
| Privacy Policy URL | `metadata/en-US/privacy_url.txt` | |
| Copyright | `metadata/copyright.txt` | |
| Category | `metadata/primary_category.txt`, `metadata/secondary_category.txt` (Reference, Books) | |
| Notes for App Review | `metadata/review_information/notes.txt` | 4000 |

Other answers: price Free, primary language English (U.S.), SKU `bible-ios`. The keywords and
subtitle leave out "bible", "fast" and "offline" because the store already indexes the name.

## App Privacy (the nutrition label)

Answer **"No, we do not collect data from this app."** The listing then shows *Data Not
Collected*. That's true of the iPhone and iPad app:

- no account, sign-in, analytics, advertising, crash-reporting SDK or tracking;
- no network access at all: the build leaves out `bible-core`'s `tls` feature and the online
  translations, and `scripts/build-ios.sh` fails if rustls comes back;
- highlights, notes, bookmarks, history and settings stay in the app's own SQLite file and leave
  the device only when the reader exports or shares them (which Apple doesn't count as
  collection).

`Bible/PrivacyInfo.xcprivacy` says the same to Apple's tools: no tracking, no collected data
types, and reasons for the two required-reason API categories the binary links through SQLite,
file timestamps (C617.1) and disk space (E174.1). The Swift code uses no `UserDefaults`, so that
category isn't declared. Re-check with `nm -u` on the built binary when dependencies change.

The privacy policy is https://simplymybible.com/privacy and already describes the phone app
("On iPhone and iPad, never" goes online).

## Other questions

- **Encryption (export compliance).** `ITSAppUsesNonExemptEncryption` is `NO` in the Info.plist,
  so App Store Connect doesn't ask on each upload. The app contains no encryption code.
- **Age rating.** "None" or "No" to everything gives 4+. Nothing users write is shared, and the app
  has no web view, messaging, user-generated content feed or ads.
- **Content rights.** Yes, it shows third-party content, and you have the rights: the seven
  translations (KJV, WEB, BSB, ASV, YLT, Darby, Webster) are in the public domain. The KJV is
  Crown copyright in the United Kingdom; see "Decide first" in APP_STORE.md.
- **Sign-in for App Review.** Not needed.
- **App Review contact.** Your name, phone number and email.

## Screenshots

`scripts/ios-screenshots.sh --app-store` takes them in simulators (on a Mac with Xcode) in
listing order, into `target/ios/app-store-screenshots/`:

| Display set | Required because | Size | Simulator |
|---|---|---|---|
| iPhone 6.9-inch | the app runs on iPhone | 1320 × 2868 | iPhone 17 Pro Max |
| iPad 13-inch | the app runs on iPad | 2064 × 2752 | iPad Pro 13-inch (M4) |

App Store Connect accepts the 6.9-inch set in place of the 6.5-inch one and scales both sets down
for the smaller iPhones and iPads, so no 6.5-inch or 5.5-inch shots are needed. Each set takes up
to ten; upload `01` to `10`. The first three show in search results: the reader, a selected verse
and search, in light.
