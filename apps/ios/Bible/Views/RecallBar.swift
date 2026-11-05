import SwiftUI

/// Along the foot of the reader while recalling: back, the passage, what to do, and once it's all
/// shown, the grades, each with when the passage would come back.
struct RecallBar: View {
    @Environment(Store.self) private var store
    let theme: Theme

    var body: some View {
        if let run = store.recall {
            VStack(spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    Button {
                        store.endRecall()
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(Color(theme.ink))
                            .frame(width: 36, height: 36)
                            .background(Color(theme.fill), in: Circle())
                    }
                    .accessibilityLabel("Back to reading")
                    .accessibilityIdentifier("end-recall")
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(run.passage.reference)
                                .font(.subheadline.weight(.semibold))
                                .foregroundStyle(Color(theme.ink))
                            if run.queue.count > 1 {
                                Text("\(run.index + 1) of \(run.queue.count)")
                                    .font(.subheadline)
                                    .foregroundStyle(Color(theme.ink3))
                            }
                        }
                        Text(status(run.status))
                            .font(.footnote)
                            .foregroundStyle(Color(theme.ink2))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 0)
                    if run.status.unavailable {
                        skip
                    } else if !run.status.done {
                        hints
                    }
                }
                if run.status.done {
                    grades
                }
            }
            .padding(14)
            .frame(maxWidth: 520)
            .glass(in: RoundedRectangle(cornerRadius: 28, style: .continuous), theme: theme, interactive: false)
        }
    }

    private func status(_ status: RecallStatus) -> String {
        if status.unavailable { return "Not all of it is in this translation" }
        if status.done { return "How well did you know it?" }
        return status.started ? "Tap for the next word" : "Say it from memory, then tap or drag to check"
    }

    /// On to the next passage, leaving this one due.
    private var skip: some View {
        Button("Skip") { store.skipRecall() }
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Color(theme.onAccent))
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
            .background(Color(theme.accent), in: Capsule())
            .accessibilityHint("Leaves it due, and goes on to the next passage")
            .accessibilityIdentifier("skip-recall")
    }

    /// The hint style, as a pop-up of its two choices.
    private var hints: some View {
        let letters = Binding(get: { store.settings.recallLetters }, set: { store.settings.recallLetters = $0 })
        return Menu {
            Picker("Hints", selection: letters) {
                Text("First letters").tag(true)
                Text("No hints").tag(false)
            }
        } label: {
            HStack(spacing: 4) {
                Text(letters.wrappedValue ? "First letters" : "No hints")
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 9, weight: .semibold))
            }
            .font(.footnote.weight(.medium))
            .foregroundStyle(Color(theme.ink))
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Color(theme.fill), in: Capsule())
        }
        .accessibilityLabel("Hints")
        .accessibilityValue(letters.wrappedValue ? "First letters" : "No hints")
    }

    private var grades: some View {
        let labels = store.gradeLabels
        return HStack(spacing: 8) {
            ForEach(Array([RecallGrade.again, .hard, .good, .easy].enumerated()), id: \.offset) { i, g in
                let on = g == .good
                Button {
                    store.grade(g)
                } label: {
                    VStack(spacing: 1) {
                        Text(g.label)
                            .font(.subheadline.weight(on ? .semibold : .medium))
                        if i < labels.count {
                            Text(labels[i])
                                .font(.caption2)
                                .opacity(0.75)
                        }
                    }
                    .foregroundStyle(on ? Color(theme.onAccent) : Color(theme.ink))
                    .frame(maxWidth: .infinity, minHeight: 48)
                    .background(on ? Color(theme.accent) : Color(theme.fill), in: Capsule())
                }
                .accessibilityLabel(i < labels.count ? "\(g.label), back in \(labels[i])" : g.label)
                .accessibilityIdentifier("grade-\(g.label)")
            }
        }
    }
}

/// In Study: how many memorised passages are due, and the way into recalling them.
struct RecallRow: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let theme: Theme

    var body: some View {
        let due = store.duePassages.count
        let total = store.memorisedPassages.count
        Button {
            store.startRecall()
            dismiss()
        } label: {
            HStack {
                Image(systemName: "rectangle.on.rectangle")
                    .foregroundStyle(Color(theme.accent))
                VStack(alignment: .leading, spacing: 2) {
                    Text("Memory")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(Color(theme.ink))
                    Text(due == 0 ? "\(total) memorised · nothing due" : "\(due) of \(total) due")
                        .font(.footnote)
                        .foregroundStyle(Color(theme.ink2))
                }
                Spacer()
                Text(due > 0 ? "Recall" : "Practise")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Color(theme.onAccent))
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background(Color(theme.accent), in: Capsule())
            }
            .padding(12)
            .background(Color(theme.fill), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .accessibilityIdentifier("recall")
        .id(store.marksVersion)
    }
}

extension RecallGrade {
    var label: String {
        switch self {
        case .again: "Again"
        case .hard: "Hard"
        case .good: "Good"
        case .easy: "Easy"
        }
    }
}
