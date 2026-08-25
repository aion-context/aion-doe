//! eCFR part XML → sections → the paragraph tree.
//!
//! The document is shallow: a `DIV5` part holds `DIV6` subparts holding `DIV8`
//! sections, and a section's body is a flat run of `<P>`. Everything
//! structural below the section is reconstructed in [`super::paragraph`].
//!
//! Three elements carry more than prose and are lifted out:
//!
//! - `AUTH` — the statutory authority the part rests on, which is the join to
//!   the U.S. Code source.
//! - `CITA` — the Federal Register source credit, which is the join back to
//!   the notice that produced the text.
//! - `XREF … AMDINSN` — an amendment eCFR has published but not yet folded in.
//!   That is a change that has already happened upstream and is not visible in
//!   the text, so it is captured rather than skipped.

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

use super::paragraph::{
    is_lead_in, parse_lead, strip_markers, Stack, Step, ITALIC_CLOSE, ITALIC_OPEN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Section,
    Appendix,
}

/// A single designated paragraph, addressed the way a regulation is cited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraph {
    /// Designator path, e.g. `["b", "1"]`. Empty for undesignated text.
    pub path: Vec<String>,
    /// `34 CFR 668.14(b)(1)`.
    pub citation: String,
    /// The italic paragraph heading, where the drafting gives one.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub heading: Option<String>,
    /// The term this paragraph defines, for a definitions list.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub term: Option<String>,
    pub text: String,
}

/// An amendment eCFR has published but has not yet incorporated into the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// `91 FR 40281`.
    pub fr_citation: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    /// `668.14`, or `Appendix A to Subpart B of Part 668`.
    pub identifier: String,
    pub citation: String,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpart: Option<String>,
    pub heading: String,
    pub paragraphs: Vec<Paragraph>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub pending: Vec<Pending>,
    /// The Federal Register credit line, e.g. `[52 FR 45724, Dec. 1, 1987, as
    /// amended at …]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    /// Designator sequences that did not fit the numbering systems.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub irregularities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub number: String,
    pub heading: String,
    /// Raw authority note, e.g. `20 U.S.C. 1001-1003, 1070g, …`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub sections: Vec<Section>,
}

impl Part {
    pub fn section(&self, identifier: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.identifier == identifier)
    }
}

/// Elements whose character data is collected rather than ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    Paragraph,
    Head,
    Spaced,
    Credit,
    SectionAuth,
    Amendment,
}

fn sink_for(name: &[u8]) -> Option<Sink> {
    match name {
        b"P" | b"FP" => Some(Sink::Paragraph),
        b"HEAD" => Some(Sink::Head),
        b"PSPACE" => Some(Sink::Spaced),
        b"CITA" => Some(Sink::Credit),
        b"SECAUTH" | b"PARAUTH" => Some(Sink::SectionAuth),
        b"XREF" => Some(Sink::Amendment),
        _ => None,
    }
}

/// Which container a `HEAD` belongs to. Set only by the DIV elements, so a
/// nested `AUTH` cannot leave the reader believing it is back at part level —
/// which is how a subpart heading was overwriting the part heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Part,
    Subpart,
    Section,
    None,
}

/// A note block whose `PSPACE` is a citation rather than regulatory text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Note {
    Authority,
    Source,
}

struct Builder {
    part: Part,
    owner: Owner,
    subpart: Option<String>,
    open: Option<Section>,
    stack: Stack,
    /// The section's own numbering, parked while a definition's numbering
    /// runs. A definitions list restarts at `(i)` for every term, so the two
    /// sequences cannot share one stack.
    outer: Option<Stack>,
    /// Designator path in force when the definitions list opened.
    term_prefix: Vec<String>,
    term: Option<String>,
    /// Whether the previous paragraph introduced a list.
    lead_in: bool,
    sink: Option<Sink>,
    note: Option<Note>,
    buffer: String,
    amendment_pending: bool,
}

impl Builder {
    fn new() -> Self {
        Self {
            part: Part {
                number: String::new(),
                heading: String::new(),
                authority: None,
                source: None,
                sections: Vec::new(),
            },
            owner: Owner::None,
            subpart: None,
            open: None,
            stack: Stack::new(),
            outer: None,
            term_prefix: Vec::new(),
            term: None,
            lead_in: false,
            sink: None,
            note: None,
            buffer: String::new(),
            amendment_pending: false,
        }
    }

