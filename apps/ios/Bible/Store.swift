import Foundation
import Observation
import UIKit
import os

/// The reader's preferences, saved in the study database (under the "ios" setting) as JSON.
struct ReaderSettings: Codable, Equatable {
    var translation = "kjv"
    var appearance = Appearance.system
    /// Points at the default Dynamic Type size.
    var textSize: Double = 18
    var lineSpacing: Double = 1.6
    var redLetters = true
    var verseNumbers = true
    var versePerLine = false
    var headings = true
    var wholeWords = false
    var bookLayout = BookLayout.allBooks
    /// Study Library (beta): sync notes, highlights, bookmarks and plans through this app's
    /// iCloud Drive folder. Off until the reader turns it on.
    var studyLibrary = false

    static let textSizes: ClosedRange<Double> = 14...32
    static let lineSpacings: ClosedRange<Double> = 1.3...2.0

    init() {}

    /// Settings saved by an older or newer build decode too: anything missing keeps its default.
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = ReaderSettings()
        func value<T: Decodable>(_ key: CodingKeys, _ fallback: T) -> T {
            ((try? c.decodeIfPresent(T.self, forKey: key)) ?? nil) ?? fallback
        }
        translation = value(.translation, d.translation)
        appearance = value(.appearance, d.appearance)
        textSize = value(.textSize, d.textSize).clamped(to: Self.textSizes)
        lineSpacing = value(.lineSpacing, d.lineSpacing).clamped(to: Self.lineSpacings)
        redLetters = value(.redLetters, d.redLetters)
        verseNumbers = value(.verseNumbers, d.verseNumbers)
        versePerLine = value(.versePerLine, d.versePerLine)
        headings = value(.headings, d.headings)
        wholeWords = value(.wholeWords, d.wholeWords)
        bookLayout = value(.bookLayout, d.bookLayout)
        studyLibrary = value(.studyLibrary, d.studyLibrary)
    }
}

extension Comparable {
    func clamped(to range: ClosedRange<Self>) -> Self { min(max(self, range.lowerBound), range.upperBound) }
}

/// A chapter (0–1188 in the canonical grid) and verse.
struct Position: Equatable {
    var chapter: Int
    var verse: Int
}

/// A request for the reader to move.
struct JumpRequest: Equatable {
    let id = UUID()
    var chapter: Int
    var verse: Int
    var animated = false
    var flash = false
}

enum Sheet: Identifiable, Equatable {
    case books, search, settings, study
    case note(VerseRef)

    var id: String {
        switch self {
        case .books: "books"
        case .search: "search"
        case .settings: "settings"
        case .study: "study"
        case .note(let v): "note-\(v.book)-\(v.chapter)-\(v.verse)"
        }
    }
}

@MainActor
@Observable
final class Store {
    @ObservationIgnored let bible: Bible
    @ObservationIgnored let bookList: [BookInfo]
    @ObservationIgnored let sections: [BibleSection]
    private(set) var translations: [TranslationInfo]

    var settings: ReaderSettings {
        didSet {
            guard settings != oldValue else { return }
            if settings.translation != oldValue.translation { translationChanged() }
            if settings.studyLibrary != oldValue.studyLibrary { studyLibraryToggled() }
            scheduleSave()
        }
    }

    /// Where the reader is; the reader view reports it as you scroll.
    private(set) var position: Position
    private(set) var jump: JumpRequest?
    var selection: VerseSelection?
    /// Bumped whenever highlights, notes or bookmarks change.
    private(set) var marksVersion = 0
    var sheet: Sheet?
    var barsHidden = false
    private(set) var toast: String?

    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private var toastTask: Task<Void, Never>?

    /// Study Library (beta) status shown in Settings.
    private(set) var studyLibraryLabel = "Off"
    private(set) var studyLibraryDetail: String?
    private(set) var studyLibrarySyncing = false
    @ObservationIgnored private var studyLibraryTask: Task<Void, Never>?
    @ObservationIgnored private var studyLibraryDebounce: Task<Void, Never>?
    @ObservationIgnored private var studyLibraryPoll: Task<Void, Never>?
    @ObservationIgnored private var studyLibraryRetry: Task<Void, Never>?
    @ObservationIgnored private var iCloudQuery: NSMetadataQuery?
    @ObservationIgnored private var iCloudQueryObservers: [NSObjectProtocol] = []
    @ObservationIgnored private var identityObserver: NSObjectProtocol?
    @ObservationIgnored private var queryDidGather = false
    @ObservationIgnored private var lastSyncedAt: Date?
    @ObservationIgnored private var studyLibraryQueued = false
    /// Incremented when sync is turned off so an older background pass cannot clear a newer one.
    @ObservationIgnored private var studyLibraryGeneration: UInt64 = 0

