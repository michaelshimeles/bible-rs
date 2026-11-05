//! Memorising passages: when each one is next due, and how a recall reveals it.
//!
//! Scheduling is FSRS-6 with its default weights, ported from Store the Word's scheduler (itself
//! a port of ts-fsrs 5.4.1, MIT-licensed: see `LICENSES/ts-fsrs-MIT.txt`): retention 0.9,
//! intervals of 1 to 36,500 days and no fuzz. There are no learning steps to set: Again leaves a
//! passage due, so it comes back in the same session, and FSRS-6's same-day terms (w17 to w19)
//! schedule a recall later the same day.
//!
//! A recall reveals a passage along one line, phrase by phrase: [`Reveal`] gives every char of
//! the passage a place on that line, with a pause after each phrase, so a reveal can stop between
//! phrases with all of one shown and none of the next.

use std::ops::Range;

use crate::books::VerseRef;
use crate::model::VerseView;

const DAY: i64 = 86_400;

/// How well a passage was recalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grade {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Grade {
    pub const ALL: [Grade; 4] = [Grade::Again, Grade::Hard, Grade::Good, Grade::Easy];

    /// The grade a [`Grade::name`] names.
    pub fn from_name(name: &str) -> Option<Grade> {
        Grade::ALL.into_iter().find(|g| g.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Grade::Again => "Again",
            Grade::Hard => "Hard",
            Grade::Good => "Good",
            Grade::Easy => "Easy",
        }
    }
}

/// One recall of a passage, kept so its schedule can always be worked out again (a new scheduler,
/// or weights fitted to a reader's own recalls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Review {
    /// Unix seconds.
    pub at: i64,
    pub grade: Grade,
}

/// A passage's place in the schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Schedule {
    /// Unix seconds when it is next due.
    pub due: i64,
    pub stability: f64,
    pub difficulty: f64,
    pub reps: u32,
    pub lapses: u32,
    /// Unix seconds of the last recall; `None` until the first.
    pub last_review: Option<i64>,
}

const W: [f64; 21] = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614,
    0.2629, 1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];
const RETENTION: f64 = 0.9;
const MAX_DAYS: f64 = 36_500.0;

fn round8(x: f64) -> f64 {
    (x * 100_000_000.0 + 0.5).floor() / 100_000_000.0
}

fn clamp(x: f64, low: f64, high: f64) -> f64 {
    x.max(low).min(high)
}

fn initial_difficulty(grade: f64) -> f64 {
    round8(W[4] - ((grade - 1.0) * W[5]).exp() + 1.0)
}

impl Schedule {
    /// A passage just added: due now.
    pub fn new(now: i64) -> Self {
        Self { due: now, stability: 0.0, difficulty: 0.0, reps: 0, lapses: 0, last_review: None }
    }

    pub fn is_due(&self, now: i64) -> bool {
        self.due <= now
    }

    /// When the passage would next be due after a recall at `now` graded each way, in
    /// [`Grade::ALL`] order.
    pub fn preview(&self, now: i64) -> [i64; 4] {
        Grade::ALL.map(|g| self.next(g, now).due)
    }

