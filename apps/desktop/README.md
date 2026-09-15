# FOKS desktop

The web half of `foks-desktop`, with its own trust boundary, release train, and
command surface.

In production, the application selects the Tauri bridge, loads the catalog once,
validates all responses at runtime, and passes opaque store references back unchanged. Show loads one item at its exact version and keeps the value only in the open
details panel until Hide, selection change or window blur. Link destinations
load automatically at the selected version and Open target navigates in one
click. Copy value and Download stay in Rust. List rows use
`apps/desktop/kit/virtual-list.ts`; cards are capped at 200 because the virtual list
does not model a wrapping grid. Account-store creates use must-not-exist;
edits, removes and file replacements carry the catalog's exact version.
List rows show Name and Shared readers; the catalog has no write timestamp, so
the UI does not invent a Modified value from its version counter. Folder view
derives a folders-only tree from the filtered catalog, and search temporarily
returns to the flat list without changing the selected New-item destination.
Active authenticated groups can create the same four product kinds as account
stores. Group creates carry explicit read and write roles, with their reader
preview computed from the selected role and live roster. Text edits, streamed
file replacements and removals preserve the catalog roles and carry the exact
listed version. Because v0.1.9 stores files of at most 2,040 bytes in its
inline `small-file` encoding, a value that fails the explicit text read is
presented with Download and native Replace actions without changing the wire
or database format.

A group's page is opened from the Teams list and loads the agent's roster and
federation facts in one call. Its header is the group mark, its name and
"server · Role"; it has two tabs, Members and Settings, drawn as a tablist
whose arrows walk the strip and whose open tab names the panel below it.
Members splits the one roster into People, Machines and Groups on other
servers: a row carries the
name, a "you" chip, the kind of party as its only second line, and one role
chip with the visibility band inside it ("Member (0)"). Each row's menu holds
that row's actions and nothing else — an action that does not apply stays, and
is marked `aria-disabled` rather than `disabled` so the keyboard reaches it and
reads the reason in its `title` — and an admitted group's row adds its
admission state, Restore access and Remove admission. A roster party that names
another group is listed under Groups on other servers rather than dropped,
once: with a "No admission record" chip where no record on this Mac matches it,
and an "Ambiguous admission" chip where several do, in which case those records
are not listed again beside it. Add someone on `<server>`… and Invite someone…
follow the people and machine rows and go with them when the roster could not
be read; Add a group… follows the admitted ones. Settings states the group's
name with "A name is fixed at creation.", and a Who can join row whose only
value is Invite only, with the reason nothing can be chosen read under it and
Change… inert rather than disabled. Group
creation, discovery and account-specific invite messages live on the Teams page
itself; joining is the sheet the Teams header opens, and the Members tab no
longer repeats it inline. A group that needs attention says so on its own row,
not a second time under it. Member changes are restricted to unique, locally
manageable usernames.
An inactive or ambiguous federation admission has no extra payload. Active ad-hoc
groups retain read-only roster facts but suppress member and federation actions.
Native clipboard hygiene uses `copy_text`. First run is a location
inside the same main window, with the rail replaced by the setup-step list.
Its versioned local checkpoint contains only nonsecret progress and display facts;
invite, passphrase, recovery phrase and prepared backup phrase values are held
only in their live form or one-time sheet and are never encoded. Reopening
preserves acknowledged-but-unloaded accounts and uncertain account operations.
The v3 checkpoint retains the existing storage key and reads v2 checkpoints;
it adds nonsecret operation identifiers, SSO progress, and a selected group.
Native account-operation receipts in `account-operation-receipts-v1` tie each
attempt to its client state ID, pinned host, account alias, and operation
inputs.
An acknowledged receipt permits loading the account; a missing or uncertain
receipt never permits replaying creation or treating an existing alias as proof
of success. If a receipt cannot be read but the alias already exists on the
host, setup prompts you to continue with that existing account instead of
linking it automatically. Leaving setup does not cancel an in-flight account
operation.
Reopening
queries the agent's authenticated pending-operation list before showing a
Resume action. Set up again returns to the first question without discarding
completed steps. Creating the first group advances only after a fresh catalog
contains exactly one matching active group with the expected entity kind; its
authenticated name, alias and id become the Done screen's vault target.

Servers use passive signed status plus explicit Check. A server's page keeps
its check-in, the accounts and groups on it, its pinned identity and the
response inspector behind one disclosure. The UI displays verified status from
the latest server snapshot without synthetic or inferred metrics such as
estimated check times, trust durations, or network latency. The audit log reads
the chain length and checkpoint number the agent reports rather than turning
either into a date. A signed expiry at or before the current clock stops the whole
profile when its pinned protocol requires a compatibility lease. A missing or
unreadable required expiry also fails closed under the distinct “signed
check-in unavailable” state; v0.1.9 profiles explicitly report that no lease is
required rather than manufacturing an expiry.
Reset uses a one-use preview token and names computed resumables and local
artifacts before typed confirmation. Devices exposes authenticated devices,
paper-key enrollments, Start/Accept/Finish/Resume pairing and the YubiKey
lifecycle; Settings exposes the account passphrase and card credentials.
Secret phrases, PINs, unlock codes and
passphrases remain live-form state only and are cleared when dispatched,
cancelled, concealed or unmounted.

**Account identity model:**

- **alias**: Profile-local display label.
- **profile**: Server configuration reference.
- **StoreRef**: Canonical application store identifier (`id` on `Store`, `store` on `Account`). Two profiles
  may both hold an account aliased `personal`, so People, Devices and Join
  carry the StoreRef — `?state=devices&store=<StoreRef>` — and resolve
  exactly against it. Such an address with no store names this Mac's first
  account and is rewritten to that account's StoreRef; a StoreRef that no longer
  resolves is reported as unavailable rather than silently replaced by another
  account, and reordering the catalog cannot change which account an address
  means. Switching accounts closes any open sheet and drops the device, backup
  and key lists that belonged to the previous one. The shell cancels stale
  asynchronous responses when switching accounts, ensuring responses from a
  previous account do not render in the newly selected account’s view. Alias
  comparisons survive only where the native command takes `(profile, alias)` —
  the backup-phrase and recovery operations — and there the profile comes from
  the selected StoreRef. A StoreRef is opaque and is never drawn: the agent
  answers with its own identifier, which the fixture happens to write as
  `acct:<alias>` and the real agent writes as a JSON object, so a surface that
  needs to name an account draws the username, the alias and the server
  instead. The address is the one place a ref is written, through the
  `?store=` parameter the location codec has always encoded.