    init() {
        let bible = Self.openBible()
        let bookList = books()
        let translations = bible.translations()
        var settings = ReaderSettings()
        var position = Position(chapter: Int(bookList[42].firstChapter), verse: 1)
        if let json = bible.setting(key: "ios"), let saved = try? JSONDecoder().decode(Saved.self, from: Data(json.utf8)) {
            settings = saved.settings ?? settings
            if let chapter = saved.chapter, (0..<ReaderView.chapterCount).contains(chapter) {
                position = Position(chapter: chapter, verse: max(saved.verse ?? 1, 1))
            }
        }
        if !translations.contains(where: { $0.id == settings.translation }) {
            settings.translation = translations.first?.id ?? "kjv"
        }
        self.bible = bible
        self.bookList = bookList
        sections = bibleSections()
        self.translations = translations
        self.settings = settings
        self.position = position
        jump = JumpRequest(chapter: position.chapter, verse: position.verse)
        if ProcessInfo.processInfo.arguments.contains("--open-settings") {
            sheet = .settings
        }
        if settings.studyLibrary {
            startStudyLibrary()
        }
    }

    /// The study database and imported texts live in Application Support. If that can't be
    /// opened the reader still works, keeping study data in a temporary folder.
    private static func openBible() -> Bible {
        let fm = FileManager.default
        let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        var dir = support.appendingPathComponent("bible-rs", isDirectory: true)
        let arguments = ProcessInfo.processInfo.arguments
        if arguments.contains("--uitest") {
            // UI tests start from nothing (unless relaunching) and leave your notes alone.
            dir = fm.temporaryDirectory.appendingPathComponent("bible-rs-uitest", isDirectory: true)
            if !arguments.contains("--keep-data") { try? fm.removeItem(at: dir) }
        }
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        do {
            return try Bible(dataDir: dir.path)
        } catch {
            Log.app.error("Couldn't open \(dir.path, privacy: .public): \(error.localizedDescription, privacy: .public)")
            let tmp = fm.temporaryDirectory.appendingPathComponent("bible-rs", isDirectory: true)
            try? fm.createDirectory(at: tmp, withIntermediateDirectories: true)
            return try! Bible(dataDir: tmp.path)
        }
    }

    // MARK: Text

    var translation: TranslationInfo? { translations.first { $0.id == settings.translation } }

    var contentOptions: ContentOptions {
        ContentOptions(
            versePerLine: settings.versePerLine, headings: settings.headings, verseNumbers: settings.verseNumbers)
    }

    /// Decodes the translation and builds its search text off the main thread.
    func warm() {
        let bible = bible
        let ids = [settings.translation]
        Task.detached(priority: .utility) { bible.warm(translations: ids) }
    }

    private func translationChanged() {
        selection = nil
        warm()
    }

    // MARK: Books and chapters

    func book(of chapter: Int) -> BookInfo {
        bookList.last { Int($0.firstChapter) <= chapter } ?? bookList[0]
    }

    func section(of book: BookInfo) -> BibleSection {
        sections.first { $0.contains(book) } ?? sections[0]
    }

    func ref(chapter: Int, verse: Int) -> VerseRef {
        let book = book(of: chapter)
        return VerseRef(book: book.index, chapter: UInt8(chapter - Int(book.firstChapter) + 1), verse: UInt8(verse))
    }

    var title: String { chapterLabel(index: UInt16(position.chapter)) }

    // MARK: Moving

    func go(to chapter: Int, verse: Int = 1, flash: Bool = false, animated: Bool = false) {
        selection = nil
        barsHidden = false
        position = Position(chapter: chapter, verse: verse)
        jump = JumpRequest(chapter: chapter, verse: verse, animated: animated, flash: flash)
        scheduleSave()
    }

    func go(to reference: Reference) {
        go(to: Int(reference.chapterIndex), verse: Int(reference.verse ?? 1), flash: reference.verse != nil)
    }

    func reached(_ chapter: Int, verse: Int) {
        let next = Position(chapter: chapter, verse: verse)
        guard next != position else { return }
        position = next
        scheduleSave()
    }

    // MARK: Selection

