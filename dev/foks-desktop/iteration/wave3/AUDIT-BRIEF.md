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
- `dev/foks-desktop/app-surface.html` — interface transcription of the current production application.
- `dev/foks-desktop/iteration/wave1/*.html` and `wave2/*.html`.
- `dev/foks-desktop/GAPS.md` — what FOKS can and cannot do.

## Personas and goals

| # | Persona | Model | Goal they try to reach |
| - | ------- | ----- | ---------------------- |
| 1 | **Priya, engineering lead** at a 12-person startup. Experienced with 1Password Teams. Requires shared deployment tokens and per-environment secrets with explicit read permissions. | Fable | Configure the team on the company server, add two engineers with appropriate roles, restrict production tokens to administrators while allowing team-wide access to staging tokens, and provide onboarding instructions to a new hire. |
| 2 | **Marcus, household organizer**. Non-technical user familiar with iCloud Drive and Apple Passwords. Seeks a shared household group for storing sensitive documents and home credentials. | Opus | Create a "Household" group, add the guest Wi-Fi password and a PDF document, share the group with a partner, and locate and open the document from a secondary Mac. |
| 3 | **Jun, platform and agent engineer**. Operates developer agents using MCP servers. Requires secure secret storage for automated agents with fine-grained inspection and access revocation. | Fable | Store API credentials and database connection strings, grant read access to a `deploy-bot` service identity, inspect and verify the identity's permissions, and revoke access when no longer needed. |
| 4 | **Ade, systems administrator**. Operates self-hosted FOKS infrastructure with hardware security keys and structured backup procedures. Requires verification of trust boundaries, including host pinning, compatibility leases, rollback protection, disaster recovery, and cross-server federation. | Opus | Register a secondary server, probe and pin its host identity, admit an external partner team via federation, register a hardware security key and recovery phrase, and restore account access on a new device. |
| 5 | **Sol, new team member**. First-time user joining an existing organization with minimal prior instructions ("install this and I'll add you"). | Fable | Complete onboarding from initial application launch to accessing a staging token in the Engineering team using only in-app guidance. |

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

The complete audit reports are archived in the `audits/` directory and inform the wave 4 design iterations.