    fn close_section(&mut self) {
        if let Some(section) = self.open.take() {
            self.part.sections.push(section);
        }
        self.stack = Stack::new();
        self.outer = None;
        self.term_prefix.clear();
        self.term = None;
        self.lead_in = false;
    }

    fn flush(&mut self) {
        let Some(sink) = self.sink.take() else { return };
        let text = collapse(&std::mem::take(&mut self.buffer));
        if text.is_empty() {
            return;
        }
        match sink {
            Sink::Paragraph => self.push_paragraph(&text),
            Sink::Head => match self.owner {
                // The part heading is written once. A later `HEAD` at part
                // level is a subpart's, and must not overwrite it.
                Owner::Part if self.part.heading.is_empty() => {
                    self.part.heading = strip_part_prefix(&text);
                }
                Owner::Section => {
                    if let Some(section) = self.open.as_mut() {
                        section.heading = strip_section_prefix(&text);
                    }
                }
                _ => {}
            },
            Sink::Spaced => match (self.note, self.open.as_mut()) {
                // A part carries one authority note; subpart-level notes
                // attach to their sections, never back to the part.
                (Some(Note::Authority), None) if self.part.authority.is_none() => {
                    self.part.authority = Some(text);
                }
                (Some(Note::Source), None) if self.part.source.is_none() => {
                    self.part.source = Some(text);
                }
                (Some(Note::Authority), Some(section)) => section.authority = Some(text),
                (Some(_), _) => {}
                (None, _) => self.push_paragraph(&text),
            },
            Sink::Credit => {
                if let Some(section) = self.open.as_mut() {
                    section.credit = Some(text.trim_matches(['[', ']']).to_string());
                }
            }
            Sink::SectionAuth => {
                if let Some(section) = self.open.as_mut() {
                    section.authority = Some(text);
                }
            }
            Sink::Amendment => {
                if self.amendment_pending {
                    if let Some(section) = self.open.as_mut() {
                        section.pending.push(Pending {
                            fr_citation: fr_citation(&text).unwrap_or_default(),
                            note: text,
                        });
                    }
                }
            }
        }
        self.amendment_pending = false;
    }

    fn push_paragraph(&mut self, text: &str) {
        if self.open.is_none() {
            return;
        }
        let (lead, body) = parse_lead(text);

        if let Some(term) = &lead.term {
            // A new defined term restarts numbering. The section's own stack is
            // parked on the first term and restored when a designator fits it
            // again, which is how the list ends without a marker saying so.
            if self.outer.is_none() {
                self.outer = Some(self.stack.clone());
                self.term_prefix = self.stack.path();
            }
            self.stack = Stack::new();
            self.term = Some(term.clone());
        }

        let mut irregular = Vec::new();
        let mut descend = self.lead_in && lead.term.is_none();
        for token in &lead.designators {
            let step = if descend {
                self.stack.push_after_lead_in(token)
            } else {
                self.stack.push(token)
            };
            // Only the first designator of a run descends; the rest nest
            // normally beneath it.
            descend = false;
            if let Step::Irregular { reason, .. } = step {
                // Perhaps the definitions list has ended and this belongs
                // to the section again.
                let resumed = self.outer.as_ref().map(|outer| {
                    let mut candidate = outer.clone();
                    let step = candidate.push(token);
                    (candidate, step)
                });
                match resumed {
                    Some((candidate, step)) if !matches!(step, Step::Irregular { .. }) => {
                        self.stack = candidate;
                        self.outer = None;
                        self.term_prefix.clear();
                        self.term = None;
                    }
                    _ => irregular.push(reason),
                }
            }
        }

        let path = self.stack.path();
        let citation = self.citation(&path);
        let Some(section) = self.open.as_mut() else {
            return;
        };
        for reason in irregular {
            let snippet: String = body.chars().take(60).collect();
            section.irregularities.push(format!(
                "{}: {reason} \u{2014} \u{201c}{snippet}\u{2026}\u{201d}",
                section.identifier
            ));
        }
        let text = strip_markers(if lead.designators.is_empty() && lead.term.is_none() {
            text
        } else {
            body
        });
        self.lead_in = is_lead_in(&text);
        let Some(section) = self.open.as_mut() else {
            return;
        };
        section.paragraphs.push(Paragraph {
            path,
            citation,
            heading: lead.heading,
            term: self.term.clone(),
            text,
        });
    }

