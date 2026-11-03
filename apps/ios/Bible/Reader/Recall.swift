import UIKit

/// What the reader reports about a recall as it goes.
struct RecallStatus: Equatable {
    var started = false
    var done = false
    /// The translation being read doesn't have the passage.
    var unavailable = false
}

/// A memorised passage being recalled in place. The reader keeps the chapter typeset as for
/// reading; this hides the passage's characters (keeping their place, so nothing moves) and shows
/// them as a horizontal drag reveals them, phrase by phrase. The core says where each character
/// appears (`recallPlan`).
@MainActor
final class RecallSession {
    let passage: MemorisedPassage
    var letters: Bool

    private(set) var plan: RecallPlan
    private(set) var text: ChapterLayout.RecallText
    /// The chapter's layout the text was read from: a new one (new type or width) needs a new plan.
    private(set) weak var layout: ChapterLayout?

    private(set) var position: Float = 0
    private var velocity: Float = 0
    private var target: Float?

    /// Text that isn't being recalled is dimmed this much.
    nonisolated static let dim: CGFloat = 0.28

    init?(passage: MemorisedPassage, layout: ChapterLayout, letters: Bool) {
        guard let text = layout.recallText(first: Int(passage.first.verse), last: Int(passage.last.verse)) else {
            return nil
        }
        self.passage = passage
        self.letters = letters
        self.text = text
        self.layout = layout
        plan = recallPlan(verses: text.verses)
    }

    /// Reads the passage from a new layout of its chapter, keeping how far it is revealed.
    func rebind(to layout: ChapterLayout) {
        guard layout !== self.layout,
            let text = layout.recallText(first: Int(passage.first.verse), last: Int(passage.last.verse))
        else { return }
        self.text = text
        self.layout = layout
        plan = recallPlan(verses: text.verses)
        position = min(position, plan.end)
    }

    // MARK: Where the reveal is

    /// How far the reveal has gone.
    var shown: Float { position }

    var status: RecallStatus {
        RecallStatus(started: position > 0.01, done: shown >= plan.end - 0.01)
    }

    private var stops: [Float] { [0] + plan.phrases.map(\.stop) }

    func amount(_ place: Float) -> CGFloat {
        let t = min(max((shown - place) / plan.soft, 0), 1)
        return CGFloat(t * t * (3 - 2 * t))
    }

    // MARK: Moving it

    /// Moves the reveal by `dx` points of a drag across a column `width` wide; `true` when it
    /// crosses a phrase's end.
    @discardableResult
    func drag(by dx: CGFloat, width: CGFloat) -> Bool {
        let before = stops.filter { $0 <= shown + 0.01 }.count
        position = min(max(position + Float(dx / max(width, 200)) * plan.end, 0), plan.end)
        target = nil
        return stops.filter { $0 <= shown + 0.01 }.count != before
    }

    /// The drag ended: spring back to the end of the last word it fully showed, never further
    /// (as the core's `Reveal::settle`), without the drag's momentum carrying it on first.
    func release() {
        velocity = 0
        target = ([0] + plan.wordRests).last { $0 <= position + 0.05 } ?? 0
    }

    /// Shows the next word.
    func nextWord() {
        let from = target ?? shown
        target = plan.wordRests.first { $0 > from + 0.05 } ?? plan.end
    }

    #if DEBUG
        func showPhrases(_ n: Int) {
            position = stops[min(max(n, 0), stops.count - 1)]
            target = nil
        }
    #endif

    func reset() {
        target = 0
    }

    var animating: Bool { target != nil }

    /// Advances the spring `dt` seconds; `false` once it rests.
    func tick(_ dt: Float, reduceMotion: Bool) -> Bool {
        guard let target else { return false }
        if reduceMotion {
            position = target
            self.target = nil
            return false
        }
        let dt = min(dt, 1 / 30)
        let stiffness: Float = 230
        let damping = 2 * stiffness.squareRoot() * 0.92
        velocity += (stiffness * (target - position) - damping * velocity) * dt
        position += velocity * dt
        if abs(target - position) < 0.01 && abs(velocity) < 0.05 {
            position = target
            velocity = 0
            self.target = nil
            return false
        }
        return true
    }

    // MARK: Drawing

    /// The passage's storage, in runs of characters to draw at one opacity: verse numbers and
    /// spaces fully, words as far as the reveal shows them (first letters as hints, with
    /// `letters`). A hidden run has opacity 0.
    func runs() -> [(chars: NSRange, alpha: CGFloat)] {
        var out: [(NSRange, CGFloat)] = []
        func add(_ location: Int, _ length: Int, _ alpha: CGFloat) {
            let a = (alpha * 32).rounded() / 32
            if let last = out.last, NSMaxRange(last.0) == location, last.1 == a {
                out[out.count - 1].0.length += length
            } else if length > 0 {
                out.append((NSRange(location: location, length: length), a))
            }
        }
        var at = text.range.location
        for (v, verse) in plan.verses.enumerated() {
            for (i, place) in verse.places.enumerated() {
                let location = text.locations[v][i]
                let length = text.lengths[v][i]
                if location > at { add(at, location - at, 1) }
                var alpha: CGFloat = 1
                if place >= 0 {
                    let a = amount(place)
                    alpha = letters && verse.firstLetters[i] ? 0.45 + 0.55 * a : a
                }
                add(location, length, alpha)
                at = location + length
            }
        }
        if NSMaxRange(text.range) > at { add(at, NSMaxRange(text.range) - at, 1) }
        return out
    }

    /// The faint rules standing for hidden letters, each with how strongly it shows (chapter
    /// coordinates).
    func rules() -> [(rect: CGRect, alpha: CGFloat)] {
        guard letters else { return [] }
        var out: [(CGRect, CGFloat)] = []
        for (v, verse) in plan.verses.enumerated() {
            for (i, place) in verse.places.enumerated() where place >= 0 && !verse.firstLetters[i] {
                let a = amount(place)
                guard let r = text.rules[v][i], a < 1 else { continue }
                out.append((r, 1 - a))
            }
        }
        return out
    }
}
