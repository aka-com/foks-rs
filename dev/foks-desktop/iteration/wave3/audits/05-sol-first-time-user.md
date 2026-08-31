# Audit 5 — Sol, first-time user with no context

**Persona:** Sol. A colleague, sam.ortiz, said "install this and I'll add you". I have never heard of FOKS. I do not know what a server, a profile, a probe, an alias, a party or a role is in this app.
**Model:** Fable.
**Goal:** get from first launch to "I am on the Engineering team and can see the staging token", using nothing but the app.
**Screens rendered:** app steps 1–7 plus Items (empty), Notifications, Parties, Items (team); first screens of all ten mocks, all nine wave2/03 states, and the join/get-started pages of wave1/01, 02, 05 and wave2/01, 02, 04, 05.

## Round two (wave 6)

Sol walked first launch again against `wave6/02-first-run.html` and
`wave6/01-vault.html`; the report is `../../wave6/audits/05-sol-round2.md`.
Four of the five round-one frustrations are Fixed (the screen says the server
address comes from sam and hands over the sentence to ask for it; Attention
carries the wait; the invited path has no create step, so the trap is gone; the
Invite field sits under OPTIONAL with who to ask) and one is Partly (Waiting has
Check now, but the operation that finds a group someone else added you to does
not exist yet, so it can only answer "Not yet"). The verdict: yes, because every
question that had to be messaged to sam last time is now answered on the screen
where it arises, and the only thing in the way is the app admitting, in yellow,
that it cannot yet see the group sam put him in.

## 1. Transcript against the current app

**Launch.** "Starting FOKS — Connecting to the private local agent…". I don't know what an agent is; there's nothing to press, so I wait.

**Bootstrap.** "Set up this FOKS client. The local agent is ready for its initialize-state bootstrap step." One button, **Initialize secure state**. I press it because it is the only button. I have no idea what I just did.

**Items.** I land on Items: "No catalog has been loaded." The rail has seven equal entries; "Get started" is third, not first, not highlighted. I click it because it's the only phrase written for a person.

**Get started.** A card "Set up this FOKS client" with four lines: "✓ Secure local state initialized", "○ Add a FOKS server profile", "→ Probe the selected server before account onboarding", "○ Create or resume an account store". Three of the four contain a word I'd have to search for (profile, probe, account store). Nothing on this screen mentions a team, sam, or that I'll need something from sam. I click the second line.

**Servers.** "Add a Go-compatible v0.1.9 server, then probe and pin its authenticated host identity before creating an account." Two fields: Profile name (`local`), FOKS probe address (`foks.example.net:443`). Four identical blue buttons: Add server, Probe selected server, Remove selected server, **Reset hard state**. The example looks like a real address; I'd honestly have typed `foks.example.net:443` and pressed Add server. Nothing says "ask whoever invited you for this". Message to sam: "it wants a 'FOKS probe address', what is that?" Sam says `foks.acme-corp.com`. Do I add `:443`? Only the placeholder suggests so. I leave Profile name as `local`. Add server → a JSON blob. Probe selected server → a bigger JSON blob (`host_id_hex`, `merkle_epoch`…). No sentence says "this worked"; I assume it did because nothing is red.

**Stores → Create account.** "The invite and optional passphrase go only to the private local agent; passphrase fields are consumed when submitted." Fields: Local alias `personal`, Username `rae`, Device `MacBook`, Email (optional), **Invite** (masked dots, not marked optional), Passphrase (optional). I type username `sol`. Local alias — is that also my name? I leave `personal`. The Invite box looks mandatory and I have nothing to paste. Second message to sam: "do I need an invite code?" Sam says no; I press Create account and get `{"username":"sol","user_chain_sequence":1,"directories":1,"entries":0}`. I only know sam adds people by username because sam told me.

**Back to Items.** One group "personal — Account store", empty, with a "New account-store item" form. No Engineering. Nothing says "waiting for sam". **Notifications**: "No actions need attention. Interrupted or resumable operations will appear here." I expected sam's invitation to show up here, so this sentence actively misleads me. **Parties**: a form "Create a team store" with `engineering` pre-filled in all four fields and a blue **Create named team** button. This is the trap: it looks like the way to get Engineering, and would create my own team of that name, owned by me. Below it, "Local team members — Username: sam — Add member" looks like I should add sam. I press List roster, get an error, and stop.

**What I actually do.** Message sam: "I made an account, username sol on foks.acme-corp.com. I don't see anything about Engineering." Then I sit. After sam adds me, nothing tells me to press anything. If I happen to press the blue **Refresh** on Items, "Engineering — named team · engineering · active" appears above personal, I click `/deploy/staging-token`, press **Reveal value**, and the goal is met. Every step of that I found by luck or by sam.

Score against the four key questions: server address from sam — not said; added by username, can't join myself — not said; nothing appears until added, and what it looks like — not said; what to do while waiting — not said.

## 2. The same goal against the wave 1 / wave 2 mocks

**wave2/03 Onboarding got me furthest, by a wide margin.** Step 1 "Connect a server" says under the field "The address as its administrator gave it to you" — the first sentence anywhere that tells me to ask sam. Step 2 says under Username "How others on foks.example.net add you to teams", and marks Signup invite "optional — paste it if the administrator gave you one". Step 4 has an "Ask to be added" card: "An Admin or Owner of the team adds you by username. Send them exactly this: 'Add rae on foks.example.net as a Member'" with a Copy button, and "The team appears here on its own once they have added you." Three of my four questions answered on one screen. What still failed: Step 0 reads like a log file ("InitializeState creates the native credential and the rollback boundary"); Step 3 "Protect it" sits between me and the team, and "Skip for now" jumps to the checklist rather than to Step 4; the Done pane only knows how to say "Household · you are its Owner" — there is no Done for someone who asked to be added and is waiting; and "appears here" doesn't show *where* or what it will look like. The checklist's last row is "Create or join…", not "Waiting for sam.ortiz to add sol".

