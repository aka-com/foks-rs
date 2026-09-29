# Design: *Inside foks-rs* — a book about the algorithms, data structures, and techniques in foks-rs

Date: 2026-09-25

## Intent (from the request)

Write a book about the `foks-rs` repository (`../foks-rs`) that focuses on the
**algorithms, data structures, and other techniques** the project uses rather
than the nitty-gritty code. It must be accessible to someone with a **modest
computer science background** and must **cite sources** describing the
techniques where appropriate.

## Assumptions made (the user was not available to confirm)

1. **Format: Quarto book** (converted from mdBook at the user's request on
   2026-09-25). The book lives in this directory as a Quarto project
   (`_quarto.yml`, one `.qmd` file per chapter, `index.qmd` as the preface).
   The source is plain Pandoc Markdown readable without any tooling.
   `quarto render` produces `_book/`.
2. **Language and voice:** English, third person, technically precise, plain.
   Follows the upstream repo's own writing guidance (AGENTS.md): no mannered
   prose, no anthropomorphic copy, no unnecessary analogies.
3. **Code references are deliberately light.** Each chapter ends with a short
   *Where to look* section naming the crates and modules that implement the
   ideas, without line numbers (they rot) and without code listings, except
   for a few short illustrative snippets or formulas where words would be
   worse.
4. **References** are given per chapter under *Further reading* and collected
   in a bibliography appendix. Standards (RFCs, FIPS), original papers, and
   authoritative documentation are preferred over blog posts.
5. **Ground truth is the code, not the repo's marketing docs.** `docs/GUIDE.html`
   and `docs/INTRO.html` are used for narrative but every parameter, constant,
   and claim in the book is checked against the Rust source by the research
   pass. Where the docs and code disagree, the code wins and the discrepancy is
   noted in the research notes.
6. **Scope:** the Rust repository as of its latest commit
   (`2ff09187`, 2026-09-22). Upstream Go FOKS is discussed only as the
   compatibility target.

## Audience

A reader who knows what a hash function, a public key, and a binary tree are,
but has not necessarily studied cryptography or distributed systems. Each
chapter introduces the general technique before showing how FOKS applies it,
and an appendix primer covers the primitives (hashes, signatures, AEAD, KEMs,
Merkle trees, canonical encodings) for readers who want them.

## Structure

Five parts, about two dozen chapters of 1,500–3,500 words each, plus
appendices. Final outline is in `_quarto.yml`; the working outline is:

- **Part I — Orientation:** the problem and threat model; the shape of the
  system (three tiers, two wire protocols, crate map).
- **Part II — Secrecy:** key families and derivation; hybrid post-quantum
  encryption; generational keys and forward secrecy; human-exact secret
  phrases (backup and pairing); hardware keys; the local keystore.
- **Part III — Authority:** canonical encoding and typed hashing; signature
  chains; the Patricia Merkle tree; epochs and the skip-pointer DAG;
  client-side verification and anti-rollback.
- **Part IV — Data:** the encrypted key-value store; teams, roles, invitations
  and federation; real-time chat.
- **Part V — Systems engineering:** the server's single-writer SQLite design;
  the resident agent (isolation, idempotent and resumable operations); the
  desktop (two-process model, app lock, memory hygiene); MCP and durable write
  recovery; OIDC/SSO; compatibility engineering against the Go oracle;
  performance measurement.
- **Appendices:** primer on primitives; glossary; bibliography; crate map.

## Method

1. Research pass: six parallel agents survey the code by area and write notes
   with file:line evidence and candidate references to the scratchpad.
2. Spec (this file) and a written implementation plan.
3. Writing pass: chapters written part by part from the research notes,
   re-checking constants in the source, following the style guide below.
4. Review pass: a technical reviewer agent checks every chapter against the
   code for factual errors; an editorial pass checks consistency of terms,
   cross-references, and reading level.
5. `quarto render` must succeed with no broken internal links.

## Style guide for chapters

- Open with the problem, then the general technique, then FOKS's specific
  use of it, then trade-offs and why the design was chosen.
- Define every term at first use; the glossary repeats the definition.
- Real numbers from the code (bit counts, byte sizes, constants) go in tables
  or on their own line, not buried in prose.
- Diagrams as ASCII in fenced blocks where a picture helps (tree shapes,
  message flows). Keep them small.
- Formulas in `$...$` (Pandoc math, rendered with MathJax) only when they clarify.
- Each chapter ends with **Where to look** (crates/modules) and **Further
  reading** (references, formatted `Author, *Title*, venue, year, URL`).
- Never invent a reference. Uncertain references from research notes are
  verified or dropped.
- Headings: `#` chapter title, `##` sections, `###` sparingly.

## Out of scope

- Instructions for building, running, or contributing to foks-rs.
- Line-by-line explanation of code or API documentation.
- The React component structure of the desktop UI beyond the security model.
