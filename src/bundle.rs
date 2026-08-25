//! The signed payload.
//!
//! What goes in the chain is a deliberate choice. `fedramp-aion` signs the full
//! canonical source content, which is affordable because its sources total a
//! few megabytes. Title 34 is an order of magnitude larger, and putting it in
//! the payload would make the artifact unreadable without buying anything: the
//! chain is a signed hash trail, not an archive.
//!
//! So the payload carries the **ledger** — eCFR's own record of what was
//! amended when — plus a digest per part and the derived obligation index.
//! The regulation text lives in `data/`, committed alongside, and the payload's
//! digests bind it. Editing the text without editing the chain fails
//! verification; editing both fails the signature.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::canon;
use crate::cfr::ledger::Ledger;

pub const SCHEMA: &str = "doe-aion/1";

/// Exactly which upstream bytes produced a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub publisher: String,
    /// The point-in-time date, for eCFR. Other publishers use their own token.
    pub version: String,
    pub endpoint: String,
    pub sha256: String,
    pub bytes: u64,
}

/// One part of the title as signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartRecord {
    pub heading: String,
    /// Digest of the canonical parsed part, not of the upstream XML — so a
    /// whitespace-only republish cannot look like an amendment.
    pub sha256: String,
    /// Digest of the upstream bytes, for byte-level replay.
    pub raw_sha256: String,
    pub sections: usize,
    pub paragraphs: usize,
    pub atoms: usize,
    pub binding: usize,
    /// Designator sequences the reader could not resolve. Signed, so the
    /// artifact never claims a cleaner parse than it achieved.
    pub unresolved: usize,
    /// Amendments eCFR has published but not yet incorporated.
    pub pending: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub parts: usize,
    pub sections: usize,
    pub paragraphs: usize,
    pub atoms: usize,
    pub binding: usize,
    pub unresolved: usize,
    pub pending: usize,
    pub unclassified_bearer: usize,
    pub by_force: BTreeMap<String, usize>,
    pub by_bearer: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub schema: String,
    pub title: String,
    /// The date every part was retrieved as of. This *is* the version.
    pub pinned_date: String,
    /// eCFR's newest incorporated amendment, across the whole title.
    pub latest_amendment_date: String,
    pub pins: BTreeMap<String, Pin>,
    /// eCFR's amendment ledger, section by section.
    pub ledger: Ledger,
    pub parts: BTreeMap<String, PartRecord>,
    pub totals: Totals,
}

impl Bundle {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bundle: Self =
            serde_json::from_slice(bytes).context("chain payload is not a doe-aion bundle")?;
        anyhow::ensure!(
            bundle.schema == SCHEMA,
            "chain payload declares schema `{}`, expected `{SCHEMA}`",
            bundle.schema
        );
        Ok(bundle)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let value = serde_json::to_value(self)?;
        let oversized = canon::unsafe_integers(&value);
        anyhow::ensure!(
            oversized.is_empty(),
            "refusing to sign: JCS would round these integers past 2^53 — {}",
            oversized.join(", ")
        );
        canon::canonical_bytes(&value)
    }

    pub fn digest(&self) -> Result<String> {
        Ok(canon::sha256_hex(&self.to_bytes()?))
    }

    /// The timestamp the chain version is pinned to: midnight UTC on the newest
    /// amendment incorporated upstream, never the wall clock. A rerun against
    /// the same date must produce the same version, and a wall-clock timestamp
    /// would destroy that.
    pub fn pinned_timestamp(&self) -> Option<u64> {
        epoch_seconds(&self.latest_amendment_date)
    }

    /// Sections whose text differs from the previous signed version, via the
    /// parts whose digest moved.
    pub fn parts_that_moved(&self, previous: &Bundle) -> Vec<String> {
        let mut moved: Vec<String> = self
            .parts
            .iter()
            .filter(|(number, record)| {
                previous
                    .parts
                    .get(*number)
                    .is_none_or(|old| old.sha256 != record.sha256)
            })
            .map(|(number, _)| number.clone())
            .collect();
        moved.extend(
            previous
                .parts
                .keys()
                .filter(|number| !self.parts.contains_key(*number))
                .cloned(),
        );
        moved.sort_by_key(|part| numeric_order(part));
        moved.dedup();
        moved
    }
}

