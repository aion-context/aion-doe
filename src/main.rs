use anyhow::Result;
use clap::{Parser, Subcommand};

use doe_aion::cfr::{self, Fetcher};
use std::io::Write as _;

use doe_aion::{chain, obligations, plan, report};

#[derive(Parser)]
#[command(
    name = "doe-aion",
    about = "Signed change detection for 34 CFR",
    version
)]
struct Cli {
    /// Replay captured snapshots instead of fetching.
    #[arg(long, global = true, value_name = "DIR")]
    from_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a signing identity and pin its public half in the registry.
    Keygen {
        #[arg(long, default_value_t = 1)]
        key: u64,
        #[arg(long, default_value_t = 1)]
        author: u64,
        #[arg(long, default_value = ".keys")]
        keystore: std::path::PathBuf,
        #[arg(long, default_value = "registry.json")]
        registry: std::path::PathBuf,
        /// Print the seed on stdout, for a CI secret.
        #[arg(long)]
        print_secret: bool,
    },
    /// Print the seed of the key the registry pins, on stdout only.
    Secret {
        #[arg(long, default_value_t = 1)]
        key: u64,
        #[arg(long, default_value_t = 1)]
        author: u64,
        #[arg(long, default_value = ".keys")]
        keystore: std::path::PathBuf,
        #[arg(long, default_value = "registry.json")]
        registry: std::path::PathBuf,
    },
    /// What would change. Read-only; writes nothing.
    Plan {
        #[arg(long)]
        date: Option<String>,
        #[arg(long, default_value = "doe.aion")]
        chain: std::path::PathBuf,
        /// Re-read every part instead of only those the ledger says moved.
        #[arg(long)]
        full: bool,
    },
    /// Fetch, diff, sign a new chain version, and verify it.
    Sync {
        #[arg(long)]
        date: Option<String>,
        #[arg(long, default_value = "doe.aion")]
        chain: std::path::PathBuf,
        #[arg(long, default_value = "registry.json")]
        registry: std::path::PathBuf,
        #[arg(long, default_value = "data")]
        data: std::path::PathBuf,
        #[arg(long, default_value_t = 1)]
        author: u64,
        #[arg(long, default_value_t = 1)]
        key: u64,
        #[arg(long, default_value = ".keys")]
        keystore: std::path::PathBuf,
        /// Hex Ed25519 seed, for CI. Nothing key-shaped touches the runner's disk.
        #[arg(long, env = "AION_SIGNING_KEY", hide_env_values = true)]
        signing_key: Option<String>,
        /// Where to write the human report.
        #[arg(long)]
        report: Option<std::path::PathBuf>,
        /// Append `key=value` lines for a CI step, e.g. `$GITHUB_OUTPUT`.
        #[arg(long)]
        outputs: Option<std::path::PathBuf>,
        /// Sign a version even when nothing moved.
        #[arg(long)]
        force: bool,
        /// Re-read every part instead of only those the ledger says moved.
        /// Catches a text change eCFR did not record in the ledger.
        #[arg(long)]
        full: bool,
    },
    /// Check the chain, and that `data/` matches what was signed.
    Verify {
        #[arg(long, default_value = "doe.aion")]
        chain: std::path::PathBuf,
        #[arg(long, default_value = "registry.json")]
        registry: std::path::PathBuf,
        #[arg(long, default_value = "data")]
        data: std::path::PathBuf,
    },
    /// What eCFR says about Title 34 right now.
    Status,
    /// Summarise the amendment ledger.
    Ledger {
        /// Only sections amended on or after this date.
        #[arg(long)]
        since: Option<String>,
    },
    /// Parse one part and report its structure.
    Part {
        part: String,
        #[arg(long)]
        date: Option<String>,
    },
    /// Download the ledger and every part to a directory, for offline replay.
    Capture {
        #[arg(long, default_value = "snapshots")]
        out: std::path::PathBuf,
        #[arg(long)]
        date: Option<String>,
    },
    /// Parse every part and report structural coverage across the title.
    Sweep {
        #[arg(long)]
        date: Option<String>,
        /// Show the worst parts by unresolved designator sequences.
        #[arg(long, default_value_t = 10)]
        worst: usize,
    },
    /// Derive obligation atoms from one part.
    Obligations {
        part: String,
        #[arg(long)]
        date: Option<String>,
        #[arg(long)]
        bearer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

/// Counts are in the tens of thousands, far inside f64's exact range, and the
/// dispatch is one arm per subcommand rather than logic worth splitting up.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn main() -> Result<()> {
    let cli = Cli::parse();
    let fetcher = match &cli.from_dir {
        Some(dir) => Fetcher::dir(dir.clone()),
        None => Fetcher::Http,
    };

    match cli.command {
        Command::Keygen {
            key,
            author,
            keystore,
            registry,
            print_secret,
        } => {
            let seed = chain::keygen(key, author, Some(&keystore), &registry)?;
            eprintln!(
                "key {key} generated for author {author}; public half pinned in {}",
                registry.display()
            );
            eprintln!(
                "commit {} so the chain can be verified.",
                registry.display()
            );
            if print_secret {
                println!("{seed}");
            }
        }
        Command::Secret {
            key,
            author,
            keystore,
            registry,
        } => {
            // stdout carries only the seed, so this pipes without displaying.
            let seed = chain::reveal_secret(key, author, Some(keystore), &registry)?;
            println!("{seed}");
        }
        Command::Plan {
            date,
            chain: path,
            full,
        } => {
            let previous = chain::previous_bundle(&path)?;
            let built = plan::build(
                &fetcher,
                date.as_deref(),
                previous.as_ref(),
                read_mode(full),
            )?;
            let changes = plan::compare(&built.bundle, previous.as_ref());
            println!("{}", report::headline(&changes, &built.bundle));
            println!();
            println!("{}", report::markdown(&built, &changes, previous.as_ref()));
            if changes.is_empty() {
                println!("\nNothing moved. A sync would commit nothing.");
            }
        }
        Command::Sync {
            date,
            chain: path,
            registry,
            data,
            author,
            key,
            keystore,
            signing_key,
            report: report_path,
            outputs,
            force,
            full,
        } => {
            let previous = chain::previous_bundle(&path)?;
            let built = plan::build(
                &fetcher,
                date.as_deref(),
                previous.as_ref(),
                read_mode(full),
            )?;
            let changes = plan::compare(&built.bundle, previous.as_ref());
            let text = report::markdown(&built, &changes, previous.as_ref());
            if let Some(report_path) = &report_path {
                if let Some(parent) = report_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(report_path, &text)?;
            }
            let digest = built.bundle.digest()?;
            let message = report::headline(&changes, &built.bundle);
            let emit = |committed: bool| -> Result<()> {
                let Some(path) = &outputs else { return Ok(()) };
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                writeln!(file, "committed={committed}")?;
                writeln!(file, "severity={}", changes.severity)?;
                writeln!(file, "bundle_sha256={digest}")?;
                writeln!(file, "pinned_date={}", built.bundle.pinned_date)?;
                // Upstream text never reaches a shell through interpolation.
                writeln!(file, "headline<<AION_EOF\n{message}\nAION_EOF")?;
                Ok(())
            };
            if changes.is_empty() && !force {
                emit(false)?;
                println!("nothing moved at {} — no commit", built.bundle.pinned_date);
                return Ok(());
            }
            plan::write_data(&built, &data)?;
            let registry_keys = chain::load_registry(&registry)?;
            let signer = chain::Signer {
                author,
                key,
                keystore_dir: Some(keystore),
                secret_hex: signing_key,
            };
            let version = chain::commit(&path, &built.bundle, &message, &signer, &registry_keys)?;
            let mismatches = chain::data_matches(&built.bundle, &data)?;
            anyhow::ensure!(
                mismatches.is_empty(),
                "the signed payload and {} disagree: {}",
                data.display(),
                mismatches.join("; ")
            );
            emit(true)?;
            println!("{message}");
            println!(
                "signed version {version} of {} \u{2014} bundle {}",
                path.display(),
                &digest[..16]
            );
        }
        Command::Verify {
            chain: path,
            registry,
            data,
        } => {
            let registry_keys = chain::load_registry(&registry)?;
            let verification = chain::verify(&path, &registry_keys)?;
            anyhow::ensure!(
                verification.is_valid,
                "chain {} is not valid: {:?}",
                path.display(),
                verification.errors
            );
            let bundle = chain::previous_bundle(&path)?
                .ok_or_else(|| anyhow::anyhow!("{} has no payload", path.display()))?;
            let mismatches = chain::data_matches(&bundle, &data)?;
            anyhow::ensure!(
                mismatches.is_empty(),
                "{} does not match the signed payload: {}",
                data.display(),
                mismatches.join("; ")
            );
            println!(
                "{} verified \u{2014} 34 CFR as of {}, {} parts, {} sections, bundle {}",
                path.display(),
                bundle.pinned_date,
                bundle.totals.parts,
                bundle.totals.sections,
                &bundle.digest()?[..16]
            );
        }
        Command::Capture { out, date } => {
            let date = resolve_date(&fetcher, date)?;
            std::fs::create_dir_all(&out)?;
            let raw = cfr::raw_titles(&fetcher)?;
            std::fs::write(out.join("titles.json"), &raw)?;
            let pages = cfr::capture_ledger(&fetcher, &out, &date)?;
            std::fs::write(
                out.join("structure.json"),
                cfr::raw_structure(&fetcher, &date)?,
            )?;
            let parts: Vec<String> = cfr::structure_parts(&fetcher, &date)?
                .into_iter()
                .filter(|p| !p.reserved && !p.number.is_empty())
                .map(|p| p.number)
                .collect();
            println!(
                "ledger: {pages} pages; {} live parts, as of {date}",
                parts.len()
            );
            for (index, part) in parts.iter().enumerate() {
                let xml = cfr::part_xml(&fetcher, &date, part)?;
                std::fs::write(out.join(format!("part-{part}.xml")), &xml)?;
                println!(
                    "  [{:>3}/{}] part {part:<6} {:>9} bytes",
                    index + 1,
                    parts.len(),
                    xml.len()
                );
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            std::fs::write(out.join("pinned-date"), &date)?;
            println!("captured to {}", out.display());
        }
        Command::Sweep { date, worst } => {
            let date = resolve_date(&fetcher, date)?;
            let parts: Vec<String> = cfr::structure_parts(&fetcher, &date)?
                .into_iter()
                .filter(|p| !p.reserved && !p.number.is_empty())
                .map(|p| p.number)
                .collect();
            let mut totals = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
            let mut failures = Vec::new();
            let mut ranked = Vec::new();
            for part in &parts {
                let parsed = match cfr::fetch_part(&fetcher, &date, part) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        failures.push(format!("part {part}: {e}"));
                        continue;
                    }
                };
                let paragraphs: usize = parsed.sections.iter().map(|s| s.paragraphs.len()).sum();
                let irregular: usize = parsed.sections.iter().map(|s| s.irregularities.len()).sum();
                let pending: usize = parsed.sections.iter().map(|s| s.pending.len()).sum();
                let (atoms, coverage) = obligations::extract(&parsed);
                totals.0 += parsed.sections.len();
                totals.1 += paragraphs;
                totals.2 += irregular;
                totals.3 += atoms.len();
                totals.4 += coverage.unclassified;
                totals.5 += pending;
                totals.6 += atoms.iter().filter(|a| a.force.binding()).count();
                ranked.push((irregular, paragraphs, part.clone()));
            }
            println!("34 CFR, all {} parts, as of {date}", parts.len());
            println!("  sections            : {}", totals.0);
            println!("  paragraphs          : {}", totals.1);
            println!(
                "  unresolved sequences: {} ({:.2}%)",
                totals.2,
                100.0 * totals.2 as f64 / totals.1.max(1) as f64
            );
            println!("  pending amendments  : {}", totals.5);
            println!(
                "  obligation atoms    : {} ({} binding)",
                totals.3, totals.6
            );
            println!(
                "  unclassified bearer : {} ({:.1}%)",
                totals.4,
                100.0 * totals.4 as f64 / totals.3.max(1) as f64
            );
            ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            println!("\n  worst parts by unresolved sequences:");
            for (irregular, paragraphs, part) in ranked.iter().take(worst) {
                if *irregular == 0 {
                    break;
                }
                println!(
                    "    part {part:<6} {irregular:>4} / {paragraphs:<6} ({:.1}%)",
                    100.0 * *irregular as f64 / (*paragraphs).max(1) as f64
                );
            }
            if !failures.is_empty() {
                println!("\n  parse failures:");
                for failure in &failures {
                    println!("    {failure}");
                }
                anyhow::bail!("{} parts failed to parse", failures.len());
            }
        }
        Command::Status => {
            let status = cfr::title_status(&fetcher)?;
            println!("title 34 — Education");
            println!("  up to date as of : {}", status.up_to_date_as_of);
            println!("  latest amendment : {}", status.latest_amended_on);
            println!("  latest issue     : {}", status.latest_issue_date);
            if status.import_in_progress {
                println!("  import in progress — the ledger may move again shortly");
            }
        }
        Command::Ledger { since } => {
            let ledger = cfr::fetch_ledger(&fetcher, &resolve_date(&fetcher, None)?)?;
            let parts = ledger.parts();
            println!(
                "{} sections and appendices across {} parts",
                ledger.rows.len(),
                parts.len()
            );
            println!("  latest amendment : {}", ledger.latest_amendment_date);
            let removed = ledger.rows.values().filter(|r| r.removed).count();
            let technical = ledger.rows.values().filter(|r| !r.substantive).count();
            println!("  removed          : {removed}");
            println!("  non-substantive  : {technical}");
            if let Some(since) = since {
                let mut moved: Vec<_> = ledger
                    .rows
                    .values()
                    .filter(|r| r.amendment_date >= since)
                    .collect();
                moved.sort_by(|a, b| b.amendment_date.cmp(&a.amendment_date));
                println!("\n{} amended on or after {since}:", moved.len());
                for row in moved.iter().take(40) {
                    println!(
                        "  {} {:>28}  {}{}",
                        row.amendment_date,
                        row.identifier,
                        if row.substantive { "" } else { "[technical] " },
                        if row.removed { "[removed] " } else { "" }
                    );
                }
            }
        }
        Command::Part { part, date } => {
            let date = resolve_date(&fetcher, date)?;
            let parsed = cfr::fetch_part(&fetcher, &date, &part)?;
            let paragraphs: usize = parsed.sections.iter().map(|s| s.paragraphs.len()).sum();
            let pending: usize = parsed.sections.iter().map(|s| s.pending.len()).sum();
            let irregular: usize = parsed.sections.iter().map(|s| s.irregularities.len()).sum();
            println!("34 CFR part {} — {}", parsed.number, parsed.heading);
            println!("  as of        : {date}");
            println!("  sections     : {}", parsed.sections.len());
            println!("  paragraphs   : {paragraphs}");
            println!("  pending amdt : {pending}");
            println!("  irregular    : {irregular}");
            if let Some(authority) = &parsed.authority {
                println!("  authority    : {authority}");
            }
            for section in &parsed.sections {
                for note in section.irregularities.iter().take(3) {
                    println!("  ! {note}");
                }
            }
            for section in parsed.sections.iter().take(6) {
                println!(
                    "  \u{a7} {:<12} {:>4} para  {}",
                    section.identifier,
                    section.paragraphs.len(),
                    section.heading
                );
            }
        }
        Command::Obligations {
            part,
            date,
            bearer,
            json,
            limit,
        } => {
            let date = resolve_date(&fetcher, date)?;
            let parsed = cfr::fetch_part(&fetcher, &date, &part)?;
            let (mut atoms, coverage) = obligations::extract(&parsed);
            if let Some(bearer) = &bearer {
                let wanted = bearer.to_lowercase();
                atoms.retain(|a| a.bearer.label().to_lowercase().contains(&wanted));
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&atoms)?);
                return Ok(());
            }
            println!(
                "34 CFR part {} — {} (as of {date})",
                parsed.number, parsed.heading
            );
            println!(
                "  {} paragraphs \u{2192} {} atoms ({} inherited, {} definitions skipped, {} unclassified bearer)",
                coverage.paragraphs,
                coverage.atoms,
                coverage.inherited,
                coverage.definitions_skipped,
                coverage.unclassified
            );
            println!("  force  : {:?}", coverage.by_force);
            println!("  bearer : {:?}", coverage.by_bearer);
            println!();
            for atom in atoms.iter().take(limit) {
                println!(
                    "  {:<26} {:<9} {:<22} {}",
                    atom.citation,
                    atom.force.label(),
                    atom.bearer.label(),
                    truncate(&atom.text, 88)
                );
            }
            if atoms.len() > limit {
                println!("  … {} more", atoms.len() - limit);
            }
        }
    }
    Ok(())
}

fn read_mode(full: bool) -> plan::Read {
    if full {
        plan::Read::Full
    } else {
        plan::Read::Incremental
    }
}

fn resolve_date(fetcher: &Fetcher, date: Option<String>) -> Result<String> {
    let status = cfr::title_status(fetcher)?;
    let date = date.unwrap_or_else(|| status.up_to_date_as_of.clone());
    cfr::valid_pin(&date, &status.up_to_date_as_of)?;
    Ok(date)
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    format!("{}\u{2026}", text.chars().take(width).collect::<String>())
}
