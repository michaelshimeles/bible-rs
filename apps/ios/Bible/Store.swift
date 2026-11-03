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
    /// A recall hints at hidden words with their first letters, rather than showing nothing.
    var recallLetters = true

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
        recallLetters = value(.recallLetters, d.recallLetters)
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

/// The memorised passages being recalled, one after another in the reader.
struct RecallRun: Equatable {
    /// New for each passage shown, so the reader hides it afresh even when it comes round again.
    let id = UUID()
    var queue: [MemorisedPassage]
    var index = 0
    var status = RecallStatus()

    var passage: MemorisedPassage { queue[index] }
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
    private(set) var recall: RecallRun?

    @ObservationIgnored private var saveTask: Task<Void, Never>?
    @ObservationIgnored private var toastTask: Task<Void, Never>?

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
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("--recall-demo") { recallDemo() }
        #endif
    }

    #if DEBUG
        /// Screenshots and checks without touch: memorises John 3:16–17 and recalls it.
        private func recallDemo() {
            let first = VerseRef(book: 42, chapter: 3, verse: 16)
            let last = VerseRef(book: 42, chapter: 3, verse: 17)
            if bible.memorisedAt(verse: first) == nil {
                try? bible.memorise(first: first, last: last, translation: settings.translation)
            }
            guard let m = bible.memorisedAt(verse: first) else { return }
            recall = RecallRun(queue: [m])
            showRecalled()
        }
    #endif

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
        // Another translation's words: the recall starts the passage over.
        if let run = recall { recall = RecallRun(queue: run.queue, index: run.index) }
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
        // Going to another chapter ends a recall.
        if let run = recall, Int(run.passage.chapterIndex) != chapter { recall = nil }
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
        guard recall == nil else { return }
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

    // MARK: Memorising

    /// The memorised passages the selection touches.
    private var selectionPassages: [MemorisedPassage] {
        var out: [MemorisedPassage] = []
        for ref in selectedRefs {
            if let m = bible.memorisedAt(verse: ref), !out.contains(where: { $0.first == m.first }) { out.append(m) }
        }
        return out
    }

    var selectionMemorised: Bool { !selectionPassages.isEmpty }

    /// Memorises the selected verses, or stops memorising them.
    func memoriseSelection() {
        let refs = selectedRefs
        guard let first = refs.first, let last = refs.last else { return }
        // A selection touching passages already memorised stops them, never resets them.
        let memorised = selectionPassages
        if !memorised.isEmpty {
            if study({ for m in memorised { try bible.forget(first: m.first) } }) {
                show("Stopped memorising \(memorised.map(\.reference).joined(separator: ", "))")
            }
            selection = nil
            return
        }
        guard Int(last.verse) - Int(first.verse) == refs.count - 1 else {
            show("Select verses next to each other")
            return
        }
        let label = rangeLabel(from: first, to: last)
        if study({ try bible.memorise(first: first, last: last, translation: settings.translation) }) {
            show("Memorising \(label)")
        }
        selection = nil
    }

    /// Due by the end of today: a passage is due all of the day it falls on.
    var duePassages: [MemorisedPassage] {
        let calendar = Calendar.current
        let end = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: .now)) ?? .now
        return bible.duePassages(until: Int64(end.timeIntervalSince1970))
    }
    var memorisedPassages: [MemorisedPassage] { bible.memorisedPassages() }

    /// Recalls the passages due now in the reader, or with nothing due, practises them all early.
    func startRecall() {
        let due = duePassages
        let queue = due.isEmpty ? memorisedPassages : due
        guard !queue.isEmpty else {
            show("Select verses, then Memorise")
            return
        }
        if due.isEmpty { show("Nothing is due, so this is extra practice") }
        recall = RecallRun(queue: queue)
        showRecalled()
    }

    private func showRecalled() {
        // Recalled in the translation being read.
        guard let passage = recall?.passage else { return }
        sheet = nil
        go(to: Int(passage.chapterIndex), verse: Int(passage.first.verse))
    }

    func recallChanged(_ status: RecallStatus) {
        recall?.status = status
    }

    /// When the passage being recalled would come back for each grade: "1 day", "3 weeks".
    var gradeLabels: [String] {
        guard let run = recall else { return [] }
        return bible.gradeLabels(first: run.passage.first)
    }

    /// Grades the passage and goes on to the next, or ends.
    func grade(_ grade: RecallGrade) {
        guard let run = recall, run.status.done else { return }
        // A failed save stays on the passage, to grade again.
        guard study({ try bible.review(first: run.passage.first, grade: grade) }) else { return }
        // Again stays due: it comes back at the end of this session.
        let queue = grade == .again ? run.queue + [run.passage] : run.queue
        if run.index + 1 < queue.count {
            recall = RecallRun(queue: queue, index: run.index + 1)
            showRecalled()
        } else {
            recall = nil
            let n = Set(queue.map(\.first)).count
            show(n == 1 ? "Recalled 1 passage" : "Recalled \(n) passages")
        }
    }

    func endRecall() {
        recall = nil
    }

    // MARK: Study

    func note(_ ref: VerseRef) -> String { (try? bible.note(verse: ref)) ?? "" }

    @discardableResult
    func setNote(_ ref: VerseRef, _ body: String) -> Bool {
        study { try bible.setNote(verse: ref, body: body) }
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
