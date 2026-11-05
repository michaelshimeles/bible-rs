import Foundation

/// The iPhone side of the Study Library folder the Mac app already writes: `library.json`,
/// `changes/<device>/` and `snapshots/` in this app's iCloud Drive documents container.
/// The study SQLite file stays in Application Support and is never placed in the folder.
enum StudyLibrary {
    static let containerId = "iCloud.dev.bible-rs.ios"
    static let folderName = "Bible Study Library"
    /// How long an empty folder is treated as "iCloud hasn't listed files yet" when the
    /// metadata query has not finished gathering, so a new `library.json` is not written
    /// over a library that is still downloading.
    static let emptyGrace: TimeInterval = 20

    enum Availability: Equatable {
        /// No iCloud account on this device.
        case signedOut
        /// Signed in, but this app cannot reach its ubiquity container.
        case unavailable
        case ready(URL)
    }

    enum Prepare: Equatable {
        case ready
        /// Ubiquitous items exist but are not local yet; the caller must wait instead of
        /// syncing, so not-downloaded files are never treated as missing or deleted.
        case waiting(Int)
    }

    /// Looks up the container. Must not run on the main thread: the call can wait on iCloud.
    static func availability() -> Availability {
        let fm = FileManager.default
        guard fm.ubiquityIdentityToken != nil else { return .signedOut }
        guard let root = fm.url(forUbiquityContainerIdentifier: containerId) else {
            return .unavailable
        }
        return .ready(
            root.appendingPathComponent("Documents", isDirectory: true)
                .appendingPathComponent(folderName, isDirectory: true)
        )
    }

    /// Creates the library folder if needed and starts downloading anything that is not
    /// local yet. Returns `.waiting` when it is not safe to open or write the library.
    static func prepare(_ folder: URL) throws -> Prepare {
        try coordinate(writing: folder) { url in
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        }
        let pending = downloadPending(in: folder)
        if pending > 0 { return .waiting(pending) }
        switch markerState(in: folder) {
        case .downloaded:
            return .ready
        case .pending:
            return .waiting(1)
        case .missing:
            if folderHasChildren(folder) {
                // Changes or snapshots without `library.json`: the marker is still on its way.
                return .waiting(1)
            }
            return .ready
        }
    }

    /// Whether Rust may open the folder (and create `library.json` if the folder is empty).
    static func mayOpen(_ folder: URL, queryDidGather: Bool, emptySince: Date?) -> Bool {
        switch markerState(in: folder) {
        case .downloaded:
            return downloadPending(in: folder) == 0
        case .pending:
            return false
        case .missing:
            if folderHasChildren(folder) { return false }
            if downloadPending(in: folder) > 0 { return false }
            if queryDidGather { return true }
            if let emptySince, Date().timeIntervalSince(emptySince) >= emptyGrace { return true }
            return false
        }
    }

    static func sync(bible: Bible, folder: URL) throws -> StudyLibrarySync {
        var result: Result<StudyLibrarySync, Error>?
        var error: NSError?
        NSFileCoordinator().coordinate(writingItemAt: folder, options: [], error: &error) { url in
            do {
                result = .success(try bible.syncStudyLibrary(folder: url.path))
            } catch let e {
                result = .failure(e)
            }
        }
        if let error { throw error }
        switch result {
        case .success(let sync): return sync
        case .failure(let e): throw e
        case .none: throw CocoaError(.fileWriteUnknown)
        }
    }

    static func status(bible: Bible, folder: URL) -> StudyLibraryStatus {
        bible.studyLibraryStatus(folder: folder.path)
    }

    /// Starts downloading every not-yet-local item under `folder`, including hidden
    /// `.<name>.icloud` placeholders. Returns how many are still not downloaded.
    @discardableResult
    static func downloadPending(in folder: URL) -> Int {
        var pending = 0
        var error: NSError?
        NSFileCoordinator().coordinate(readingItemAt: folder, options: [.withoutChanges], error: &error) { url in
            pending = downloadPendingUncoordinated(in: url)
        }
        return pending
    }

    static func startDownloading(_ url: URL) {
        try? FileManager.default.startDownloadingUbiquitousItem(at: url)
    }

    private enum Marker {
        case downloaded, pending, missing
    }

    private static func markerState(in folder: URL) -> Marker {
        let fm = FileManager.default
        let marker = folder.appendingPathComponent("library.json")
        let placeholder = folder.appendingPathComponent(".library.json.icloud")
        if fm.fileExists(atPath: placeholder.path) {
            startDownloading(placeholder)
            return .pending
        }
        guard fm.fileExists(atPath: marker.path) else { return .missing }
        let status = (try? marker.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]))?
            .ubiquitousItemDownloadingStatus
        if status == .notDownloaded {
            startDownloading(marker)
            return .pending
        }
        return .downloaded
    }

    private static func folderHasChildren(_ folder: URL) -> Bool {
        let contents = try? FileManager.default.contentsOfDirectory(
            at: folder, includingPropertiesForKeys: nil, options: [])
        return (contents ?? []).contains { url in
            let name = url.lastPathComponent
            return name != ".DS_Store" && name != "." && name != ".."
        }
    }

    private static func downloadPendingUncoordinated(in folder: URL) -> Int {
        let fm = FileManager.default
        guard let enumerator = fm.enumerator(
            at: folder,
            includingPropertiesForKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemDownloadingStatusKey,
            ],
            options: [],
            errorHandler: { _, _ in true }
        ) else { return 0 }

        var pending = 0
        for case let url as URL in enumerator {
            let name = url.lastPathComponent
            if name.hasPrefix(".") && name.hasSuffix(".icloud") {
                startDownloading(url)
                pending += 1
                continue
            }
            let values = try? url.resourceValues(forKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemDownloadingStatusKey,
            ])
            guard values?.isUbiquitousItem == true else { continue }
            if values?.ubiquitousItemDownloadingStatus == .notDownloaded {
                startDownloading(url)
                pending += 1
            }
        }
        return pending
    }

    private static func coordinate(writing folder: URL, body: (URL) throws -> Void) throws {
        var error: NSError?
        var thrown: Error?
        NSFileCoordinator().coordinate(writingItemAt: folder, options: [], error: &error) { url in
            do { try body(url) } catch { thrown = error }
        }
        if let error { throw error }
        if let thrown { throw thrown }
    }

    /// Result of one background pass over the folder. Primitive fields so it can leave a
    /// detached task without crossing the main actor with generated FFI types.
    enum Outcome: Sendable {
        case signedOut
        case unavailable
        case waiting(Int)
        case waitingToAppear
        case synced(changed: UInt32, conflicts: UInt32, label: String, offline: Bool, devices: UInt32)
        case failed(String)
    }

    /// Resolves the container, waits for downloads, then syncs. Call off the main thread.
    static func run(bible: Bible, queryDidGather: Bool, emptySince: Date?) -> Outcome {
        switch availability() {
        case .signedOut:
            return .signedOut
        case .unavailable:
            return .unavailable
        case .ready(let folder):
            do {
                switch try prepare(folder) {
                case .waiting(let count):
                    return .waiting(count)
                case .ready:
                    if !mayOpen(folder, queryDidGather: queryDidGather, emptySince: emptySince) {
                        return .waitingToAppear
                    }
                    let sync = try self.sync(bible: bible, folder: folder)
                    let status = self.status(bible: bible, folder: folder)
                    return .synced(
                        changed: sync.changed,
                        conflicts: sync.conflicts,
                        label: status.label,
                        offline: status.offline,
                        devices: status.devices
                    )
                }
            } catch {
                return .failed(error.localizedDescription)
            }
        }
    }
}
