//! Deriving obligation atoms from regulatory text.
//!
//! FedRAMP publishes rules with a `force` field; a machine can read `MUST`
//! straight off the record. The Department of Education publishes prose. The
//! obligations are there — 34 CFR is drafted to the Federal Register's own
//! conventions, where `must` and `may not` are terms of art — but nothing
//! marks them, so they have to be derived.
//!
//! What is derived here is deliberately narrow, because the gap between
//! "this paragraph contains an obligation" and "this obligation binds you" is
//! where a compliance tool earns the right to be trusted:
//!
//! - **Force** is read from the drafting conventions. `must`, `shall`, and
//!   `is required to` bind; `may not` and `shall not` prohibit — in Federal
//!   Register drafting `may not` is a prohibition, not an absent permission,
//!   and reading it as the latter would invert the rule.
//! - **Bearer** is classified from the grammatical subject, and is reported as
//!   `unclassified` when no pattern matches. An obligation on the Secretary is
//!   not an obligation on an institution, and a tool that blurred the two
//!   would manufacture duties.
//! - **Inheritance** follows the drafting shape where a lead-in carries the
//!   modal and its children carry the list: `(3) The agreement must be signed
//!   by— (i) An authorized representative`. The child inherits, and says so.
//! - **Definitions are not obligations.** `Loan means …` is excluded, and the
//!   count of exclusions is reported rather than silently applied.
//!
//! Nothing here decides that an atom applies to a given institution; that is
//! applicability, and it is a separate, explicitly curated judgement.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::cfr::xml::{Paragraph, Part, Section};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Force {
    /// `may`, `is permitted to` — a discretion, not a duty.
    May,
    /// `should` — recommended, not binding.
    Should,
    /// `must`, `shall`, `is required to`.
    Must,
    /// `must not`, `shall not`, `may not`, `is prohibited from`.
    MustNot,
}

impl Force {
    pub fn label(self) -> &'static str {
        match self {
            Self::May => "MAY",
            Self::Should => "SHOULD",
            Self::Must => "MUST",
            Self::MustNot => "MUST NOT",
        }
    }

    /// Whether the atom constrains conduct at all.
    pub fn binding(self) -> bool {
        matches!(self, Self::Must | Self::MustNot)
    }
}

/// Who the duty falls on. Classified from the subject phrase, never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bearer {
    Institution,
    Secretary,
    Lender,
    GuarantyAgency,
    Accreditor,
    StateAgency,
    LocalAgency,
    ThirdPartyServicer,
    Student,
    Recipient,
    /// A party to a proceeding under subpart H, or a petitioner before one.
    Party,
    /// The presiding official in an administrative proceeding.
    HearingOfficial,
    /// A publisher of an approved ability-to-benefit test.
    TestPublisher,
    /// A State advisory panel, rehabilitation council, or similar body.
    AdvisoryBody,
    /// The IEP Team, which 34 CFR 300 gives duties to in its own right.
    IepTeam,
    /// The regulation addresses the reader directly as `you`. Subparts of
    /// 34 CFR 668 are drafted in the second person and define `you` locally —
    /// as the institution in one subpart and the borrower in another — so the
    /// bearer is real but only the section can say who it is.
    Addressee,
    /// No pattern matched. Reported, not assigned to a default.
    Unclassified,
}

impl Bearer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Institution => "institution",
            Self::Secretary => "Secretary",
            Self::Lender => "lender",
            Self::GuarantyAgency => "guaranty agency",
            Self::Accreditor => "accrediting agency",
            Self::StateAgency => "State agency",
            Self::LocalAgency => "local agency",
            Self::ThirdPartyServicer => "third-party servicer",
            Self::Student => "student or borrower",
            Self::Recipient => "recipient of federal assistance",
            Self::Party => "party to a proceeding",
            Self::HearingOfficial => "hearing official",
            Self::TestPublisher => "test publisher",
            Self::AdvisoryBody => "advisory body",
            Self::IepTeam => "IEP Team",
            Self::Addressee => "addressee (\"you\")",
            Self::Unclassified => "unclassified",
        }
    }
}

