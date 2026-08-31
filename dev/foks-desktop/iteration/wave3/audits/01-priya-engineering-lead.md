# Audit 1 — Priya, engineering lead

**Persona:** Priya, engineering lead at a 12-person startup; ran the team on 1Password Teams for three years.
**Model:** Fable.
**Goal:** Set the team up on the company server (foks.acme-corp.com), add two engineers with the right roles (one Admin, one plain member), put a production deploy token where only admins can read it and a staging token everyone can read, and write the message I would send a new hire so they can get in.

Screens rendered and read: the app at `?step=start`, `?step=server`, `?step=account`, `?step=team`, `?step=party` (top and scrolled), `?catalog=team`, `?step=items`, `?modal=demote-party`; wave 1 `03-onepassword` (login, new, lease), `02-keychain` (access, empty), `04-apple-passwords` (manage, group), `01-finder` (team); wave 2 `02-teams-hub` (team, add, join, store, lease, create, demote), `03-onboarding` (server, team, done), `01-system-settings` (team), `04-servers-identity` (lapsed), `05-pills-evolved` (parties-members).

## Round two (wave 6)

Priya walked the same goal again against the implemented combination in
`wave6/01-vault.html` and `wave6/02-first-run.html`; her report is
`../../wave6/audits/01-priya-round2.md`. Of her five round-one frustrations,
three are Fixed (Member / Admin / Owner radios on the Manage sheet, the invite
field marked optional with who issues it, and the Parties / Stores vocabulary
with its JSON panel gone), one is Partly (team writes are live and honest under
one amber band, but the sheet no longer says that nothing writes a team item
today, not even a command) and one is Not (there is still no read-role control
anywhere, not even greyed out). Her verdict: she would run her team on this the
day a Who-can-read control ships on the New sheet, because the roster, the roles
and the readable-by answers already beat 1Password Teams, but she still cannot
create the one secret the exercise was about.

## 1. The current app, step by step

I open it and land on **Get started**: three lines in a card — "✓ Secure local state initialized", "○ Add a FOKS server profile", "→ Probe the selected server before account onboarding", "○ Create or resume an account store". Fine, I'll take a checklist. In 1Password I'd have typed my email; here I click "Add a FOKS server profile" and get **Servers**: two fields, "Profile name" and "FOKS probe address", with the sentence "Add a Go-compatible v0.1.9 server, then probe and pin its authenticated host identity before creating an account." I don't know what Go-compatible v0.1.9 means and I don't care. I type `foks.acme-corp.com:443`, press **Add server**, then **Probe selected server**. What comes back is a raw JSON panel with `host_id_hex`, `host_chain_sequence`, `merkle_epoch`. Nothing says "pinned, you're good". Also: **Reset hard state** is the fourth button, the same blue as Add server. I did not press it. I nearly did.

Back to Get started, "Create or resume an account store" — which takes me to a screen called **Stores**, not Account. The form is Local alias / Username / Device / Email / **Invite** (masked, and not marked optional the way Passphrase is) / Passphrase. Where do I get an invite? I run this team; nobody has given *me* one. Nothing on the screen says whether acme needs one or who issues it. I leave it blank and press Create account; the response is `{"username":"rae","user_chain_sequence":1,...}`. I assume that's success.

Now the team. There's no "Teams" in the rail — it's under **Parties**, which I only tried because I had run out of other words. "Create a team store" wants four fields: Owner account alias, Local team alias, FOKS team name, and "Pending team alias". Three names for one team, and I never learn what "pending" is for. I fill in `engineering` everywhere and press **Create named team**. JSON again.

Adding the two engineers is where it stops working. The "Local team members" card has Named team, Username, **Member visibility** — and buttons List roster / Add member / Resume add / Demote to member / Remove member / Resume edit. There is no role picker. The only role I can give someone is Member, and the only change I can make afterwards is *demote*. The hint under the mock even says it: "the desktop's add/demote path currently exposes Member visibility." So Dana as a plain member: yes. Sam as an Admin who can add people while I'm on holiday: no, not from this app, and nothing tells me how else. "Member visibility 0" is never explained — is 0 the most or the least?

The tokens. **Items** shows the Engineering store with `/deploy/production-token` (Read role Admin, Write role Owner) and `/release/bundle.tar` (Member (0) / Admin) — so the *shape* I want clearly exists. But the inspector says, twice, "Team stores are list/read-only in this release." The only "New account-store item" form is for my personal store and states "new items default to Owner read and write roles" — no read-role control at all. So I cannot put either token in the team, and even in my own store I can't choose who reads it. The entire point of my goal is unreachable here, and the app says so only in one grey sentence.

Telling a new hire how to get in: I have nothing to forward. Get started is the only onboarding text; it never mentions usernames, invites, or "ask an admin to add you". **Notifications** says "No actions need attention."

## 2. The same goal against the wave 1 / wave 2 mocks

**Wave 2 `03-onboarding` got me furthest on setup.** Five steps, one question each: Connect a server ("Whoever runs a server cannot read what is in it"), Create your identity (the Username hint reads "How others on foks.acme-corp.com add you to teams" — that one line does more than the whole current app), Protect it, Join or create a team, Done. Step 4's right-hand card, **Ask to be added**, gives a copyable sentence — "Add rae on foks.example.net as a Member" — plus Username and Server rows with copy buttons. That is my new-hire message, written for me.

