//! The eCFR versioner client.
//!
//! Point-in-time retrieval is the pin. `full/{date}/title-34.xml?part=N`
//! returns the regulation as it stood on `date`, and — measured, see
//! DESIGN.md §2 — returns *byte-identical* content for every date in a window
//! with no intervening amendment. There is no volatile header to project away
//! and no commit SHA to resolve: the date is the version, and a run is
//! replayable by anyone with the same date.

pub mod ledger;
pub mod paragraph;
pub mod xml;

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use ledger::Ledger;
use xml::Part;

pub const TITLE: &str = "34";
const BASE: &str = "https://www.ecfr.gov/api/versioner/v1";
const USER_AGENT: &str = "doe-aion (+https://github.com/aion-context/aion-doe)";

/// Live fetch, or replay of a captured snapshot so the logic runs offline.
pub enum Fetcher {
    Http,
    Dir { root: PathBuf },
}

impl Fetcher {
    pub fn dir(root: impl Into<PathBuf>) -> Self {
        Self::Dir { root: root.into() }
    }

    fn get(&self, url: &str, offline: &Path) -> Result<Vec<u8>> {
        match self {
            Self::Http => http_get(url),
            Self::Dir { root } => {
                let path = root.join(offline);
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
            }
        }
    }
}

/// Spacing between live requests. Reading a whole title is ~110 requests, and
/// eCFR is a public service with no published quota; going flat out gets a run
/// throttled into a multi-minute crawl, which is slower than pausing.
const COURTESY_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Attempts before a transport failure aborts the run.
///
/// Four was not enough. A full read is ~110 requests and eCFR answers some of
/// them with 503 under that load; with backoff capped near eight seconds the
/// attempts were exhausted while the service was still merely busy. A retried
/// transport error is not a change signal, so the cost of giving up early is a
/// run that fails rather than one that reports something false — but it is
/// still a run that fails, and the weekly full pass depends on this.
const ATTEMPTS: u32 = 7;

/// eCFR rate-limits and occasionally times out on whole-title requests. A
/// retried transport error is not a change signal, so it must never surface as
/// one — the run aborts instead of committing a partial view.
fn http_get(url: &str) -> Result<Vec<u8>> {
    std::thread::sleep(COURTESY_DELAY);
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(2u64.pow(attempt)));
        }
        match ureq::get(url)
            .set("User-Agent", USER_AGENT)
            .timeout(std::time::Duration::from_secs(180))
            .call()
        {
            Ok(response) => {
                let mut body = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut body)
                    .with_context(|| format!("reading body of {url}"))?;
                return Ok(body);
            }
            Err(ureq::Error::Status(code, _)) if code == 429 || code >= 500 => {
                last = Some(anyhow::anyhow!("GET {url} returned {code}"));
            }
            Err(e) => return Err(anyhow::anyhow!("GET {url}: {e}")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("GET {url} exhausted retries")))
}

/// The date eCFR considers the title current through, and whether an import
/// is running. A title mid-import can report a ledger that is about to move,
/// so the flag is carried into the report rather than silently ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleStatus {
    pub up_to_date_as_of: String,
    pub latest_amended_on: String,
    pub latest_issue_date: String,
    pub import_in_progress: bool,
}

pub fn raw_titles(fetcher: &Fetcher) -> Result<Vec<u8>> {
    fetcher.get(&format!("{BASE}/titles.json"), Path::new("titles.json"))
}

/// Writes every ledger page to `out`, returning the number written.
pub fn capture_ledger(fetcher: &Fetcher, out: &Path, date: &str) -> Result<usize> {
    let mut page = 1usize;
    let mut total = 1usize;
    while page <= total {
        let raw = fetcher.get(
            &format!("{BASE}/versions/title-{TITLE}.json?issue_date%5Blte%5D={date}&page={page}"),
            &PathBuf::from(format!("ledger-{page}.json")),
        )?;
        let value: Value = serde_json::from_slice(&raw)
            .with_context(|| format!("ledger page {page} was not JSON"))?;
        if page == 1 {
            total = Ledger::total_pages(&value);
        }
        std::fs::write(out.join(format!("ledger-{page}.json")), &raw)?;
        page += 1;
    }
    Ok(total)
}