    /// `34 CFR 668.46(a) "Campus"(i)` — a definition's numbering is addressed
    /// through the term, because `(i)` alone would collide with the section's
    /// own paragraph `(i)`.
    fn citation(&self, path: &[String]) -> String {
        let Some(section) = self.open.as_ref() else {
            return String::new();
        };
        let mut citation = section.citation.clone();
        let prefix = if self.term.is_some() {
            &self.term_prefix[..]
        } else {
            &[][..]
        };
        for token in prefix {
            let _ = std::fmt::Write::write_fmt(&mut citation, format_args!("({token})"));
        }
        if let Some(term) = &self.term {
            let _ = std::fmt::Write::write_fmt(&mut citation, format_args!(" \"{term}\""));
        }
        for token in path {
            let _ = std::fmt::Write::write_fmt(&mut citation, format_args!("({token})"));
        }
        citation
    }
}

/// `PART 668—Student Assistance General Provisions` → the title alone.
fn strip_part_prefix(text: &str) -> String {
    text.split_once('\u{2014}')
        .map_or(text, |(_, tail)| tail)
        .trim()
        .to_string()
}

/// `§ 668.14 Program participation agreement.` → the heading alone.
fn strip_section_prefix(text: &str) -> String {
    let trimmed = text.trim_start_matches('\u{a7}').trim_start();
    match trimmed.split_once(char::is_whitespace) {
        Some((first, rest)) if first.chars().any(|c| c.is_ascii_digit()) => rest.trim().to_string(),
        _ => trimmed.to_string(),
    }
}

/// `Link to an amendment published at 91 FR 40281, July 1, 2026.` → `91 FR 40281`.
fn fr_citation(text: &str) -> Option<String> {
    let at = text.find(" FR ")?;
    let volume: String = text[..at]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let page: String = text[at + 4..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!volume.is_empty() && !page.is_empty()).then(|| format!("{volume} FR {page}"))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn attribute(event: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == key).then(|| String::from_utf8_lossy(a.value.as_ref()).into_owned())
    })
}

