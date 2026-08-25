//! The human-readable change report — the PR body.

use std::fmt::Write as _;

use crate::bundle::Bundle;
use crate::plan::{Changes, Plan};

/// A one-line summary for a commit message or PR title.
pub fn headline(changes: &Changes, bundle: &Bundle) -> String {
    if changes.genesis {
        return format!(
            "34 CFR genesis \u{2014} {} parts, {} sections as of {}",
            bundle.totals.parts, bundle.totals.sections, bundle.pinned_date
        );
    }
    let substantive = changes
        .ledger
        .amended
        .iter()
        .filter(|a| a.row.substantive)
        .count();
    let sections = changes.ledger.amended.len() + changes.ledger.added.len();
    format!(
        "{} \u{2014} {sections} section(s) moved ({substantive} substantive) across {} part(s), as of {}",
        changes.severity.headline(),
        changes.parts_moved.len(),
        bundle.pinned_date
    )
}

#[allow(clippy::too_many_lines)]
pub fn markdown(plan: &Plan, changes: &Changes, previous: Option<&Bundle>) -> String {
    let bundle = &plan.bundle;
    let mut out = String::new();
    let _ = writeln!(out, "# {}", headline(changes, bundle));
    let _ = writeln!(out);
    let _ = writeln!(out, "| | |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| pinned date | `{}` |", bundle.pinned_date);
    let _ = writeln!(
        out,
        "| latest amendment | `{}` |",
        bundle.latest_amendment_date
    );
    let _ = writeln!(out, "| severity | **{}** |", changes.severity);
    let _ = writeln!(out, "| parts | {} |", bundle.totals.parts);
    let _ = writeln!(out, "| sections | {} |", bundle.totals.sections);
    let _ = writeln!(
        out,
        "| obligation atoms | {} ({} binding) |",
        bundle.totals.atoms, bundle.totals.binding
    );
    if let Some(previous) = previous {
        let _ = writeln!(out, "| previous pin | `{}` |", previous.pinned_date);
    }
    let _ = writeln!(out);

    if plan.import_in_progress {
        let _ = writeln!(
            out,
            "> eCFR reported an import in progress when this ran. The ledger may move \
             again shortly; the pin is no later than the title's `up_to_date_as_of`.\n"
        );
    }

    if changes.genesis {
        let _ = writeln!(
            out,
            "Genesis. The whole title is signed for the first time; there is no delta to show.\n"
        );
    }

    // On genesis every section is "added"; listing 8,127 of them says nothing.
    if !changes.ledger.amended.is_empty() && !changes.genesis {
        let _ = writeln!(out, "## Sections amended\n");
        let mut rows = changes.ledger.amended.clone();
        rows.sort_by(|a, b| b.row.amendment_date.cmp(&a.row.amendment_date));
        for amended in rows.iter().take(80) {
            let _ = writeln!(
                out,
                "- `34 CFR {}` {} \u{2192} {}{}{} \u{2014} {}",
                amended.row.identifier,
                amended.from,
                amended.row.amendment_date,
                if amended.row.substantive {
                    ""
                } else {
                    " *(technical)*"
                },
                if amended.row.removed {
                    " **(removed)**"
                } else {
                    ""
                },
                amended.row.name
            );
        }
        if rows.len() > 80 {
            let _ = writeln!(out, "- \u{2026} {} more", rows.len() - 80);
        }
        let _ = writeln!(out);
    }

    if !changes.ledger.added.is_empty() && !changes.genesis {
        let _ = writeln!(out, "## Sections added\n");
        for row in changes.ledger.added.iter().take(40) {
            let _ = writeln!(out, "- `34 CFR {}` \u{2014} {}", row.identifier, row.name);
        }
        let _ = writeln!(out);
    }

    if !changes.ledger.dropped.is_empty() && !changes.genesis {
        let _ = writeln!(out, "## Sections no longer listed\n");
        for row in changes.ledger.dropped.iter().take(40) {
            let _ = writeln!(out, "- `34 CFR {}` \u{2014} {}", row.identifier, row.name);
        }
        let _ = writeln!(out);
    }

    if !changes.parts_moved.is_empty() && !changes.genesis {
        let _ = writeln!(out, "## Parts whose text moved\n");
        let _ = writeln!(out, "| part | heading | sections | atoms | binding |");
        let _ = writeln!(out, "|---|---|---|---|---|");
        for number in changes.parts_moved.iter().take(40) {
            match bundle.parts.get(number) {
                Some(record) => {
                    let _ = writeln!(
                        out,
                        "| {number} | {} | {} | {} | {} |",
                        record.heading, record.sections, record.atoms, record.binding
                    );
                }
                None => {
                    let _ = writeln!(out, "| {number} | *(no longer in the title)* | — | — | — |");
                }
            }
        }
        let _ = writeln!(out);
    }

    if !plan.retired_parts.is_empty() {
        let _ = writeln!(
            out,
            "## Retired parts\n\nNamed by the amendment ledger but absent from the title at \
             this date, so they were not fetched: {}\n",
            plan.retired_parts.join(", ")
        );
    }

    if bundle.totals.pending > 0 {
        let _ = writeln!(
            out,
            "## Published but not yet incorporated\n\n{} amendment(s) are published in the \
             Federal Register and not yet folded into the text this pin retrieves. They are \
             changes that have already happened and are invisible in the regulation as read.\n",
            bundle.totals.pending
        );
    }

    let _ = writeln!(out, "## Derivation coverage\n");
    let _ = writeln!(
        out,
        "Obligation atoms are derived from regulatory prose, not read from a `force` field. \
         What the derivation could not settle is reported rather than assumed.\n"
    );
    let _ = writeln!(out, "| | |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| paragraphs read | {} |", bundle.totals.paragraphs);
    let _ = writeln!(
        out,
        "| unresolved designator sequences | {} ({:.2}%) |",
        bundle.totals.unresolved,
        percent(bundle.totals.unresolved, bundle.totals.paragraphs)
    );
    let _ = writeln!(
        out,
        "| unclassified bearer | {} ({:.1}%) |",
        bundle.totals.unclassified_bearer,
        percent(bundle.totals.unclassified_bearer, bundle.totals.atoms)
    );
    for (force, count) in &bundle.totals.by_force {
        let _ = writeln!(out, "| `{force}` | {count} |");
    }
    out
}