pub fn title_status(fetcher: &Fetcher) -> Result<TitleStatus> {
    let raw = raw_titles(fetcher)?;
    let value: Value = serde_json::from_slice(&raw).context("titles.json was not JSON")?;
    let row = value
        .get("titles")
        .and_then(Value::as_array)
        .context("titles.json has no titles array")?
        .iter()
        .find(|t| t.get("number").and_then(Value::as_u64) == Some(34))
        .context("titles.json does not list title 34")?;
    let text = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok(TitleStatus {
        up_to_date_as_of: text("up_to_date_as_of"),
        latest_amended_on: text("latest_amended_on"),
        latest_issue_date: text("latest_issue_date"),
        import_in_progress: value
            .get("meta")
            .and_then(|m| m.get("import_in_progress"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// The amendment ledger for Title 34 **as of `date`**.
///
/// The date filter is not optional. The unfiltered endpoint returns the ledger
/// as it stands today, so pinning the text to a past date while reading a
/// current ledger would sign a bundle whose ledger describes amendments the
/// text does not contain. Measured: `issue_date[lte]=2026-07-15` returns 8,105
/// rows with a latest amendment of 2026-07-01, against 8,127 and 2026-07-24
/// unfiltered.
pub fn fetch_ledger(fetcher: &Fetcher, date: &str) -> Result<Ledger> {
    let mut ledger = Ledger::default();
    let mut page = 1usize;
    let mut total = 1usize;
    while page <= total {
        let raw = fetcher.get(
            &format!("{BASE}/versions/title-{TITLE}.json?issue_date%5Blte%5D={date}&page={page}"),
            &PathBuf::from(format!("ledger-{page}.json")),
        )?;
        let value: Value = serde_json::from_slice(&raw)
            .with_context(|| format!("ledger page {page} was not JSON"))?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            bail!("eCFR rejected the ledger request: {error}");
        }
        if page == 1 {
            total = Ledger::total_pages(&value);
            anyhow::ensure!(total > 0 && total < 1000, "implausible page count {total}");
        }
        ledger.absorb_page(&value)?;
        page += 1;
    }
    anyhow::ensure!(
        !ledger.rows.is_empty(),
        "the ledger came back empty; refusing to treat that as `everything was removed`"
    );
    Ok(ledger)
}

/// A part as the title's table of contents lists it at a given date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRef {
    pub number: String,
    pub heading: String,
    pub reserved: bool,
}

/// The parts that exist at `date`.
///
/// This is not the same set as the ledger's. The ledger is a record of every
/// amendment the title has ever had, so it still names parts that have since
/// been removed — part 230 appears in it and 404s at 2026-08-21. Fetching by
/// ledger part therefore fails on history; the structure is the table of
/// contents, and the difference between the two is exactly the set of parts
/// that no longer exist.
pub fn structure_parts(fetcher: &Fetcher, date: &str) -> Result<Vec<PartRef>> {
    let raw = fetcher.get(
        &format!("{BASE}/structure/{date}/title-{TITLE}.json"),
        Path::new("structure.json"),
    )?;
    let value: Value = serde_json::from_slice(&raw).context("title structure was not JSON")?;
    let mut parts = Vec::new();
    collect_parts(&value, &mut parts);
    anyhow::ensure!(
        !parts.is_empty(),
        "the title structure listed no parts; refusing to treat that as an empty title"
    );
    Ok(parts)
}

fn collect_parts(node: &Value, out: &mut Vec<PartRef>) {
    if node.get("type").and_then(Value::as_str) == Some("part") {
        out.push(PartRef {
            number: node
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            heading: node
                .get("label_description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            reserved: node
                .get("reserved")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            collect_parts(child, out);
        }
    }
}

pub fn raw_structure(fetcher: &Fetcher, date: &str) -> Result<Vec<u8>> {
    fetcher.get(
        &format!("{BASE}/structure/{date}/title-{TITLE}.json"),
        Path::new("structure.json"),
    )
}

pub fn part_xml(fetcher: &Fetcher, date: &str, part: &str) -> Result<Vec<u8>> {
    let raw = fetcher.get(
        &format!("{BASE}/full/{date}/title-{TITLE}.xml?part={part}"),
        &PathBuf::from(format!("part-{part}.xml")),
    )?;
    anyhow::ensure!(
        !raw.is_empty(),
        "eCFR returned an empty body for part {part} at {date}"
    );
    Ok(raw)
}

pub fn fetch_part(fetcher: &Fetcher, date: &str, part: &str) -> Result<Part> {
    let raw = part_xml(fetcher, date, part)?;
    xml::parse_part(&raw).with_context(|| format!("parsing 34 CFR part {part} as of {date}"))
}

/// eCFR retains point-in-time content back to 2017; a date before that, or in
/// the future, is a request the API cannot honour and must not be pinned to.
pub fn valid_pin(date: &str, up_to_date_as_of: &str) -> Result<()> {
    anyhow::ensure!(
        date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-',
        "`{date}` is not an ISO date"
    );
    anyhow::ensure!(
        date >= "2017-01-01",
        "eCFR has no point-in-time content before 2017-01-01"
    );
    anyhow::ensure!(
        date <= up_to_date_as_of,
        "eCFR is only current through {up_to_date_as_of}; pinning {date} would \
         retrieve content the title has not been updated to yet"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_future_pin_is_refused() {
        let error = valid_pin("2026-09-01", "2026-08-21")
            .unwrap_err()
            .to_string();
        assert!(error.contains("only current through 2026-08-21"));
    }

    #[test]
    fn a_pin_before_the_archive_is_refused() {
        assert!(valid_pin("2015-01-01", "2026-08-21").is_err());
    }

    #[test]
    fn a_malformed_date_is_refused_before_it_reaches_the_api() {
        assert!(valid_pin("2026-8-1", "2026-08-21").is_err());
        assert!(valid_pin("yesterday", "2026-08-21").is_err());
    }

    #[test]
    fn the_current_date_is_a_valid_pin() {
        assert!(valid_pin("2026-08-21", "2026-08-21").is_ok());
        assert!(valid_pin("2026-07-25", "2026-08-21").is_ok());
    }
}
