//! Reconstructing the paragraph tree from a flat sequence of designators.
//!
//! eCFR emits a section's paragraphs as a flat run of `<P>` elements. The
//! hierarchy lives only in the designator that opens each one — `(a)`, `(1)`,
//! `(i)`, `(A)` — so the tree has to be rebuilt by reading the sequence.
//!
//! The trap is that the numbering systems overlap. `(i)` is the ninth
//! lowercase letter *and* the first lowercase roman numeral; `(ii)` is the
//! second roman numeral *and* the letter that follows `(hh)`. A depth table
//! keyed on the token cannot decide between them.
//!
//! What decides is the sequence. Each open level remembers the system it is
//! counting in and the value it currently holds. An incoming designator is
//! tested against the successor of every open level, deepest first; the first
//! level whose successor it matches is the level it belongs to, and everything
//! below that level closes. A designator that succeeds nothing open must be
//! opening a new level, and the only tokens that can open one are the first
//! value of a system — `a`, `1`, `i`, `A`, `I` — which is unambiguous.
//!
//! So `(h)` then `(i)` advances the letters, while `(a)(2)` then `(i)` opens
//! romans, from the same token and with no lookahead.

use std::fmt;

/// The numbering systems the CFR counts paragraphs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    LowerAlpha,
    Arabic,
    LowerRoman,
    UpperAlpha,
    UpperRoman,
}

impl System {
    /// The token a level opens with. Openers do not overlap across systems,
    /// which is what makes a new level unambiguous.
    fn opener(self) -> &'static str {
        match self {
            Self::LowerAlpha => "a",
            Self::Arabic => "1",
            Self::LowerRoman => "i",
            Self::UpperAlpha => "A",
            Self::UpperRoman => "I",
        }
    }

    /// The system a level opening with `token` must be counting in.
    fn opening(token: &str) -> Option<Self> {
        [
            Self::LowerAlpha,
            Self::Arabic,
            Self::LowerRoman,
            Self::UpperAlpha,
            Self::UpperRoman,
        ]
        .into_iter()
        .find(|system| system.opener() == token)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LowerAlpha => "lower-alpha",
            Self::Arabic => "arabic",
            Self::LowerRoman => "lower-roman",
            Self::UpperAlpha => "upper-alpha",
            Self::UpperRoman => "upper-roman",
        }
    }

    /// The token that follows `current` in this system.
    fn successor(self, current: &str) -> Option<String> {
        match self {
            Self::Arabic => current.parse::<u32>().ok().map(|n| (n + 1).to_string()),
            Self::LowerAlpha => alpha_successor(current, false),
            Self::UpperAlpha => alpha_successor(current, true),
            Self::LowerRoman => roman_value(current).map(|n| roman(n + 1, false)),
            Self::UpperRoman => roman_value(current).map(|n| roman(n + 1, true)),
        }
    }
}

/// CFR letter designators run `a … z`, then double as `aa … zz`, then triple.
/// They are not base-26 numerals: `z` is followed by `aa`, never by `ba`.
fn alpha_successor(current: &str, upper: bool) -> Option<String> {
    let mut chars = current.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || first.is_ascii_uppercase() != upper {
        return None;
    }
    let width = current.chars().count();
    if !current.chars().all(|c| c == first) {
        return None;
    }
    let last = if upper { 'Z' } else { 'z' };
    let start = if upper { 'A' } else { 'a' };
    if first == last {
        return Some(start.to_string().repeat(width + 1));
    }
    let next = char::from(first as u8 + 1);
    Some(next.to_string().repeat(width))
}

