# Bible on the App Store

How to get the iPhone and iPad app into the App Store, and what to paste into App Store Connect.
The repository holds everything that doesn't need an Apple account: the privacy manifest, the
release script and its `ExportOptions.plist`, the screenshots, the privacy and support pages and
the listing in [AppStore/](AppStore/README.md).
Creating the app's record, signing, uploading and submitting need the account, so they're steps
for its holder.

## Decide first

The App Store name is **Bible: Fast & Offline**. The name under the icon, `CFBundleDisplayName`,
stays "Bible". No live app in the US, UK, Canadian or Australian store used the name on 20
October 2026; App Store Connect confirms it's free when the record is created.

The bundle ID (`dev.bible-rs.ios`), the version (0.2.0), the seller (Fabrika LLC) and the
territories (all, the UK included) are settled; [AppStore/](AppStore/README.md#decisions) records
them. Still open:

1. **A support email.** The support page sends people to GitHub issues, which needs a GitHub
   account. An email address there would reach everyone else.

## One-time setup

1. **The app's record.** In App Store Connect, Apps, +, New App: platform iOS, name "Bible: Fast & Offline", primary
   language English (U.S.), bundle ID `dev.bible-rs.ios`, SKU `bible-ios`. If the bundle ID isn't
   in the list yet, register it first under Certificates, Identifiers & Profiles, Identifiers, as
   an explicit App ID with no capabilities.
2. **Signing.** The team is Fabrika LLC, 5S3KCRYBD2, as on the Mac. The simplest route is to sign
   in to Xcode with the Apple ID under Xcode, Settings, Accounts. `scripts/release-ios.sh` then
   lets xcodebuild create the Apple Distribution certificate and the App Store profile on first
   use. Without Xcode signed in, make an App Store Connect API key with the Admin role under Users
   and Access, Integrations, Team Keys, and set `ASC_KEY_PATH`, `ASC_KEY_ID` and `ASC_ISSUER_ID`
   for the script. Keep the `.p8` file out of the repository.
3. **The website.** The listing links to `https://simplymybible.com/privacy` and `/support`. They
   go live when the `site/` changes reach `main`, the site deploys and the domain is attached to
   the Vercel project; check both open before submitting.

## Each release

1. Set the version in `Cargo.toml`, which the Mac app shares
   ([RELEASING.md](../../RELEASING.md)), and check out a clean `main`.
2. Run `scripts/release-ios.sh --upload`. It builds the Rust core for the device, archives the app
   with its debug symbols, signs it for App Store distribution and uploads it. The build number is
   the commit count, so every upload from a newer commit is higher; set `BUILD_NUMBER` to upload
   twice from one commit. Without `--upload` it leaves `dist/ios/Bible.ipa` for Transporter, and
   `--unsigned` only checks that the release build archives.
3. The build appears under TestFlight once Apple has processed it, usually within half an hour.
   Install it through TestFlight on an iPhone and an iPad and read for a while: rotate, change
   the text size, search, take notes.
4. On the App Store tab, add the version, pick the build, fill in What's New and submit for
   review.

## Listing, privacy answers and screenshots

The listing text, the App Privacy, encryption, age rating and content rights answers, the notes
for App Review and the screenshot sizes are in [AppStore/](AppStore/README.md).

## Before submitting

- [ ] `/privacy` and `/support` open on simplymybible.com
- [ ] The TestFlight build works on a real iPhone and iPad, in both orientations
- [ ] Screenshots, description, keywords, URLs and the review notes are filled in for the version
- [ ] App Privacy, age rating, content rights and Pricing and Availability are answered
