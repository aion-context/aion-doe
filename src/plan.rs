//! Building the bundle: resolve → fetch → parse → derive → digest → compare.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

use crate::bundle::{Bundle, PartRecord, Pin, Totals, SCHEMA};
use crate::canon;
use crate::cfr::{self, ledger::Ledger, Fetcher};
use crate::obligations;
use crate::severity::Severity;

/// A part as read, kept alongside its record so `data/` and the payload are
/// written from the same parse.
#[derive(Debug)]
pub struct Parsed {
    pub number: String,
    pub record: PartRecord,
    pub document: serde_json::Value,
}

#[derive(Debug)]
pub struct Plan {
    pub bundle: Bundle,
    pub parsed: Vec<Parsed>,
    /// eCFR was mid-import when the ledger was read.
    pub import_in_progress: bool,
    /// Parts named by the ledger that the title no longer contains.
    pub retired_parts: Vec<String>,
}

/// Reads the whole title at `date` and derives everything the payload carries.
pub fn build(fetcher: &Fetcher, date: Option<&str>) -> Result<Plan> {
    let status = cfr::title_status(fetcher)?;
    let date = date.unwrap_or(&status.up_to_date_as_of).to_string();
    cfr::valid_pin(&date, &status.up_to_date_as_of)?;

    let ledger = cfr::fetch_ledger(fetcher, &date)?;
    let live = cfr::structure_parts(fetcher, &date)?;
    let live_numbers: std::collections::BTreeSet<String> =
        live.iter().map(|p| p.number.clone()).collect();
    // The ledger records every amendment the title has ever had, so it still
    // names parts that no longer exist. Fetching those 404s.
    let retired_parts: Vec<String> = ledger
        .parts()
        .into_keys()
        .filter(|part| !part.is_empty() && !live_numbers.contains(part))
        .collect();

    let mut parts = BTreeMap::new();
    let mut parsed = Vec::new();
    let mut totals = Totals::default();

    for reference in live.iter().filter(|p| !p.reserved && !p.number.is_empty()) {
        let raw = cfr::part_xml(fetcher, &date, &reference.number)?;
        let part = cfr::xml::parse_part(&raw)
            .with_context(|| format!("parsing 34 CFR part {} as of {date}", reference.number))?;
        let (atoms, coverage) = obligations::extract(&part);

        let document = serde_json::to_value(&part)?;
        let record = PartRecord {
            heading: if part.heading.is_empty() {
                reference.heading.clone()
            } else {
                part.heading.clone()
            },
            sha256: canon::digest_value(&document)?,
            raw_sha256: canon::sha256_hex(&raw),
            sections: part.sections.len(),
            paragraphs: coverage.paragraphs,
            atoms: atoms.len(),
            binding: atoms.iter().filter(|a| a.force.binding()).count(),
            unresolved: part.sections.iter().map(|s| s.irregularities.len()).sum(),
            pending: part.sections.iter().map(|s| s.pending.len()).sum(),
        };

        totals.parts += 1;
        totals.sections += record.sections;
        totals.paragraphs += record.paragraphs;
        totals.atoms += record.atoms;
        totals.binding += record.binding;
        totals.unresolved += record.unresolved;
        totals.pending += record.pending;
        totals.unclassified_bearer += coverage.unclassified;
        for (key, count) in coverage.by_force {
            *totals.by_force.entry(key).or_insert(0) += count;
        }
        for (key, count) in coverage.by_bearer {
            *totals.by_bearer.entry(key).or_insert(0) += count;
        }

        parts.insert(reference.number.clone(), record.clone());
        parsed.push(Parsed {
            number: reference.number.clone(),
            record,
            document,
        });
    }

    let bundle = Bundle {
        schema: SCHEMA.to_string(),
        title: cfr::TITLE.to_string(),
        pinned_date: date.clone(),
        latest_amendment_date: ledger.latest_amendment_date.clone(),
        pins: BTreeMap::from([(
            "cfr".to_string(),
            Pin {
                publisher: "eCFR".to_string(),
                version: date,
                endpoint: "versioner/v1".to_string(),
                sha256: canon::digest_value(&serde_json::to_value(&ledger)?)?,
                bytes: u64::try_from(ledger.rows.len()).unwrap_or_default(),
            },
        )]),
        ledger,
        parts,
        totals,
    };

    Ok(Plan {
        bundle,
        parsed,
        import_in_progress: status.import_in_progress,
        retired_parts,
    })
}

/// What moved between two signed versions, and how loud it is.
pub struct Changes {
    pub severity: Severity,
    pub ledger: crate::cfr::ledger::Changes,
    pub parts_moved: Vec<String>,
    pub genesis: bool,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        !self.genesis && self.ledger.is_empty() && self.parts_moved.is_empty()
    }
}

pub fn compare(current: &Bundle, previous: Option<&Bundle>) -> Changes {
    let Some(previous) = previous else {
        return Changes {
            severity: Severity::Major,
            ledger: current.ledger.changes_since(&Ledger::default()),
            parts_moved: current.parts.keys().cloned().collect(),
            genesis: true,
        };
    };
    let ledger = current.ledger.changes_since(&previous.ledger);
    let parts_moved = current.parts_that_moved(previous);
    let severity = if ledger.has_substantive() {
        Severity::Major
    } else if !ledger.is_empty() {
        // eCFR itself called every amendment here non-substantive.
        Severity::Minor
    } else if !parts_moved.is_empty() {
        // Text moved with no ledger entry: a correction, or a reader change.
        Severity::Minor
    } else if current.pinned_date != previous.pinned_date {
        Severity::Metadata
    } else {
        Severity::None
    };
    Changes {
        severity,
        ledger,
        parts_moved,
        genesis: false,
    }
}