const ROMAN: [(u32, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

fn roman(mut value: u32, upper: bool) -> String {
    let mut out = String::new();
    for (weight, glyph) in ROMAN {
        while value >= weight {
            out.push_str(glyph);
            value -= weight;
        }
    }
    if upper {
        out.to_ascii_uppercase()
    } else {
        out
    }
}

/// Parses a roman numeral, rejecting anything whose canonical rendering
/// differs — so `iiii` is not silently read as 4.
fn roman_value(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    if lower.is_empty() || !lower.chars().all(|c| "ivxlcdm".contains(c)) {
        return None;
    }
    let mut total = 0u32;
    let mut rest = lower.as_str();
    'outer: while !rest.is_empty() {
        for (weight, glyph) in ROMAN {
            if let Some(tail) = rest.strip_prefix(glyph) {
                total += weight;
                rest = tail;
                continue 'outer;
            }
        }
        return None;
    }
    (roman(total, false) == lower).then_some(total)
}

/// One open level of the paragraph stack.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Level {
    system: System,
    current: String,
}

/// How an incoming designator related to the stack. Recorded so that upstream
/// numbering that does not fit is surfaced rather than absorbed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Advanced the level at `depth` to its successor.
    Advanced { depth: usize },
    /// Opened a new level below the previous deepest.
    Opened { depth: usize, system: System },
    /// Fit nothing. The designator is recorded at the deepest plausible level
    /// and reported, because dropping it would lose regulatory text.
    Irregular { depth: usize, reason: String },
}

/// Rebuilds paragraph depth from the designator sequence of one section.
#[derive(Debug, Default, Clone)]
pub struct Stack {
    levels: Vec<Level>,
}

impl Stack {
    pub fn new() -> Self {
        Self::default()
    }

    /// The citation path of the currently open levels, e.g. `["b", "1", "ii"]`.
    pub fn path(&self) -> Vec<String> {
        self.levels.iter().map(|l| l.current.clone()).collect()
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// Places `token` in the tree and returns where it landed.
    pub fn push(&mut self, token: &str) -> Step {
        self.push_with(token, false)
    }

    /// Places `token` where the previous paragraph ended in a lead-in — a dash
    /// or colon introducing a list.
    ///
    /// This is the only thing that resolves the hardest ambiguity in the
    /// numbering. After `(h)(2) Demonstrates … that—`, the token `(i)` is both
    /// the successor of the letter `(h)` and the first roman numeral beneath
    /// `(2)`, and the successor reading wins by depth alone — wrongly, since a
    /// lead-in cannot be followed by its own ancestor's sibling. Measured on
    /// 34 CFR 668, reading the lead-in fixes 27 of the 32 remaining sequences.
    pub fn push_after_lead_in(&mut self, token: &str) -> Step {
        self.push_with(token, true)
    }

    fn push_with(&mut self, token: &str, descend: bool) -> Step {
        if descend {
            if let Some(system) = System::opening(token) {
                self.levels.push(Level {
                    system,
                    current: token.to_string(),
                });
                return Step::Opened {
                    depth: self.levels.len() - 1,
                    system,
                };
            }
        }
        for depth in (0..self.levels.len()).rev() {
            let level = &self.levels[depth];
            if level.system.successor(&level.current).as_deref() == Some(token) {
                self.levels.truncate(depth + 1);
                self.levels[depth].current = token.to_string();
                return Step::Advanced { depth };
            }
        }

        if let Some(system) = System::opening(token) {
            // An opener that repeats a system already open at the same value
            // would be a restart, not a descent; the successor scan above has
            // already ruled that out, so this is a genuine new level.
            self.levels.push(Level {
                system,
                current: token.to_string(),
            });
            return Step::Opened {
                depth: self.levels.len() - 1,
                system,
            };
        }

        // Neither a successor nor an opener: upstream skipped a designator, or
        // used a form this does not model. Keep the text, flag the sequence.
        let reason = format!(
            "`({token})` follows `{}` but is neither its successor nor a level opener",
            self.path().join(")(")
        );
        let depth = self.levels.len().saturating_sub(1);
        if self.levels.is_empty() {
            self.levels.push(Level {
                system: System::LowerAlpha,
                current: token.to_string(),
            });
        } else {
            self.levels[depth].current = token.to_string();
        }
        Step::Irregular { depth, reason }
    }
}

impl fmt::Display for Stack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for level in &self.levels {
            write!(f, "({})", level.current)?;
        }
        Ok(())
    }
}