**Wave 2 `02-teams-hub` got me furthest on people and on "who can read this".** The **Add a member** sheet has Member / Admin / Owner radios, each with one sentence ("Admin: Manages the roster — adds, demotes and removes parties — and reads items whose read role is Admin or below"), and the precondition I needed: "The party must already hold an account on foks.acme-corp.com. Someone who has not signed up there yet cannot be added." The **Store** tab answers my headline question with "Who can read what: Admin — 1 item; Member · 0 — 3 items", each with the avatars of who qualifies. The demote sheet is blunt: "The desktop only lowers a role. Raising one is not offered here." I'd rather be told than left guessing. The **Join a team** sheet — "There is nothing to accept and no link to follow" — is the honest version of 1Password's invite email.

**Wave 1 `02-keychain`'s Access tab** is the best per-item answer: "Read role Admin · Write role Owner" and then a Party / Kind / Role here / Role at source table. **Wave 1 `04-apple-passwords`'s Manage sheet** is the only place that explains visibility in words ("0 is the widest band") and says what an Admin can't do. **Wave 1 `03-onepassword`** looks like home — the "only you" chip on Personal, "shared with N parties" on Engineering — but its New item sheet marks every team vault **DEFERRED** with "Creating into a team vault arrives in a later release; until then, team items are read-only here."

**What still failed in every mock:** I cannot create the production token with read role Admin anywhere. No mock draws a read-role picker on the new-item sheet, not even a disabled one; the fixture's Admin-only token simply exists and no screen says how it got that way. No mock says where an invite code comes from. And in every mock my company server is red — "Nothing on foks.acme-corp.com can be read or changed" — with **Wait for lease refresh** as a disabled button. The copy is clear; the helplessness is complete.

## 3. Frustrations and things that worked

**Top five frustrations**
1. Team writes are deferred, so neither token can be put in Engineering from the app or any mock; the current app says it in one grey sentence and nothing says what to do instead.
2. No way to make anyone an Admin: the app only adds Members and only demotes. The hub mock adds Admins but still can't promote.
3. No read-role control anywhere, so "only admins can read this" cannot be *set* — only observed on items that already exist.
4. Invite: a masked, required-looking field on Create account with no word about where it comes from or whether my server needs one.
5. The current app's vocabulary and layout: Parties for teams, Stores for accounts, "Pending team alias", JSON as the result of every action, Reset hard state in the same blue as Add server.

**Top five things that worked**
1. Teams hub's Store tab: "Who can read what" with counts and avatars per read role.
2. Onboarding's "Ask to be added" card with the copyable sentence and username/server rows.
3. The Add-member sheet's role descriptions plus "must already hold an account on foks.acme-corp.com".
4. Keychain's Access tab table (Role here / Role at source / Party id) — precise without prose.
5. The lease-lapsed copy: "Reads stop with writes... items are not listed rather than shown stale." I trust a tool that says this.

## 4. Three changes I would ask for

1. "When I make a new secret, give me a **Who can read** dropdown right there — Everyone in the team / Admins only / Owners only — and a **Who can change** one under it. If that isn't shipping yet, show it greyed with 'coming later' and *tell me the command that does it today*. Don't just print 'list/read-only in this release'."
2. "Put **Teams** in the sidebar and let me pick **Admin** when I add someone. If the app can only lower roles, say 'Priya can't promote from here yet — an Owner does this from the command line' in the sheet, the way the demote sheet already does."
3. "Give me a **'Send to a new hire'** button on the team page that produces: install link, server address, 'create your account with username firstname.lastname', 'then tell Priya your username'. And on my Create account screen, mark Invite as optional or tell me where to get one."

## 5. Where a screen told me something wrong or missing

- **CLI-only first step is contradicted.** GAPS.md #17 says keystore init and profile-add are CLI-only and that Get started should "state the real order and the real commands." The app's Get started shows "✓ Secure local state initialized" and an in-app "Add a FOKS server profile"; wave 2 onboarding says "Every step is an operation this Mac's agent already has." One of these is wrong, and no screen mentions a CLI at all.
- **Federation roles.** The app's Federated team admission card says "CLI and agent clients can explicitly select admin or owner." GAPS #26 says the agent *rejects* Admin/Owner for federation roles. The sentence is false as written.
- **Visibility direction.** Apple Passwords' Manage sheet says "0 is the widest band." BRIEF §2 and GAPS #3 say only that visibility is an *ordered* band; neither says which way. Unverified claim, and the only explanation of visibility anywhere.
- **Admin limits.** Apple Passwords: "Admin ... Cannot change other admins or owners." Not in BRIEF or GAPS; plausible, unverified.
- **Invite semantics.** The app treats Invite as required-looking; onboarding says "optional"; servers-identity says "if the server requires one." Nothing states whether acme requires one or where an admin obtains one. Missing, not wrong.
- **Demote fixture.** The app's sheet reads "Demote sam in engineering to Member (0)?" while its own roster JSON shows sam already at Member visibility 0 — a demotion to the same role, so the confirmation doesn't reflect what "strictly lowers" means.
- **Read-role creation.** Every screen shows read/write roles per item (correct per BRIEF §2) but none says roles are set at write time and cannot be edited on a team item; the fixture's Admin-only token implies a capability no surface exposes.