    /// The schedule after a recall graded `grade` at `now`.
    pub fn next(&self, grade: Grade, now: i64) -> Schedule {
        // ts-fsrs counts UTC date boundaries crossed, not whole 24-hour periods.
        let elapsed =
            self.last_review.map_or(0.0, |last| (now.div_euclid(DAY) - last.div_euclid(DAY)).max(0) as f64);
        let decay = -W[20];
        let factor = round8((RETENTION.ln() / decay).exp() - 1.0);
        let interval_modifier = round8((RETENTION.powf(1.0 / decay) - 1.0) / factor);
        let (s, d) = (self.stability, self.difficulty);
        let retrievability = if s > 0.0 { round8((1.0 + factor * elapsed / s).powf(decay)) } else { 0.0 };
        let reviewed = self.last_review.is_some();
        let same_day = reviewed && elapsed == 0.0;
        // The last grade was Again (it left the passage due at once): relearning, as ts-fsrs has it.
        let relearning = self.last_review == Some(self.due);

        let mut candidates = [*self; 4];
        let mut days = [0.0f64; 4];
        for (i, next) in candidates.iter_mut().enumerate() {
            let rating = (i + 1) as f64;
            if d == 0.0 && s == 0.0 {
                next.difficulty = clamp(initial_difficulty(rating), 1.0, 10.0);
                next.stability = W[i].max(0.1);
            } else {
                let delta = -W[6] * (rating - 3.0);
                let damped = d + round8(delta * (10.0 - d) / 9.0);
                next.difficulty =
                    clamp(round8(W[7] * initial_difficulty(4.0) + (1.0 - W[7]) * damped), 1.0, 10.0);
                next.stability = if same_day {
                    let growth = s.powf(-W[19]) * (W[17] * (rating - 3.0 + W[18])).exp();
                    let growth = if i >= 1 { growth.max(1.0) } else { growth };
                    round8(clamp(s * growth, 0.001, MAX_DAYS))
                } else if i == 0 {
                    let after_fail = round8(clamp(
                        W[11]
                            * d.powf(-W[12])
                            * ((s + 1.0).powf(W[13]) - 1.0)
                            * ((1.0 - retrievability) * W[14]).exp(),
                        0.001,
                        MAX_DAYS,
                    ));
                    clamp(round8(s / (W[17] * W[18]).exp()), 0.001, after_fail)
                } else {
                    let hard = if i == 1 { W[15] } else { 1.0 };
                    let easy = if i == 3 { W[16] } else { 1.0 };
                    let grown = s
                        * (1.0
                            + W[8].exp()
                                * (11.0 - d)
                                * s.powf(-W[9])
                                * (((1.0 - retrievability) * W[10]).exp() - 1.0)
                                * hard
                                * easy);
                    round8(clamp(grown, 0.001, MAX_DAYS))
                };
            }
            next.reps += 1;
            next.last_review = Some(now);
            if reviewed && !relearning && i == 0 {
                next.lapses += 1;
            }
            days[i] = clamp((next.stability * interval_modifier + 0.5).floor(), 1.0, MAX_DAYS);
        }
        // Again leaves it due: it comes back in this session. Relearning, the others only keep
        // their order; otherwise each waits at least a day longer than the one before.
        days[0] = 0.0;
        let step = if relearning { 0.0 } else { 1.0 };
        for i in 1..4 {
            days[i] = days[i].max(days[i - 1] + step);
        }
        let i = grade as usize - 1;
        let mut out = candidates[i];
        out.due = now + days[i] as i64 * DAY;
        out
    }
}

/// How long from `now` until `due`, briefly: "today", "1 day", "5 days", "3 weeks", "2 months",
/// "1 year".
pub fn interval_label(now: i64, due: i64) -> String {
    let days = ((due - now) as f64 / DAY as f64).round() as i64;
    let (n, unit) = match days {
        ..1 => return "today".to_string(),
        1..14 => (days, "day"),
        14..60 => ((days as f64 / 7.0).round() as i64, "week"),
        60..365 => ((days as f64 / 30.0).round() as i64, "month"),
        _ => ((days as f64 / 365.0).round() as i64, "year"),
    };
    if n == 1 { format!("1 {unit}") } else { format!("{n} {unit}s") }
}

/// A passage being memorised: verses `first..=last` of one chapter, learnt in one translation.
#[derive(Debug, Clone, PartialEq)]
pub struct Memorised {
    pub first: VerseRef,
    pub last: VerseRef,
    pub translation: String,
    pub created: i64,
    pub schedule: Schedule,
}

impl Memorised {
    pub fn contains(&self, v: VerseRef) -> bool {
        self.first <= v && v <= self.last
    }