/// Markers the XML reader wraps italic runs in, so that a paragraph heading
/// survives the flattening of `<I>` into text. CFR uses an italic run as the
/// heading of a designated paragraph, and it sits *between* two designators:
/// `(a) <I>Written arrangements.</I> (1) Except as provided …`. Without the
/// marker the `(1)` reads as prose and the entire arabic level below `(a)` is
/// lost — measured at 593 broken sequences in part 668 alone.
pub const ITALIC_OPEN: char = '\u{1}';
pub const ITALIC_CLOSE: char = '\u{2}';

/// What opens a paragraph: its designator run, its italic heading, and — for a
/// definitions list — the term being defined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lead {
    pub designators: Vec<String>,
    /// The italic label of the deepest designator in the run.
    pub heading: Option<String>,
    /// A defined term. Present only when the italic run opens the paragraph
    /// with no designator before it, which is how CFR writes a definitions
    /// list: `*Campus.* (i) Any building …`. The numbering that follows
    /// belongs to the definition, not to the section — the regulation itself
    /// says so, citing "paragraph (i) of this definition".
    pub term: Option<String>,
}

/// Separators CFR puts between an italic heading and the designator that
/// follows it: `(a) *General*—(1) *Independent auditor.*`.
fn skip_joiner(rest: &str) -> &str {
    let trimmed = rest.trim_start_matches(['\u{2014}', '\u{2013}', '-', ':', ';']);
    if std::ptr::eq(trimmed, rest) {
        return rest;
    }
    let trimmed = trimmed.trim_start();
    if trimmed.starts_with('(') || trimmed.starts_with(ITALIC_OPEN) {
        trimmed
    } else {
        rest
    }
}

/// Reads what opens a paragraph, stepping over italic headings and the
/// joiners that attach them to the next designator.
pub fn parse_lead(text: &str) -> (Lead, &str) {
    let mut lead = Lead::default();
    let mut rest = text.trim_start();

    // An italic run before any designator is a defined term, not a heading.
    if let Some(after) = rest.strip_prefix(ITALIC_OPEN) {
        if let Some(close) = after.find(ITALIC_CLOSE) {
            let term = after[..close].trim();
            if term.ends_with('.') || term.ends_with(':') {
                lead.term = Some(term.trim_end_matches(['.', ':']).to_string());
                rest = after[close + ITALIC_CLOSE.len_utf8()..].trim_start();
            }
        }
    }

    loop {
        let width_before = rest.len();
        while let Some(after) = rest.strip_prefix('(') {
            let Some(close) = after.find(')') else { break };
            let token = &after[..close];
            if token.is_empty()
                || token.len() > 6
                || !token.chars().all(|c| c.is_ascii_alphanumeric())
            {
                break;
            }
            lead.designators.push(token.to_string());
            rest = after[close + 1..].trim_start();
        }
        rest = skip_joiner(rest);
        // The deepest designator is the one the heading labels, so a later
        // heading in the same run replaces an earlier, shallower one.
        if !lead.designators.is_empty() {
            if let Some(after) = rest.strip_prefix(ITALIC_OPEN) {
                if let Some(close) = after.find(ITALIC_CLOSE) {
                    lead.heading = Some(after[..close].trim().to_string());
                    rest = skip_joiner(after[close + ITALIC_CLOSE.len_utf8()..].trim_start());
                }
            }
        }
        if rest.len() == width_before {
            break;
        }
    }
    (lead, rest)
}