**Unsupported operations:** Link editing is not supported directly; links must be
deleted and recreated. Role promotions require removal and re-addition with key
rotation. Un-admit and invite-link operations are not supported.
The live bridge now loads each group roster and federation
admission, so Sharing and every readable-by count are computed from agent
responses rather than fixture facts.

## Running it

```sh
npm run dev:frontend # Vite dev server on http://127.0.0.1:1421 (strict)
npm run build:frontend # production bundle into apps/desktop/dist/
npm run test:foks-ui      # serial node:test via tsx — model, location, invariants, render
npm exec -- playwright-core install chromium # one-time browser setup
npm run acceptance:foks-ui # build with the mock, then drive Chromium at 1280×860
tsc --noEmit -p apps/desktop/tsconfig.json
```

The development server uses port **1421**.
`npm run dev` builds and supervises the local server, agent, and Tauri
app. `npm run dev:tauri` and `npm start` run the Tauri layer
and its frontend without supervising a server or agent.

## The bridge, and the mock

Everything the webview knows arrives through `src/bridge.ts`. Two things there
are decisions, not details:

- **`invoke` is imported from `@tauri-apps/api/core`.** `withGlobalTauri` is
  **false** for this app, so there is no `window.__TAURI__` — publishing the
  whole IPC surface on a global object increases the blast radius of script
  execution in a secrets app. `tests/react-boundary.test.ts` fails the build
  if `__TAURI__` appears anywhere in first-party source.
- **The mock is used** only for an explicit `VITE_FOKS_MOCK=1` build or a
  non-native Vite development page. A Tauri development window still uses the
  real commands. An ordinary production build outside Tauri fails closed, and
  Vite removes the mock and fixture graph from that bundle. Playwright uses the
  explicit mock build:

  ```sh
  VITE_FOKS_MOCK=1 npm run dev:frontend
  ```

  `src/mock-bridge.ts` uses `src/fixture.ts` with no network or timers;
  it loads only the app's versioned nonsecret first-run checkpoint so a mocked
  quit/resume walk includes completed setup facts. It is
  reached only by dynamic `import()`. `app-root.tsx` calls
  `selectBridge`, checks the Rust app lock, and calls `loadSnapshot` only after
  an armed lock has been authenticated; only an explicitly mocked browser gets
  the fixture snapshot. A native load constructs its snapshot solely from
  validated command responses, loads passive signed status before exposing
  catalog facts, and then loads group rosters and federation admissions only
  for available profiles. Status notices come only from typed catalog failures,
  typed status failures, or the absence of a usable signed lease expiry. Device
  facts load only for available accounts in the relevant Settings surface.

The Rust contract uses Rust field names in responses and camelCase Tauri
arguments. `list_catalog` returns profiles, stores, items, failures and
`blockedProfiles`; `list_servers`, `list_parties` and `list_federation` supply
the other facts needed by the Phase 2 shell; `read_item`,
`copy_item_value`, `copy_item_path` and `download_file` take
`{storeId, path, version}`. `tests/bridge.test.ts` pins the decoder side and
the Rust command tests pin serialization so a drift fails on one side or the
other.

Phase 3 adds `create_text_item`, `create_link`, `edit_text_item`,
`remove_item`, dropped/native-picker file import and exact-version file
replacement, and `resume_group_creation`. Rust-originated drop events contain
paths only; file bytes never cross into the renderer. The shell polls
`take_agent_connection_loss`; Retry calls `retry_agent_connection` and then
reloads the catalog, never replaying the interrupted write.

Phase 4 adds `list_accounts`, `create_group`, member add/demote/remove and their
resumable continuations, federated admission/re-run, and `copy_text`. Each
mutation is followed by a fresh catalog/roster load; an ambiguous result is
never replayed. Group creation supports named and ad-hoc groups, while active
ad-hoc groups are read-only for roster and federation management.

Phase 5 adds `initialize_client_state`, `check_and_add_profile`, authenticated
pending-operation reads, account create/resume, passphrase protection, one-time
owner-backup prepare/commit, owner recovery/resume and `discover_groups`.
Explicit authenticated discovery is wired behind Check now. Waiting retains a
Proposed band for automatic product support, and makes clear that nothing runs
while FOKS is closed or silently in the background. First-run profile checks
return the selected profile and pin as typed facts learned by that check. A
saved profile at the same normalized host and port is checked through its
existing trust policy; otherwise the check publishes a new WebPKI profile. The
checked screen keeps the New / Same / Advanced result vocabulary explicit.
Recovery still precedes pairing in first run; Accept and Resume are wired
directly there so a new Mac does not need an account store first.

Phase 6 adds passive/explicit server status, add/forget/reset, authenticated
device and backup reads, guarded device removal, pairing, account passphrase
commands, and the YubiKey lifecycle. `app_info` supplies the exact build
version and local agent socket; the UI neither invents a protocol version nor
offers a healthy-agent restart. Every response has a command-specific runtime
decoder, including the canonical 68-character Yubi identity.

Phase 7 group-item integration extends the create commands with paired
`readRole` / `writeRole` inputs for group stores; account-store creates omit
both and retain their native Owner defaults. `create_link` is the product Link
operation over a protocol symlink. Folder creation exists at the native
boundary for protocol completeness but is not exposed as a fifth product kind.
Edit, replacement and removal send no role arguments and preserve the roles
authenticated in the catalog.

## Layout

