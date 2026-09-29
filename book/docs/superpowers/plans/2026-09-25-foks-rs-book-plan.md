# Plan: writing *Inside foks-rs*

Spec: `docs/superpowers/specs/2026-09-25-foks-rs-book-design.md`.
Outline: `_quarto.yml` (originally `src/SUMMARY.md` under mdBook; converted to Quarto 2026-09-25) (file names are fixed; cross-references use them).

## Inputs

Research notes in the session scratchpad, each with file:line evidence and
[VERIFY]-flagged references:

- `research-crypto.md` — Part II
- `research-protocol.md` — chapters 09, 17, 23
- `research-merkle.md` — chapters 10–13
- `research-client.md` — chapters 13, 14, 16, 19
- `research-server.md` — chapters 15, 16, 18, 22
- `research-desktop-overview.md` — chapters 02, 20, 21, 23, 24, appendix D

## Tasks

1. [x] Scaffold: `book.toml`, `src/SUMMARY.md`, `src/preface.md`.
2. [x] Part II chapters 03–08 and Appendix A primer (writer agent, from
       `research-crypto.md`).
3. [x] Part III chapters 09–13 (writer agent, from `research-merkle.md`,
       `research-protocol.md`, `research-client.md`).
4. [x] Part IV chapters 14–16 (writer agent, from `research-client.md`,
       `research-server.md`).
5. [x] Part V chapters 17–24 (writer agent, from `research-protocol.md`,
       `research-server.md`, `research-client.md`,
       `research-desktop-overview.md`).
6. [x] Part I chapters 01–02 and Appendix D crate map (written last so they
       can point forward accurately).
7. [x] Appendix B glossary and Appendix C bibliography compiled from the
       chapters' definitions and *Further reading* sections; every
       [VERIFY] reference checked or dropped.
8. [x] Technical review: reviewer agents check each chapter against the
       source for factual errors and fix them.
9. [x] Editorial pass: consistent terminology, cross-references resolve,
       reading level, style guide compliance.
10. [x] `mdbook build` succeeds (later `quarto render`); no missing files, no broken internal links.

Each writer agent receives the spec's style guide, the outline, its research
notes, and the instruction to re-check every constant against the source.