**wave2/02 Teams hub** is the best answer to "why isn't it here yet". The card "Not seeing a team you expect? FOKS has no join button. An Admin or Owner of that team adds your username on the server where the team lives; then it appears here after the next sync" and the **Join a team** sheet ("There is nothing to accept and no link to follow", "Your usernames" per server, "What to send them" with Copy, "the alias you chose on this Mac is not it") are exactly what I needed on day one. But the page assumes I'm already set up, and "after the next sync" gives me nothing to press.

**wave1/02 Keychain** is the only wave 1 mock with a **Join a team…** button on its first screen, and its sidebar line "Teams you create or are admitted to appear here as their own stores" is the only place that tells me what "appears" looks like. **wave1/05 Drive** has "Join or create a team" in the rail: "Nobody joins one on their own: a team admin adds you as a party, by your username on that server" — good, though "party" is unexplained. **wave2/01** has one useful line ("This account is on no team yet. Create one or be added under Teams"). **wave2/04** separates "Username" from "Alias on this Mac" and marks Invite "if the server requires one" — that alone would have saved me a message to sam. **wave1/01**'s Get started has no join copy; **wave1/03**'s Get started is a toast; **wave1/04** shows "Shared Groups +" with a Manage button, which made me think I could add myself; **wave2/05** is the app's checklist with a progress bar and the same vocabulary.

## 3. Top five frustrations and top five things that worked

Frustrations, ranked:
1. Nowhere in the app says I need an address from sam, and the placeholder `foks.example.net:443` looks like a real one to type.
2. Notifications says "Interrupted or resumable operations will appear here" — I waited there for an invitation that can never come.
3. Parties opens on **Create named team** with `engineering` pre-filled; the obvious button would make a second team.
4. The Invite field looks required; the app never says whether sam's server needs one.
5. After sam adds me, nothing tells me to press Refresh.

Things that worked, ranked:
1. wave2/03 "Ask to be added" card with the exact sentence to send and a Copy button.
2. wave2/02 "Not seeing a team you expect? FOKS has no join button."
3. wave2/03 "The address as its administrator gave it to you" under the server field.
4. wave1/02 "Teams you create or are admitted to appear here as their own stores."
5. wave2/04 splitting Username from "Alias on this Mac", and Invite "if the server requires one".

## 4. Three concrete UI changes I would ask for

1. "On the first screen after it finishes starting, ask me one question: 'Did someone invite you, or are you setting this up yourself?' If I pick invited, say right there: 'You'll need the server address from them. They add you by the username you pick next. You can't add yourself.'"
2. "When my account exists and I'm on no team yet, show me a plain page: 'Waiting for sam.ortiz to add sol on foks.acme-corp.com', a Copy button for the sentence to send, a 'Check now' button, and 'When they've added you, Engineering will show up here as a group above Personal.' Put the same thing in Notifications instead of 'No actions need attention'."
3. "Take 'Create team', 'Reset hard state' and the raw JSON off the path I walk on day one. If a button can make a second Engineering or wipe something, it shouldn't be the same blue as Add server."

## 5. Where the app or a mock told me something wrong or missing about FOKS

- **App, Notifications:** "Interrupted or resumable operations will appear here" is true (BRIEF §2) but omits that being added to a team never produces a notification — the one thing a newcomer waits for.
- **App, Create account:** the Invite field is drawn as a masked secret with no "optional". GAPS #26 confirms CreateAccount works in-app; neither BRIEF nor GAPS says when an invite is required. wave2/03 says "Servers open to anyone don't need it", wave2/04 says "if the server requires one", the app says nothing.
- **App vs GAPS #17:** GAPS says keystore init and profile-add are CLI-only and "the first step has no in-app equivalent yet", yet the app offers **Initialize secure state** and **Add server**, and wave2/03 draws both in-app with "No PROPOSED bands were needed". One of these is wrong; if GAPS is right I have to run `foks-rs --state-dir <D> init` and nothing told me.
- **How a team appears:** "appears here on its own once they have added you" (wave2/03) versus "after the next sync" (wave2/02) versus the app, where listing is a live network call per view (GAPS #11) behind a manual Refresh. BRIEF §2 says "nothing refreshes on its own", so "on its own" overpromises; neither BRIEF nor GAPS states what a newly-added member must do on their Mac for the team to be listed.
- **wave2/03 Protect it:** "Asked for when FOKS starts and after it has sat idle" describes an idle lock. Nothing in BRIEF or GAPS describes one; wave1/02 says outright "nothing here locks or unlocks."
- **wave2/03 existing-account pane:** "finish on a Mac or phone that already holds the account" — no phone client appears in BRIEF, GAPS or the app; the app's equivalent is "Provision owner device" run from the existing device.
- **wave2/03 Done pane:** for someone who chose "Ask to be added", the summary would still print "Team · Household · you are its Owner", which would be false. A "Waiting to be added" outcome is missing.
- **wave1/04 Apple Passwords:** "Shared Groups +" and a Manage sheet that adds people with a role picker implies a self-serve group model; BRIEF §2 and wave2/02 are clear that only an Admin or Owner adds a party, by username, and team writes and role edits are deferred this release.
- **App, Parties:** "Create a team store" pre-filled with `engineering` is not wrong about FOKS, but it makes creating a team named like the one I'm waiting for the default action. Every mock except wave2/02 and wave2/03 leaves the same trap open in a softer form.