    pub fn overlaps(&self, first: VerseRef, last: VerseRef) -> bool {
        self.first <= last && first <= self.last
    }

    /// "John 3:16" or "John 3:16–18".
    pub fn reference(&self) -> String {
        if self.first == self.last {
            self.first.to_string()
        } else {
            format!("{}–{}", self.first, self.last.verse)
        }
    }
}

/// Where each char of a passage appears as a recall reveals it.
///
/// The reveal runs along one line measured in units, about one per char. A char starts to show
/// when the reveal reaches its place and is fully shown `soft` units later, so the reveal's edge
/// is soft. After each phrase the line pauses for longer than that, so a reveal resting at a
/// phrase's [`stop`](Phrase::stop) shows all of that phrase and none of the next.
#[derive(Debug, Clone, PartialEq)]
pub struct Reveal {
    /// Each verse's chars: their place on the line, or `None` for spaces.
    pub places: Vec<Vec<Option<f32>>>,
    /// Each verse's chars: whether it is a word's first letter.
    pub first_letters: Vec<Vec<bool>>,
    pub phrases: Vec<Phrase>,
    /// Where each word is fully shown, in order.
    pub word_ends: Vec<f32>,
    /// Where the whole passage is shown.
    pub end: f32,
    pub soft: f32,
}

/// A phrase: chars of one or more verses (verse index, char range), ending at punctuation or at a
/// line or verse end.
#[derive(Debug, Clone, PartialEq)]
pub struct Phrase {
    pub parts: Vec<(usize, Range<usize>)>,
    /// Where its last char is fully shown.
    pub shown: f32,
    /// Where a reveal rests after it: in the pause before the next phrase.
    pub stop: f32,
}

/// The pause after a phrase, beyond the soft edge.
const PAUSE: f32 = 3.0;

fn ends_phrase(word: &[char]) -> bool {
    let mut trimmed = word.iter().rev().skip_while(|c| matches!(c, '”' | '’' | '"' | '\'' | ')' | ']'));
    matches!(trimmed.next(), Some(',' | ';' | ':' | '.' | '!' | '?' | '—'))
}