/// Whether a paragraph introduces a list, so that whatever follows is one of
/// its children rather than a sibling of one of its ancestors.
pub fn is_lead_in(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with('\u{2014}') || trimmed.ends_with('\u{2013}') || trimmed.ends_with(':')
}

/// Removes the italic markers from text not being parsed for structure.
pub fn strip_markers(text: &str) -> String {
    text.replace([ITALIC_OPEN, ITALIC_CLOSE], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(h)(2)`, reached the way upstream reaches it — one designator at a time
    /// from `(a)`, because a stack cannot jump to the middle of a sequence.
    fn walk_to_h_2() -> Stack {
        let mut stack = Stack::new();
        for token in ["a", "b", "c", "d", "e", "f", "g", "h", "1", "2"] {
            stack.push(token);
        }
        stack
    }

    fn path_after(tokens: &[&str]) -> String {
        let mut stack = Stack::new();
        for token in tokens {
            stack.push(token);
        }
        stack.to_string()
    }

    #[test]
    fn i_after_h_continues_the_letters() {
        assert_eq!(path_after(&["f", "g", "h", "i"]), "(i)");
    }

    #[test]
    fn i_after_a_number_opens_romans() {
        assert_eq!(path_after(&["a", "1", "i"]), "(a)(1)(i)");
        assert_eq!(path_after(&["a", "1", "i", "ii"]), "(a)(1)(ii)");
    }

    #[test]
    fn the_same_token_resolves_differently_from_the_same_sequence_position() {
        // The ambiguity this module exists for: identical token, opposite reading.
        let mut letters = Stack::new();
        for t in ["g", "h"] {
            letters.push(t);
        }
        assert_eq!(letters.push("i"), Step::Advanced { depth: 0 });

        let mut romans = Stack::new();
        for t in ["a", "1"] {
            romans.push(t);
        }
        assert_eq!(
            romans.push("i"),
            Step::Opened {
                depth: 2,
                system: System::LowerRoman
            }
        );
    }

    #[test]
    fn ii_after_hh_continues_the_doubled_letters() {
        let mut stack = Stack::new();
        stack.push("a");
        // walk to hh the way upstream does, one designator at a time
        for token in [
            "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
            "s", "t", "u", "v", "w", "x", "y", "z", "aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh",
        ] {
            stack.push(token);
        }
        assert_eq!(stack.push("ii"), Step::Advanced { depth: 0 });
        assert_eq!(stack.to_string(), "(ii)");
    }

    #[test]
    fn z_is_followed_by_aa_not_ba() {
        assert_eq!(System::LowerAlpha.successor("z").as_deref(), Some("aa"));
        assert_eq!(System::LowerAlpha.successor("zz").as_deref(), Some("aaa"));
        assert_eq!(System::UpperAlpha.successor("Z").as_deref(), Some("AA"));
    }

    #[test]
    fn closing_a_deep_level_reopens_the_shallow_one() {
        assert_eq!(
            path_after(&["a", "1", "i", "ii", "2"]),
            "(a)(2)",
            "an arabic successor closes the romans beneath it"
        );
        assert_eq!(path_after(&["a", "1", "i", "ii", "2", "b"]), "(b)");
    }

    #[test]
    fn four_levels_nest_in_order() {
        assert_eq!(path_after(&["b", "1", "i", "A"]), "(b)(1)(i)(A)");
        assert_eq!(path_after(&["b", "1", "i", "A", "B"]), "(b)(1)(i)(B)");
    }

    #[test]
    fn a_fifth_level_repeats_arabic_without_colliding() {
        // (a)(1)(i)(A)(1) — the arabic level five, distinct from level two.
        let mut stack = Stack::new();
        for token in ["a", "1", "i", "A"] {
            stack.push(token);
        }
        assert_eq!(
            stack.push("1"),
            Step::Opened {
                depth: 4,
                system: System::Arabic
            }
        );
        assert_eq!(stack.to_string(), "(a)(1)(i)(A)(1)");
        assert_eq!(stack.push("2"), Step::Advanced { depth: 4 });
    }

    #[test]
    fn a_lead_in_forces_a_descent_over_a_shallower_successor() {
        // 34 CFR 668.35(h)(2)(i). Without the lead-in, `(i)` reads as the
        // letter after `(h)` and the roman list is lost.
        let mut stack = walk_to_h_2();
        assert_eq!(stack.to_string(), "(h)(2)");
        assert_eq!(
            stack.push_after_lead_in("i"),
            Step::Opened {
                depth: 2,
                system: System::LowerRoman
            }
        );
        assert_eq!(stack.to_string(), "(h)(2)(i)");
        assert_eq!(stack.push("ii"), Step::Advanced { depth: 2 });
    }

    #[test]
    fn without_a_lead_in_the_letter_successor_still_wins() {
        // 34 CFR 668.35(i), which really is the letter after (h).
        let mut stack = walk_to_h_2();
        assert_eq!(stack.push("i"), Step::Advanced { depth: 0 });
        assert_eq!(stack.to_string(), "(i)");
    }

    #[test]
    fn a_lead_in_with_a_non_opener_falls_back_to_the_successor_scan() {
        let mut stack = Stack::new();
        for token in ["a", "1"] {
            stack.push(token);
        }
        assert_eq!(stack.push_after_lead_in("2"), Step::Advanced { depth: 1 });
    }

    #[test]
    fn lead_ins_are_recognised_by_their_punctuation() {
        assert!(is_lead_in("The agreement must be signed by\u{2014}"));
        assert!(is_lead_in("An institution shall report the following:"));
        assert!(!is_lead_in(
            "The debt otherwise qualifies for discharge; and"
        ));
        assert!(!is_lead_in("Repays the loan in full."));
    }

    #[test]
    fn a_body_that_starts_mid_sequence_is_reported() {
        // Section bodies open at (a)(1); a fragment starting at (a)(2) is a
        // sequence this cannot verify, and says so rather than inventing a level.
        let mut stack = Stack::new();
        stack.push("a");
        assert!(matches!(stack.push("2"), Step::Irregular { .. }));
    }

    #[test]
    fn a_skipped_designator_is_reported_not_swallowed() {
        let mut stack = Stack::new();
        stack.push("a");
        let step = stack.push("c");
        assert!(matches!(step, Step::Irregular { .. }));
        assert_eq!(stack.to_string(), "(c)", "the text still gets a citation");
    }

    #[test]
    fn roman_values_reject_non_canonical_forms() {
        assert_eq!(roman_value("iv"), Some(4));
        assert_eq!(roman_value("viii"), Some(8));
        assert_eq!(roman_value("iiii"), None);
        assert_eq!(roman_value("q"), None);
        assert_eq!(roman(9, false), "ix");
        assert_eq!(roman(14, true), "XIV");
    }

    #[test]
    fn leading_run_splits_designators_from_text() {
        let (lead, rest) = parse_lead("(a)(1) An institution may participate");
        assert_eq!(lead.designators, vec!["a", "1"]);
        assert_eq!(lead.heading, None);
        assert_eq!(rest, "An institution may participate");
    }

    #[test]
    fn mid_sentence_parentheses_are_prose() {
        let (lead, rest) = parse_lead("(b) See paragraph (a)(1) of this section.");
        assert_eq!(lead.designators, vec!["b"]);
        assert_eq!(rest, "See paragraph (a)(1) of this section.");
    }

    #[test]
    fn undesignated_text_yields_no_tokens() {
        let (lead, rest) = parse_lead("The Secretary considers the following:");
        assert!(lead.designators.is_empty());
        assert_eq!(rest, "The Secretary considers the following:");
    }

    #[test]
    fn an_italic_heading_does_not_break_the_designator_run() {
        // 34 CFR 668.5(a)(1) as upstream writes it. Ending the run at `(a)`
        // loses the whole arabic level beneath it.
        let text = "(a) \u{1}Written arrangements between eligible institutions.\u{2} \
                    (1) Except as provided in paragraph (a)(2), if an institution applies";
        let (lead, rest) = parse_lead(text);
        assert_eq!(lead.designators, vec!["a", "1"]);
        assert_eq!(
            lead.heading.as_deref(),
            Some("Written arrangements between eligible institutions.")
        );
        assert!(rest.starts_with("Except as provided"));
    }

    #[test]
    fn an_italic_run_with_no_designator_before_it_is_prose() {
        let (lead, rest) = parse_lead("\u{1}See\u{2} the schedule in appendix A.");
        assert!(lead.designators.is_empty());
        assert_eq!(lead.heading, None);
        assert_eq!(strip_markers(rest), "See the schedule in appendix A.");
    }

    #[test]
    fn only_the_first_italic_run_is_taken_as_the_heading() {
        let (lead, rest) =
            parse_lead("(c) \u{1}Heading.\u{2} Text with \u{1}emphasis\u{2} inside.");
        assert_eq!(lead.designators, vec!["c"]);
        assert_eq!(lead.heading.as_deref(), Some("Heading."));
        assert_eq!(strip_markers(rest), "Text with emphasis inside.");
    }

    #[test]
    fn an_em_dash_joins_a_heading_to_the_designator_that_follows() {
        // 34 CFR 668.23(a)(1) as upstream writes it. Stopping at the dash
        // drops the arabic level that carries the audit obligations.
        let (lead, rest) = parse_lead(
            "(a) \u{1}General\u{2}\u{2014}(1) \u{1}Independent auditor.\u{2} For purposes of this section",
        );
        assert_eq!(lead.designators, vec!["a", "1"]);
        assert_eq!(
            lead.heading.as_deref(),
            Some("Independent auditor."),
            "the heading labels the deepest designator"
        );
        assert_eq!(rest, "For purposes of this section");
    }

    #[test]
    fn a_dash_that_is_not_a_joiner_stays_in_the_body() {
        let (lead, rest) = parse_lead("(a) \u{1}General\u{2}\u{2014}the rule applies broadly");
        assert_eq!(lead.designators, vec!["a"]);
        assert_eq!(lead.heading.as_deref(), Some("General"));
        assert!(rest.starts_with('\u{2014}'), "prose keeps its punctuation");
    }

    #[test]
    fn a_leading_italic_run_is_a_defined_term_not_a_heading() {
        // 34 CFR 668.46(a) definitions list.
        let (lead, rest) = parse_lead("\u{1}Campus.\u{2} (i) Any building or property owned");
        assert_eq!(lead.term.as_deref(), Some("Campus"));
        assert_eq!(lead.heading, None);
        assert_eq!(lead.designators, vec!["i"]);
        assert_eq!(rest, "Any building or property owned");
    }

    #[test]
    fn a_defined_term_may_use_a_colon() {
        let (lead, _) = parse_lead("\u{1}Specified year:\u{2} (1) The calendar year preceding");
        assert_eq!(lead.term.as_deref(), Some("Specified year"));
        assert_eq!(lead.designators, vec!["1"]);
    }

    #[test]
    fn an_unpunctuated_leading_italic_is_not_a_term() {
        let (lead, rest) = parse_lead("\u{1}Note\u{2} that the deadline applies.");
        assert_eq!(lead.term, None);
        assert!(rest.starts_with(ITALIC_OPEN));
    }

    #[test]
    fn a_heading_between_three_designators_still_resolves() {
        let (lead, rest) = parse_lead("(b)(2) \u{1}Reporting.\u{2} (i) The school reports");
        assert_eq!(lead.designators, vec!["b", "2", "i"]);
        assert_eq!(lead.heading.as_deref(), Some("Reporting."));
        assert_eq!(rest, "The school reports");
    }
}