| Path                       | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/main.tsx`             | Entry point. One window; mounts `src/app-root.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `src/app-root.tsx`         | App shell: window, rail, screen, details panel, deep links.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/components/`          | Components: Button, Chip, Tag, Badge, KindIcon, Inset, SectionLabel, Notice, Band, SegmentedControl, SplitButton, MenuButton, SearchField.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/shell/`               | Window chrome: `sidebar.tsx` (the rail), `page-header.tsx`, `toolbar.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/screens/`             | Screens: `items-screen.tsx` (list, cards, notices, empties), `groups-screen.tsx`, `store-access.tsx` (the shared unavailable-store takeover and All items summaries), `first-run-screen.tsx`, `servers-screen.tsx` (`ServersSection`, the server list Settings draws, and one server's page), `settings-screen.tsx` (the one Settings page), `devices-screen.tsx` (one account's Macs and device keys, paper keys and security key enrollments, and one key's own page), `device-sheets.tsx` (the sheets Devices and Settings share), `device-model.ts` (the four per-account key calls and the one row model People and Devices share), `account-switcher.tsx` (the switcher People, Devices and Settings share), `details-panel.tsx`, `write-workflows.tsx`, `edit-value.ts`, `people-screen.tsx` (the attention list and the account profile: teams, devices, keys and the account rows), `files-screen.tsx` (the roots page), `teams-screen.tsx`, `chat-tab.tsx` (the Chat tab: the inbox column, the open conversation and the info panel), `chat-teams.tsx` (the cross-team inbox column), `chat-new.tsx` (the two-step New chat sheet), `chat-screen.tsx` (one team's conversation), `chat-info.tsx` (the channel info panel), `scope.ts` (what is listed, in what order). |
| `src/bridge.ts`            | The typed `Bridge` interface and the Tauri implementation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/mock-bridge.ts`       | The same interface, using the fixture.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/fixture.ts`           | The stable desktop fixture, as typed data.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/model/`               | The pure model — roles, kinds, readers, format, lease. TypeScript only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src/location.ts`          | `Location`, `Selection`, `transition`, the `?state=` codec, the store.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/first-run-state.ts`   | Pure versioned resumable setup state and its explicit nonsecret checkpoint codec.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/sidebar-prefs.ts`     | The rail's stored width: the `sideCollapsed` and `sidePinned` `localStorage` keys.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `src/icons.ts`             | The mock's 47 icons as structured data.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src/components/icon.tsx`  | `<Icon name size />`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/styles/shell.css`     | `wave6/shell.css` copied in full, minus the shared tokens.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/styles/app.css`       | Styles that replace the mock browser chrome with the app window.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `tests/`                   | `node:test` via `tsx`: goldens, source invariants, render tests.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `tests/acceptance/run.mjs` | Layer 3: Chromium over the built UI, one load per deep link.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

### The rail

`nav.side.rail` is a fixed six-tab rail, 200px wide and blue
(`--rail`, `--rail-ink`, `--rail-line`, `--rail-active` in
`apps/desktop/kit/tokens.css`). It does not enumerate stores. Its tabs are
People, Chat, Files, Teams, Devices and Settings; `railTabOf(location)` in
`src/location.ts` decides which one a location belongs to, so All items and a
store page mark Files, a group's settings page marks Teams, and a team chat
marks Chat. Chat carries one badge summing the unread across every team whose
chat this Mac can read, in the same orange every unread count uses
(`--unread`); People carries a dot while anything needs attention. Chat's tab
goes to `chatTabLocation()` — the team and channel the Chat tab last had open.
Control-Tab walks the six tabs.

Above the tabs, the account header names the active account — the one the
location's `store` parameter names, else the first account store — and opens a
menu of the accounts on this Mac grouped by server, "Add an account or
server…" (the first-run flow) and "Lock" (the command Settings › About also
offers). An account whose access has stopped carries a chip naming the state.

Below the tabs, the foot holds the Collapse/Expand row and nothing else. The
rail collapses to a 56px icon-only strip. Collapsing is CSS alone —
`nav.side` gains `is-narrow`, and every row stays in the document — so the
collapsed rail keeps its glyphs, its dots and its counts, the latter
drawn on the glyph's corner. A collapsed rail expands over the main
column on hover, and on keyboard focus whether or not the width was pinned;
the grid track stays at the collapsed width, so the page underneath does not
shift. The Collapse/Expand row writes `sideCollapsed` and
`sidePinned` to `localStorage` through `src/sidebar-prefs.ts`. Opening the
details panel collapses the rail and closing it restores the rail, on
transitions only and without touching the stored preference. First run
replaces the rail with its own step list, which is `nav.side` without `rail`
and has no toggle; the blue styling is scoped to `.side.rail` for that reason.

### What each tab is

People is the list of what needs attention — the page that used to be called
Alerts — over one account. Each attention card keeps its severity — as a
colour and as a name, so it is not colour alone — its title and its
detail, and carries its action at the right end: the catalog note retries in
place, and a note whose id names a server or a group opens that server's page
or that group's page — a `fed-` note names the group that was _admitted_, so
it opens the host group the catalog's one inactive federation entry names,
where the admission is restored; two such entries, or none, name no page. A
card that leads somewhere captions itself with where it leads and the agent's
own word for what it asks ("Settings › Servers · the agent reports this as
…"). A note whose place cannot be derived keeps the agent's
action as a chip instead, and drops the caption that would repeat it word for
word. Under
the cards, a switcher lists every account on this Mac — username, alias,
server, and a dot and the visible reason for one whose access has stopped — and
choosing one rewrites the address to that account's StoreRef. The chosen
account is a profile band — a coloured band with the account's mark half over
it, the username, and the server line the alias chip and the availability chip
sit on — followed
by three sections of what that account holds and then the rows the Accounts
pane had. Teams you're in lists the groups and shares the catalog holds for
that account: the group mark, its name, the caption every group list uses,
without the server, which the band above it already names — the roster
summary, a
bare role chip, an abnormal state as a chip at the end of the row, a row
action that opens the group's page and All teams, which opens the Teams tab at
this account. Devices counts what the account holds — "5 devices and keys" and
their names — with Manage devices. Keys is the mock's proofs list: one row per
key, the shortened id leading in mono with what it belongs to beside it, and
the check only on the key this Mac is authenticated with, because that is the
only verification the agent reports. Two of the three lists behind it are the
account's — its devices and the paper-key enrollments this Mac holds — while
the agent answers `list_yubi_accounts` for the whole server profile, so the
objects it answers with are enrollments: such a row's kind is "Enrollment", it
is captioned with the server rather than the account, and the footnote says
an enrollment on a card is listed for that server and not for one account on
it. If key retrieval fails, the row displays "Keys could not be read." or
"Devices and keys could not be read." rather than presenting an account with no
keys. If the agent replaces the catalog during a read, the shell refreshes it
and retries once, as Devices does. This page lists no connected
card, so it does not drive the card reader to find one. There are no proofs:
FOKS has no identity
proofs to list. Under them the rows are:
username with Change…, the local alias, the passphrase with a
link to
Settings › Account, then Organization sign-in, Bot accounts, Web admin and
Join a group…, each opening the panel it always opened. While access is
stopped the username change is disabled with the reason and the other four —
Organization sign-in, Bot accounts, Web admin and Join a group — stay
available, because each is a way to get access back. An account's mark is the
initial of the username, over a colour derived from it, on the account band
and in the switcher: those surfaces name the account by the username the
server knows it by, while a store elsewhere keeps its `GroupMark`. There is no
people search because the bridge has no directory lookup. New people can be
added only by invitation, which is managed in Teams. Chat is one inbox column, listing
every team with chat, beside the open conversation; the tab with no
conversation chosen opens the most recent one, and says how chat gets turned on
when no team has any. Files is a roots page listing All items and
then the vaults, groups and shares, each row opening its item page; an item
page returns here through the header's back chevron. Teams lists the groups and
shares — the group mark, name, server, the roster summary the per-group
`list_group_details` call loaded on the last refresh, the role this Mac's
account holds and a chip for an abnormal state — and the row itself is the
button that opens that group's page, which returns here. One caption builder
writes the line under a group's name wherever one is listed — on Teams, on
People and on a server's page: what the object is ("Named group", "Ad-hoc
share"), then the server it lives on, dropped on a page that is already about
one server, and the account it is held through only where this Mac holds two
accounts on that server. The tab and its headings say Teams; the object in
body copy is a group. Beside the row, not inside it, sits a menu of that
group's actions; an action that does not apply stays, inert, with its reason in
the item's `title` — Leave always, and the server-dependent entries while its
server is out of reach. Below the
list is one row per account store: what its server lists when Check for groups
is pressed, reported on the row itself and nowhere else, an Invite someone…
entry, and the same abnormal-state chip at the row's end. The header
carries Create a group and Join a group…, which opens the existing invitation
panel. Creating, inviting and joining act as one account — the one
`?state=teams&store=<StoreRef>` names, else this Mac's first — and the Create
sheet opens on that account, seeded to it and saying who is creating the group
on which server, so the account
menu keeps the page when it switches. Devices is one page per account, with
the same switcher at the top and the header counting each kind it lists —
"2 Macs · 1 key on a card · 1 paper key · 1 enrollment", and Loading… until
all four reads have answered, because counting what has not been read yet
would report zeroes. Device keys and security-key enrollments are tracked
separately: a hardware token stores an account device key, while
`list_yubi_accounts` returns enrollments for the entire server profile. Each
record uses its corresponding type in the header, row, and detail page. The three sections are Macs and device keys
(a laptop or key glyph, the name, the
kind and the role, the shortened key id, a
“This device” chip on the one that cannot remove itself and Remove… with a
typed confirmation on the other Macs; a key on a card carries a Key on a card chip
and no Remove, because a card's key is revoked under its enrollment rather
than removed here), Paper keys (each enrollment's name and shortened id,
Revoke…, Add a paper key…, and the row that recovers an account from one) and
Security key enrollments (each enrollment with its Enrolled or Incomplete
chip, its own
Revoke… — off, with the reason, while the enrollment is unfinished — and the
note that the agent reports no serial for one, the serial of the card connected
now with Provision… and PIN status — which asks the card in the port, so a
connected card and an enrollment to name are all it needs — and the
card PIN's link to Settings › Account). That last section is the one part of
the page that is not about the account above it: enrollments are listed for
the server, and an enrollment's own page is captioned with the server rather
than the account. The mock
draws one flat list of keys;
the page keeps the three sections deliberately, because each kind has actions
a flat list has nowhere to put: Add a paper key…, the recovery menu, and the
connected card and Card PIN rows all belong to one section and to no row.
A phrase you write down is called a
paper key
on these tabs, while the enrollment keeps the name the agent stores for it. The
header's Add a device or paper key opens a chooser — four cards, each with the
glyph of what it adds: pair another Mac, enter a
pairing phrase, paper key, security key — that leads into the pairing sheet in
the direction it names, and into the paper-key and provisioning
sheets. The pairing sheet is the two numbered steps of one exchange, written
for the direction the chooser picked, with Back to the chooser instead of a
control that silently changes what Start would do: getting a phrase is Start
and Resume offer in step 1 and the phrase with Copy, and step 2 is quiet until
there is one; entering a phrase is the other Mac's step 1 and this Mac's
fields in step 2. A phrase that came from Resume says the agent is still
holding that offer, and nothing counts down, because `PairingOffer` carries
only an alias and a phrase. The paper-key sheet reveals the 17 words the
account's phrase is, one grid, not copyable, behind "I have written down these
words" and Save paper key. Cancel beside it discards the revealed phrase:
`prepare_owner_backup` generates it and nothing else — it writes no local
state and does not contact the server — so there is nothing to clean up, and
the phrase is gone rather than committed. Esc and a click on the backdrop
leave the same way and discard the same phrase; a revealed phrase is not a
reason to hold the reader in the sheet. Every row on the page also opens that key's own
page — `?state=devices&store=<StoreRef>&device=<key>` — which carries the
kind, the role, the account, the full key id in mono with Copy, and the one
destructive action that kind has: Remove this device… for another Mac, Revoke…
for a paper key or an enrollment, both with the typed confirmation the row
used; this Mac's page says it cannot remove itself and points at Settings, and
a key on a card says it is revoked under its enrollment and carries the action
that goes there — its kind decides that
before its currency does, so the card this Mac is authenticated with now is
not told it cannot remove itself. An enrollment's page is captioned with the
server, not the account, because that is what the agent answered for. The `device=` value is
the key id for a Mac or a paper key and `yubi:<alias>` for an enrollment,
which the agent names by alias and reports no id for. A `device=` that no
longer resolves says the key is not on this account rather than showing an
empty page. The nine card operations that are recovery paths for a key already in
trouble, and creating an account on a YubiKey, sit in the enrollment
section's menu, each saying there when it does not apply, and each menu entry
and the sheet it opens read the same label. A stopped account
lists nothing and disables every action with the reason. Each section label
names a region of its own — Macs and device keys, Paper keys, Security key
enrollments — and
`section=macs` and `section=keys` focus the first and the last of them, except
while a sheet is open:
a scene that opens one with the page keeps the keyboard inside it, and the
address takes the page when the sheet closes. A scene is entered once: the
conceal that takes a phrase off the screen remounts the tab, and the tab does
not reopen the sheet its scene opened. An address naming an account
this Mac no longer holds says so and offers the ones it holds — that notice
alone, not beside a switcher offering the same accounts again — rather than
reporting that no account is configured. Settings is one
scrolling page: Servers (the list, grouped as Needs attention — which holds the
never-checked as well as the locked, since nothing on either can be used — and
Ready, each
row with its state chip, a Check on the never-checked and lapsed rows, and
Open for the server's own page), Account (set, change and verify a passphrase
per account, the reason on the row of one whose access has stopped, and the
card PIN and unlock code under a label naming the account they act on, with the
same switcher under it so that account can be changed), About
(version, agent state
and socket, Lock now), This Mac (export, import, verify online, move the data
folder) and a danger zone whose Reset this Mac runs the per-server reset once
per profile, each with its own one-use preview token and typed profile name;
the sheet states a token lifetime only once every server has reported one, and
names the servers, not the profile ids, when they differ. The redesign's
Notifications section is not built: the app stores no notification
preferences — the agent delivers chat notifications and the snapshot's
notifications are derived warnings — so the three switches the mock drew would
be controls over nothing. A server's own page
carries a Check in its header, and the band a never-checked or lapsed server
draws offers the same check beside the reason. Under the check-in rows, one
disclosure — Inspect last check response — holds the whole diagnostic
response. Then the page answers what stops if this server lapses: Accounts on
this server (the account's mark, username and alias, with People) and Teams on
this server (each group or share on it, the same caption the other lists
write, its roster summary or its
state chip, and a row action that opens it; with none, "No groups on this
server"). Identity and trust holds the
address, the pinned host id with Copy and the audit log's two numbers, and the
danger zone keeps two rows, because `forget_server` and `reset_server` are two
commands with two outcomes.
A store's abnormal state
is a chip at the end of its row on each of the four lists that draw one —
Files, Teams, People's Teams you're in, and a server's page —
rather than a caption under its name. One model function, `storeAttentionState`,
decides it for all four, so a group whose roster could not be read draws the
chip rather than printing "Roster unavailable" where the roster summary goes;
`storeDescriptionState`, which knows only whether the store can be reached, is
what dims the row. A group carries one mark everywhere it
is listed — the same initial over the same colour on Files, on Teams, in the
Chat inbox column and on its own page — while an account vault keeps its vault
glyph. Two notes in the Teams mock are
deliberately not built: the strip explaining that a group page has no Channels
or Files tab, and the band explaining that it has two tabs. Both describe the
design rather than the group, and the page already shows what it has.

Item pages open in the folder browser. The toolbar's list / grid / folders
toggle still chooses, and the choice survives the next navigation.

### The Chat tab

One location: `{ kind: 'chat', ref?, channel? }`, where `ref` is the team whose
conversation is mounted and `channel` the open channel. `?state=chat&store=…`
is its deep link, and `?state=team-chat&store=…`, the name it had before the
tab, still decodes to it; a `channel` with no `store` is dropped, because a
channel belongs to the team that names it. With no `ref`, `chat-tab.tsx` opens
the conversation with the most recent message across every team this Mac can
reach, and falls back to the first team that has chat when no conversation has
any message; while every reachable team is still on its first synchronization
the pane says "Loading conversations…" rather than opening a team it would have
to leave. That wait is bounded: it ends as soon as one reachable team answers,
and a team the inbox service does not keep is never waited on, because its entry
would never arrive. The service decides which teams it keeps on the same
availability clock the tab decides reachability on, so the two cannot disagree
about a check-in that expired this second. The choice the tab made is
provisional — a better conversation arriving while the other inboxes are still
answering replaces it — but only until it lands: once the tab has navigated to a
team whose inbox has arrived, or the location has changed to one the tab did not
write (a notification activation, a pick in the column, the rail's memory), the
choice is settled and a message arriving in another team does not take the
reader out of the conversation they are reading. A dismissible note in the pane
says which conversation it opened and why, until the reader picks one or
dismisses it. The tab remembers the team and
channel it had open, and the rail's Chat tab returns there rather than
re-running the fallback. A conceal forgets that memory, because the account
that comes back may not have the team on this Mac; a location naming a team
without chat leaves it alone unless the remembered team is the one that lost
chat.

`chat-tab.tsx` owns the layout — the column, the conversation and the info
panel — so a team switch replaces only the conversation, and exactly one
`useChatConversation` is mounted at a time. The column (`chat-teams.tsx`) is
the whole inbox: every named team whose server offers chat is listed at once,
under one "Conversations" label, in the shell's navigation order, ensuring
consistent team ordering across Files, Teams, and Chat for spatial
predictability. A team whose channels are the general channel alone is a single row —
its mark, its name, the last message ("You: …", "sam.ortiz: …", else "No
messages yet"), when that message arrived and its unread badge. That row is the
channel's row, so the count it is bold for, the badge's muted styling and the
caption beside it ("Muted", "Hidden", "Restricted", "Channel stopped") are the
ones the channel would have drawn. Teams with custom channels render as a group
heading with nested channels. Each channel carries its own unread count and, for
an admins-only channel, a lock; nothing folds away, so a heading selects nothing
and its gear, which opens that team's group settings, is the one control it
carries. The team's name is a heading of level 3 and its channels are a `group`
labelled by it. The open channel's row carries `aria-current="page"`, from the
first render that resolves it: the column and the pane resolve the open channel
through the same `openChannel` helper, so neither marks a row the other did not
mount.

Every row is built from `useSidebarInbox()` — the `TeamInbox` projections
`ChatInboxService` already keeps and polls for each eligible team, the same
ones the rail's badge sums — so previews, channel lists and counts arrive for
every team without a conversation being mounted for any of them. Only a plain
count is interpolated into a row; the states `teamUnread` also reports ("…",
"!", "3+") stay in the badge and its description. A team this Mac cannot reach
right now states its own reason ("Check-in expired") in place of the preview
and carries "!" instead of a count; its rows are dimmed but still open, onto
the pane that states the reason, because an inert row answers nothing. A team
whose inbox failed — nothing arrived, or it is unavailable or blocked — carries
that error, except for the open team, whose pane already states the error and
carries the retry, so its row says only "Channels unavailable". A
synchronization that succeeded but could not finish everything ("Read status
will retry.", "Some previews are unavailable.") is not a failure: the row keeps
its preview and its time, and says what is incomplete in a caption under them. A
heading draws a badge only for what its channels cannot already say — a state
rather than a count, so "…", "!", "3+" and "5·" appear there and a bare number
does not. A hidden conversation keeps its channel listed, marked "Hidden",
dimmed and placed last, rather than being dropped, which used to make a team
whose only conversation was hidden claim it had no channels. Named teams whose server
offers no chat sit dimmed at the foot under "No chat" with the reason; shares
are not listed. Unread counts are the rail's orange (`--unread`) wherever they
appear.

The column's search field filters teams and channels by name across the whole
column — a team match keeps all of that team's channels, a channel match keeps
its team's heading and that channel — and says when nothing matches. It is a
`type="search"` field naming itself, not an empty `<label>` wrapped around one,
and how many teams it narrowed the column to is announced in a visually hidden
`role="status"`, because the column narrowing is not something a screen reader
sees. It searches the "No chat" teams too, so it is live whenever the column
lists anything, not only when a team has chat. There is
no message-content search: the agent has no message index, so the conversation
header's search button focuses this field rather than promising one.

New chat (`chat-new.tsx`) opens from the column's header as a two-step sheet:
step one picks a team, with a team that cannot chat listed inert and carrying
its reason rather than hidden; step two picks one of that team's channels or
creates one. A channel says the same thing about itself here as in the column —
"Channel stopped", "Restricted", "Hidden", "Muted", through the same
`channelMeta` — and the team's count line counts the channels it offers, so a
hidden conversation is listed without being counted. Neither step is answerable
until that team's channel list has
arrived — an empty name is the general channel, and whether the team already has
one is the difference between creating it and being refused — so Continue and
Create wait on it behind a "Loading channels…" row. A team that loses chat while
step two is open disables both, with the reason in the body. A step change moves
focus to the new step's first control, and the create form is a step of its own:
it takes focus to the name field, and Back takes it to the step it returns to.

Creating is `prepare-channel` then `attempt` on a client of its own, keyed by
one durable submission identifier, so a lost reply is recovered rather than
repeated; the fields are the name, the description and the audience (Everyone on
the team / Admins and owners) the action carries. Both requests are guarded the
way `useChatConversation` guards a write: the availability clock and the
server's access generation are checked before and after each one — off the
snapshot, the clock and the generations the shell holds now, read through refs
rather than the ones the request started with, so a check-in that lapses or an
access generation that moves mid-request is seen — and a reply under a scope
other than the one the service holds for the team quarantines the account rather
than being trusted. The preparation is retired only by a settled
`attempt` — until then Recover re-issues that same operation, so an `attempt`
that failed is retried rather than turned into a second channel, and whatever
the agent is still holding is invalidated into the conversation's "Needs
attention" in case the sheet is closed on it. Once a request has failed
ambiguously the submission is the agent's to settle: a later refusal, of a
request that never left this Mac, does not discard it, and Recover is disabled
only while a recovery is running — not by a name collision, a lapsed check-in or
anything else that would refuse a new channel, because none of that refuses the
one already prepared. A fatal failure is the exception: the session the
submission was made in is over, so the sheet states the reason and can be
closed. A team switch does not take the
sheet away while a submission is unresolved, because a submission has to be
settled where it was made. The name is checked before it is sent for the rules a
reader can act on — 3–32 characters, lowercased, "general" reserved for the
empty name, no doubled hyphen, no space, no name the team already has — and the
description for the 3-to-512 band `ChatLimits` enforces, so the hint's numbers
are checked rather than merely printed; the agent stays the authority on the
rest. Both bands come from `crates/foks-agent-proto/chat-limits.json`, the same
policy the Rust build compiles its constants from, and `foks-agent` asserts them
against `ChatLimits`; the fields carry no `maxLength`, because a UTF-16 cap
would cut a name the agent counts in scalars. Lowercasing follows the agent —
one scalar per character, keeping the first of the mapping — so a mapping that
expands does not push a name over the bound here that the agent would take.
Whatever refuses the button refuses Enter, which submits through the same
check. The pane's empty state opens the same sheet on the team it is showing.

The conversation is `chat-screen.tsx`: the header reads "Team · #channel" — the
team name gives way and ellipsizes first, because the channel is what the header
is for — with
the description and access line ("Member can read and write" — the visibility
band is a FOKS internal and is not drawn, except when the read and write roles
differ by the band alone, where the band is the only thing that tells them
apart), and carries one Refresh that reloads the history, the channel list and
the saved work, the search that returns to the column, Team files, and the ⓘ
that opens `chat-info.tsx` — a third column while the tab is wide enough for
one, and below about 1000px of tab width an overlay over the conversation
instead, so opening it does not squeeze the thread to a sliver at the 960px
minimum window. The overlay covers the conversation rather than part of it: a
panel over half of every message line, and over the header's controls, would be
one a reader has to work around. The inbox column stays either way, so the way
out is where it was. It carries the channel's description, who can take part, the
team roster with its size and roles (the shell's `roleName`, never the band —
the chat contract's own role text is stripped by `roleTextWithoutBand`, which is
named for what it does so the two cannot be confused), the per-device alert
settings that used to be a strip above every thread, and "Manage in Teams".
Leaving, muting, renaming and deleting a channel have no `ChatAction`, so the
panel does not draw them and says so once at its foot.

The composer draws the attach, emoji and exploding-timer controls the reader
knows from other chat apps, inert, each with the reason in its `title` and in a
visually hidden description: chat carries text only, `ChatAction` has no
attachment or expiry, and there is no emoji picker. They carry `aria-disabled`
rather than `disabled` and stay in the tab order, because a control whose only
content is the reason it cannot be used has to be reachable to state it. Under
it, a hint line offers only the markup the thread renders (`**bold**`,
`*italics*`, `` `code` ``, `> quote`, `- list`, `[label](https://…)`); mentions
are not among them.

Saved work that has not finished sits above the conversation in a bounded
"Needs attention" section with the per-operation rows and their verbs; once the
last of it is accounted for the section says "All caught up" rather than
disappearing without a word. When the team's server check-in has lapsed the
column stays and the pane carries the notice, with "Check in" opening that
server in Settings › Servers; every other unavailable state is stated without
an action, because no button in Chat resolves it, and "Open another team" is
offered only when another team can actually be opened. A location naming a team
that has no chat says so in the pane rather than waiting for an inbox that will
not arrive. A location naming a channel the team does not list _yet_ is the
opposite case: while that team has been invalidated and its next synchronization
has not landed, the pane says it is loading rather than calling the channel
unavailable, which is what a channel created a moment ago would otherwise read
as. A synchronization that failed carries its error instead, so the wait always
has something behind it.

Four things in `dev/mockups/keybase-redesign/chat-*.html` are deliberately not
built, because no `ChatAction` reaches them: reactions, "Retry all" and
"Discard" over every pending operation at once (one attempt per operation is
the rule the recovery model keeps), Leave / Delete / Mute / Edit description in
the ⓘ panel, and the mock's "Join a team" button, which has no FOKS equivalent
— a team arrives through an invitation or a per-server check for groups, both
of which live in Teams, so the column's foot keeps the one "Create or join a
team" button.

Two things are built differently from the mock rather than left out. The
conversation header carries a Refresh beside the mock's search, files and ⓘ: the
history, the channel list and the saved work are projections that can go stale
behind the live connection the mock assumes, and nothing else in the pane asks
for them again. And the timer, attach and emoji controls sit together at the
left of the row under the text box, beside the send hint, rather than flanking
the text box inside one bordered field as the mock draws them — they are inert,
and a reader reaching past them for the box they sit inside would be reaching
past three controls that do nothing.

### `apps/desktop/kit`

`apps/desktop/kit/` holds reusable presentation primitives that depend on nothing
app-specific: `overlay-primitives.tsx`, `menu-position.ts`, `toasts.tsx`,
`virtual-list.ts`, `icon.tsx`, and `tokens.css`. `apps/desktop/kit/README.md` is the
authority on what is in it and why — including why `ui/src/sheet.tsx` stayed
behind. Reach it as `/kit/*` (a Vite alias and a tsconfig path). Application-specific models, fixtures, icons, and shell styles remain in
`src/` to keep application logic separate from reusable UI components.

The model logic is implemented exclusively in TypeScript because item kind classification
and reader computations are client-side presentation models without protocol equivalents.

## Design tokens and the theme decision

`src/styles/shell.css` defines the desktop shell styles. Make shell design
changes directly in this stylesheet.

Layout is a separate concern from color, and its custom properties are
declared on `.app` rather than on `:root`: `--side-track`, `--side-w-open`
(224px), `--side-w`, `--side-pad`, `--side-head-pad`, `--side-open-content`
and `--details-w` (300px; `--side-w-open` is 200px where the rail is drawn).
The rail animates its own width instead of the
grid track, which avoids interpolating `grid-template-columns`, and
`.app.side-narrow` is what sets the collapsed width (`3.5rem`).

The one edit is the token block. Measured against `ui/styles.css`, the mock's
48 `:root` tokens split **35 identical / 5 same-name-different-value / 8
FOKS-only**. The 35 moved to `apps/desktop/kit/tokens.css`, which this sheet `@import`s
first; the thirteen that remain are declared after it, so the cascade gives
FOKS its own `--faint`, `--surface`, `--main-surface`, `--hover` and
`--shadow-menu` plus `--sans`, `--mono` and the six `--c-*` kind tints.
`tests/styles-geometry.test.ts` fails if a token is ever declared in both
places.

**Theme: FOKS is light-only in Phase 1, and the fork is at the kit seam.**
`apps/desktop/kit/tokens.css` carries light values only and there is no theme script in
`index.html`. If dark theme support is added to FOKS, it should be declared in
this stylesheet.

## Testing

The unit and render-test layers live in `tests/`:

- `model.test.ts` — verifies reader calculations, role arithmetic, and store
  aggregation invariants against expected baseline values.
- `location.test.ts` — pure transitions and the `?state=` round trip, including
  the original design-state names used by the Playwright walks.
- `react-boundary.test.ts` — the negative invariants: no raw-HTML sink
  (the mock's whole render layer is `innerHTML`, and none of it came along), no
  `window.__TAURI__`, the Tauri API imported in one file, the model free of the
  DOM, the mock bundled out.
- `styles-geometry.test.ts` — the token split, the light-only rule and the
  shell's grid, read off the CSS since jsdom computes no layout.
- `app-root.render.test.tsx` — jsdom plus Vite `ssrLoadModule`, the same boot
  as `ui/tests/app-root.render.test.tsx`: the app mounts itself into `#root`
  from `src/main.tsx`, exactly as it does in the browser.

Layer 3 is `tests/acceptance/run.mjs`: `npm run acceptance:foks-ui` builds
with `VITE_FOKS_MOCK=1`, serves `dist/` over a local HTTP server on a random
port (a Vite build cannot be loaded over `file://`), and drives the installed
Chromium through `playwright-core` — `chromium.launch({ executablePath, args:
['--no-sandbox'] })`, no `playwright install`. Each state is loaded at
**1280×860** and must produce **no console error, no page error and
`document.documentElement.scrollWidth === 1280`**; PNGs are saved in
`tests/acceptance/shots/` (gitignored) beside `mock-all.png`, the design's own
`01-vault.html`, `02-first-run.html`, `03-groups.html`, `04-servers.html` or `05-settings.html` rendered from `file://` for the by-eye
comparison. The acceptance map names the exact design file/state for every app
state so similarly named Vault and Groups surfaces cannot be compared to the
wrong mock.
It drives Chromium, never the WebKit webview the app ships in, so it validates
layout and logic and nothing about the runtime.

Layer 4 is the Rust command layer. Runtime-dependent app lock, clipboard
concealment/clearing, native picker/drop streaming and webview resident-set
behavior are tested there or manually, not in Chromium.

## Deep links

The current screen, selection, and presentation are encoded in
the address bar, so every state can be reloaded and the acceptance run can
walk them. `?state=` is the mock's own vocabulary (`01-vault.html`'s `STATES`),
kept so deep links defined in the design specification resolve to this location.

| `?state=`                                                                              | Opens                                   | What else it fixes                                                                                      |
| -------------------------------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `all`                                                                                  | All items                               | —                                                                                                       |
| `personal`                                                                             | Personal (`acct:personal`)              | —                                                                                                       |
| `work` · `household` · `homelab`                                                       | that store                              | —                                                                                                       |
| `password`                                                                             | All items                               | selects the masked GitHub login                                                                         |
| `show`                                                                                 | All items                               | loads and reveals GitHub version 9 once                                                                 |
| `resource`                                                                             | All items                               | selects the masked Anthropic API key                                                                    |
| `file`                                                                                 | All items                               | selects Household's emergency PDF                                                                       |
| `link`                                                                                 | All items                               | selects latest-key and loads its target at the exact catalog version                                    |
| `group`                                                                                | Household (`team:household`)            | selects the Wi-Fi password                                                                              |
| `new`                                                                                  | All items                               | Password sheet in Household with explicit group roles and computed reader preview                       |
| `new-group`                                                                            | All items                               | Resource sheet in Engineering with explicit roles and computed reader preview                           |
| `new-resource`                                                                         | All items                               | Resource sheet in Personal                                                                              |
| `new-file`                                                                             | All items                               | File sheet in Household; renderer receives paths, never bytes                                           |
| `new-link`                                                                             | All items                               | Link sheet in Personal                                                                                  |
| `group-new-text` · `group-new-link` · `group-new-file`                                 | Engineering                             | Phase 7 group create acceptance scenes for text, Link/symlink and streamed File writes                  |
| `exists`                                                                               | All items                               | must-not-exist refusal; Open refreshes the invalidated catalog first                                    |
| `conflict`                                                                             | All items                               | exact-version refusal with retained draft and Refresh and review                                        |
| `grid`                                                                                 | All items                               | `view=grid`                                                                                             |
| `folders`                                                                              | All items                               | `view=folders`; store roots and folders are derived from catalog paths                                  |
| `lease`                                                                                | Work (Acme)                             | `lease=lapsed` — the whole snapshot, not a place                                                        |
| `inactive`                                                                             | Homelab                                 | group reports inactive; Resume creation uses its resumable operation                                    |
| `alerts`                                                                               | People, on its attention list           | `lease=lapsed`, so the list has its critical entry                                                      |
| `agent-lost`                                                                           | Full window stop                        | Retry reconnects and refreshes without replay                                                           |
| `groups`                                                                               | Teams                                   | the list, then one check-and-invite row per account store; `store=` names the account they act as       |
| `people`                                                                               | People                                  | the attention list over one account's panel; `store=` names the account                                 |
| `group-people` · `party` · `federation`                                                | Engineering group page                  | Members tab; `party` opens a member row's menu                                                          |
| `danger`                                                                               | Engineering group page                  | Settings tab                                                                                            |
| `store` · `items`                                                                      | Engineering                             | group vault                                                                                             |
| `invite` · `add` · `demote` · `remove` · `admit`                                       | Engineering group page                  | the named Group sheet                                                                                   |
| `create`                                                                               | Teams                                   | named/ad-hoc Create group sheet on the acting account; `store=` names it, else this Mac's first         |
| `groups-lease` · `groups-inactive`                                                     | Engineering group page or Homelab vault | distinct lease/inactive takeovers                                                                       |
| `manage`                                                                               | Household group page                    | Members tab, without a Manage overlay                                                                   |
| `party-remove`                                                                         | Engineering group page                  | the non-local removal refusal                                                                           |
| `join`                                                                                 | Teams                                   | account-specific discovery and invite choices                                                           |
| `join-invite`                                                                          | Teams                                   | invite sheet opened on the exact `acct:work` fixture store                                              |
| `boot` · `who` · `address` · `no-address` · `checked` · `compare` · `error`            | First run, steps 0–2                    | `path=invited` or `path=own` selects the setup route                                                    |
| `account` · `existing` · `protect` · `phrase`                                          | First run, steps 3–4                    | account creation/recovery and the one-time backup sheet                                                 |
| `waiting` · `added`                                                                    | First run, steps 5–6                    | invited group discovery and completion                                                                  |
| `checklist-invited` · `checklist-own`                                                  | Get started inside the ordinary shell   | resumable nonsecret progress summary                                                                    |
| `first-run&step=<step>&path=<path>`                                                    | the resumable first-run location codec  | used after the first in-app transition and across reload                                                |
| `servers-list` · `servers-server` · `servers-lapsed` · `servers-rollback`              | Settings › Servers                      | the list, then one server's page; `profile=` opens it                                                   |
| `servers-reset` · `servers-add` · `servers-unprobed` · `servers-check`                 | Settings › Servers                      | typed reset, add/check and explicit result states                                                       |
| `settings&section=servers` · `settings&section=credentials` · `settings&section=about` | Settings                                | the one page, scrolled to and focused on that section                                                   |
| `settings-macs` · `settings-macs-work` · `settings-phrase`                             | Devices                                 | Macs, pairing and the one-time paper-key reveal; `settings-macs-work` names the exact `acct:work` store |
| `settings-keys` · `settings-enrol`                                                     | Devices                                 | the Security key enrollments section and the YubiKey account sheet                                      |
| `devices&store=<StoreRef>&device=<key>`                                                | Devices › one key                       | that key's own page; `device=` is the key id, or `yubi:<alias>` for an enrollment                       |
| `settings-account`                                                                     | People                                  | the account panel and its workflows                                                                     |
| `settings-agent` · `settings-about`                                                    | Settings                                | the About section: agent status, socket, version                                                        |

`decodeLocation` returns `null` for display mode, item selection, or lease
modifiers because those properties represent presentation options or environmental
status rather than distinct navigation destinations. `decodeScene` encapsulates
these orthogonal state properties alongside the active location.
Everything is also addressable on its own — `sel=<store>|<path>`, `view=`,
`kind=`, `sort=`, `folder=`, `closed=`, `lease=` — and `sceneHref` writes back only what differs
from the default, so an ordinary `?state=all` stays `?state=all`.

Search query text is intentionally omitted from the URL address state so that
active filter queries persist across store navigation transitions within a session.