/// Parses one part's point-in-time XML.
#[allow(clippy::too_many_lines)]
pub fn parse_part(xml: &[u8]) -> Result<Part> {
    let mut reader = Reader::from_reader(xml);
    let config = reader.config_mut();
    config.trim_text(false);
    config.check_end_names = false;

    let mut builder = Builder::new();
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .context("eCFR part XML is malformed")?
        {
            Event::Start(event) => {
                let name = event.name().as_ref().to_vec();
                if matches!(name.as_slice(), b"I" | b"E") && builder.sink == Some(Sink::Paragraph) {
                    builder.buffer.push(ITALIC_OPEN);
                    continue;
                }
                if let Some(sink) = sink_for(&name) {
                    builder.flush();
                    builder.sink = Some(sink);
                    if sink == Sink::Amendment {
                        builder.amendment_pending = attribute(&event, b"AMDINSN").is_some();
                    }
                    continue;
                }
                match name.as_slice() {
                    b"DIV5" => {
                        builder.owner = Owner::Part;
                        builder.part.number = attribute(&event, b"N").unwrap_or_default();
                    }
                    b"DIV6" => {
                        builder.close_section();
                        builder.owner = Owner::Subpart;
                        builder.subpart = attribute(&event, b"N");
                    }
                    b"DIV8" | b"DIV9" => {
                        builder.close_section();
                        let kind = if name == b"DIV9" {
                            Kind::Appendix
                        } else {
                            Kind::Section
                        };
                        let identifier = attribute(&event, b"N").unwrap_or_default();
                        let citation = attribute(&event, b"hierarchy_metadata")
                            .as_deref()
                            .and_then(citation_from_metadata)
                            .unwrap_or_else(|| format!("34 CFR {identifier}"));
                        builder.owner = Owner::Section;
                        builder.open = Some(Section {
                            identifier,
                            citation,
                            kind,
                            subpart: builder.subpart.clone(),
                            heading: String::new(),
                            paragraphs: Vec::new(),
                            pending: Vec::new(),
                            credit: None,
                            authority: None,
                            irregularities: Vec::new(),
                        });
                    }
                    b"AUTH" => builder.note = Some(Note::Authority),
                    b"SOURCE" => builder.note = Some(Note::Source),
                    _ => {}
                }
            }
            Event::End(event) => {
                let name = event.name().as_ref().to_vec();
                if matches!(name.as_slice(), b"I" | b"E") && builder.sink == Some(Sink::Paragraph) {
                    builder.buffer.push(ITALIC_CLOSE);
                    continue;
                }
                if sink_for(&name).is_some() {
                    builder.flush();
                } else if matches!(name.as_slice(), b"AUTH" | b"SOURCE") {
                    builder.note = None;
                } else if name == b"DIV5" {
                    builder.close_section();
                }
            }
            Event::Text(text) => {
                if builder.sink.is_some() {
                    builder
                        .buffer
                        .push_str(&text.unescape().unwrap_or_default());
                }
            }
            Event::CData(data) => {
                if builder.sink.is_some() {
                    builder.buffer.push_str(&String::from_utf8_lossy(&data));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    builder.flush();
    builder.close_section();

    anyhow::ensure!(
        !builder.part.number.is_empty(),
        "no DIV5 part element found — upstream returned something other than part XML"
    );
    Ok(builder.part)
}

/// `{"path":"…","citation":"34 CFR 668.14"}`, double-escaped by eCFR.
fn citation_from_metadata(raw: &str) -> Option<String> {
    let cleaned = raw.replace("&quot;", "\"").replace("&amp;", "&");
    let marker = cleaned.find("\"citation\"")?;
    let rest = &cleaned[marker + "\"citation\"".len()..];
    let open = rest.find(':').and_then(|i| rest[i..].find('"'))? + rest.find(':')?;
    let tail = &rest[open + 1..];
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"<?xml version="1.0"?>
<DIV5 N="668" TYPE="PART">
<HEAD>PART 668&#x2014;Student Assistance General Provisions</HEAD>
<AUTH><HED>Authority:</HED><PSPACE>20 U.S.C. 1001-1003, 1094.</PSPACE></AUTH>
<SOURCE><HED>Source:</HED><PSPACE>52 FR 45727, Dec. 1, 1987.</PSPACE></SOURCE>
<DIV6 N="A" TYPE="SUBPART">
<HEAD>Subpart A&#x2014;General</HEAD>
<DIV8 N="668.14" TYPE="SECTION" hierarchy_metadata="{&quot;path&quot;:&quot;/on/_SUBSTITUTE_DATE_/title-34/section-668.14&quot;,&quot;citation&quot;:&quot;34 CFR 668.14&quot;}">
<HEAD>&#xA7; 668.14 Program participation agreement.</HEAD>
<XREF ID="20260701" REFID="12" AMDINSN="6">Link to an amendment published at 91 FR 40281, July 1, 2026.</XREF>
<P>(a)(1) An institution may participate only if it enters into an agreement.</P>
<P>(2) The agreement applies to each branch campus.</P>
<P>(3) The agreement must be signed by&#x2014;</P>
<P>(i) An authorized representative; and</P>
<P>(ii) For a proprietary institution, an owner.</P>
<P>(b) By entering into an agreement, an institution agrees that&#x2014;</P>
<P>(1) It will comply with all statutory provisions.</P>
<CITA TYPE="N">[52 FR 45724, Dec. 1, 1987, as amended at 85 FR 54813, Sept. 2, 2020]</CITA>
</DIV8>
</DIV6>
</DIV5>"#;

    fn sample() -> Part {
        parse_part(SAMPLE).expect("sample parses")
    }

    #[test]
    fn part_metadata_is_lifted() {
        let part = sample();
        assert_eq!(part.number, "668");
        assert_eq!(part.heading, "Student Assistance General Provisions");
        assert_eq!(
            part.authority.as_deref(),
            Some("20 U.S.C. 1001-1003, 1094.")
        );
        assert_eq!(part.source.as_deref(), Some("52 FR 45727, Dec. 1, 1987."));
    }

    #[test]
    fn section_identity_comes_from_the_hierarchy_metadata() {
        let section = &sample().sections[0];
        assert_eq!(section.identifier, "668.14");
        assert_eq!(section.citation, "34 CFR 668.14");
        assert_eq!(section.heading, "Program participation agreement.");
        assert_eq!(section.subpart.as_deref(), Some("A"));
        assert_eq!(section.kind, Kind::Section);
    }

    #[test]
    fn paragraphs_are_addressed_the_way_they_are_cited() {
        let section = &sample().sections[0];
        let citations: Vec<&str> = section
            .paragraphs
            .iter()
            .map(|p| p.citation.as_str())
            .collect();
        assert_eq!(
            citations,
            vec![
                "34 CFR 668.14(a)(1)",
                "34 CFR 668.14(a)(2)",
                "34 CFR 668.14(a)(3)",
                "34 CFR 668.14(a)(3)(i)",
                "34 CFR 668.14(a)(3)(ii)",
                "34 CFR 668.14(b)",
                "34 CFR 668.14(b)(1)",
            ]
        );
    }

    #[test]
    fn the_designator_is_removed_from_the_text_it_labels() {
        let section = &sample().sections[0];
        assert_eq!(
            section.paragraphs[0].text,
            "An institution may participate only if it enters into an agreement."
        );
    }

    #[test]
    fn a_published_but_unincorporated_amendment_is_captured() {
        let section = &sample().sections[0];
        assert_eq!(section.pending.len(), 1);
        assert_eq!(section.pending[0].fr_citation, "91 FR 40281");
    }

    #[test]
    fn the_source_credit_is_kept_as_the_join_to_the_register() {
        let credit = sample().sections[0].credit.clone().unwrap();
        assert!(credit.starts_with("52 FR 45724"));
        assert!(credit.ends_with("Sept. 2, 2020"));
    }

    #[test]
    fn entities_are_decoded() {
        let section = &sample().sections[0];
        assert!(section.paragraphs[2].text.ends_with('\u{2014}'));
        assert!(!section.heading.contains("&#"));
    }

    #[test]
    fn a_definitions_list_numbers_inside_each_term() {
        let xml = br#"<DIV5 N="668" TYPE="PART"><HEAD>PART 668&#x2014;Test</HEAD>
<DIV8 N="668.46" TYPE="SECTION" hierarchy_metadata="{&quot;citation&quot;:&quot;34 CFR 668.46&quot;}">
<HEAD>&#xA7; 668.46 Institutional security policies.</HEAD>
<P>(a) <I>Definitions.</I> Additional definitions that apply to this section:</P>
<P><I>Campus.</I> (i) Any building owned by an institution.</P>
<P>(ii) Any building reasonably contiguous to that area.</P>
<P><I>Campus security authority.</I> (i) A campus police department.</P>
<P>(b) The institution must publish an annual security report.</P>
</DIV8></DIV5>"#;
        let part = parse_part(xml).unwrap();
        let section = &part.sections[0];
        let citations: Vec<&str> = section
            .paragraphs
            .iter()
            .map(|p| p.citation.as_str())
            .collect();
        assert_eq!(
            citations,
            vec![
                "34 CFR 668.46(a)",
                "34 CFR 668.46(a) \"Campus\"(i)",
                "34 CFR 668.46(a) \"Campus\"(ii)",
                "34 CFR 668.46(a) \"Campus security authority\"(i)",
                "34 CFR 668.46(b)",
            ],
            "each term restarts at (i), and (b) resumes the section"
        );
        assert!(
            section.irregularities.is_empty(),
            "unexpected: {:?}",
            section.irregularities
        );
        assert_eq!(section.paragraphs[1].term.as_deref(), Some("Campus"));
        assert_eq!(section.paragraphs[4].term, None);
    }

    #[test]
    fn non_part_xml_is_rejected_rather_than_returning_an_empty_part() {
        let error = parse_part(b"<html><body>404</body></html>").unwrap_err();
        assert!(error.to_string().contains("no DIV5 part element"));
    }

    #[test]
    fn fr_citations_are_extracted_from_the_amendment_note() {
        assert_eq!(
            fr_citation("Link to an amendment published at 91 FR 40281, July 1, 2026.").as_deref(),
            Some("91 FR 40281")
        );
        assert_eq!(fr_citation("no citation here"), None);
    }
}