/// Midnight UTC on an ISO date, as Unix seconds.
pub fn epoch_seconds(date: &str) -> Option<u64> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    let parsed = time::Date::parse(date, format).ok()?;
    u64::try_from(parsed.midnight().assume_utc().unix_timestamp()).ok()
}

fn numeric_order(part: &str) -> (u32, String) {
    let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
    (digits.parse().unwrap_or(u32::MAX), part.to_string())
}

/// A canonical JSON view of one part, for `data/` and for the part digest.
pub fn part_document(part: &crate::cfr::xml::Part) -> Result<Value> {
    Ok(serde_json::to_value(part)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfr::ledger::Ledger;

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

    fn bundle(date: &str, parts: &[(&str, &str)]) -> Bundle {
        Bundle {
            schema: SCHEMA.into(),
            title: "34".into(),
            pinned_date: date.into(),
            latest_amendment_date: date.into(),
            pins: BTreeMap::new(),
            ledger: Ledger::default(),
            parts: parts
                .iter()
                .map(|(n, sha)| ((*n).to_string(), record(sha)))
                .collect(),
            totals: Totals::default(),
        }
    }

    #[test]
    fn a_bundle_round_trips_through_its_canonical_bytes() {
        let original = bundle("2026-08-21", &[("668", "aa")]);
        let parsed = Bundle::parse(&original.to_bytes().unwrap()).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn the_digest_is_stable_across_serialisations() {
        let one = bundle("2026-08-21", &[("668", "aa"), ("99", "bb")]);
        let two = Bundle::parse(&one.to_bytes().unwrap()).unwrap();
        assert_eq!(one.digest().unwrap(), two.digest().unwrap());
    }

    #[test]
    fn a_foreign_payload_is_refused_rather_than_misread() {
        let error = Bundle::parse(br#"{"schema":"fedramp-aion/1"}"#)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a doe-aion bundle") || error.contains("schema"));
    }

    #[test]
    fn the_version_timestamp_is_the_amendment_date_not_the_clock() {
        // 2026-08-21T00:00:00Z
        assert_eq!(
            bundle("2026-08-21", &[]).pinned_timestamp(),
            Some(1_787_270_400)
        );
        assert_eq!(epoch_seconds("1970-01-01"), Some(0));
        assert_eq!(epoch_seconds("not-a-date"), None);
    }

    #[test]
    fn two_runs_on_different_days_at_the_same_pin_agree_on_the_timestamp() {
        assert_eq!(
            bundle("2026-08-21", &[]).pinned_timestamp(),
            bundle("2026-08-21", &[]).pinned_timestamp()
        );
    }

    #[test]
    fn only_parts_whose_digest_moved_are_reported() {
        let before = bundle("2026-07-25", &[("99", "aa"), ("668", "bb")]);
        let after = bundle("2026-08-21", &[("99", "aa"), ("668", "cc")]);
        assert_eq!(after.parts_that_moved(&before), vec!["668"]);
    }

    #[test]
    fn an_added_or_removed_part_counts_as_movement() {
        let before = bundle("2026-07-25", &[("99", "aa"), ("230", "zz")]);
        let after = bundle("2026-08-21", &[("99", "aa"), ("668", "cc")]);
        assert_eq!(after.parts_that_moved(&before), vec!["230", "668"]);
    }

    #[test]
    fn a_rerun_at_the_same_date_produces_identical_bytes() {
        let one = bundle("2026-08-21", &[("668", "aa")]);
        let two = bundle("2026-08-21", &[("668", "aa")]);
        assert_eq!(one.to_bytes().unwrap(), two.to_bytes().unwrap());
    }
}