/// Counts here are in the tens of thousands, far inside f64's exact range.
#[allow(clippy::cast_precision_loss)]
fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    100.0 * part as f64 / whole as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::Totals;
    use crate::cfr::ledger::Ledger;
    use crate::severity::Severity;
    use std::collections::BTreeMap;

    fn bundle() -> Bundle {
        Bundle {
            schema: crate::bundle::SCHEMA.into(),
            title: "34".into(),
            pinned_date: "2026-08-21".into(),
            latest_amendment_date: "2026-07-24".into(),
            pins: BTreeMap::new(),
            ledger: Ledger::default(),
            parts: BTreeMap::new(),
            totals: Totals {
                parts: 110,
                sections: 8127,
                paragraphs: 50_000,
                atoms: 20_000,
                binding: 12_000,
                unresolved: 125,
                ..Totals::default()
            },
        }
    }

    fn changes(severity: Severity, genesis: bool) -> Changes {
        Changes {
            severity,
            ledger: crate::cfr::ledger::Changes::default(),
            parts_moved: vec!["668".into()],
            genesis,
        }
    }

    #[test]
    fn the_headline_names_the_pin_so_a_reader_knows_what_version_moved() {
        let text = headline(&changes(Severity::Major, false), &bundle());
        assert!(text.contains("TITLE 34 CHANGED"));
        assert!(text.contains("2026-08-21"));
    }

    #[test]
    fn genesis_says_so_rather_than_claiming_a_delta() {
        let text = headline(&changes(Severity::Major, true), &bundle());
        assert!(text.contains("genesis"));
        assert!(text.contains("110 parts"));
    }

    #[test]
    fn genesis_does_not_list_every_section_as_an_addition() {
        let mut genesis = changes(Severity::Major, true);
        genesis.ledger.added = vec![crate::cfr::ledger::Row {
            identifier: "100.1".into(),
            name: "\u{a7} 100.1 Purpose.".into(),
            part: "100".into(),
            subpart: None,
            amendment_date: "2016-12-08".into(),
            issue_date: "2016-12-19".into(),
            substantive: true,
            removed: false,
            kind: "section".into(),
        }];
        let plan = Plan {
            bundle: bundle(),
            parsed: Vec::new(),
            import_in_progress: false,
            retired_parts: Vec::new(),
        };
        let text = markdown(&plan, &genesis, None);
        assert!(text.contains("no delta to show"));
        assert!(
            !text.contains("Sections added"),
            "genesis must not present the whole title as a change"
        );
    }

    #[test]
    fn the_heading_is_not_doubled() {
        let plan = Plan {
            bundle: bundle(),
            parsed: Vec::new(),
            import_in_progress: false,
            retired_parts: Vec::new(),
        };
        let first = markdown(&plan, &changes(Severity::Major, false), None)
            .lines()
            .next()
            .unwrap()
            .to_string();
        assert_eq!(first.matches("34 CFR").count(), 0, "got: {first}");
        assert!(first.starts_with("# TITLE 34 CHANGED"));
    }

    #[test]
    fn the_report_always_states_what_the_derivation_could_not_settle() {
        let plan = Plan {
            bundle: bundle(),
            parsed: Vec::new(),
            import_in_progress: false,
            retired_parts: vec!["230".into()],
        };
        let text = markdown(&plan, &changes(Severity::Major, false), None);
        assert!(text.contains("unresolved designator sequences"));
        assert!(text.contains("0.25%"), "the rate is reported, not hidden");
        assert!(text.contains("Retired parts"));
        assert!(text.contains("230"));
    }

    #[test]
    fn an_import_in_progress_is_surfaced_to_the_reviewer() {
        let plan = Plan {
            bundle: bundle(),
            parsed: Vec::new(),
            import_in_progress: true,
            retired_parts: Vec::new(),
        };
        let text = markdown(&plan, &changes(Severity::Metadata, false), None);
        assert!(text.contains("import in progress"));
    }

    #[test]
    fn percentages_do_not_divide_by_zero() {
        assert!((percent(0, 0) - 0.0).abs() < f64::EPSILON);
        assert!((percent(1, 4) - 25.0).abs() < f64::EPSILON);
    }
}