    func tap(verse: Int, chapter: Int) {
        barsHidden = false
        if var s = selection, s.chapter == chapter {
            if s.verses.contains(verse) { s.verses.remove(verse) } else { s.verses.insert(verse) }
            selection = s.verses.isEmpty ? nil : s
        } else {
            selection = VerseSelection(chapter: chapter, verses: [verse])
        }
    }

    func tapPage() {
        if selection != nil {
            selection = nil
        } else {
            barsHidden.toggle()
        }
    }

    var selectedRefs: [VerseRef] {
        guard let s = selection else { return [] }
        return s.verses.sorted().map { ref(chapter: s.chapter, verse: $0) }
    }

    /// "John 3:16", "John 3:16–18" or "John 3:16, 18".
    var selectionLabel: String {
        let refs = selectedRefs
        guard let first = refs.first, let last = refs.last else { return "" }
        let contiguous = Int(last.verse) - Int(first.verse) == refs.count - 1
        if contiguous { return rangeLabel(from: first, to: last) }
        return rangeLabel(from: first, to: first) + ", " + refs.dropFirst().map { "\($0.verse)" }.joined(separator: ", ")
    }

    var selectionMarks: [VerseMarks] {
        guard let s = selection else { return [] }
        return bible.marks(translation: settings.translation, chapter: UInt16(s.chapter))
            .filter { s.verses.contains(Int($0.verse)) }
    }

    func highlightSelection(_ color: HighlightColor?) {
        study { for ref in selectedRefs { try bible.setHighlight(verse: ref, color: color) } }
        selection = nil
    }

    func toggleBookmarkOnSelection() {
        let refs = selectedRefs
        let marked = Set(selectionMarks.filter(\.bookmark).map(\.verse))
        let on = refs.contains { !marked.contains($0.verse) }
        study { for ref in refs { try bible.setBookmark(verse: ref, on: on) } }
        show(on ? (refs.count == 1 ? "Bookmarked" : "Bookmarked \(refs.count) verses") : "Bookmark removed")
        selection = nil
    }

    func noteOnSelection() {
        guard let first = selectedRefs.first else { return }
        sheet = .note(first)
    }

    /// The selected verses as text with their reference, as the Mac copies them.
    var selectionText: String {
        (try? bible.copyText(translation: settings.translation, verses: selectedRefs)) ?? ""
    }

    func copySelection() {
        UIPasteboard.general.string = selectionText
        show("Copied \(selectionLabel)")
        selection = nil
    }

    // MARK: Study

    func note(_ ref: VerseRef) -> String { (try? bible.note(verse: ref)) ?? "" }

    @discardableResult
    func setNote(_ ref: VerseRef, _ body: String, over base: String) -> Bool {
        var conflict = false
        let ok = study { conflict = try bible.setNoteOver(verse: ref, body: body, base: base) }
        if ok, conflict {
            show(
                "A note was edited on two devices. Both versions are kept; you can choose on a Mac in Settings › Data."
            )
        }
        return ok
    }

    func setHighlight(_ ref: VerseRef, _ color: HighlightColor?) {
        study { try bible.setHighlight(verse: ref, color: color) }
    }

    /// Removes one of a verse's highlights (a study item's `start`), leaving its others.
    func removeHighlight(_ ref: VerseRef, _ start: UInt32) {
        study { try bible.removeHighlight(verse: ref, start: start) }
    }

    func setBookmark(_ ref: VerseRef, _ on: Bool) {
        study { try bible.setBookmark(verse: ref, on: on) }
    }

    func items(_ kind: StudyKind) -> [StudyItem] {
        bible.studyItems(kind: kind, translation: settings.translation)
    }

    func export(_ format: ExportFormat) -> String? {
        try? bible.export(format: format, translation: settings.translation)
    }

    @discardableResult
    private func study(_ change: () throws -> Void) -> Bool {
        defer { marksVersion += 1 }
        do {
            try change()
            scheduleStudyLibrarySync()
            return true
        } catch {
            show("Couldn’t save: \(error.localizedDescription)")
            return false
        }
    }

    // MARK: Toasts

    func show(_ message: String) {
        toast = message
        toastTask?.cancel()
        toastTask = Task {
            try? await Task.sleep(for: .seconds(1.6))
            guard !Task.isCancelled else { return }
            toast = nil
        }
    }

    // MARK: Saving

    private struct Saved: Codable {
        var settings: ReaderSettings?
        var chapter: Int?
        var verse: Int?
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task {
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            save()
        }
    }

