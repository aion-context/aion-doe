//! End-to-end pipeline tests. No network: every source is replayed from a
//! synthetic snapshot directory, so the fixtures state exactly what upstream
//! shape the logic is claimed to handle.

use std::path::{Path, PathBuf};

use doe_aion::cfr::Fetcher;
use doe_aion::severity::Severity;
use doe_aion::{chain, plan};

struct Upstream {
    root: PathBuf,
}

impl Upstream {
    /// A snapshot directory shaped exactly like `doe-aion capture` writes one.
    fn new(name: &str, up_to_date: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("doe-aion-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let upstream = Self { root };
        upstream.write(
            "titles.json",
            &format!(
                r#"{{"meta":{{"date":"{up_to_date}","import_in_progress":false}},
                     "titles":[{{"number":34,"name":"Education","latest_amended_on":"{up_to_date}",
                                 "latest_issue_date":"{up_to_date}","up_to_date_as_of":"{up_to_date}",
                                 "reserved":false}}]}}"#
            ),
        );
        upstream.write(
            "structure.json",
            r#"{"type":"title","identifier":"34","children":[
                 {"type":"part","identifier":"99","label_description":"Family Educational Rights and Privacy","reserved":false},
                 {"type":"part","identifier":"668","label_description":"Student Assistance General Provisions","reserved":false},
                 {"type":"part","identifier":"669","label_description":"Reserved","reserved":true}]}"#,
        );
        upstream
    }

    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.root.join(name), body).unwrap();
    }

    /// One ledger page. `rows` is `(identifier, part, amendment_date, substantive)`.
    fn ledger(&self, latest: &str, rows: &[(&str, &str, &str, bool)]) {
        let entries: Vec<String> = rows
            .iter()
            .map(|(id, part, date, substantive)| {
                format!(
                    r#"{{"identifier":"{id}","name":"§ {id}   Heading.","part":"{part}",
                         "subpart":null,"amendment_date":"{date}","issue_date":"{date}",
                         "substantive":{substantive},"removed":false,"type":"section"}}"#
                )
            })
            .collect();
        self.write(
            "ledger-1.json",
            &format!(
                r#"{{"meta":{{"title":"34","latest_amendment_date":"{latest}",
                     "latest_issue_date":"{latest}","total_pages":"1"}},
                     "content_versions":[{}]}}"#,
                entries.join(",")
            ),
        );
    }

    fn part(&self, number: &str, heading: &str, body: &str) {
        self.write(
            &format!("part-{number}.xml"),
            &format!(
                r#"<?xml version="1.0"?>
<DIV5 N="{number}" TYPE="PART"><HEAD>PART {number}&#x2014;{heading}</HEAD>
<AUTH><HED>Authority:</HED><PSPACE>20 U.S.C. 1094.</PSPACE></AUTH>
{body}
</DIV5>"#
            ),
        );
    }

    fn fetcher(&self) -> Fetcher {
        Fetcher::dir(self.root.clone())
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn section(id: &str, heading: &str, paragraphs: &str) -> String {
    format!(
        r#"<DIV8 N="{id}" TYPE="SECTION" hierarchy_metadata="{{&quot;citation&quot;:&quot;34 CFR {id}&quot;}}">
<HEAD>&#xA7; {id} {heading}</HEAD>
{paragraphs}
</DIV8>"#
    )
}

/// A workspace with a keystore and a registry, as `keygen` produces.
struct Repo {
    root: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("doe-aion-repo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let repo = Self { root };
        chain::keygen(1, 1, Some(&repo.keystore()), &repo.registry()).unwrap();
        repo
    }

    fn chain(&self) -> PathBuf {
        self.root.join("doe.aion")
    }
    fn registry(&self) -> PathBuf {
        self.root.join("registry.json")
    }
    fn keystore(&self) -> PathBuf {
        self.root.join(".keys")
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn signer(&self) -> chain::Signer {
        chain::Signer {
            author: 1,
            key: 1,
            keystore_dir: Some(self.keystore()),
            secret_hex: None,
        }
    }

    /// Build, compare, write `data/`, and sign — the body of `sync`.
    fn sync(&self, fetcher: &Fetcher, date: Option<&str>) -> (Severity, bool) {
        let built = plan::build(fetcher, date).unwrap();
        let previous = chain::previous_bundle(&self.chain()).unwrap();
        let changes = plan::compare(&built.bundle, previous.as_ref());
        if changes.is_empty() {
            return (changes.severity, false);
        }
        plan::write_data(&built, &self.data()).unwrap();
        let registry = chain::load_registry(&self.registry()).unwrap();
        chain::commit(
            &self.chain(),
            &built.bundle,
            "test",
            &self.signer(),
            &registry,
        )
        .unwrap();
        (changes.severity, true)
    }

    fn verify(&self) -> bool {
        let registry = chain::load_registry(&self.registry()).unwrap();
        chain::verify(&self.chain(), &registry).unwrap().is_valid
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn quiet_upstream(name: &str) -> Upstream {
    let upstream = Upstream::new(name, "2026-08-21");
    upstream.ledger(
        "2026-07-20",
        &[
            ("99.3", "99", "2026-01-15", true),
            ("668.14", "668", "2026-07-20", true),
        ],
    );
    upstream.part(
        "99",
        "Family Educational Rights and Privacy",
        &section(
            "99.3",
            "What definitions apply?",
            "<P>An educational agency must provide access to education records.</P>",
        ),
    );
    upstream.part(
        "668",
        "Student Assistance General Provisions",
        &section(
            "668.14",
            "Program participation agreement.",
            "<P>(a)(1) An institution may participate only if it enters into an agreement.</P>\n\
             <P>(2) The agreement must be signed by&#x2014;</P>\n\
             <P>(i) An authorized representative; and</P>\n\
             <P>(ii) An owner.</P>",
        ),
    );
    upstream
}

#[test]
fn genesis_signs_the_title_and_verifies() {
    let upstream = quiet_upstream("genesis");
    let repo = Repo::new("genesis");

    let (severity, committed) = repo.sync(&upstream.fetcher(), None);
    assert!(committed);
    assert_eq!(severity, Severity::Major, "genesis is never quiet");
    assert!(repo.verify());

    let bundle = chain::previous_bundle(&repo.chain()).unwrap().unwrap();
    assert_eq!(bundle.pinned_date, "2026-08-21");
    assert_eq!(bundle.totals.parts, 2, "the reserved part is not fetched");
    assert_eq!(bundle.totals.sections, 2);
    assert!(bundle.parts.contains_key("668"));
    assert!(!bundle.parts.contains_key("669"));
}

#[test]
fn a_rerun_against_the_same_pin_commits_nothing() {
    let upstream = quiet_upstream("idempotent");
    let repo = Repo::new("idempotent");

    assert!(repo.sync(&upstream.fetcher(), None).1);
    let after_genesis = std::fs::read(repo.chain()).unwrap();

    let (_, committed) = repo.sync(&upstream.fetcher(), None);
    assert!(!committed, "nothing moved, so nothing may be signed");
    assert_eq!(
        std::fs::read(repo.chain()).unwrap(),
        after_genesis,
        "a no-op run must not touch the artifact at all"
    );
}

#[test]
fn a_substantive_amendment_is_detected_signed_and_named() {
    let upstream = quiet_upstream("amended");
    let repo = Repo::new("amended");
    assert!(repo.sync(&upstream.fetcher(), None).1);

    // Upstream amends 668.14: the ledger moves and so does the text.
    upstream.ledger(
        "2026-09-02",
        &[
            ("99.3", "99", "2026-01-15", true),
            ("668.14", "668", "2026-09-02", true),
        ],
    );
    upstream.part(
        "668",
        "Student Assistance General Provisions",
        &section(
            "668.14",
            "Program participation agreement.",
            "<P>(a)(1) An institution may participate only if it enters into an agreement.</P>\n\
             <P>(2) The agreement must be signed by&#x2014;</P>\n\
             <P>(i) An authorized representative; and</P>\n\
             <P>(ii) An owner and a chief financial officer.</P>",
        ),
    );

    let built = plan::build(&upstream.fetcher(), None).unwrap();
    let previous = chain::previous_bundle(&repo.chain()).unwrap();
    let changes = plan::compare(&built.bundle, previous.as_ref());

    assert_eq!(changes.severity, Severity::Major);
    assert_eq!(changes.parts_moved, vec!["668"], "part 99 did not move");
    assert_eq!(changes.ledger.amended.len(), 1);
    assert_eq!(changes.ledger.amended[0].row.identifier, "668.14");
    assert_eq!(changes.ledger.amended[0].from, "2026-07-20");

    let report = doe_aion::report::markdown(&built, &changes, previous.as_ref());
    assert!(report.contains("34 CFR 668.14"));
    assert!(report.contains("2026-07-20 \u{2192} 2026-09-02"));
}

#[test]
fn a_technical_amendment_does_not_shout() {
    let upstream = quiet_upstream("technical");
    let repo = Repo::new("technical");
    assert!(repo.sync(&upstream.fetcher(), None).1);

    upstream.ledger(
        "2026-09-02",
        &[
            ("99.3", "99", "2026-09-02", false),
            ("668.14", "668", "2026-07-20", true),
        ],
    );
    let built = plan::build(&upstream.fetcher(), None).unwrap();
    let previous = chain::previous_bundle(&repo.chain()).unwrap();
    let changes = plan::compare(&built.bundle, previous.as_ref());
    assert_eq!(
        changes.severity,
        Severity::Minor,
        "eCFR called it non-substantive, so the report must not call it major"
    );
}

#[test]
fn text_edited_without_a_ledger_entry_is_still_caught() {
    let upstream = quiet_upstream("silent");
    let repo = Repo::new("silent");
    assert!(repo.sync(&upstream.fetcher(), None).1);

    // Same ledger, different text — a correction, or an upstream slip.
    upstream.part(
        "99",
        "Family Educational Rights and Privacy",
        &section(
            "99.3",
            "What definitions apply?",
            "<P>An educational agency may not release education records.</P>",
        ),
    );
    let built = plan::build(&upstream.fetcher(), None).unwrap();
    let previous = chain::previous_bundle(&repo.chain()).unwrap();
    let changes = plan::compare(&built.bundle, previous.as_ref());
    assert!(!changes.is_empty(), "a silent edit must not pass the gate");
    assert_eq!(changes.parts_moved, vec!["99"]);
}

#[test]
fn a_pin_past_what_ecfr_is_current_through_is_refused() {
    let upstream = quiet_upstream("future");
    let error = plan::build(&upstream.fetcher(), Some("2026-12-01"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("only current through"),
        "unexpected error: {error}"
    );
}

#[test]
fn an_html_error_body_aborts_instead_of_looking_like_a_removal() {
    let upstream = quiet_upstream("html");
    upstream.write("part-668.xml", "<html><body>502 Bad Gateway</body></html>");
    let error = plan::build(&upstream.fetcher(), None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("668"),
        "the failure must name the part: {error}"
    );
}

#[test]
fn an_empty_ledger_is_never_read_as_everything_removed() {
    let upstream = quiet_upstream("empty");
    upstream.ledger("2026-07-20", &[]);
    assert!(plan::build(&upstream.fetcher(), None).is_err());
}

#[test]
fn a_ledger_part_missing_from_the_title_is_reported_not_fetched() {
    let upstream = quiet_upstream("retired");
    upstream.ledger(
        "2026-07-20",
        &[
            ("99.3", "99", "2026-01-15", true),
            ("668.14", "668", "2026-07-20", true),
            // 34 CFR 230 was removed years ago; its ledger rows survive.
            ("230.1", "230", "2019-05-01", true),
        ],
    );
    let built = plan::build(&upstream.fetcher(), None).unwrap();
    assert_eq!(built.retired_parts, vec!["230"]);
    assert!(!built.bundle.parts.contains_key("230"));
}

#[test]
fn editing_data_after_signing_fails_verification() {
    let upstream = quiet_upstream("tamper");
    let repo = Repo::new("tamper");
    assert!(repo.sync(&upstream.fetcher(), None).1);

    let bundle = chain::previous_bundle(&repo.chain()).unwrap().unwrap();
    assert!(chain::data_matches(&bundle, &repo.data())
        .unwrap()
        .is_empty());

    let target = repo.data().join("part-668.json");
    let edited = std::fs::read_to_string(&target)
        .unwrap()
        .replace("An owner", "Anyone at all");
    std::fs::write(&target, edited).unwrap();

    let mismatches = chain::data_matches(&bundle, &repo.data()).unwrap();
    assert_eq!(mismatches.len(), 1);
    assert!(mismatches[0].contains("does not match the signed"));
    assert!(
        repo.verify(),
        "the chain itself is intact; it is data/ that drifted, and that is the \
         distinction the two checks exist to draw"
    );
}

#[test]
fn a_removed_part_is_deleted_from_data_rather_than_left_behind() {
    let upstream = quiet_upstream("removed-part");
    let repo = Repo::new("removed-part");
    assert!(repo.sync(&upstream.fetcher(), None).1);
    assert!(repo.data().join("part-99.json").exists());

    upstream.write(
        "structure.json",
        r#"{"type":"title","identifier":"34","children":[
             {"type":"part","identifier":"668","label_description":"Student Assistance General Provisions","reserved":false}]}"#,
    );
    let built = plan::build(&upstream.fetcher(), None).unwrap();
    plan::write_data(&built, &repo.data()).unwrap();
    assert!(
        !repo.data().join("part-99.json").exists(),
        "a committed tree must not claim a regulation the title no longer has"
    );
}

#[test]
fn the_payload_carries_what_the_derivation_could_not_settle() {
    let upstream = Upstream::new("coverage", "2026-08-21");
    upstream.ledger("2026-07-20", &[("668.14", "668", "2026-07-20", true)]);
    upstream.write(
        "structure.json",
        r#"{"type":"title","identifier":"34","children":[
             {"type":"part","identifier":"668","label_description":"Student Assistance","reserved":false}]}"#,
    );
    // (a) then (c): a designator the numbering cannot account for.
    upstream.part(
        "668",
        "Student Assistance General Provisions",
        &section(
            "668.14",
            "Program participation agreement.",
            "<P>(a) An institution must apply.</P>\n<P>(c) An institution must report.</P>",
        ),
    );
    let built = plan::build(&upstream.fetcher(), None).unwrap();
    assert_eq!(
        built.bundle.totals.unresolved, 1,
        "the signature must cover the parse's own limits"
    );
    assert_eq!(built.bundle.totals.atoms, 2);
    assert_eq!(built.bundle.totals.binding, 2);
}

#[test]
fn the_signed_bytes_do_not_depend_on_when_the_run_happened() {
    let upstream = quiet_upstream("determinism");
    let one = plan::build(&upstream.fetcher(), None).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let two = plan::build(&upstream.fetcher(), None).unwrap();
    assert_eq!(
        one.bundle.to_bytes().unwrap(),
        two.bundle.to_bytes().unwrap(),
        "a wall-clock value in the payload would destroy idempotency"
    );
    assert_eq!(one.bundle.pinned_timestamp(), two.bundle.pinned_timestamp());
}

#[test]
fn the_bundle_depends_on_the_pin_and_not_on_the_path_taken_to_it() {
    // Arriving at a date through an intermediate version must produce the same
    // signed bundle as pinning it directly, or the artifact would say something
    // different depending on when the watcher happened to run.
    let upstream = quiet_upstream("path");

    let direct = plan::build(&upstream.fetcher(), None).unwrap();

    let stepwise = Repo::new("path");
    upstream.ledger("2026-05-01", &[("668.14", "668", "2026-05-01", true)]);
    assert!(stepwise.sync(&upstream.fetcher(), None).1);
    upstream.ledger(
        "2026-07-20",
        &[
            ("99.3", "99", "2026-01-15", true),
            ("668.14", "668", "2026-07-20", true),
        ],
    );
    assert!(stepwise.sync(&upstream.fetcher(), None).1);

    let arrived = chain::previous_bundle(&stepwise.chain()).unwrap().unwrap();
    assert_eq!(
        arrived.digest().unwrap(),
        direct.bundle.digest().unwrap(),
        "two paths to the same pin must sign identical bytes"
    );
}

#[test]
fn data_written_by_the_run_matches_the_payload_it_signed() {
    let upstream = quiet_upstream("agreement");
    let repo = Repo::new("agreement");
    assert!(repo.sync(&upstream.fetcher(), None).1);
    let bundle = chain::previous_bundle(&repo.chain()).unwrap().unwrap();
    assert!(
        chain::data_matches(&bundle, &repo.data())
            .unwrap()
            .is_empty(),
        "the chain proves the payload; data/ is what a human reads, and they must agree"
    );
    assert!(Path::new(&repo.data().join("ledger.json")).exists());
}
