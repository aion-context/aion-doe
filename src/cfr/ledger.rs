//! Title 34's amendment ledger — the change gate.
//!
//! eCFR publishes, for every section and appendix in a title, the date it was
//! last amended and whether that amendment was substantive. That is a far
//! better gate than a content digest: the ledger is ~250 KB per page against
//! megabytes of regulation text, and it names *which* sections moved, so only
//! the parts that actually changed need to be fetched.
//!
//! It also distinguishes a substantive amendment from a technical one, and
//! marks removals — two facts a digest comparison could never recover.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// One section or appendix, with the date it last moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub identifier: String,
    pub name: String,
    pub part: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpart: Option<String>,
    pub amendment_date: String,
    pub issue_date: String,
    pub substantive: bool,
    pub removed: bool,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Row {
    fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            identifier: value.get("identifier")?.as_str()?.to_string(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            part: value
                .get("part")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            subpart: value
                .get("subpart")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            amendment_date: value
                .get("amendment_date")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            issue_date: value
                .get("issue_date")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            substantive: value
                .get("substantive")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            removed: value
                .get("removed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            kind: value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("section")
                .to_string(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub title: String,
    /// The newest amendment eCFR has folded into the text.
    pub latest_amendment_date: String,
    pub latest_issue_date: String,
    /// Keyed by identifier, so the diff names sections rather than array slots.
    pub rows: BTreeMap<String, Row>,
}

impl Ledger {
    /// Merges one page of `versions/title-34.json`.
    pub fn absorb_page(&mut self, page: &Value) -> Result<usize> {
        let meta = page.get("meta").context("ledger page has no meta")?;
        if self.title.is_empty() {
            self.title = meta
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("34")
                .to_string();
        }
        for key in ["latest_amendment_date", "latest_issue_date"] {
            if let Some(value) = meta.get(key).and_then(Value::as_str) {
                let slot = if key == "latest_amendment_date" {
                    &mut self.latest_amendment_date
                } else {
                    &mut self.latest_issue_date
                };
                if value > slot.as_str() {
                    *slot = value.to_string();
                }
            }
        }
        let versions = page
            .get("content_versions")
            .and_then(Value::as_array)
            .context("ledger page has no content_versions array")?;
        let before = self.rows.len();
        for entry in versions {
            let Some(row) = Row::parse(entry) else {
                anyhow::bail!("ledger row is missing an identifier: {entry}");
            };
            // A section can appear once per amendment; keep the newest, which
            // is what "when did this last move" means.
            match self.rows.get(&row.identifier) {
                Some(existing) if existing.amendment_date >= row.amendment_date => {}
                _ => {
                    self.rows.insert(row.identifier.clone(), row);
                }
            }
        }
        Ok(self.rows.len() - before)
    }

    pub fn total_pages(page: &Value) -> usize {
        page.get("meta")
            .and_then(|m| m.get("total_pages"))
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v.as_u64().map(|n| usize::try_from(n).unwrap_or(1)))
            })
            .unwrap_or(1)
    }

    pub fn parts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for row in self.rows.values() {
            *counts.entry(row.part.clone()).or_insert(0) += 1;
        }
        counts
    }

    /// Sections whose amendment date moved, plus additions and removals.
    pub fn changes_since(&self, previous: &Ledger) -> Changes {
        let mut changes = Changes::default();
        for (id, row) in &self.rows {
            match previous.rows.get(id) {
                None => changes.added.push(row.clone()),
                Some(old) if old.amendment_date != row.amendment_date => {
                    changes.amended.push(Amended {
                        row: row.clone(),
                        from: old.amendment_date.clone(),
                    });
                }
                Some(old) if old.removed != row.removed => changes.amended.push(Amended {
                    row: row.clone(),
                    from: old.amendment_date.clone(),
                }),
                Some(_) => {}
            }
        }
        for (id, row) in &previous.rows {
            if !self.rows.contains_key(id) {
                changes.dropped.push(row.clone());
            }
        }
        changes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amended {
    pub row: Row,
    pub from: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Sections eCFR did not previously list.
    pub added: Vec<Row>,
    /// Sections whose amendment date moved.
    pub amended: Vec<Amended>,
    /// Sections that vanished from the ledger entirely.
    pub dropped: Vec<Row>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.amended.is_empty() && self.dropped.is_empty()
    }

    /// The parts that have to be re-fetched to see what actually changed.
    pub fn parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = self
            .added
            .iter()
            .chain(self.dropped.iter())
            .chain(self.amended.iter().map(|a| &a.row))
            .map(|row| row.part.clone())
            .filter(|part| !part.is_empty())
            .collect();
        parts.sort_by_key(|part| numeric_part_order(part));
        parts.dedup();
        parts
    }

    /// Any substantive movement, as eCFR itself classifies it.
    pub fn has_substantive(&self) -> bool {
        self.added.iter().any(|r| r.substantive)
            || self.dropped.iter().any(|r| r.substantive)
            || self.amended.iter().any(|a| a.row.substantive)
    }
}

