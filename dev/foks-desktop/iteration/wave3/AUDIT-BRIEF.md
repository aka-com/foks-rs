# Wave 3 — persona audits

Before the wave 3 and wave 4 mocks were drawn, the application and the earlier
mocks were walked by simulated users. Each auditor was a separate model
session given one persona, one goal, and the material below; it was told to
attempt the goal by reading the application's screens (the as-built
transcription `../../app-surface.html` and the GPUI source in
`crates/foks-desktop/src/gui.rs`), then to try the same goal against the wave 1
and wave 2 mocks, and to report as that person would.

## Material every auditor received

- `dev/foks-desktop/iteration/BRIEF.md` — the object model and wire rules.
- `dev/foks-desktop/app-surface.html` — the application as it is.
- `dev/foks-desktop/iteration/wave1/*.html` and `wave2/*.html`.
- `dev/foks-desktop/GAPS.md` — what FOKS can and cannot do.

## Personas and goals

| # | Persona | Model | Goal they try to reach |
| - | ------- | ----- | ---------------------- |
| 1 | **Priya, engineering lead** at a 12-person startup. Has used 1Password Teams. Wants shared deploy tokens and per-environment secrets with clear "who can read this". | Fable | Set up the team on the company server, add two engineers with the right roles, put a production token where only admins can read it and a staging token everyone can read, and tell a new hire how to get in. |
| 2 | **Marcus, household organiser**. Non-technical. Uses iCloud Drive and Apple Passwords. Wants a shared family folder for documents and the Wi-Fi password. | Opus | Create a "Household" group, put the guest Wi-Fi password and a PDF in it, share it with a partner, and later find and open the PDF from another Mac. |
| 3 | **Jun, agent/platform engineer**. Runs Claude Code and Codex with MCP servers. Wants API keys stored in FOKS and handed to agents at run time, with a way to see and revoke what an agent can reach. | Fable | Store an Anthropic API key and a database URL, make them readable by a `deploy-bot` identity that an agent uses, understand exactly what the bot can and cannot read, and revoke it. |
| 4 | **Ade, security-minded self-hoster / small-company admin**. Runs the FOKS server. Comfortable with YubiKeys and backups. Wants to know the trust story: host pinning, leases, rollback, recovery, and federation with a partner company's server. | Opus | Add a second server, probe and pin it, admit a partner team into an internal team via federation, enrol a YubiKey and a backup phrase, and recover on a new Mac. |
| 5 | **Sol, first-time user with no context**. Was told "install this and I'll add you". | Fable | Get from first launch to "I am on the Engineering team and can see the staging token" with nothing but the app. |

## What each auditor reports

1. A step-by-step transcript of the attempt against the current app: what they
   clicked, what they expected, what they got, where they were lost, in
   first person.
2. The same goal against the wave 1 / wave 2 mocks: which mock (or which
   elements of which mocks) got them furthest, and what still failed.
3. Their top five frustrations and top five things that worked, ranked.
4. Three concrete UI changes they would ask for, written as they would
   phrase them (not as designers).
5. Any place the app or a mock told them something that turned out to be
   wrong or missing about FOKS (checked against BRIEF.md and GAPS.md).

The reports are saved verbatim under `audits/` and drive the wave 4 mocks.
