import SwiftUI

/// Theme, type and reading options, applied as you change them.
struct SettingsView: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let theme: Theme

    var body: some View {
        @Bindable var store = store
        NavigationStack {
            Form {
                Section {
                    HStack(spacing: 12) {
                        ForEach(Appearance.allCases) { appearance in
                            ThemeSwatch(
                                appearance: appearance, selected: store.settings.appearance == appearance, accent: theme
                            ) {
                                store.settings.appearance = appearance
                            }
                        }
                    }
                    .padding(.vertical, 6)
                }

                Section("Text") {
                    HStack(spacing: 14) {
                        Image(systemName: "textformat.size.smaller")
                            .foregroundStyle(.secondary)
                        Slider(value: $store.settings.textSize, in: ReaderSettings.textSizes, step: 1)
                            .accessibilityLabel("Text size")
                            .accessibilityValue("\(Int(store.settings.textSize)) points")
                        Image(systemName: "textformat.size.larger")
                            .foregroundStyle(.secondary)
                    }
                    HStack(spacing: 14) {
                        Image(systemName: "text.alignleft")
                            .foregroundStyle(.secondary)
                        Slider(value: $store.settings.lineSpacing, in: ReaderSettings.lineSpacings, step: 0.1)
                            .accessibilityLabel("Line spacing")
                            .accessibilityValue(store.settings.lineSpacing.formatted(.number.precision(.fractionLength(1))))
                        Image(systemName: "text.justify")
                            .foregroundStyle(.secondary)
                    }
                    Text("Text follows the system’s text size too.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }

                Section("Reading") {
                    Toggle("Words of Jesus in red", isOn: $store.settings.redLetters)
                    Toggle("Verse numbers", isOn: $store.settings.verseNumbers)
                    Toggle("One verse per line", isOn: $store.settings.versePerLine)
                    Toggle("Section headings", isOn: $store.settings.headings)
                    Toggle("Search whole words only", isOn: $store.settings.wholeWords)
                }

                Section {
                    Toggle("Sync with iCloud", isOn: $store.settings.studyLibrary)
                        .accessibilityIdentifier("study-library-icloud")
                    if store.settings.studyLibrary {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(store.studyLibraryLabel)
                            if let detail = store.studyLibraryDetail {
                                Text(detail)
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("study-library-status")
                        Button(store.studyLibrarySyncing ? "Syncing…" : "Sync Now") {
                            store.syncStudyLibraryNow()
                        }
                        .disabled(store.studyLibrarySyncing)
                        .accessibilityIdentifier("study-library-sync-now")
                    }
                } header: {
                    Text("Study Library (beta)")
                } footer: {
                    Text(
                        "Keep your notes, highlights, bookmarks and plans in iCloud Drive to use them on a Mac that opens the same library. Off by default. Your study stays on this iPhone until you turn this on."
                    )
                }

                Section("Notes and highlights") {
                    ExportButton(title: "Export as Markdown", format: .markdown, file: "Bible notes.md")
                    ExportButton(title: "Export as JSON", format: .json, file: "Bible notes.json")
                }

                if let t = store.translation {
                    Section("About the \(t.abbrev)") {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(t.name).font(.headline)
                            if let description = t.description {
                                Text(description).font(.subheadline).foregroundStyle(.secondary)
                            }
                            Text(t.copyright.isEmpty ? t.license : t.copyright)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        .padding(.vertical, 2)
                    }
                }

                Section {
                    LabeledContent("Version", value: Self.version)
                    NavigationLink("Font licences") { FontLicences() }
                }
            }
            .navigationTitle("Reading")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationBackgroundInteraction(.enabled(upThrough: .medium))
    }

    static var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(short) (\(build))"
    }
}

private struct ThemeSwatch: View {
    let appearance: Appearance
    let selected: Bool
    let accent: Theme
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                ZStack {
                    if appearance == .system {
                        HStack(spacing: 0) {
                            Color(Theme.light.paper)
                            Color(Theme.dark.paper)
                        }
                    } else {
                        Color(Theme.named(appearance.resolved(.light)).paper)
                    }
                    Text("Aa")
                        .font(.reading(17))
                        .foregroundStyle(appearance == .system ? Color.gray : Color(Theme.named(appearance.resolved(.light)).ink))
                }
                .frame(height: 52)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(selected ? Color(accent.accent) : Color(accent.border), lineWidth: selected ? 2.5 : 1)
                }
                Text(appearance.label)
                    .font(.caption.weight(selected ? .semibold : .regular))
                    .foregroundStyle(selected ? Color(accent.accent) : .secondary)
            }
            .frame(maxWidth: .infinity)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(appearance.label) theme")
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// The SIL Open Font License notices for the bundled Literata and Inter, which the licence asks
/// to travel with the fonts.
private struct FontLicences: View {
    private static let files = [("Literata", "OFL-Literata"), ("Inter", "OFL-Inter")]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                Text("Literata and Inter are bundled under the SIL Open Font License 1.1.")
                    .font(.subheadline)
                ForEach(Self.files, id: \.1) { name, file in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(name).font(.headline)
                        Text(Self.text(file))
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Font licences")
        .navigationBarTitleDisplayMode(.inline)
    }

    private static func text(_ file: String) -> String {
        guard let url = Bundle.main.url(forResource: file, withExtension: "txt"),
            let text = try? String(contentsOf: url, encoding: .utf8)
        else { return "SIL Open Font License 1.1: https://openfontlicense.org" }
        return text
    }
}

/// Writes the export to a file and offers it to share or save.
private struct ExportButton: View {
    @Environment(Store.self) private var store
    let title: String
    let format: ExportFormat
    let file: String
    @State private var url: URL?

    var body: some View {
        Group {
            if let url {
                ShareLink(item: url) { Label(title, systemImage: "square.and.arrow.up") }
            } else {
                Label(title, systemImage: "square.and.arrow.up").foregroundStyle(.secondary)
            }
        }
        .task {
            guard let text = store.export(format) else { return }
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(file)
            if (try? text.write(to: url, atomically: true, encoding: .utf8)) != nil { self.url = url }
        }
    }
}