/// Ordered longest-first within each bearer so `State educational agency`
/// is not swallowed by `State`.
/// Longest first within each bearer, and abbreviations included: the K-12 and
/// disability parts of Title 34 are drafted almost entirely in `SEA`, `LEA`
/// and `public agency`, which the Title IV vocabulary does not contain.
/// Measured across ten non-Title-IV parts, adding them cut unclassified
/// subjects from 47% to the figure DESIGN.md records.
const BEARER_PATTERNS: &[(&str, Bearer)] = &[
    ("state educational agency", Bearer::StateAgency),
    ("sea", Bearer::StateAgency),
    ("lea", Bearer::LocalAgency),
    ("public agency", Bearer::LocalAgency),
    ("participating agency", Bearer::LocalAgency),
    ("governor", Bearer::StateAgency),
    ("subgrantee", Bearer::Recipient),
    ("insular area", Bearer::Recipient),
    ("eligible entity", Bearer::Recipient),
    ("advisory panel", Bearer::AdvisoryBody),
    ("advisory council", Bearer::AdvisoryBody),
    ("council", Bearer::AdvisoryBody),
    ("iep team", Bearer::IepTeam),
    ("child", Bearer::Student),
    ("states", Bearer::StateAgency),
    ("state agency", Bearer::StateAgency),
    ("state vocational rehabilitation", Bearer::StateAgency),
    ("designated state unit", Bearer::StateAgency),
    ("local educational agency", Bearer::LocalAgency),
    ("school district", Bearer::LocalAgency),
    ("third-party servicer", Bearer::ThirdPartyServicer),
    ("guaranty agency", Bearer::GuarantyAgency),
    ("accrediting agency", Bearer::Accreditor),
    ("accrediting association", Bearer::Accreditor),
    ("secretary", Bearer::Secretary),
    ("department", Bearer::Secretary),
    ("institution", Bearer::Institution),
    ("school", Bearer::Institution),
    ("educational agency", Bearer::LocalAgency),
    ("lender", Bearer::Lender),
    ("holder", Bearer::Lender),
    ("servicer", Bearer::ThirdPartyServicer),
    ("borrower", Bearer::Student),
    ("student", Bearer::Student),
    ("applicant", Bearer::Student),
    ("parent", Bearer::Student),
    ("grantee", Bearer::Recipient),
    ("recipient", Bearer::Recipient),
    ("participant", Bearer::Recipient),
    ("test publisher", Bearer::TestPublisher),
    ("hearing official", Bearer::HearingOfficial),
    ("presiding official", Bearer::HearingOfficial),
    ("petitioner", Bearer::Party),
    ("respondent", Bearer::Party),
    ("party", Bearer::Party),
    ("state", Bearer::StateAgency),
    ("you", Bearer::Addressee),
    ("your", Bearer::Addressee),
];

/// Longest first, so `must not` is never read as `must`.
const FORCE_PATTERNS: &[(&str, Force)] = &[
    ("is prohibited from", Force::MustNot),
    ("are prohibited from", Force::MustNot),
    ("must not", Force::MustNot),
    ("shall not", Force::MustNot),
    ("may not", Force::MustNot),
    ("is required to", Force::Must),
    ("are required to", Force::Must),
    ("must", Force::Must),
    ("shall", Force::Must),
    ("should", Force::Should),
    ("may", Force::May),
];

/// Words that turn a permission into a condition: `may participate … only if`.
const CONDITIONS: &[&str] = &["only if", "only when", "only to the extent", "unless"];

/// One derived duty, addressed by the citation a lawyer would use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Atom {
    /// `34 CFR 668.14(b)(1)`.
    pub citation: String,
    pub part: String,
    pub section: String,
    pub force: Force,
    pub bearer: Bearer,
    /// The subject phrase the bearer was read from, kept so the classification
    /// can be checked rather than trusted.
    pub subject: String,
    /// A permission narrowed by `only if` / `unless`, which functions as a
    /// condition on conduct even though the modal is `may`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conditioned: bool,
    /// The force came from an ancestor lead-in, not from this paragraph.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inherited: bool,
    /// More than one kind of actor appears in the subject phrase, so the
    /// classification is the closest one and may not be the right one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bearer_ambiguous: bool,
    pub text: String,
}

/// What extraction did and did not do, so a report never overstates coverage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub paragraphs: usize,
    pub atoms: usize,
    pub inherited: usize,
    /// Paragraphs in a definitions section, or reading `X means Y`.
    pub definitions_skipped: usize,
    /// Atoms whose bearer no pattern matched.
    pub unclassified: usize,
    /// Atoms whose subject named more than one kind of actor.
    pub bearer_ambiguous: usize,
    pub by_force: BTreeMap<String, usize>,
    pub by_bearer: BTreeMap<String, usize>,
}

