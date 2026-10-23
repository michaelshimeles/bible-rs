import SwiftUI

struct RootView: View {
    @Environment(Store.self) private var store
    @Environment(\.colorScheme) private var scheme
    @Environment(\.dynamicTypeSize) private var dynamicType

    var body: some View {
        @Bindable var store = store
        let theme = Theme.named(store.settings.appearance.resolved(scheme))
        GeometryReader { geo in
            let safeTop = geo.safeAreaInsets.top
            ZStack(alignment: .top) {
                ReaderContainer(
                    store: store, theme: theme, style: typography(theme), translation: store.settings.translation,
                    readingLine: safeTop + 60, selection: store.selection, marksVersion: store.marksVersion,
                    jump: store.jump
                )
                .overlay(alignment: .top) {
                    // Paper under the status bar, fading out below it.
                    LinearGradient(
                        stops: [
                            .init(color: Color(theme.paper), location: 0),
                            .init(color: Color(theme.paper), location: 0.55),
                            .init(color: Color(theme.paper).opacity(0), location: 1),
                        ], startPoint: .top, endPoint: .bottom
                    )
                    .frame(height: safeTop + 28)
                    .allowsHitTesting(false)
                }
                .ignoresSafeArea()

                TopBar(theme: theme)
                    .padding(.top, 4)
                    .offset(y: store.barsHidden ? -(safeTop + 72) : 0)
                    .opacity(store.barsHidden ? 0 : 1)
            }
            .overlay(alignment: .bottom) {
                VStack(spacing: 10) {
                    if let toast = store.toast {
                        Text(toast)
                            .font(.subheadline.weight(.medium))
                            .padding(.horizontal, 16)
                            .padding(.vertical, 10)
                            .glass(in: Capsule(), theme: theme, interactive: false)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                    if store.selection != nil {
                        SelectionBar(theme: theme)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    }
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
            }
        }
        .animation(.smooth(duration: 0.28), value: store.barsHidden)
        .animation(.smooth(duration: 0.28), value: store.selection != nil)
        .animation(.smooth(duration: 0.28), value: store.toast)
        .statusBarHidden(store.barsHidden)
        .preferredColorScheme(preferredScheme)
        .tint(Color(theme.accent))
        .sensoryFeedback(.selection, trigger: store.selection)
        .sheet(item: $store.sheet) { sheet in
            Group {
                switch sheet {
                case .books: BookPicker(theme: theme)
                case .search: SearchView(theme: theme)
                case .settings: SettingsView(theme: theme)
                case .study: StudyView(theme: theme)
                case .note(let ref): NoteEditor(ref: ref, theme: theme)
                }
            }
            .environment(store)
            .sheetChrome(theme)
        }
    }

    private var preferredScheme: ColorScheme? {
        switch store.settings.appearance {
        case .system: nil
        case .light, .sepia: .light
        case .dark: .dark
        }
    }

    /// The reading size follows Dynamic Type: the setting is the size at the default text size.
    private func typography(_ theme: Theme) -> Typography {
        let s = store.settings
        let traits = UITraitCollection(preferredContentSizeCategory: UIContentSizeCategory(dynamicType))
        let size = UIFontMetrics(forTextStyle: .body).scaledValue(for: CGFloat(s.textSize), compatibleWith: traits)
        return Typography(
            size: min(size, 44).rounded(), lineSpacing: CGFloat(s.lineSpacing), redLetters: s.redLetters,
            options: store.contentOptions, theme: theme.name, width: 0)
    }
}

/// The reference, translation and tools over the top of the page.
struct TopBar: View {
    @Environment(Store.self) private var store
    let theme: Theme

    var body: some View {
        HStack(spacing: 10) {
            HStack(spacing: 0) {
                Button {
                    store.sheet = .books
                } label: {
                    ViewThatFits(in: .horizontal) {
                        referenceLabel(store.title)
                        referenceLabel(shortReference)
                        // Narrower still, as in a Slide Over window on iPad: the short form
                        // truncates, as the full title did before, rather than push the bar
                        // past the screen.
                        referenceLabel(shortReference, truncates: true)
                    }
                    .font(.headline)
                    // Keep the bar compact; long-press presents the uncapped full title.
                    .dynamicTypeSize(...DynamicTypeSize.xxLarge)
                }
                .accessibilityLabel("\(store.title). Choose a book and chapter")
                .accessibilityIdentifier("reference")
                .accessibilityShowsLargeContentViewer { Text(store.title) }

                Rectangle()
                    .fill(Color(theme.separator))
                    .frame(width: 1, height: 20)

                Menu {
                    Picker("Translation", selection: Binding(
                        get: { store.settings.translation },
                        set: { store.settings.translation = $0 }
                    )) {
                        ForEach(store.translations, id: \.id) { t in
                            Text("\(t.abbrev) · \(t.name)").tag(t.id)
                        }
                    }
                } label: {
                    Text(store.translation?.abbrev ?? "")
                        .font(.subheadline.weight(.semibold))
                        // Leave room for the reference in this compact control, even at
                        // accessibility sizes. The reader text still follows Dynamic Type.
                        .dynamicTypeSize(...DynamicTypeSize.large)
                        .fixedSize(horizontal: true, vertical: false)
                        .padding(.leading, 10)
                        .padding(.trailing, 16)
                        .frame(minHeight: 44)
                }
                .accessibilityLabel("Translation: \(store.translation?.name ?? "")")
                .accessibilityIdentifier("translation")
                .accessibilityShowsLargeContentViewer { Text(store.translation?.name ?? "") }
            }
            .foregroundStyle(Color(theme.ink))
            .glass(in: Capsule(), theme: theme)
            // Offer the reference all free width before considering its abbreviation.
            .layoutPriority(1)

            Spacer(minLength: 0)

            HStack(spacing: 0) {
                tool("magnifyingglass", "Search", id: "search") { store.sheet = .search }
                tool("textformat.size", "Text and theme", id: "settings") { store.sheet = .settings }
                tool("bookmark", "Bookmarks, highlights and notes", id: "study") { store.sheet = .study }
            }
            .padding(.horizontal, 4)
            .glass(in: Capsule(), theme: theme)
        }
        .padding(.horizontal, 16)
    }

    private var shortReference: String {
        let book = store.book(of: store.position.chapter)
        return "\(book.abbrev) \(store.position.chapter - Int(book.firstChapter) + 1)"
    }

    private func referenceLabel(_ title: String, truncates: Bool = false) -> some View {
        Text(title)
            .lineLimit(1)
            .fixedSize(horizontal: !truncates, vertical: false)
            // Fit and size the entire candidate, including its hit target and padding.
            .padding(.leading, 16)
            .padding(.trailing, 10)
            .frame(minWidth: 44, minHeight: 44)
            .accessibilityValue(title)
    }

    private func tool(_ symbol: String, _ label: String, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 17, weight: .medium))
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .foregroundStyle(Color(theme.ink))
        .accessibilityLabel(label)
        .accessibilityIdentifier(id)
        .accessibilityShowsLargeContentViewer { Label(label, systemImage: symbol) }
    }
}