impl Reveal {
    /// The reveal of verses' texts. `breaks` are each verse's char offsets where a new line
    /// starts inside it (a poetry line, a paragraph): stored text has no space there.
    pub fn new(verses: &[(&str, Vec<usize>)], soft: f32) -> Self {
        let soft = soft.max(1.0);
        let mut places = Vec::with_capacity(verses.len());
        let mut first_letters = Vec::with_capacity(verses.len());
        let mut phrases: Vec<Phrase> = Vec::new();
        let mut word_ends = Vec::new();
        let mut unit = 0.0f32;
        let mut current: Option<Phrase> = None;

        let close = |current: &mut Option<Phrase>, phrases: &mut Vec<Phrase>, unit: &mut f32, last: f32| {
            if let Some(mut p) = current.take() {
                p.shown = last + soft;
                p.stop = p.shown + PAUSE / 2.0;
                *unit = last + 1.0 + soft + PAUSE;
                phrases.push(p);
            }
        };

        for (vi, (text, breaks)) in verses.iter().enumerate() {
            let chars: Vec<char> = text.chars().collect();
            let mut verse_places = vec![None; chars.len()];
            let mut verse_firsts = vec![false; chars.len()];
            // Words: runs of non-space chars, also split at line breaks and after a dash.
            let mut words: Vec<Range<usize>> = Vec::new();
            let mut start: Option<usize> = None;
            for (i, c) in chars.iter().enumerate() {
                if breaks.contains(&i)
                    && let Some(s) = start.take()
                {
                    words.push(s..i);
                }
                if c.is_whitespace() {
                    if let Some(s) = start.take() {
                        words.push(s..i);
                    }
                    continue;
                }
                start.get_or_insert(i);
                if *c == '—' {
                    words.push(start.take().unwrap()..i + 1);
                }
            }
            if let Some(s) = start {
                words.push(s..chars.len());
            }

            let mut last_place = unit;
            for (wi, w) in words.iter().enumerate() {
                if current.as_ref().is_some_and(|p| !p.parts.is_empty()) {
                    // Between words, a pause as long as the soft edge: a reveal resting at a
                    // word's end shows none of the next.
                    unit += soft;
                }
                let phrase =
                    current.get_or_insert_with(|| Phrase { parts: Vec::new(), shown: 0.0, stop: 0.0 });
                match phrase.parts.last_mut() {
                    Some((v, r)) if *v == vi => r.end = w.end,
                    _ => phrase.parts.push((vi, w.clone())),
                }
                let first_letter = (w.start..w.end).find(|&i| chars[i].is_alphanumeric());
                for i in w.clone() {
                    verse_places[i] = Some(unit);
                    verse_firsts[i] = Some(i) == first_letter;
                    last_place = unit;
                    unit += 1.0;
                }
                unit -= 1.0;
                word_ends.push(unit + soft);
                unit += 1.0;
                let line_ends = words
                    .get(wi + 1)
                    .is_none_or(|next| breaks.iter().any(|&b| w.end <= b && b <= next.start));
                if ends_phrase(&chars[w.clone()]) || line_ends {
                    close(&mut current, &mut phrases, &mut unit, last_place);
                }
            }
            places.push(verse_places);
            first_letters.push(verse_firsts);
        }
        let last = unit - 1.0;
        close(&mut current, &mut phrases, &mut unit, last);
        let end = phrases.last().map_or(0.0, |p| p.shown);
        if let Some(p) = phrases.last_mut() {
            p.stop = end;
        }
        Self { places, first_letters, phrases, word_ends, end, soft }
    }

    /// The reveal of verses as a translation has them.
    pub fn of_verses(verses: &[VerseView<'_>], soft: f32) -> Self {
        let input: Vec<(&str, Vec<usize>)> = verses
            .iter()
            .map(|v| {
                let breaks = v
                    .marks
                    .iter()
                    .filter(|m| m.kind.breaks_line() && m.offset > 0)
                    .filter_map(|m| v.text.get(..m.offset as usize).map(|before| before.chars().count()))
                    .collect();
                (v.text, breaks)
            })
            .collect();
        Self::new(&input, soft)
    }

    /// How much of a char at `place` shows when the reveal is at `position`: 0 to 1, eased.
    pub fn amount(&self, place: f32, position: f32) -> f32 {
        let t = ((position - place) / self.soft).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Where a reveal can rest: the start, and after each phrase.
    pub fn stops(&self) -> impl DoubleEndedIterator<Item = f32> + '_ {
        std::iter::once(0.0).chain(self.phrases.iter().map(|p| p.stop))
    }

    pub fn nearest_stop(&self, position: f32) -> f32 {
        self.stops().min_by(|a, b| (a - position).abs().total_cmp(&(b - position).abs())).unwrap_or(0.0)
    }

    /// The first stop after `position`, or the end.
    pub fn next_stop(&self, position: f32) -> f32 {
        self.stops().find(|s| *s > position + 0.05).unwrap_or(self.end)
    }

    /// The last stop before `position`, or the start.
    pub fn previous_stop(&self, position: f32) -> f32 {
        self.stops().rev().find(|s| *s < position - 0.05).unwrap_or(0.0)
    }

    /// Where a reveal rests with each word shown: just after it, or at its phrase's stop for the
    /// last word of a phrase.
    pub fn word_rests(&self) -> impl DoubleEndedIterator<Item = f32> + '_ {
        self.word_ends
            .iter()
            .map(|&e| self.phrases.iter().find(|p| (p.shown - e).abs() < 0.01).map_or(e, |p| p.stop))
    }

    pub fn next_word(&self, position: f32) -> f32 {
        self.word_rests().find(|r| *r > position + 0.05).unwrap_or(self.end)
    }