/// `99` before `100`, and `5b` after `5`.
fn numeric_part_order(part: &str) -> (u32, String) {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    (digits.parse().unwrap_or(u32::MAX), part.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[allow(clippy::needless_pass_by_value)]
    fn page(rows: Value, latest: &str) -> Value {
        json!({
            "meta": {"title": "34", "latest_amendment_date": latest,
                     "latest_issue_date": latest, "total_pages": "1"},
            "content_versions": rows
        })
    }

    fn row(id: &str, part: &str, date: &str, substantive: bool, removed: bool) -> Value {
        json!({"identifier": id, "name": format!("\u{a7} {id}   Heading."), "part": part,
               "subpart": null, "amendment_date": date, "issue_date": date,
               "substantive": substantive, "removed": removed, "type": "section"})
    }

    fn ledger(rows: Value, latest: &str) -> Ledger {
        let mut l = Ledger::default();
        l.absorb_page(&page(rows, latest)).unwrap();
        l
    }

    #[test]
    fn a_quiet_day_produces_no_changes() {
        let before = ledger(
            json!([row("668.14", "668", "2026-07-20", true, false)]),
            "2026-07-20",
        );
        let after = before.clone();
        assert!(after.changes_since(&before).is_empty());
    }

    #[test]
    fn a_moved_amendment_date_is_the_signal() {
        let before = ledger(
            json!([row("668.14", "668", "2026-07-20", true, false)]),
            "2026-07-20",
        );
        let after = ledger(
            json!([row("668.14", "668", "2026-08-03", true, false)]),
            "2026-08-03",
        );
        let changes = after.changes_since(&before);
        assert_eq!(changes.amended.len(), 1);
        assert_eq!(changes.amended[0].from, "2026-07-20");
        assert_eq!(changes.parts(), vec!["668"]);
        assert!(changes.has_substantive());
    }

    #[test]
    fn a_technical_amendment_is_visible_but_not_substantive() {
        let before = ledger(
            json!([row("99.3", "99", "2026-01-01", true, false)]),
            "2026-01-01",
        );
        let after = ledger(
            json!([row("99.3", "99", "2026-02-01", false, false)]),
            "2026-02-01",
        );
        let changes = after.changes_since(&before);
        assert_eq!(changes.amended.len(), 1);
        assert!(
            !changes.has_substantive(),
            "eCFR flagged it non-substantive"
        );
    }

    #[test]
    fn removal_is_reported_even_when_the_row_survives() {
        let before = ledger(
            json!([row("100.5", "100", "2026-07-24", true, false)]),
            "2026-07-24",
        );
        let after = ledger(
            json!([row("100.5", "100", "2026-07-24", true, true)]),
            "2026-07-24",
        );
        assert_eq!(after.changes_since(&before).amended.len(), 1);
    }

    #[test]
    fn parts_are_ordered_numerically_not_lexically() {
        let before = Ledger::default();
        let after = ledger(
            json!([
                row("668.14", "668", "2026-07-20", true, false),
                row("99.3", "99", "2026-07-20", true, false),
                row("100.1", "100", "2026-07-20", true, false),
            ]),
            "2026-07-20",
        );
        assert_eq!(
            after.changes_since(&before).parts(),
            vec!["99", "100", "668"]
        );
    }

    #[test]
    fn the_newest_amendment_wins_when_a_section_appears_twice() {
        let mut l = Ledger::default();
        l.absorb_page(&page(
            json!([row("668.14", "668", "2020-01-01", true, false)]),
            "2020-01-01",
        ))
        .unwrap();
        l.absorb_page(&page(
            json!([row("668.14", "668", "2026-07-20", true, false)]),
            "2026-07-20",
        ))
        .unwrap();
        assert_eq!(l.rows["668.14"].amendment_date, "2026-07-20");
        assert_eq!(l.latest_amendment_date, "2026-07-20");
        assert_eq!(l.rows.len(), 1);
    }

    #[test]
    fn a_page_without_content_versions_is_an_error_not_an_empty_ledger() {
        let mut l = Ledger::default();
        assert!(l.absorb_page(&json!({"meta": {"title": "34"}})).is_err());
    }

    #[test]
    fn total_pages_reads_the_string_upstream_actually_sends() {
        assert_eq!(Ledger::total_pages(&page(json!([]), "2026-01-01")), 1);
        assert_eq!(
            Ledger::total_pages(&json!({"meta": {"total_pages": "9"}})),
            9
        );
    }
}