/// The first modal in `text`, with the subject phrase preceding it.
///
/// Case folding is ASCII-only on purpose: `to_lowercase` can change a string's
/// byte length, and every offset found here is used to slice the *original*
/// text. All the modals are ASCII, so nothing is lost and the indices stay
/// valid across the em dashes and curly quotes the CFR is full of.
fn detect(text: &str) -> Option<(Force, String, bool)> {
    let lower = text.to_ascii_lowercase();
    debug_assert_eq!(lower.len(), text.len());
    let (at, force, width) = FORCE_PATTERNS
        .iter()
        .filter_map(|(pattern, force)| {
            find_word(&lower, pattern).map(|at| (at, *force, pattern.len()))
        })
        .min_by_key(|(at, _, width)| (*at, std::cmp::Reverse(*width)))?;

    let subject = subject_phrase(&text[..at]);
    let tail = &lower[at + width..];
    let conditioned = force == Force::May && CONDITIONS.iter().any(|marker| tail.contains(marker));
    Some((force, subject, conditioned))
}

/// Word-boundary search, so `may` does not match inside `maybe` and `shall`
/// does not match inside `shallow`.
fn find_word(haystack: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle) {
        let at = from + offset;
        let before_ok = at == 0
            || !haystack[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '-');
        let after = at + needle.len();
        let after_ok = after >= haystack.len()
            || !haystack[after..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '-');
        if before_ok && after_ok {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// The clause the modal attaches to: back to the last sentence or clause break.
fn subject_phrase(before: &str) -> String {
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '.' | ';' | ':' | '\u{2014}'))
        .map_or(0, |(at, c)| at + c.len_utf8());
    before[start..]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// The bearer of a duty, read from the subject phrase.
///
/// The match closest to the modal wins, because that is where English puts the
/// grammatical subject: in `Upon the written request of an institution, the
/// Secretary may approve …` the duty is the Secretary's, and taking the first
/// mention would hand it to the institution instead. Measured on 34 CFR 668,
/// 80 subjects name both.
///
/// The failure mode this trades for is a relative clause — `An institution
/// that contracts with a third-party servicer must …` reads as the servicer.
/// Those are reported through [`Atom::bearer_ambiguous`] rather than hidden.
pub fn classify_bearer(subject: &str) -> Bearer {
    classify(subject).0
}

fn classify(subject: &str) -> (Bearer, bool) {
    let lower = subject.to_ascii_lowercase();
    let found: Vec<(usize, usize, Bearer)> = BEARER_PATTERNS
        .iter()
        .filter_map(|(pattern, bearer)| {
            find_word(&lower, pattern).map(|at| (at, at + pattern.len(), *bearer))
        })
        .collect();
    // `State educational agency` contains both `State` and `educational
    // agency`; a match inside another match is part of it, not a rival actor.
    let outer: Vec<(usize, usize, Bearer)> = found
        .iter()
        .copied()
        .filter(|(start, end, _)| {
            !found
                .iter()
                .any(|(s, e, _)| (s, e) != (start, end) && s <= start && end <= e)
        })
        .collect();
    let Some((_, _, bearer)) = outer.iter().copied().max_by_key(|(start, _, _)| *start) else {
        return (Bearer::Unclassified, false);
    };
    let distinct: std::collections::BTreeSet<Bearer> = outer.iter().map(|(_, _, b)| *b).collect();
    (bearer, distinct.len() > 1)
}

/// A definition, not a duty. `Loan means …`, and anything under a section
/// headed `Definitions`.
fn is_definition(section: &Section, paragraph: &Paragraph) -> bool {
    // The parser already identified this paragraph as part of a definitions
    // list by its term scope; no text heuristic can beat that.
    if paragraph.term.is_some() {
        return true;
    }
    let heading = section.heading.to_lowercase();
    if heading.contains("definition") {
        return true;
    }
    let lower = paragraph.text.to_lowercase();
    find_word(&lower, "means").is_some_and(|at| lower[..at].split_whitespace().count() <= 12)
}

/// The nearest ancestor that introduces the list this paragraph is an item of.
fn ancestor_lead_in(paragraphs: &[Paragraph], index: usize) -> Option<&Paragraph> {
    let path = &paragraphs[index].path;
    for candidate in paragraphs[..index].iter().rev() {
        if candidate.path.len() >= path.len() || !path.starts_with(&candidate.path) {
            continue;
        }
        let trimmed = candidate.text.trim_end();
        let introduces = trimmed.ends_with('\u{2014}')
            || trimmed.ends_with(':')
            || trimmed.ends_with("that")
            || trimmed.ends_with("following");
        return introduces.then_some(candidate);
    }
    None
}

/// The force a lead-in ancestor carries down to its list items.
fn lead_in(paragraphs: &[Paragraph], index: usize) -> Option<(Force, String, bool)> {
    detect(&ancestor_lead_in(paragraphs, index)?.text)
}

pub fn extract(part: &Part) -> (Vec<Atom>, Coverage) {
    let mut atoms = Vec::new();
    let mut coverage = Coverage::default();

    for section in &part.sections {
        for (index, paragraph) in section.paragraphs.iter().enumerate() {
            coverage.paragraphs += 1;
            if is_definition(section, paragraph) {
                coverage.definitions_skipped += 1;
                continue;
            }
            let (force, mut subject, conditioned, inherited) = match detect(&paragraph.text) {
                Some((force, subject, conditioned)) => (force, subject, conditioned, false),
                None => match lead_in(&section.paragraphs, index) {
                    Some((force, subject, conditioned)) => (force, subject, conditioned, true),
                    None => continue,
                },
            };
            // A list item can open with the modal itself — `Must submit an
            // application`. The actor is then in the lead-in that introduced
            // the list, which need not carry a modal of its own.
            if subject.trim().is_empty() {
                if let Some(ancestor) = ancestor_lead_in(&section.paragraphs, index) {
                    subject = ancestor
                        .text
                        .trim_end_matches(['\u{2014}', ':'])
                        .trim()
                        .to_string();
                }
            }
            let (bearer, bearer_ambiguous) = classify(&subject);
            if bearer_ambiguous {
                coverage.bearer_ambiguous += 1;
            }
            if bearer == Bearer::Unclassified {
                coverage.unclassified += 1;
            }
            if inherited {
                coverage.inherited += 1;
            }
            *coverage
                .by_force
                .entry(force.label().to_string())
                .or_insert(0) += 1;
            *coverage
                .by_bearer
                .entry(bearer.label().to_string())
                .or_insert(0) += 1;
            atoms.push(Atom {
                citation: paragraph.citation.clone(),
                part: part.number.clone(),
                section: section.identifier.clone(),
                force,
                bearer,
                subject,
                conditioned,
                inherited,
                bearer_ambiguous,
                text: paragraph.text.clone(),
            });
        }
    }
    coverage.atoms = atoms.len();
    (atoms, coverage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfr::xml::Kind;

    fn paragraph(path: &[&str], text: &str) -> Paragraph {
        Paragraph {
            path: path.iter().map(ToString::to_string).collect(),
            citation: format!("34 CFR 668.14({})", path.join(")(")),
            heading: None,
            term: None,
            text: text.to_string(),
        }
    }

    fn section(heading: &str, paragraphs: Vec<Paragraph>) -> Section {
        Section {
            identifier: "668.14".into(),
            citation: "34 CFR 668.14".into(),
            kind: Kind::Section,
            subpart: None,
            heading: heading.into(),
            paragraphs,
            pending: Vec::new(),
            credit: None,
            authority: None,
            irregularities: Vec::new(),
        }
    }

    fn part(sections: Vec<Section>) -> Part {
        Part {
            number: "668".into(),
            heading: "Student Assistance General Provisions".into(),
            authority: None,
            source: None,
            sections,
        }
    }

    #[test]
    fn may_not_is_a_prohibition_not_an_absent_permission() {
        let (force, _, _) = detect("An institution may not disburse funds before verification.")
            .expect("modal found");
        assert_eq!(force, Force::MustNot, "`may not` inverts if read as `may`");
    }

    #[test]
    fn the_earliest_modal_wins_and_longest_match_breaks_the_tie() {
        assert_eq!(
            detect("The institution must not release records that it may hold.")
                .unwrap()
                .0,
            Force::MustNot
        );
        assert_eq!(
            detect("The lender may request records that the school must keep.")
                .unwrap()
                .0,
            Force::May
        );
    }

    #[test]
    fn modals_match_on_word_boundaries_only() {
        assert!(detect("The water was shallow and the ditch was maybe deep.").is_none());
        assert!(detect("Shall-not-hyphenated is still prose").is_none());
    }

    #[test]
    fn a_conditioned_permission_is_flagged() {
        let (force, _, conditioned) =
            detect("An institution may participate only if it enters into an agreement.").unwrap();
        assert_eq!(force, Force::May);
        assert!(conditioned, "`only if` narrows the permission into a duty");

        let (_, _, unconditioned) = detect("An institution may use the funds.").unwrap();
        assert!(!unconditioned);
    }

    #[test]
    fn the_secretary_is_not_the_institution() {
        assert_eq!(classify_bearer("The Secretary"), Bearer::Secretary);
        assert_eq!(classify_bearer("An institution"), Bearer::Institution);
        // In the pipeline the subject is already cut at the modal, so this is
        // what the classifier actually receives.
        assert_eq!(classify_bearer("The Secretary"), Bearer::Secretary);
    }

    #[test]
    fn longer_bearer_phrases_win_over_their_prefixes() {
        assert_eq!(
            classify_bearer("A State educational agency"),
            Bearer::StateAgency
        );
        assert_eq!(
            classify_bearer("A local educational agency"),
            Bearer::LocalAgency
        );
        assert_eq!(
            classify_bearer("A third-party servicer"),
            Bearer::ThirdPartyServicer
        );
    }

    #[test]
    fn the_actor_nearest_the_modal_is_the_one_bound() {
        // 34 CFR 668.3(c)(1): the Secretary approves, on the institution's request.
        assert_eq!(
            classify_bearer("Upon the written request of an institution, the Secretary"),
            Bearer::Secretary,
            "an opening prepositional phrase is not the subject"
        );
    }

    #[test]
    fn a_subject_naming_two_actors_is_flagged() {
        let (bearer, ambiguous) =
            classify("Upon the written request of an institution, the Secretary");
        assert_eq!(bearer, Bearer::Secretary);
        assert!(
            ambiguous,
            "a reviewer needs to know this was a judgement call"
        );

        let (_, single) = classify("An institution");
        assert!(!single);
    }

    #[test]
    fn second_person_drafting_is_a_bearer_not_a_gap() {
        assert_eq!(classify_bearer("you"), Bearer::Addressee);
        assert_eq!(
            classify_bearer("If you lose your appeal, you"),
            Bearer::Addressee
        );
    }

    #[test]
    fn a_list_item_that_opens_with_the_modal_takes_the_lead_ins_actor() {
        let part = part(vec![section(
            "Program participation agreement.",
            vec![
                paragraph(&["a"], "An institution seeking to participate\u{2014}"),
                paragraph(&["a", "1"], "Must submit an application; and"),
            ],
        )]);
        let (atoms, _) = extract(&part);
        assert_eq!(atoms.len(), 1, "only the list item carries a modal");
        assert_eq!(atoms[0].citation, "34 CFR 668.14(a)(1)");
        assert_eq!(
            atoms[0].bearer,
            Bearer::Institution,
            "the actor is in the lead-in, not in the list item"
        );
    }

    #[test]
    fn the_k12_and_disability_vocabulary_is_recognised() {
        assert_eq!(classify_bearer("Each public agency"), Bearer::LocalAgency);
        assert_eq!(classify_bearer("The SEA"), Bearer::StateAgency);
        assert_eq!(classify_bearer("An LEA"), Bearer::LocalAgency);
        assert_eq!(classify_bearer("The IEP Team"), Bearer::IepTeam);
        assert_eq!(classify_bearer("A subgrantee"), Bearer::Recipient);
        assert_eq!(
            classify_bearer("The State advisory panel"),
            Bearer::AdvisoryBody
        );
    }

    #[test]
    fn abbreviations_still_respect_word_boundaries() {
        // `sea` must not fire inside `research` or `seas`.
        assert_eq!(
            classify_bearer("The research findings"),
            Bearer::Unclassified
        );
        assert_eq!(
            classify_bearer("A leader in the field"),
            Bearer::Unclassified
        );
    }

    #[test]
    fn an_impersonal_subject_stays_unclassified() {
        // `Nothing in paragraph (c) of this section may be construed …` has no
        // actor at all, and inventing one would invent a duty.
        assert_eq!(
            classify_bearer("Nothing in paragraph (c) of this section"),
            Bearer::Unclassified
        );
    }

    #[test]
    fn an_unmatched_subject_is_reported_rather_than_defaulted() {
        assert_eq!(
            classify_bearer("A widget vendor"),
            Bearer::Unclassified,
            "assigning a default bearer would manufacture a duty"
        );
    }

    #[test]
    fn a_lead_in_modal_reaches_its_listed_children() {
        let part = part(vec![section(
            "Program participation agreement.",
            vec![
                paragraph(&["a", "3"], "The agreement must be signed by\u{2014}"),
                paragraph(&["a", "3", "i"], "An authorized representative; and"),
                paragraph(
                    &["a", "3", "ii"],
                    "For a proprietary institution, an owner.",
                ),
            ],
        )]);
        let (atoms, coverage) = extract(&part);
        assert_eq!(atoms.len(), 3);
        assert!(atoms[1].inherited && atoms[2].inherited);
        assert_eq!(atoms[1].force, Force::Must);
        assert_eq!(coverage.inherited, 2);
    }

    #[test]
    fn a_sibling_without_a_lead_in_inherits_nothing() {
        let part = part(vec![section(
            "Scope.",
            vec![
                paragraph(&["a"], "This part governs participation."),
                paragraph(&["b"], "Definitions appear in \u{a7} 668.2."),
            ],
        )]);
        let (atoms, _) = extract(&part);
        assert!(atoms.is_empty(), "no modal anywhere means no obligation");
    }

    #[test]
    fn definitions_are_not_duties() {
        let part = part(vec![
            section(
                "Definitions.",
                vec![paragraph(
                    &["a"],
                    "Award year means the period that shall begin on July 1.",
                )],
            ),
            section(
                "Recordkeeping.",
                vec![paragraph(
                    &["a"],
                    "Eligible program means a program that must be at least 600 hours.",
                )],
            ),
        ]);
        let (atoms, coverage) = extract(&part);
        assert!(atoms.is_empty());
        assert_eq!(coverage.definitions_skipped, 2);
    }

    #[test]
    fn a_clause_break_on_a_multibyte_character_does_not_split_it() {
        // 34 CFR 668.22 is full of em dashes; slicing one mid-character
        // panicked the extractor on real upstream text.
        let text = "Within 30 days of the institution's determination\u{2014}the institution \
                    must return the funds.";
        let (force, subject, _) = detect(text).expect("modal found");
        assert_eq!(force, Force::Must);
        assert_eq!(subject, "the institution");
        assert_eq!(classify_bearer(&subject), Bearer::Institution);
    }

    #[test]
    fn curly_quotes_do_not_shift_the_modal_offset() {
        let text = "The term \u{201c}institution\u{201d} means a school that must report.";
        let (_, subject, _) = detect(text).expect("modal found");
        assert!(
            subject.contains("school"),
            "byte offsets survived the non-ASCII quotes: {subject:?}"
        );
    }

    #[test]
    fn a_term_scoped_paragraph_is_a_definition_whatever_it_says() {
        let mut p = paragraph(
            &["i"],
            "Any building owned by an institution must be listed.",
        );
        p.term = Some("Campus".into());
        let part = part(vec![section("Institutional security policies.", vec![p])]);
        let (atoms, coverage) = extract(&part);
        assert!(atoms.is_empty());
        assert_eq!(coverage.definitions_skipped, 1);
    }

    #[test]
    fn a_late_means_is_not_treated_as_a_definition() {
        let long = "An institution that participates in any Title IV program and that has \
                    entered into an agreement means to comply and must retain records.";
        let part = part(vec![section(
            "Recordkeeping.",
            vec![paragraph(&["a"], long)],
        )]);
        let (atoms, coverage) = extract(&part);
        assert_eq!(coverage.definitions_skipped, 0);
        assert_eq!(atoms.len(), 1);
    }

    #[test]
    fn coverage_totals_reconcile_with_the_atoms_returned() {
        let part = part(vec![section(
            "Recordkeeping.",
            vec![
                paragraph(&["a"], "An institution must retain records."),
                paragraph(&["b"], "The Secretary may audit the institution."),
                paragraph(&["c"], "This paragraph states a fact."),
            ],
        )]);
        let (atoms, coverage) = extract(&part);
        assert_eq!(coverage.paragraphs, 3);
        assert_eq!(coverage.atoms, atoms.len());
        assert_eq!(coverage.atoms, 2);
        assert_eq!(coverage.by_force["MUST"], 1);
        assert_eq!(coverage.by_bearer["Secretary"], 1);
        assert_eq!(coverage.by_force.values().sum::<usize>(), atoms.len());
    }

    #[test]
    fn binding_separates_duties_from_discretion() {
        assert!(Force::Must.binding() && Force::MustNot.binding());
        assert!(!Force::May.binding() && !Force::Should.binding());
    }
}