/// Writes the reviewable per-part JSON the payload's digests bind.
pub fn write_data(plan: &Plan, data_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let mut expected: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for parsed in &plan.parsed {
        let path = data_dir.join(format!("part-{}.json", parsed.number));
        expected.insert(format!("part-{}.json", parsed.number));
        std::fs::write(&path, canon::canonical_bytes(&parsed.document)?)?;
    }
    std::fs::write(
        data_dir.join("ledger.json"),
        canon::canonical_bytes(&serde_json::to_value(&plan.bundle.ledger)?)?,
    )?;
    expected.insert("ledger.json".to_string());
    // A part that was removed upstream must not linger in `data/`, or the
    // committed tree would claim a regulation that no longer exists.
    for entry in std::fs::read_dir(data_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if std::path::Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            && !expected.contains(&name)
        {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::PartRecord;

    fn record(sha: &str) -> PartRecord {
        PartRecord {
            heading: "Test".into(),
            sha256: sha.into(),
            raw_sha256: sha.into(),
            sections: 1,
            paragraphs: 1,
            atoms: 1,
            binding: 1,
            unresolved: 0,
            pending: 0,
        }
    }

    fn bundle(date: &str, ledger: Ledger, parts: &[(&str, &str)]) -> Bundle {
        Bundle {
            schema: SCHEMA.into(),
            title: "34".into(),
            pinned_date: date.into(),
            latest_amendment_date: ledger.latest_amendment_date.clone(),
            pins: BTreeMap::new(),
            ledger,
            parts: parts
                .iter()
                .map(|(n, sha)| ((*n).to_string(), record(sha)))
                .collect(),
            totals: Totals::default(),
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn ledger(rows: serde_json::Value, latest: &str) -> Ledger {
        let mut l = Ledger::default();
        l.absorb_page(&serde_json::json!({
            "meta": {"title": "34", "latest_amendment_date": latest,
                     "latest_issue_date": latest, "total_pages": "1"},
            "content_versions": rows
        }))
        .unwrap();
        l
    }

    fn row(id: &str, part: &str, date: &str, substantive: bool) -> serde_json::Value {
        serde_json::json!({"identifier": id, "name": "x", "part": part, "subpart": null,
            "amendment_date": date, "issue_date": date, "substantive": substantive,
            "removed": false, "type": "section"})
    }

    #[test]
    fn genesis_is_major_and_names_every_part() {
        let current = bundle(
            "2026-08-21",
            ledger(
                serde_json::json!([row("668.14", "668", "2026-07-20", true)]),
                "2026-07-20",
            ),
            &[("668", "aa")],
        );
        let changes = compare(&current, None);
        assert!(changes.genesis);
        assert_eq!(changes.severity, Severity::Major);
        assert_eq!(changes.parts_moved, vec!["668"]);
    }

    #[test]
    fn a_rerun_at_the_same_pin_reports_nothing() {
        let l = ledger(
            serde_json::json!([row("668.14", "668", "2026-07-20", true)]),
            "2026-07-20",
        );
        let one = bundle("2026-08-21", l.clone(), &[("668", "aa")]);
        let two = bundle("2026-08-21", l, &[("668", "aa")]);
        let changes = compare(&two, Some(&one));
        assert!(changes.is_empty());
        assert_eq!(changes.severity, Severity::None);
    }

    #[test]
    fn a_new_pin_with_no_movement_is_metadata_not_a_change() {
        let l = ledger(
            serde_json::json!([row("668.14", "668", "2026-07-20", true)]),
            "2026-07-20",
        );
        let before = bundle("2026-08-21", l.clone(), &[("668", "aa")]);
        let after = bundle("2026-08-24", l, &[("668", "aa")]);
        assert_eq!(compare(&after, Some(&before)).severity, Severity::Metadata);
    }

    #[test]
    fn a_substantive_amendment_is_major() {
        let before = bundle(
            "2026-08-21",
            ledger(
                serde_json::json!([row("668.14", "668", "2026-07-20", true)]),
                "2026-07-20",
            ),
            &[("668", "aa")],
        );
        let after = bundle(
            "2026-09-01",
            ledger(
                serde_json::json!([row("668.14", "668", "2026-08-30", true)]),
                "2026-08-30",
            ),
            &[("668", "bb")],
        );
        let changes = compare(&after, Some(&before));
        assert_eq!(changes.severity, Severity::Major);
        assert_eq!(changes.parts_moved, vec!["668"]);
        assert_eq!(changes.ledger.amended.len(), 1);
    }

    #[test]
    fn a_technical_amendment_is_minor_so_a_real_one_is_never_buried() {
        let before = bundle(
            "2026-08-21",
            ledger(
                serde_json::json!([row("99.3", "99", "2026-01-01", true)]),
                "2026-01-01",
            ),
            &[("99", "aa")],
        );
        let after = bundle(
            "2026-09-01",
            ledger(
                serde_json::json!([row("99.3", "99", "2026-08-30", false)]),
                "2026-08-30",
            ),
            &[("99", "bb")],
        );
        assert_eq!(compare(&after, Some(&before)).severity, Severity::Minor);
    }

    #[test]
    fn text_moving_without_a_ledger_entry_is_still_reported() {
        let l = ledger(
            serde_json::json!([row("668.14", "668", "2026-07-20", true)]),
            "2026-07-20",
        );
        let before = bundle("2026-08-21", l.clone(), &[("668", "aa")]);
        let after = bundle("2026-08-21", l, &[("668", "bb")]);
        let changes = compare(&after, Some(&before));
        assert_eq!(changes.severity, Severity::Minor);
        assert!(!changes.is_empty(), "a silent correction must not pass");
    }
}
