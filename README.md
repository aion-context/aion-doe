# doe-aion

Watches the Department of Education's authoritative machine-readable sources,
detects real change, and emits a cryptographically signed
[`.aion`](https://crates.io/crates/aion-context) chain as the deliverable.

The chain answers one question with a signature behind it: **what did the
Department of Education require on date X, and who says so?**

The sibling of [`fedramp-aion`](https://github.com/aion-context/fedramp-aion),
built on the same idea and almost none of the same machinery — because ED
publishes nothing on GitHub, ships no `force` field, and versions its rules by
date rather than by commit.

## Sources

| id | publisher | what |
|---|---|---|
| `cfr` | [eCFR](https://www.ecfr.gov/developers) | **Title 34 — Education.** 141 parts, ~110 live, 8,127 sections and appendices |
| `register` | [Federal Register](https://www.federalregister.gov/developers/api/v1) | ED rules, proposed rules and notices, with the dates they take effect |
| `statute` | [govinfo](https://api.govinfo.gov) | U.S. Code Title 20 — the statute 34 CFR implements and cites but never restates |
| `accreditation` | [DAPIP](https://ope.ed.gov/dapip) | accredited institutions, accrediting agencies, and programs |

There is no single ED "rules file" the way FedRAMP has one, and the FSA
Knowledge Center — where the Dear Colleague Letters live — serves HTML only,
with no API. A signed artifact whose content depends on what a page rendered
today would be worse than none, so it is deliberately excluded.
[DESIGN.md](DESIGN.md) records what was rejected and why.

## The pin is a date

`fedramp-aion` resolves `main` to a commit SHA before every fetch. eCFR needs
no such thing: `full/{date}/title-34.xml?part=668` returns the part as it stood
on that date, and returns the **same bytes** every time.

Measured against 34 CFR 668, whose last amendment was 2026-07-20:

| date requested | bytes | sha256 |
|---|---|---|
| 2026-07-15 | 1,124,312 | `4ac594097c17a920…` |
| 2026-07-25 | 1,124,686 | `2b88f8289bc4a1c3…` |
| 2026-08-10 | 1,124,686 | `2b88f8289bc4a1c3…` |
| 2026-08-21 | 1,124,686 | `2b88f8289bc4a1c3…` |

Three dates spanning four weeks, byte-identical; the fourth differs across the
real amendment. There is no generation timestamp, no document uuid, nothing to
project away before the gate — the whole class of problem that dominates
`fedramp-aion`'s design does not exist here. And a date is a better provenance
token than a commit SHA, because replaying a run does not depend on a
repository still existing.

## The gate is a ledger

eCFR publishes, for every section in a title, the date it was last amended,
whether that amendment was **substantive**, and whether the section was
**removed**:

```json
{"amendment_date": "2026-07-24", "identifier": "100.3", "part": "100",
 "name": "§ 100.3   Discrimination prohibited.", "substantive": true, "removed": false}
```

So a run fetches ~2 MB of ledger, compares `amendment_date` per section, and
pulls regulation text **only for the parts that moved**. On a quiet day that is
one request. And severity comes from ED's own judgement of what is substantive,
rather than from a heuristic over a diff.

## Reading the regulation

A section's body is a flat run of `<P>`. All the hierarchy — `(a)`, `(1)`,
`(i)`, `(A)` — lives in the designator that opens each paragraph, and the
numbering systems overlap: `(i)` is the ninth letter *and* the first roman
numeral. Rebuilding the tree is most of the work.

The reader tests each designator against the successor of every open level,
deepest first, so the sequence decides what the token cannot. That alone left
**593 of 4,721 paragraphs unresolved** in part 668. Three drafting conventions
account for nearly all of them:

- **Italic headings interrupt the run.** `(a) *Written arrangements.* (1) Except as …`
  — flatten the italics and the `(1)` reads as prose, losing the entire arabic
  level beneath `(a)`. **593 → 104.**
- **Defined terms open their own scope.** A definitions list restarts at `(i)`
  for every term, and the regulation says so itself, citing "paragraph (i) of
  *this definition*". Those are cited as `34 CFR 668.46(a) "Campus"(i)`.
  **104 → 32.**
- **A lead-in forces a descent.** After `(h)(2) … that—`, the token `(i)` is
  legitimately both the letter after `(h)` and the first roman beneath `(2)`.
  Nothing about the token decides it; the em dash does. **32 → 12.**

**12 in 4,721 — 0.25%** — each reported with the token, the path it did not fit
and the text. Dropping a paragraph would drop regulatory text.

```sh
doe-aion part 668
```
```
34 CFR part 668 — STUDENT ASSISTANCE GENERAL PROVISIONS
  as of        : 2026-08-21
  sections     : 216
  paragraphs   : 4721
  pending amdt : 5
  irregular    : 12
  authority    : 20 U.S.C. 1001-1003, 1070g, 1085, 1088, 1091, 1092, 1094, 1099c, …
```

`pending amdt` is eCFR telling you an amendment has been *published* and not
yet folded into the text — a change that has already happened upstream and is
invisible in the regulation you are reading.

## Obligations

FedRAMP publishes `force: MUST`. ED publishes prose. The atoms are derived, and
every judgement in the derivation is reported rather than asserted.

```sh
doe-aion obligations 668 --bearer institution
```
```
34 CFR part 668 — STUDENT ASSISTANCE GENERAL PROVISIONS (as of 2026-08-21)
  4721 paragraphs → 2156 atoms (1041 inherited, 460 definitions skipped, 355 unclassified bearer)
  force  : {"MAY": 657, "MUST": 1330, "MUST NOT": 125, "SHOULD": 44}

  34 CFR 668.3(c)(2)     MUST   institution   An institution's written request must—
  34 CFR 668.3(c)(2)(i)  MUST   institution   Identify each educational program for which …
```

Three things make this more than a grep for "must".

**`may not` is a prohibition.** In Federal Register drafting it is a
prohibition, not an absent permission, and reading it the other way inverts the
rule. Negative forms match first; matching is word-boundary anchored, so
`shall` does not fire inside `shallow`.

**The Secretary is not the institution.** The bearer is read from the subject
phrase, and the match *closest to the modal* wins, because that is where
English puts the grammatical subject: in `Upon the written request of an
institution, the Secretary may approve …` the duty is the Secretary's. **80
subject phrases in part 668 name both.** Where more than one kind of actor
appears, the atom is flagged `bearer_ambiguous` rather than quietly resolved,
and a subject that matches nothing is reported `unclassified` rather than
defaulted — a default would manufacture duties.

**Definitions are not duties.** `Award year means the period that shall begin
on July 1` is excluded, and the count of exclusions is reported.

One finding worth its own line: parts of 34 CFR 668 address the reader as
**`you`**, and define `you` *locally* — the institution in one subpart, the
borrower in another. That is 221 atoms in part 668. They are classified as
`addressee` and left for the applicability layer to resolve against the
subpart's own definition, because guessing would be worse than saying so.

## Quick start

```sh
cargo build --release

# what does eCFR say about Title 34 right now?
doe-aion status

# the amendment ledger, and what has moved lately
doe-aion ledger --since 2026-07-01

# one part: structure, pending amendments, unresolved sequences
doe-aion part 99

# derived obligations
doe-aion obligations 99 --bearer institution
doe-aion obligations 668 --json
```

Working offline against captured snapshots — how the logic is iterated:

```sh
doe-aion capture --out snapshots
doe-aion --from-dir snapshots sweep
```

## Tests

```sh
cargo test          # no network
cargo clippy --all-targets
```

The suite pins the behaviour that was hard to get right: `(i)` after `(h)`
versus `(i)` after `(a)(1)`, `(ii)` after `(hh)`, `z → aa` rather than `ba`,
the lead-in descent, definition scoping, `may not` as a prohibition, the actor
nearest the modal, and the UTF-8 boundary that an em dash puts in the middle of
a subject phrase.

## Status

Ingestion, parsing, obligation derivation and offline replay are in place and
measured. The signed chain, the change report, applicability profiles, receipts
and the MCP server are specified in [DESIGN.md](DESIGN.md) and not yet built.