    func save() {
        saveTask?.cancel()
        let saved = Saved(settings: settings, chapter: position.chapter, verse: position.verse)
        guard let data = try? JSONEncoder().encode(saved), let json = String(data: data, encoding: .utf8) else { return }
        try? bible.setSetting(key: "ios", value: json)
    }

    // MARK: Study Library (iCloud)

    func studyLibraryForeground() {
        guard settings.studyLibrary else { return }
        startStudyLibrary()
        syncStudyLibraryNow()
    }

    func syncStudyLibraryNow() {
        guard settings.studyLibrary else { return }
        studyLibraryDebounce?.cancel()
        studyLibraryRetry?.cancel()
        runStudyLibrarySync()
    }

    private func studyLibraryToggled() {
        if settings.studyLibrary {
            startStudyLibrary()
            syncStudyLibraryNow()
        } else {
            stopStudyLibrary()
            studyLibraryLabel = "Off"
            studyLibraryDetail = nil
            studyLibrarySyncing = false
        }
    }

    private func scheduleStudyLibrarySync() {
        guard settings.studyLibrary else { return }
        studyLibraryDebounce?.cancel()
        studyLibraryDebounce = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.syncStudyLibraryNow()
        }
    }

    private func startStudyLibrary() {
        guard !ProcessInfo.processInfo.arguments.contains("--uitest") else { return }
        startIdentityObserver()
        startICloudQuery()
        startStudyLibraryPoll()
    }

    private func stopStudyLibrary() {
        studyLibraryGeneration += 1
        studyLibraryDebounce?.cancel()
        studyLibraryDebounce = nil
        studyLibraryRetry?.cancel()
        studyLibraryRetry = nil
        studyLibraryPoll?.cancel()
        studyLibraryPoll = nil
        // Leave `studyLibraryTask` until that pass finishes and reports in; cancel() does not
        // stop the folder work already running, and clearing the handle here would let a new
        // pass start while the old one can still clear it.
        studyLibraryQueued = false
        queryDidGather = false
        stopICloudQuery()
        stopIdentityObserver()
    }

    private func startStudyLibraryPoll() {
        guard studyLibraryPoll == nil else { return }
        studyLibraryPoll = Task { @MainActor [weak self] in
            while let self, !Task.isCancelled, self.settings.studyLibrary {
                try? await Task.sleep(for: .seconds(30))
                guard !Task.isCancelled, self.settings.studyLibrary else { return }
                self.syncStudyLibraryNow()
            }
        }
    }

    private func startIdentityObserver() {
        guard identityObserver == nil else { return }
        identityObserver = NotificationCenter.default.addObserver(
            forName: .NSUbiquityIdentityDidChange, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.queryDidGather = false
                self?.syncStudyLibraryNow()
            }
        }
    }

    private func stopIdentityObserver() {
        if let identityObserver {
            NotificationCenter.default.removeObserver(identityObserver)
            self.identityObserver = nil
        }
    }

    private func startICloudQuery() {
        guard iCloudQuery == nil else { return }
        let query = NSMetadataQuery()
        query.predicate = NSPredicate(value: true)
        query.searchScopes = [NSMetadataQueryUbiquitousDocumentsScope]
        let center = NotificationCenter.default
        let gathered = center.addObserver(
            forName: .NSMetadataQueryDidFinishGathering, object: query, queue: .main
        ) { [weak self] note in
            Task { @MainActor [weak self] in
                self?.handleICloudQuery(note.object as? NSMetadataQuery, gathered: true)
            }
        }
        let updated = center.addObserver(
            forName: .NSMetadataQueryDidUpdate, object: query, queue: .main
        ) { [weak self] note in
            Task { @MainActor [weak self] in
                self?.handleICloudQuery(note.object as? NSMetadataQuery, gathered: false)
            }
        }
        iCloudQueryObservers = [gathered, updated]
        iCloudQuery = query
        query.start()
    }

    private func stopICloudQuery() {
        iCloudQuery?.stop()
        iCloudQuery = nil
        for observer in iCloudQueryObservers {
            NotificationCenter.default.removeObserver(observer)
        }
        iCloudQueryObservers = []
    }

    private func handleICloudQuery(_ query: NSMetadataQuery?, gathered: Bool) {
        guard let query else { return }
        query.disableUpdates()
        defer { query.enableUpdates() }
        if gathered { queryDidGather = true }
        var pending = 0
        for item in query.results {
            guard let item = item as? NSMetadataItem else { continue }
            let path = item.value(forAttribute: NSMetadataItemPathKey) as? String ?? ""
            guard path.contains(StudyLibrary.folderName) else { continue }
            let status = item.value(forAttribute: NSMetadataUbiquitousItemDownloadingStatusKey) as? String
            if status == NSMetadataUbiquitousItemDownloadingStatusNotDownloaded {
                if let url = item.value(forAttribute: NSMetadataItemURLKey) as? URL {
                    StudyLibrary.startDownloading(url)
                }
                pending += 1
            }
        }
        if pending > 0 || gathered {
            scheduleStudyLibrarySync()
        }
    }

    private func runStudyLibrarySync() {
        if studyLibraryTask != nil {
            studyLibraryQueued = true
            return
        }
        studyLibrarySyncing = true
        if studyLibraryLabel == "Off" || studyLibraryLabel.isEmpty {
            studyLibraryLabel = "Syncing…"
        }
        let bible = bible
        let queryDidGather = queryDidGather
        let generation = studyLibraryGeneration
        studyLibraryTask = Task.detached(priority: .utility) { [weak self] in
            let outcome = StudyLibrary.run(bible: bible, queryDidGather: queryDidGather)
            await self?.applyStudyLibrary(outcome, generation: generation)
        }
    }

    private func applyStudyLibrary(_ outcome: StudyLibrary.Outcome, generation: UInt64) {
        studyLibraryTask = nil
        if generation != studyLibraryGeneration {
            if settings.studyLibrary, studyLibraryQueued {
                studyLibraryQueued = false
                runStudyLibrarySync()
            }
            return
        }
        studyLibrarySyncing = false
        switch outcome {
        case .signedOut:
            studyLibraryLabel = "iCloud is signed out"
            studyLibraryDetail = "Sign in to iCloud in the Settings app to sync this library."
        case .unavailable:
            studyLibraryLabel = "iCloud isn’t available"
            studyLibraryDetail = "This iPhone couldn’t reach the Study Library folder."
        case .waiting(let count):
            studyLibraryLabel = "Waiting for iCloud…"
            studyLibraryDetail =
                count == 1 ? "1 file is still downloading." : "\(count) files are still downloading."
            retryStudyLibrary(after: 3)
        case .waitingToAppear:
            studyLibraryLabel = "Waiting for iCloud…"
            studyLibraryDetail = "Checking for a library already in iCloud Drive."
            retryStudyLibrary(after: 3)
        case .synced(let changed, let conflicts, let label, let offline, let devices):
            let now = Date()
            lastSyncedAt = now
            studyLibraryLabel = label
            if offline {
                studyLibraryDetail = "The folder can’t be reached right now."
            } else {
                let others: String
                switch devices {
                case 0: others = "no other devices yet"
                case 1: others = "1 other device"
                default: others = "\(devices) other devices"
                }
                studyLibraryDetail = "Last synced \(now.formatted(date: .omitted, time: .shortened)) · \(others)"
            }
            if changed > 0 { marksVersion += 1 }
            if conflicts > 0 {
                show(
                    "A note was edited on two devices. Both versions are kept; you can choose on a Mac in Settings › Data."
                )
            }
        case .failed(let message):
            studyLibraryLabel = "Couldn’t sync"
            studyLibraryDetail = message
            retryStudyLibrary(after: 8)
        }
        if studyLibraryQueued {
            studyLibraryQueued = false
            runStudyLibrarySync()
        }
    }

    private func retryStudyLibrary(after seconds: Double) {
        studyLibraryRetry?.cancel()
        studyLibraryRetry = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            self?.syncStudyLibraryNow()
        }
    }
}

enum Log {
    static let app = Logger(subsystem: "dev.bible-rs.ios", category: "app")
}

/// Time since the process started, for launch measurements.
enum LaunchClock {
    static let processStart: Date = {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, 4, &info, &size, nil, 0) == 0 else { return Date() }
        let t = info.kp_proc.p_starttime
        return Date(timeIntervalSince1970: Double(t.tv_sec) + Double(t.tv_usec) / 1_000_000)
    }()

    private static var logged = Set<String>()

    static func mark(_ event: String) {
        guard !logged.contains(event) else { return }
        logged.insert(event)
        let ms = Date().timeIntervalSince(processStart) * 1000
        Log.app.notice("\(event, privacy: .public): \(ms, format: .fixed(precision: 0), privacy: .public) ms after launch")
    }
}
