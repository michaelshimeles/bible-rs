import SwiftUI

/// The books by section of the canon: the sections under their testaments, the open section's
/// books folded out beneath it, then the chosen book's chapters. It opens on the book being read,
/// so its chapters are one tap away and any chapter at most three.
struct BookSections: View {
    @Environment(Store.self) private var store
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let theme: Theme
    let go: (Int) -> Void

    /// The book whose chapters show; `nil` is the book being read. Its section is the one that
    /// unfolds.
    @State private var chosen: UInt8?
    /// Whether the chosen book's section has its books folded away.
    @State private var folded = false
    /// What to bring into view next, and how many times that's been asked, so choosing the
    /// book already chosen scrolls to it again.
    @State private var reveal = Reveal.current
    @State private var reveals = 0
    /// The sections' height with the open one's books, as laid out.
    @State private var browseHeight: CGFloat = 0
    /// Compact rows, as in a sidebar, so a phone has room for the chapters beneath.
    @ScaledMetric(relativeTo: .subheadline) private var rowHeight: CGFloat = 32
    @ScaledMetric(relativeTo: .footnote) private var headerHeight: CGFloat = 28
    @ScaledMetric(relativeTo: .subheadline) private var bookIndent: CGFloat = 12

    private enum Reveal {
        /// The chapter being read.
        case current
        /// A section just opened.
        case books
        /// The chapters of a book just chosen.
        case chapters
    }

    private var current: BookInfo { store.book(of: store.position.chapter) }
    private var book: BookInfo { chosen.map { store.bookList[Int($0)] } ?? current }
    private var section: BibleSection { store.section(of: book) }
    /// One column of sections rather than a column per testament, at the largest text sizes.
    private var stacked: Bool { dynamicTypeSize.isAccessibilitySize }

