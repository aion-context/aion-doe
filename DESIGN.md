# doe-aion — pipeline logic

Detect real change in the Department of Education's machine-readable rules, and
emit a cryptographically signed `.aion` chain as the deliverable.

The chain answers one question with a signature behind it: **what did the
Department of Education require on date X, and who says so?**

This is the sibling of [`fedramp-aion`](https://github.com/aion-context/fedramp-aion),
and the differences are not cosmetic. FedRAMP publishes a rules JSON with a
`force` field, through GitHub, pinned by commit SHA. ED publishes none of those
things. Everything below follows from that.

Status: **logic under review.** Measurements are dated; re-check them if the
pipeline misbehaves.

## 1. Sources

There is no single ED "rules file". The authority is split across four
publishers, each with its own idea of what a version is.

| id | publisher | endpoint | what | pin |
|---|---|---|---|---|
| `cfr` | eCFR (GPO/NARA) | `ecfr.gov/api/versioner/v1` | Title 34 — 141 parts, ~110 live, 8,127 sections and appendices | the **date**, §2 |
| `register` | Federal Register | `federalregister.gov/api/v1` | ED rules, proposed rules and notices, with `effective_on` | `document_number` |
| `statute` | govinfo | `api.govinfo.gov` | U.S. Code Title 20 — the statute 34 CFR implements and cites but never restates | package `lastModified` |
| `accreditation` | ED/OPE | `ope.ed.gov/dapip/api` | DAPIP — accredited institutions, agencies, programs | content digest, §6 |

**Rejected sources**, recorded so the question is not reopened:

- **FSA Knowledge Center** (`fsapartners.ed.gov`) — Dear Colleague Letters and
  the FSA Handbook are subregulatory guidance and would be valuable, but the
  site serves HTML only; `api/knowledge-center/search` 404s. Nothing there is
  pinnable, and a signed artifact whose content depends on what a page rendered
  today is worse than no artifact.
- **regulations.gov v4** — live and useful (`filter[agencyId]=ED`), but it
  requires an api.data.gov key. Held as an optional fifth source rather than a
  dependency, because a chain that cannot be rebuilt without someone's key is
  not independently verifiable.
- **`ed.gov/data.json`** — redirects to `data.ed.gov/data.json`, a 906-dataset
  DCAT inventory that is overwhelmingly XLS and PDF. It is a catalogue of
  reports, not of rules.

## 2. The pin is a date, and it is stronger than a commit SHA

`GET /versioner/v1/full/{date}/title-34.xml?part={n}` returns the part as it
stood on `date`. Measured 2026-08-25 against 34 CFR 668, whose ledger shows
amendments on 2026-05-19, 2026-07-01 and 2026-07-20 and nothing since:

| date requested | bytes | sha256 |
|---|---|---|
| 2026-07-15 | 1,124,312 | `4ac594097c17a920…` |
| 2026-07-25 | 1,124,686 | `2b88f8289bc4a1c3…` |
| 2026-08-10 | 1,124,686 | `2b88f8289bc4a1c3…` |
| 2026-08-21 | 1,124,686 | `2b88f8289bc4a1c3…` |

Three different dates spanning four weeks return **byte-identical** content,
and the fourth differs across the real 2026-07-20 amendment.

This matters more than it looks. `fedramp-aion` exists largely to work around
the fact that its sources are rewritten daily whether or not anything changed —
the marketplace file churns at 06:27 UTC every day, NIST republishes with a
fresh document `uuid`, CISA moves `dateReleased` on every publish. Each needed a
per-source projection stripping volatile fields before the gate digest.

**eCFR needs none of that.** There is no timestamp in the payload, no document
uuid, no generation marker. A date is a version, the same bytes come back
tomorrow, and anyone can replay a run by asking for the same date — which is a
better provenance story than a commit SHA, because it does not depend on a
repository still existing.

The volatile fields live in `titles.json` instead (`meta.date` moves daily,
`meta.import_in_progress` flips during a load), and that document never enters
the payload.

## 3. The gate is a ledger, not a digest

`GET /versioner/v1/versions/title-34.json` returns one row per section and
appendix, paginated at 1,000 rows over 9 pages (~250 KB each). Measured
2026-08-25: **8,127 rows**, of which 83 are marked removed and 871 of the first
page are marked substantive.

```json
{"date": "2026-07-24", "amendment_date": "2026-07-24", "issue_date": "2026-07-24",
 "identifier": "100.3", "name": "§ 100.3   Discrimination prohibited.",
 "part": "100", "substantive": true, "removed": false, "type": "section"}
```

So the pipeline does not fetch the title and diff it. It fetches ~2 MB of
ledger, compares `amendment_date` per section, and pulls XML **only for the
parts that moved**, carrying every other part's record forward from the
previous signed version unchanged. On a quiet day a run is one ledger fetch and
nothing else, against ~110 part fetches.

That is not a micro-optimisation. Reading the whole title live takes long enough
on a shared runner to be throttled into a multi-minute crawl — measured, on the
first CI run, before the incremental path existed.

**What incremental gives up, and what pays for it.** An edit eCFR did not record
in the ledger — a correction, a publishing slip — is invisible to an incremental
run, because the part is never re-read. `--full` re-reads everything, and the
schedule runs one full pass a week for exactly that reason. Genesis is always a
full read, because there is no previous version to carry forward. Both cases are
asserted as tests, including the negative one: an incremental run *not* seeing a
silent edit is a documented limitation, not an accident.

Per-part records therefore carry their own force and bearer breakdowns, not just
their digest, so a carried part contributes the same figures a re-read one would
and the payload totals are a plain sum in either case.

Two facts the ledger gives that a content digest could never recover:

- **`substantive`** — eCFR itself distinguishes a real amendment from a
  technical correction. That drives severity directly instead of being guessed.
- **`removed`** — a section can be marked removed while its row survives.

The ledger is also the reconciliation point with the Federal Register. Title 34
reports `latest_amended_on: 2026-07-24`; FR document `2026-15019` ("Rescinding
Portions of the Department of Education Title VI Regulations…", 34 CFR 100)
carries `effective_on: 2026-07-24`. The Register leads, the CFR follows, and
the two can be checked against each other.

### 3.1 The ledger is history, not a table of contents

Measured 2026-08-25: the ledger holds **8,127 distinct section identifiers**,
while the title actually contains **3,296** live sections across 108
non-reserved parts. The difference is every section the title has ever had.

That is not a curiosity — driving the fetch from the ledger's part list makes
the run fail. Part 230 appears in the ledger and **404s** at 2026-08-21,
because the part no longer exists. The live set comes from
`structure/{date}/title-34.json`; the ledger drives change detection only. The
difference between the two is reported as **retired parts**, which is
information neither endpoint gives on its own.

### 3.2 The ledger must be pinned to the same date as the text

`versions/title-34.json` unfiltered returns the ledger **as it stands today**.
Pinning the text to a past date while reading a current ledger would sign a
bundle whose ledger describes amendments the text does not contain — and it
would do so silently, because today's ledger happens to agree with today's
text. Measured:

| request | rows | latest amendment |
|---|---|---|
| `issue_date[lte]=2026-07-15` | 8,105 | 2026-07-01 |
| `issue_date[lte]=2026-08-21` | 8,127 | 2026-07-24 |
| unfiltered | 8,127 | 2026-07-24 |

The filter is therefore mandatory, not an optimisation.

## 4. Reading the regulation

`cfr` is XML, not JSON, and the structure that matters is not in the markup.

A part is `DIV5` → `DIV6` subparts → `DIV8` sections (`DIV9` for appendices).
A section's body is a **flat run of `<P>`**. All paragraph hierarchy — `(a)`,
`(1)`, `(i)`, `(A)` — is encoded in the designator that opens each paragraph and
nowhere else. Rebuilding that tree is the whole job, and it is harder than it
looks because the numbering systems overlap.

### 4.1 The successor algorithm

`(i)` is the ninth lowercase letter *and* the first lowercase roman numeral.
`(ii)` is the second roman numeral *and* the letter after `(hh)`. A depth table
keyed on the token cannot decide between them; the sequence can.

Each open level remembers the system it counts in and the value it holds. An
incoming designator is tested against the successor of every open level,
**deepest first**; the first level whose successor it matches is where it
belongs, and everything deeper closes. A designator that succeeds nothing open
must open a new level, and only the first value of a system — `a`, `1`, `i`,
`A`, `I` — can do that, which is unambiguous.

`(h)` then `(i)` advances the letters. `(a)(1)` then `(i)` opens the romans.
Same token, opposite reading, no lookahead. Letters run `a…z`, then `aa…zz`:
`z` is followed by `aa`, never `ba`, so they are not base-26 numerals.

### 4.2 Three things that break it, all found by measurement

Running the successor algorithm alone over 34 CFR 668 left **593 of 4,721
paragraphs** (12.5%) with sequences it could not resolve. Each cause was a real
drafting convention, not upstream noise:

**Italic paragraph headings interrupt the designator run.** Upstream writes:

```xml
<P>(a) <I>Written arrangements between eligible institutions.</I> (1) Except as provided …</P>
```

Flattening `<I>` into text makes the `(1)` look like prose, and the *entire
arabic level below `(a)` is lost* — every following `(2)`, `(3)` then fails.
The reader now wraps italic runs in markers so the parser can step over a
heading and keep reading designators. **593 → 104.**

**Defined terms open their own numbering scope.** A definitions list restarts
at `(i)` for every term:

```xml
<P><I>Campus.</I> (i) Any building or property owned or controlled by an institution …</P>
<P>(ii) Any building or property that is within or reasonably contiguous to the area
       identified in paragraph (i) of this definition …</P>
```

The regulation says so itself — "paragraph (i) of **this definition**". An
italic run with no designator before it, ending in `.` or `:`, is a term: it
parks the section's stack, starts a fresh one, and the section's own numbering
resumes when a designator fits the parked stack again. Those paragraphs are
cited as `34 CFR 668.46(a) "Campus"(i)`, because `(i)` alone would collide with
the section's own paragraph `(i)`. **104 → 32.**

**A lead-in forces a descent.** This is the one that cannot be resolved by
looking at the designator at all. 34 CFR 668.35:

```
(h)(2) Demonstrates to the satisfaction of the holder of the debt that—
   (i) When the student filed the petition for bankruptcy relief …
   (ii) The debt otherwise qualifies for discharge …; and
(i) In the case of a student who has been convicted of … fraud …
```

Both `(i)` tokens are legitimate under the successor rule — the first as the
letter after `(h)`, the second likewise. Deepest-first picks the letter both
times, which is right for the second and wrong for the first, and the whole
roman list is lost.

The disambiguator is punctuation: a paragraph ending in an em dash or a colon
**introduces a list**, so what follows is one of its children and cannot be a
sibling of one of its ancestors. After a lead-in the parser prefers to open a
level; otherwise it prefers the successor. **32 → 12.**

### 4.3 Where it stands

**12 unresolved sequences in 4,721 paragraphs of 34 CFR 668 — 0.25%.** Across
the whole title: **88 in 36,828 paragraphs, also 0.24%**, over 108 parts and
3,296 sections. The worst part is 34 CFR 642 at 4.9% (9 of 183); no other part
exceeds 3%.

They are reported per section with the offending token, the path it did not
fit, and the text, never silently absorbed. Dropping a paragraph would drop
regulatory text; a citation that is wrong and flagged is recoverable, one that
is wrong and silent is not. The count is carried in the signed payload, so the
artifact cannot claim a cleaner parse than it achieved.

## 5. Obligations

FedRAMP hands you `force: MUST`. ED hands you prose. The atoms have to be
derived, and the gap between "this paragraph contains an obligation" and "this
obligation binds *you*" is where a compliance tool earns the right to be
trusted, so each step is reported rather than asserted.

**Force** comes from Federal Register drafting conventions. `must`, `shall` and
`is required to` bind; `may not`, `must not` and `shall not` prohibit. In this
register **`may not` is a prohibition, not an absent permission** — reading it
as the latter inverts the rule, so negative forms are matched first and matching
is word-boundary anchored (`shall` must not match inside `shallow`). A `may`
narrowed by `only if` or `unless` is flagged as conditioned: the modal is
permissive but the sentence constrains conduct.

**Bearer** is classified from the subject phrase, and the match **closest to the
modal** wins, because that is where English puts the grammatical subject. In
`Upon the written request of an institution, the Secretary may approve …` the
duty is the Secretary's; taking the first mention would hand it to the
institution. Measured on part 668, **80 subject phrases name both**. The
failure mode this trades for is a relative clause — `An institution that
contracts with a third-party servicer must …` reads as the servicer — so any
subject naming more than one kind of actor is flagged `bearer_ambiguous`. A
match contained inside a longer one is part of it, not a rival: `State
educational agency` is not `State` plus `educational agency`.

An unmatched subject is reported as `unclassified`, never defaulted. **An
obligation on the Secretary is not an obligation on an institution**, and a
tool that blurred them would manufacture duties.

**Inheritance** follows the drafting shape where the lead-in carries the modal
and the children carry the list (`The agreement must be signed by— (i) An
authorized representative`), and where the list item carries the modal and the
lead-in carries the actor (`An institution seeking to participate— (1) Must
submit an application`). Both directions are resolved; inherited atoms say so.

**Definitions are excluded** — both a term-scoped paragraph (§4.2) and an
early `X means Y`, since `Award year means the period that shall begin on
July 1` is not a duty. The count of exclusions is reported.

Measured on 34 CFR 668 as of 2026-08-21: **4,721 paragraphs → 2,156 atoms**
(1,330 `MUST`, 125 `MUST NOT`, 657 `MAY`, 44 `SHOULD`), 1,041 inherited, 460
definitions skipped.

Bearer classification found 969 institution, 221 second-person, 195 Secretary,
83 third-party servicer, 161 student or borrower, 68 State agency, 44 party to
a proceeding, 32 hearing official, 19 test publisher, 8 accrediting agency.
**355 unclassified — 16.5%.** The first pass was 34%; the gap closed by adding
the actors ED actually drafts for rather than the ones FedRAMP has.

Across the whole title as of 2026-08-21: **36,828 paragraphs → 18,088 atoms,
12,064 of them binding.**

Bearer classification generalises worse than parsing does, and the measurement
says so. Patterns tuned on Title IV left **39.9% unclassified title-wide**
against 16.5% in part 668, because the K-12, disability and grants parts are
drafted in a different vocabulary: `SEA`, `LEA`, `public agency`, `subgrantee`,
`insular area`, `the Governor`, the `IEP Team`, a State `advisory panel`. Adding
the actors ED actually drafts for brought it to **32.3%**.

Most of what remains is genuinely impersonal and should stay unclassified —
`Nothing in paragraph (c) of this section may be construed to …` has no actor,
and inventing one would invent a duty. `applicant` is a known imprecision: it
means a student in Title IV and a grant applicant in the grants parts, and
resolving it needs part context the classifier does not yet have.

Second-person drafting deserves its own note. Subparts of 34 CFR 668 address
the reader as **`you`**, and `you` is defined *locally* — the institution in one
subpart, the borrower in another. That is a real bearer, so it is classified as
`addressee`, and resolving which actor it means is left to the applicability
layer that can read the subpart's own definition. Guessing would be worse.

Deliberately out of scope for now: deciding that an atom applies to a given
institution. That is applicability, and it is a separate, explicitly curated
judgement — see §9.

## 5.1 Replay against real upstream history

The pipeline was run against the title as it stood on **2026-07-15** and then
advanced to **2026-08-21**, both from captured snapshots, and its output was
checked against the Federal Register rather than against itself.

```
34 CFR genesis — 108 parts, 3290 sections as of 2026-07-15
signed version 1 of doe.aion — bundle d529b547b36c0dc4

TITLE 34 CHANGED — 22 section(s) moved (13 substantive) across 6 part(s), as of 2026-08-21
signed version 2 of doe.aion — bundle 6d8c7d04d36ea7e1
```

The Federal Register lists exactly three ED rules in that window. All three are
reproduced, with the right sections and the right dates:

| FR document | effective | detected |
|---|---|---|
| Rescinding Guidelines for Eliminating Discrimination… (34 CFR 100, 104, 106) | 2026-07-23 | Appendix B to Part 100, Appendix B to Part 104, Appendix A to Part 106 — all three **removed**, dated 2026-07-23 |
| Rescinding Portions of the Title VI Regulations (34 CFR 100) | 2026-07-24 | `100.3` and `100.5` amended, dated 2026-07-24 |
| Accountability in Higher Education… Demand-Driven Workforce Pell (34 CFR 600, 668, 685) | 2027-07-01 | `600.10`, `668.5`, `668.8`, `668.20`, `668.32`, `690.2`, `690.6`, `690.11` amended 2026-07-20, plus a new subpart `690.90`–`690.97` |

The third row is the interesting one. The Federal Register names **34 CFR 685**,
and the diff does not report it — correctly. Those amendments are *published and
not yet incorporated*: the run reports 9 pending amendments, and every one of
them cites the same rule (91 FR 40280–40287), including `685.102` and `685.300`.
The regulation an institution reads today has not changed; the rule that will
change it exists. Reporting only one of those two facts would be misleading, so
the artifact carries both.

**Path independence.** Version 2's bundle digest, `6d8c7d04d36ea7e1`, is
byte-identical to the digest produced by pinning 2026-08-21 directly with no
intermediate version. The signed artifact is a function of the pin, not of when
the watcher happened to run or what it saw on the way.

## 6. Determinism invariants

The signed payload must be a pure function of the pinned date and the pinned
upstream identifiers.

- **No wall-clock anywhere in the payload.** Fetch time may appear in the
  report and the commit message only. Violating this makes every rerun produce
  a new digest and destroys idempotency.
- The `.aion` version timestamp is pinned to the newest upstream amendment
  date, not to `now`.
- All maps are JCS-ordered (RFC 8785); all keyed collections are sorted by key.
- **Never pin a date past `up_to_date_as_of`.** eCFR reports Title 34 current
  through 2026-08-21 while `meta.date` reads 2026-08-24 and
  `meta.import_in_progress` is true. Asking for content the title has not been
  updated to yet is refused before the request is made.
- **JCS number precision.** JCS serializes numbers with ECMAScript semantics,
  so any integer above 2^53 silently rounds — the trap that corrupted a
  `file_id` in `fedramp-aion`. Sources are scanned for oversized integers
  before signing rather than after the digest is already wrong.
- DAPIP has no version token at all, so it is gated on a content digest. It is
  the one source here that needs the `fedramp-aion` treatment, and its volatile
  fields must be measured before it is wired in.

## 7. Severity

| severity | trigger |
|---|---|
| `major` | a section eCFR marks `substantive` was amended, added, or removed |
| `minor` | a non-substantive amendment, or a published Federal Register rule not yet in force |
| `routine` | accreditation roster movement, or an FR notice with no CFR effect |
| `metadata` | only publication metadata moved |
| `none` | nothing moved — no commit |

Severity leads the PR title **and names the source that moved**, because with
four sources `major` alone would not distinguish a Title 34 amendment from a
DAPIP refresh.

## 8. Failure modes the logic must handle

| failure | required behaviour |
|---|---|
| eCFR returns HTML or an error body instead of XML | no `DIV5` ⇒ abort, no commit |
| eCFR 429s or 5xxs | retry with backoff; a transport error is never a change signal |
| the ledger comes back empty | abort — never read it as "everything was removed" |
| a source is temporarily unavailable | abort the whole run; never commit a partial bundle |
| a title is mid-import | pin no later than `up_to_date_as_of`; carry the flag into the report |
| an unresolved designator sequence | keep the text, flag the sequence, still commit |
| chain fails verification after commit | fail the run loudly; the bad file is not pushed |
| CI signing key not in the registry | commit refuses |

## 8.1 Releases

Each signed version is published as `chain-vN`, carrying the chain, the
registry, and a reproducibly-built archive of the part snapshots. Two
assertions guard it, both of the same shape: **publish nothing that does not
verify**, and **assert the archive's contents against the payload** — one
snapshot per part plus the ledger. An archive carrying fewer snapshots than the
chain signed is worse than a failed release, because it looks complete.

The release notes state the derivation's limits next to its results, for the
same reason the payload carries them.

`verify --json` is the contract the release job reads, so no step in the
pipeline greps prose for a version number.

## 9. Still open

- **Applicability.** Which parts bind which entity — an institution of higher
  education by control, an LEA, an SEA, an accrediting agency, a lender, a
  State VR agency — assembled from each part's scope section and the programs
  the entity participates in. This is curated and signed, not inferred, and it
  is what turns 34 CFR into an answer to "what binds *me*".
- Resolving `you` against the subpart that defines it.
- `statute` and `accreditation` are specified here but not yet wired.
- Receipts: an institution attesting, under its own key, that it discharged an
  obligation against an exact signed version. Evidence by digest only — that
  constraint is sharper here than under FedRAMP, because the evidence behind an
  ED obligation is frequently student records under FERPA.