    pub fn previous_word(&self, position: f32) -> f32 {
        self.word_rests().rev().find(|r| *r < position - 0.05).unwrap_or(0.0)
    }

    /// Where a reveal let go of at `position` comes to rest: back at the end of the last word it
    /// fully showed (or the start). Never further, so letting go never shows more.
    pub fn settle(&self, position: f32) -> f32 {
        std::iter::once(0.0).chain(self.word_rests()).rev().find(|r| *r <= position + 0.05).unwrap_or(0.0)
    }

    pub fn is_done(&self, position: f32) -> bool {
        position >= self.end - 0.01
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000;

    #[test]
    fn a_new_passage_is_due_at_once_and_good_waits_days() {
        let s = Schedule::new(NOW);
        assert!(s.is_due(NOW));
        let good = s.next(Grade::Good, NOW);
        let again = s.next(Grade::Again, NOW);
        let easy = s.next(Grade::Easy, NOW);
        assert!(!good.is_due(NOW + DAY - 1));
        assert!(again.due <= good.due && good.due < easy.due, "{again:?} {good:?} {easy:?}");
        assert_eq!(good.reps, 1);
        assert_eq!(again.lapses, 0, "a first recall is not a lapse");
    }

    #[test]
    fn intervals_grow_with_good_recalls_and_shrink_after_a_lapse() {
        let mut s = Schedule::new(NOW).next(Grade::Good, NOW);
        let mut gaps = Vec::new();
        for _ in 0..4 {
            let now = s.due;
            let next = s.next(Grade::Good, now);
            gaps.push(next.due - now);
            s = next;
        }
        assert!(gaps.windows(2).all(|w| w[1] > w[0]), "{gaps:?}");
        let lapsed = s.next(Grade::Again, s.due);
        assert_eq!(lapsed.lapses, 1);
        assert!(lapsed.due - s.due < *gaps.last().unwrap());
    }

    #[test]
    fn first_ratings_match_fsrs_initial_stability() {
        let s = Schedule::new(NOW);
        for (g, w) in Grade::ALL.into_iter().zip(W) {
            assert_eq!(s.next(g, NOW).stability, w.max(0.1));
        }
    }

    #[test]
    fn each_grade_previews_when_it_comes_back() {
        let s = Schedule::new(NOW);
        let due = s.preview(NOW);
        assert!(due.windows(2).all(|w| w[0] < w[1]), "Again soonest, Easy latest: {due:?}");
        let labels: Vec<String> = due.iter().map(|&d| interval_label(NOW, d)).collect();
        assert_eq!(labels, ["today", "1 day", "2 days", "8 days"]);
        assert_eq!(interval_label(NOW, NOW + 20 * DAY), "3 weeks");
        assert_eq!(interval_label(NOW, NOW + 100 * DAY), "3 months");
        assert_eq!(interval_label(NOW, NOW + 800 * DAY), "2 years");
    }

    #[test]
    fn again_leaves_it_due_and_a_recall_later_that_day_schedules_it() {
        let again = Schedule::new(NOW).next(Grade::Again, NOW);
        assert!(again.is_due(NOW), "Again brings it back in the same session");
        let later = NOW + 600;
        let good = again.next(Grade::Good, later);
        assert!(good.stability > again.stability, "same-day terms grow it a little");
        assert!(!good.is_due(later + DAY - 1), "then it waits a day or more");
        let labels: Vec<String> = again.preview(later).iter().map(|&d| interval_label(later, d)).collect();
        assert_eq!(labels[2], "1 day", "Good after Again isn't pushed back to keep a gap: {labels:?}");
        let again_twice = again.next(Grade::Again, later);
        assert!(again_twice.stability < again.stability);
        let lapsed = good.next(Grade::Again, good.due).next(Grade::Again, good.due + 60);
        assert_eq!(lapsed.lapses, 1, "Again while relearning isn't another lapse");
    }

    #[test]
    fn practising_early_never_brings_a_passage_back_sooner() {
        let s = Schedule::new(NOW).next(Grade::Good, NOW);
        for at in [NOW + 600, NOW + DAY] {
            let practised = s.next(Grade::Good, at);
            assert!(practised.due >= s.due, "{at}: {practised:?} vs {s:?}");
        }
    }

    fn phrase_text(r: &Reveal, texts: &[&str], i: usize) -> String {
        r.phrases[i]
            .parts
            .iter()
            .map(|(v, range)| texts[*v].chars().skip(range.start).take(range.len()).collect::<String>())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    #[test]
    fn phrases_end_at_punctuation_and_verse_ends() {
        let texts = ["For God so loved the world, that he gave his only begotten Son.", "And so on"];
        let r = Reveal::new(&[(texts[0], vec![]), (texts[1], vec![])], 4.0);
        let phrases: Vec<String> = (0..r.phrases.len()).map(|i| phrase_text(&r, &texts, i)).collect();
        assert_eq!(
            phrases,
            ["For God so loved the world,", "that he gave his only begotten Son.", "And so on"]
        );
    }

    #[test]
    fn a_line_break_without_a_space_splits_words_and_phrases() {
        let text = "I have hidden Your word in my heartthat I might not sin against You.";
        let at = text.find("that").unwrap();
        let r = Reveal::new(&[(text, vec![at])], 4.0);
        assert_eq!(phrase_text(&r, &[text], 0), "I have hidden Your word in my heart");
        assert!(r.first_letters[0][at], "the line's first word has its own first letter");
    }

    #[test]
    fn resting_at_a_stop_shows_one_phrase_and_none_of_the_next() {
        let text = "Jesus wept, and the Jews said, Behold how he loved him!";
        let r = Reveal::new(&[(text, vec![])], 5.0);
        let stop = r.phrases[0].stop;
        let shown: String = text
            .chars()
            .zip(&r.places[0])
            .filter(|(_, p)| p.is_some_and(|p| r.amount(p, stop) >= 1.0))
            .map(|(c, _)| c)
            .collect();
        assert_eq!(shown, "Jesuswept,");
        let started = r.places[0].iter().flatten().filter(|p| r.amount(**p, stop) > 0.0).count();
        assert_eq!(started, "Jesuswept,".len());
        assert_eq!(r.next_stop(0.0), stop);
        assert_eq!(r.previous_stop(stop), 0.0);
        assert!(r.is_done(r.next_stop(r.phrases[2].stop - 0.1)));
    }

    #[test]
    fn letting_go_settles_back_on_the_last_whole_word() {
        let r = Reveal::new(&[("In the beginning was the Word, and the Word was God.", vec![])], 5.0);
        let rests: Vec<f32> = r.word_rests().collect();
        // Resting at a word's end shows all of it and none of the next.
        let shown = |at: f32| -> String {
            let text = "In the beginning was the Word, and the Word was God.";
            text.chars()
                .zip(&r.places[0])
                .filter(|(_, p)| p.is_some_and(|p| r.amount(p, at) > 0.0))
                .map(|(c, _)| c)
                .collect()
        };
        assert_eq!(shown(rests[2]), "Inthebeginning");
        assert!(r.places[0].iter().flatten().all(|p| {
            let a = r.amount(*p, rests[2]);
            a == 0.0 || a == 1.0
        }));
        // Just past "the": back to the end of "the".
        assert_eq!(r.settle(rests[1] + 1.0), rests[1]);
        // Most of the way through "beginning": still back to "the".
        assert_eq!(r.settle(rests[2] - 1.0), rests[1]);
        assert_eq!(r.settle(rests[2]), rests[2]);
        assert_eq!(r.settle(0.5), 0.0);
        assert_eq!(r.settle(r.end), r.end);
    }
}