    var body: some View {
        GeometryReader { geo in
            let room = browseRoom(in: geo.size.height)
            ScrollViewReader { proxy in
                Group {
                    if let room {
                        // The sections stay put above the chapters, and scroll only when the open
                        // section's books don't fit.
                        VStack(spacing: 0) {
                            ScrollView {
                                browse
                                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                                        // Folding animates; the first layout doesn't.
                                        withAnimation(browseHeight == 0 ? nil : .smooth(duration: 0.25)) {
                                            browseHeight = height
                                        }
                                    }
                            }
                            .frame(height: min(browseHeight, room))
                            .scrollBounceBehavior(.basedOnSize)
                            chaptersHeader
                            ScrollView {
                                chapters.padding(.bottom, 16)
                            }
                            .scrollBounceBehavior(.basedOnSize)
                        }
                    } else {
                        ScrollView {
                            VStack(spacing: 0) {
                                browse
                                chaptersHeader.id(Reveal.chapters)
                                chapters.padding(.bottom, 16)
                            }
                        }
                    }
                }
                .task(id: reveals) {
                    // Once the new books and chapters are laid out.
                    await Task.yield()
                    let fixed = room != nil
                    let chapter = book.index == current.index ? store.position.chapter : Int(book.firstChapter)
                    switch reveal {
                    case .current:
                        if fixed { proxy.scrollTo(Self.id(book)) }
                        proxy.scrollTo(ChapterGrid.id(chapter), anchor: .center)
                    case .books:
                        withAnimation { proxy.scrollTo(Self.id(section), anchor: .top) }
                        if fixed { proxy.scrollTo(ChapterGrid.id(chapter), anchor: .center) }
                    case .chapters:
                        if fixed {
                            proxy.scrollTo(ChapterGrid.id(chapter), anchor: .center)
                        } else {
                            withAnimation { proxy.scrollTo(Reveal.chapters, anchor: .top) }
                        }
                    }
                }
            }
        }
    }

    /// How tall the sections may be in `height` with two and a half rows of chapters beneath, or
    /// `nil` if that leaves too little for the sections and a few books (a phone on its side, the
    /// largest text sizes), and everything scrolls together instead.
    private func browseRoom(in height: CGFloat) -> CGFloat? {
        guard !stacked else { return nil }
        let room = height - (headerHeight + 4) - 2.5 * (44 + 8)
        let rows = store.sections.filter { !$0.newTestament }.count + 3
        return room >= headerHeight + 4 + CGFloat(rows) * (rowHeight + 2) ? room : nil
    }

    /// Each testament's sections, the open one with its books beneath it.
    private var browse: some View {
        let layout =
            stacked
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 0))
            : AnyLayout(HStackLayout(alignment: .top, spacing: 0))
        return layout {
            testament("Old Testament", store.sections.filter { !$0.newTestament })
            testament("New Testament", store.sections.filter(\.newTestament))
        }
        .padding(.horizontal, 10)
    }

    private func testament(_ title: String, _ sections: [BibleSection]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            header(title)
            ForEach(sections, id: \.name) { s in
                sectionRow(s)
                if s.name == section.name && !folded {
                    ForEach(store.bookList.filter { s.contains($0) }, id: \.index) { b in
                        bookRow(b)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
        // VoiceOver reads down one testament, then the other.
        .accessibilityElement(children: .contain)
    }

    /// A section's row, which folds its books in and out: the open one is bold, and the one
    /// holding the chapter being read is in the accent.
    private func sectionRow(_ s: BibleSection) -> some View {
        let open = s.name == section.name && !folded
        let reading = s.contains(current)
        let state = open ? "Expanded" : "Collapsed"
        return Button {
            withAnimation(.smooth(duration: 0.25)) {
                if s.name == section.name {
                    folded.toggle()
                } else {
                    folded = false
                    choose(reading ? current.index : s.firstBook, revealing: .books)
                }
            }
        } label: {
            HStack(spacing: 6) {
                Text(s.name)
                    .font(.subheadline.weight(open ? .semibold : reading ? .medium : .regular))
                    .foregroundStyle(reading ? Color(theme.accent) : Color(theme.ink))
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
                Spacer(minLength: 0)
                Image(systemName: "chevron.down")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Color(theme.ink3))
                    .rotationEffect(.degrees(open ? 0 : -90))
                    // The value says whether it's open.
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 10)
            .frame(minHeight: rowHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .id(Self.id(s))
        .accessibilityValue(reading ? "\(state), has the chapter you're reading" : state)
        .accessibilityHint(open ? "Folds its books away" : "Shows its books")
    }

    /// A book under its section: the chosen one sits on the soft accent, as the Mac sidebar's
    /// selected rows do, and the one being read is in the accent.
    private func bookRow(_ b: BookInfo) -> some View {
        let selected = b.index == book.index
        let reading = b.index == current.index
        return Button {
            if b.chapters == 1 {
                go(Int(b.firstChapter))
            } else {
                choose(b.index, revealing: .chapters)
            }
        } label: {
            Text(b.name)
                .font(.subheadline.weight(selected ? .semibold : reading ? .medium : .regular))
                .foregroundStyle(selected || reading ? Color(theme.accent) : Color(theme.ink))
                .lineLimit(1)
                .minimumScaleFactor(0.85)
                .padding(.horizontal, 10)
                .frame(maxWidth: .infinity, minHeight: rowHeight, alignment: .leading)
                .background(
                    selected ? Color(theme.accentSoft) : .clear,
                    in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.leading, bookIndent)
        .id(Self.id(b))
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityValue(reading ? "Book you're reading" : "")
        .accessibilityHint(b.chapters == 1 ? "Opens the book" : "Shows its chapters")
    }

    /// Shows the chapters of the book at `index`, then brings `what` into view.
    private func choose(_ index: UInt8, revealing what: Reveal) {
        chosen = index
        reveal = what
        reveals += 1
    }

    private var chaptersHeader: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            header("Chapters")
                .fixedSize()
            Text(book.name)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Color(theme.ink2))
                .accessibilityHidden(true)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Chapters of \(book.name)")
        .accessibilityAddTraits(.isHeader)
    }

    private var chapters: some View {
        ChapterGrid(book: book, reading: store.position.chapter, theme: theme, go: go)
            .padding(.horizontal, 20)
    }

    private func header(_ title: String) -> some View {
        Text(title)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Color(theme.ink3))
            .padding(.horizontal, 10)
            .frame(minHeight: headerHeight, alignment: .bottomLeading)
            .padding(.bottom, 4)
            .accessibilityAddTraits(.isHeader)
    }

    /// The scroll ids of a section's row and a book's row.
    private static func id(_ section: BibleSection) -> String { "section-\(section.name)" }
    private static func id(_ book: BookInfo) -> String { "book-\(book.index)" }
}
