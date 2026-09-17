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
"server · Role"; it has four tabs — Members, Channels, Files and Settings —
implemented as a tablist. Arrow keys select adjacent tabs, and the selected tab
labels the panel below it. Channels reads the same per-group inbox entry as the
rail and Chat column, so its count and rows use the same data. Each channel row
displays the name, description or access summary, last activity, unread count,
and Open in Chat. Add channel opens the New chat sheet at the create step and
shows Cancel instead of Back because team selection was skipped. Hidden
conversations remain listed but are excluded from the unread count, matching
New chat. If the server does not support chat, the page displays an explanation
instead of a list. If access lapses while the tab is open, it displays the same
condition as the store access view, evaluated with the shell's current clock.
If synchronization completes with unfinished work, the page displays a note
above the loaded rows. Files links to the group vault without duplicating the
file browser; the row displays a folder mark, the vault name, the catalog item
count, and Open in Files. Both tabs are addressable as `tab=channels` and
`tab=files`. Arrow-key navigation replaces the current location rather than
adding a history entry.
Members splits the one roster into Accounts, Machines and Groups on other
servers: a row carries the name, a "you" chip, the kind of party as its only
second line, and one role chip with the visibility band inside it ("Member
(0)"). Role badges and actions appear on each member row. Unavailable actions
remain visible in a disabled state with an explanatory tooltip, clarifying why
the action is currently restricted. An admitted group's row adds its
admission state, Restore access and Remove admission. A roster party that names
another group is listed under Groups on other servers rather than dropped,
once: with a "No admission record" chip where no record on this Mac matches it,
and an "Ambiguous admission" chip where several do, in which case those records
are not listed again beside it. Add someone on `<server>`… and Send setup instructions…
follow the people and machine rows and go with them when the roster could not
be read; Add a group… follows the admitted ones. The member addition dialog
provides two modes: person mode and federated team mode. Person mode preselects
the server, displays role selection cards (disabling unauthorized roles with
tooltips), and validates that the username exists before sending. Federated
team mode lists remote groups already available on this device because
`admit_group` accepts a store rather than a name and host. Switching modes
resets mode-specific role, visibility, and validation state. If an entered
username already exists in the team roster, client-side validation rejects it
locally. Server-side validation errors returned by the agent are displayed
inline beneath the input field and cleared when the field value changes.
Settings states the group's
name with "Team names cannot be changed after creation." and the static Invite only policy.
A group with incomplete setup displays no tabs. The application navigates to the new
group once created. If creation is interrupted, an incomplete-setup banner
displays a “Finish setup” action above the locally available group details and
uses the same status text as the store access view. Group
creation, discovery and invitations live on the Teams page
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
  may both hold an account aliased `personal`, so Accounts, Devices and Join
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
replacement, `resume_group_creation` and `abandon_group_creation`, the local
forgetting of a group whose creation can no longer be finished. Rust-originated
drop events contain
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

| Path                       | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.tsx`             | Entry point. One window; mounts `src/app-root.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `src/app-root.tsx`         | App shell: window, rail, screen, details panel, deep links.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/components/`          | Components: Button, Chip, Tag, Badge, KindIcon, Inset, SectionLabel, Notice, Band, SegmentedControl, SplitButton, MenuButton, SearchField.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/shell/`               | Window chrome: `sidebar.tsx` (the rail), `topbar.tsx` (the crumbs, the back chevron and the shell's own controls), `page-header.tsx`, `search-palette.tsx` (⌘K), `toolbar.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/screens/`             | Screens: `items-screen.tsx` (list, cards, notices, empties), `groups-screen.tsx` (the group page, its Members and Settings tabs and the add/role/remove/create sheets), `group-tabs.tsx` (the Channels and Files tabs and the unfinished-group page), `group-model.ts` (the permission rules and reasons the Teams list and the group page share), `group-mark.tsx` (the one mark a group carries everywhere), `invite-sheet.tsx` (the per-account invitation), `store-access.tsx` (the shared unavailable-store takeover and All items summaries), `first-run-screen.tsx`, `servers-screen.tsx` (`ServersSection`, the server list Settings draws, and one server's page), `settings-screen.tsx` (the one Settings page), `devices-screen.tsx` (one account's Macs and device keys, paper keys and security key enrollments, and one key's own page), `device-sheets.tsx` (the sheets Devices and Settings share), `device-model.ts` (the four per-account key calls and the one row model Accounts and Devices share), `account-switcher.tsx` (the switcher Accounts, Devices and Settings share), `details-panel.tsx`, `write-workflows.tsx`, `edit-value.ts`, `people-screen.tsx` (the attention list and the account profile: teams, devices, keys and the account rows), `files-screen.tsx` (the roots page), `teams-screen.tsx`, `chat-tab.tsx` (the Chat tab: the inbox column, the open conversation and the info panel), `chat-teams.tsx` (the cross-team inbox column), `chat-new.tsx` (the two-step New chat sheet), `chat-screen.tsx` (one team's conversation), `chat-info.tsx` (the channel info panel), `scope.ts` (what is listed, in what order). |
| `src/bridge.ts`            | The typed `Bridge` interface and the Tauri implementation.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/mock-bridge.ts`       | The same interface, using the fixture.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/fixture.ts`           | The stable desktop fixture, as typed data.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/model/`               | The pure model — roles, kinds, readers, format, lease. TypeScript only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `src/location.ts`          | `Location`, `Selection`, `transition`, the `?state=` codec, the store.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `src/first-run-state.ts`   | Pure versioned resumable setup state and its explicit nonsecret checkpoint codec.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `src/sidebar-prefs.ts`     | The rail's stored width: the `sideCollapsed` `localStorage` key.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/icons.ts`             | The mock's 47 icons as structured data.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `src/components/icon.tsx`  | `<Icon name size />`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `src/styles/shell.css`     | `wave6/shell.css` copied in full, minus the shared tokens.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/styles/app.css`       | Styles that replace the mock browser chrome with the app window.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `tests/`                   | `node:test` via `tsx`: goldens, source invariants, render tests.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `tests/acceptance/run.mjs` | Layer 3: Chromium over the built UI, one load per deep link.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |

### The rail

`nav.side.rail` is a fixed six-tab rail, 200px wide and blue
(`--rail`, `--rail-ink`, `--rail-line`, `--rail-active` in
`apps/desktop/kit/tokens.css`). It does not enumerate stores. Its tabs are
Accounts, Chat, Files, Teams, Devices and Settings; `railTabOf(location)` in
`src/location.ts` decides which one a location belongs to, so All items and a
store page mark Files, a group's settings page marks Teams, and a team chat
marks Chat. Chat carries one badge summing the unread across every team whose
chat this Mac can read, in the same orange every unread count uses
(`--unread`); Accounts carries a dot while anything needs attention. Chat's tab
goes to `chatTabLocation()` — the team and channel the Chat tab last had open.
Control-Tab walks the six tabs. Returning to a tab restores its last page,
folder and filters; account-specific pages follow the acting account. Item
details are closed on return. A conceal clears this in-memory tab history.
The topbar's chevron opens the current page's parent explicitly; it is not a
browser-history Back button. URLs restore the current scene on reload, without
creating a browser history stack.

There is no title bar. The rail's top is the window's drag strip
(`.traffic`, `data-tauri-drag-region`): the native build leaves it empty for
the controls macOS draws over it, and the web mock draws three rail-tinted
circles in their place. First run's step list carries the same strip.

Under the strip, the account header names the acting account. Account-specific
tabs retain that account when switching tabs. Opening a group, chat or vault
uses the account through which that object is held. Files, Chat and Teams
still list all accounts on this Mac. The header opens a menu of the accounts on
this Mac grouped by server, then "Add an account or server…" (the first-run
flow) and "Lock" (the command Settings › About also offers). The account the
window is acting as carries a check at the right end of its row; an account
whose access has stopped is dimmed and carries an amber line under its caption
with the one link that would restore it — "Check in" for a lapsed or missing
check-in, "Server settings" otherwise. An orange dot on the avatar is the only
place attention is advertised; it is a button, and it opens the Accounts tab.

The foot holds the agent light and nothing else: a 7px dot and a label in four
states — green "Agent ready", amber pulsing "Starting agent…", red "Agent
stopped" and grey "Locked". It never names an internal step.

The rail collapses to a 56px icon-only track. Collapsing is CSS alone —
`nav.side` gains `is-narrow`, and every row stays in the document — so the
collapsed rail keeps its glyphs, the unread count on the Chat glyph's corner
and the attention dot on the avatar's, with the labels becoming tooltips. The
width is the one the reader chose: a collapsed rail does not expand on hover or
on focus. The toggle is in the topbar, not in the rail, and writes
`sideCollapsed` to `localStorage` through `src/sidebar-prefs.ts`. Opening the
details panel collapses the rail and closing it restores the rail, on
transitions only and without touching the stored preference. First run
replaces the rail with its own step list, which is `nav.side` without `rail`
and has no toggle; the blue styling is scoped to `.side.rail` for that reason.

### The topbar

`.topbar` is a 44px bar at the top of the content column, and the only bar with
a hairline under it: the page header below it reads as the top of the page
rather than as a second toolbar. It carries, left to right, the back chevron
(disabled at a tab's root; `parentLocation(location)` in `src/location.ts`
decides where it goes), the crumbs — the tab in bold, then the page under it
when the address names one, such as "Files › Engineering", "Chat › Household
#general" or "Settings › Servers" — a flexible gap, the "Search everything"
trigger with its ⌘K hint, the rail's collapse toggle and the catalog refresh.
Its empty parts carry `data-tauri-drag-region`, because the rail's strip alone
does not reach across the window. First run draws no topbar.

Swiping right with two fingers on the trackpad navigates to the parent location
of the current page, matching the back chevron (`src/shell/swipe-back.ts`). The webview's own back/forward gestures stay off —
the app records its scene with `history.replaceState` only, so the back-forward
list holds one entry — and with them off the swipe arrives as `wheel` events
carrying `deltaX`, on macOS and on the Linux and Windows webviews alike. The
gesture is read from that stream: modified and ctrl-wheel (pinch-zoom) events
are skipped, `|deltaX|` must exceed twice `|deltaY|`, an ancestor pane that can
take the horizontal scroll itself keeps it, and 120px of back-directed travel
in events no more than 120ms apart navigates once. The back navigation gesture
is throttled until wheel events cease for 300ms, preventing trackpad inertia
from triggering multiple page transitions. The gesture is disabled at the
navigation root or when a modal dialog is open.
The page changes in one step: there is no animation and no rubber-band preview
of the page behind.

### Blocking states

Three takeovers leave as much of the shell readable as they safely can, and the
rail's light says which one is in force.

- **Starting.** The rail and topbar are displayed before a snapshot exists,
  dimmed and inert. The content area displays a spinner, "Starting the FOKS
  agent…", and "Startup usually takes a few seconds." Internal step names remain hidden, and
  maintenance uses the same presentation as the first connection.
- **Stopped** (`restart-required`, `recovery-required`, `restoration-failed`).
  The page remains visible behind a light veil below the topbar (`.stopveil`).
  A centred `.card.stopcard` displays the outcome, details, any copyable
  `foks-rs` recovery commands, and the available actions. `restart-required`
  adds: "Your vaults remain on this Mac and on their configured servers." The
  same card appears at startup over the empty frame.
- **Locked.** A full-window takeover over the rail as well (`.lock-back`,
  blurred), because the rail names accounts and counts unread messages: a round
  accent glyph, "FOKS is locked", the one sentence naming the authentication
  the platform offers, and a full-width Unlock. The boot error card shares the
  same box.

### What each tab is

Accounts is the list of what needs attention — the page that used to be called
Alerts — over one account. Each attention card keeps its severity — as a
colour and as an accessible name — its title and its detail, and displays the
exact action label reported by the agent (formatted as "Required action: …")
alongside an action button at the right. The catalog note retries in place, and
a note whose id names a server or a group opens that server's page or that
group's page. A `fed-` note names the admitted group, so it opens the host group
identified by the catalog's one inactive federation entry; two such entries,
or none, identify no page. A note whose destination cannot be derived displays
the action as a chip instead and omits the caption that would repeat it. Under
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
page returns here through the topbar's back chevron. Teams lists the groups and
shares — the group mark, name, server, the roster summary the per-group
`list_group_details` call loaded on the last refresh, the role this Mac's
account holds and a chip for an abnormal state — and the row itself is the
button that opens that group's page, which returns here. One caption builder
writes the line under a group's name wherever one is listed — on Teams, on
Accounts and on a server's page: what the object is ("Named group", "Ad-hoc
share"), then the server it lives on, and the account it is held through only
where this Mac holds two accounts on that server. Each part is dropped where
the surface already says it: the server on a page that is about one server, the
kind on Teams, whose Groups and Shares section labels say it one row above.
The tab and its headings say Teams; the object in
body copy is a group. Beside the row, not inside it, sits a menu of that
group's actions; an action that does not apply stays, inert, with its reason in
the item's `title` — the server-dependent entries while its
server is out of reach, and Send setup instructions… where this Mac holds no account on
the group's own server to send it as. Below the
list, the per-account checks are folded into one disclosure row, "Check other
servers for groups", counting the servers those accounts sign in to: discovery
is an occasional per-server action, not a landing surface. Opened, it is one
row per account store, headed by the server it signs in to: what that server
lists when Check for groups is pressed, announced on the row itself and
nowhere else, a Send setup instructions… entry, and the same
abnormal-state chip at the row's end. The disclosure opens when an account
enters an abnormal state. After the user changes the disclosure state, that
preference is preserved. The
header carries Create a group and Join a group…, which opens the existing
invitation panel. Creating and joining act as one account — the one
`?state=teams&store=<StoreRef>` names, else this Mac's first — and the Create
sheet opens on that account, seeded to it and saying who is creating the group
on which server, so the account
menu keeps the page when it switches. Setup instructions belong to an account and name its server; an optional group
names the group the sender plans to add the recipient to. The message asks the
sender to supply an installer and directs the recipient to the server
administrator if a signup code is required. It contains no provisional download
URL and grants no access. Team invitations are a separate protocol workflow:
Members → Invitations and requests lets administrators create invitations,
review and approve or reject requests, and recover pending operations. Join a
group accepts those invitations and verifies membership before opening access.
Devices is one page per account, with
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
lists nothing and disables every action with the reason.
Each section is an ARIA region focused via the `section` query parameter. When
a dialog opens immediately upon loading a scene, keyboard focus is constrained
within the dialog, and the target section receives focus only after the dialog
closes. A scene is entered once: concealing a phrase remounts the tab without
reopening the dialog that the scene opened. Navigating to an account ID that is
no longer present locally displays an account-unavailable screen listing valid
accounts, rather than reporting that no accounts are configured. Settings is one
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
names the servers, not the profile ids, when they differ. Notifications holds this device's desktop-alert and message-preview preferences,
which the desktop persists locally. Channel overrides stay in Chat's info panel,
with a link to Settings; none of these preferences sync between devices.
A server's own page
carries a Check in its header, and the band a never-checked or lapsed server
draws offers the same check beside the reason. Under the check-in rows, one
disclosure — Inspect last check response — holds the whole diagnostic
response. Then the page answers what stops if this server lapses: Accounts on
this server (the account's mark, username and alias, with Accounts) and Teams on
this server (each group or share on it, the same caption the other lists
write, its roster summary or its
state chip, and a row action that opens it; with none, "No groups on this
server"). Identity and trust holds the
address, the pinned host id with Copy and the audit log's two numbers, and the
danger zone keeps two rows, because `forget_server` and `reset_server` are two
commands with two outcomes.
A store's abnormal state
is a chip at the end of its row on each of the four lists that draw one —
Files, Teams, Accounts’ Teams you're in, and a server's page —
rather than a caption under its name. One model function, `storeAttentionState`,
decides it for all four, so a group whose roster could not be read draws the
chip rather than printing "Roster unavailable" where the roster summary goes;
`storeDescriptionState`, which knows only whether the store can be reached, is
what dims the row. A group carries one mark everywhere it
is listed — the same initial over the same colour on Files, on Teams, in the
Chat inbox column and on its own page — while an account vault keeps its vault
glyph.

Several things the Teams mocks draw are deliberately not built, because the
agent reports no such fact and drawing one would be inventing it: a group
description (no command stores one, so the Create sheet and Settings have no
description field); a "Who can join" choice (nothing sets a join policy, so the
row states Invite only and says why); a signup code, its expiry and a
"New code" action on the invite page, and the "What each line does" breakdown
under the message; whether a given server requires a signup code before an
account can be created, which nothing this Mac holds reports — the note under
the message says only that one may; an invitation preview's expiry and inviter, which the Join
sheet does not claim; a Recent-items list on a group's Files tab, which would
be a second file browser over the same catalog; and a Created-by line and a
"server checked" chip on Settings. Unsupported Leave, Delete and unfinished-group
removal actions are omitted. The annotation strips in the mocks describe the
design rather than the group and are not built either.

Item pages open in the folder browser. The toolbar's list / grid / folders
toggle still chooses, and the choice survives the next navigation.

### The Chat tab

One location: `{ kind: 'chat', ref?, channel? }`, where `ref` is the team whose
conversation is mounted and `channel` the open channel. `?state=chat&store=…`
is its deep link, and `?state=team-chat&store=…`, the name it had before the
tab, still decodes to it; a `channel` with no `store` is dropped, because a
channel belongs to the team that names it. With no `ref`, `chat-tab.tsx`
opens the conversation with the most recent message across every reachable
team and falls back to the first chat-enabled team when no conversation has a
message. While all reachable teams are synchronizing, the pane displays
"Loading conversations…". The wait ends when any reachable team responds.
Teams excluded by the inbox service are not included in the wait. The service
and tab use the same availability clock for reachability checks.

Initial selection is provisional. A more recent conversation can replace it
until initial selection completes. Selection becomes final after the selected
team's inbox loads or after external navigation, such as a notification,
column selection, or restored navigation state. Later messages in other teams
do not change the active conversation. A dismissible note identifies the
automatically selected conversation and the reason for the selection until the
user selects a conversation or dismisses the note. The tab remembers the team and
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
and carries "!" instead of a count; its rows are dimmed but remain clickable,
opening the pane that displays the
lock reason so the user can see why the team is unavailable. A team
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
its team's heading and that channel — and says when nothing matches.
The input is a `type="search"` field with its own accessible name rather than a
wrapper `<label>`. Search match counts are announced via a visually hidden
`role="status"` element so screen reader users are notified when the team list
filters. It searches the "No chat" teams too, so it is live whenever the column
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
settled where it was made.
The channel name is validated client-side against user-actionable constraints
(3–32 Unicode scalar characters, automatically lowercased, `general` reserved
for the default channel, no consecutive hyphens, no whitespace, and no
duplicate channel names), and the description is validated against the 3–512
character limits defined in `ChatLimits`. Both limits originate from
`crates/foks-agent-proto/chat-limits.json`, ensuring parity between frontend and
backend validation. Input elements omit `maxLength` to prevent UTF-16
truncation of Unicode scalar counts. Case folding normalizes one scalar per
character (retaining the first mapped scalar) to align with backend length
checks. Form submission via Enter is disabled whenever the Create button is
disabled. The pane's empty state opens the same sheet on the team it is showing.

The conversation view is implemented in `chat-screen.tsx`: the header displays
"Team · #channel", where the team name truncates with an ellipsis first under
constrained widths to prioritize channel name visibility, with
the description and access line ("Member can read and write" — the visibility
band is a FOKS internal and is not drawn, except when the read and write roles
differ by the band alone, where the band is the only thing that tells them
apart), and carries one Refresh that reloads the history, the channel list and
the saved work, the search that returns to the column, Team files, and the ⓘ
that opens `chat-info.tsx` — rendered as a third column when space permits, and
as an overlay covering the conversation pane below 1000px tab width to preserve
readability at the 960px minimum window width. The overlay covers the entire
conversation pane to avoid partially obscuring message text and header
controls. The team column remains visible to allow navigating away at any time. It carries the channel's description, who can take part, the
team roster with its size and roles (the shell's `roleName`, never the band —
the chat contract's own role text is stripped by `roleTextWithoutBand`, which is
named for what it does so the two cannot be confused), the per-device alert
settings that used to be a strip above every thread, and "Manage in Teams".
Leaving, muting, renaming and deleting a channel have no `ChatAction`, so the
panel does not draw them.

The composer exposes text entry and delivery actions. Attachments, an emoji
picker and message expiry are omitted until supported. Under
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

Four things the chat redesign drafts showed are deliberately not built, because
no `ChatAction` reaches them: reactions, "Retry all" and "Discard" over every
pending operation at once (one attempt per operation is the rule the recovery
model keeps), Leave / Delete / Mute / Edit description in the ⓘ panel, and a
"Join a team" button, which has no FOKS equivalent
— a team arrives through an invitation or a per-server check for groups, both
of which live in Teams, so the column's foot keeps the one "Create or join a
team" button.

The conversation header carries Refresh alongside search, files and ⓘ, because
the history, channel list and saved work can become stale.

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
`.app.side-narrow` is what sets the collapsed width (`56px`).

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

## Navigation guards

Sheets are not addressable by `Location` or a URL. Rail-tab switches retain
approved input forms in the optional `LocationState.sheet` memory: new items,
new channels, member addition, group creation, invitation details, and typed
pairing acceptance. Screens explicitly opt their selectors and fields into
`useTabSheetState`; PINs, passphrases, paper-key reveals, card setup, revocation,
and in-flight writes are not restored. Confirming a discard clears this memory.
Other navigation still unmounts the workflow and asks its guard. Every entry point — the rail's tabs, Control-Tab,
the topbar's back chevron, the account switcher, the ⌘K palette, attention
links, the trackpad's back swipe — reaches the same three methods on
`LocationStore`: `navigate`, `navigateTab` and `select`. Guards allow the active
screen to intercept and validate all three before they take effect.

A guard is a function of one intent — a navigation's address, or the item a
selection is about to open — returning one of three answers:

- `null` allows the move. This is the only answer with no visible effect, and
  a screen with nothing to protect returns it for every intent.
- `{ verdict: 'prompt', title, body, confirm, onConfirm? }` displays a
  confirmation dialog. The shell draws the confirmation `NavigationPrompt` — the discard
  dialog under the guard's own words, with `confirm` on the danger button and
  Cancel holding focus. Confirming runs `onConfirm` and then makes the move;
  Cancel, Escape and the backdrop leave both alone.
- `{ verdict: 'refuse', reason }` stops the move outright and shows `reason` in
  a toast. It is for a state that cannot be abandoned at all — a write the
  agent has already been given — not for an operation the user may choose to
  discard.

Guards are registered with `useNavigationGuard` from `src/navigation-guard.tsx`
and are asked in registration order. Any guard refusal immediately halts navigation. If no
guard refuses, the first prompt is displayed only after every guard has been
evaluated, so a confirmed prompt can never bypass another guard’s refusal.
A guard must be a pure answer: it may not navigate, write state, or start work
of its own.

`navigationVerdict(intent)` runs the same guards without acting on anything. A
caller that has to stay inert rather than raise a dialog asks it first: the
back swipe is a trackpad movement, not a decision, so a page it would have to
ask about is a page it does not go to.

`{ force: true }` skips the guards. It belongs to a move the shell makes on its
own behalf and no screen may refuse:

- the navigation a guard's own prompt was just confirmed for, so confirming
  cannot ask again;
- a redirect away from an inaccessible view, such as a lapsed lease, a store
  removed from inventory, or concealment that ends a session;
- a canonicalizing replacement that stays on the same page, such as a default
  route resolving to an account's exact `StoreRef`;
- a workflow applying its own resolution as it closes, such as the write
  overlay selecting the item whose path it just resolved.

All user navigation requests pass through the guards, including the rail tabs
and the ⌘K palette. A palette item result is one move rather than two:
`navigateAndSelect` opens the store page with the item selected on it, so a
held-back navigation cannot leave another page's item in the details panel.

Two shortcuts sit on the document, where an open dialog cannot intercept them,
so they test `anyDialogOpen()` themselves: Control-Tab in `shell/sidebar.tsx`
and ⌘K in `shell/search-palette.tsx` do nothing while a dialog owns the window.

## Testing

The unit and render-test layers live in `tests/`:

- `model.test.ts` — verifies reader calculations, role arithmetic, and store
  aggregation invariants against expected baseline values.
- `location.test.ts` — pure transitions and the `?state=` round trip, including
  the original design-state names used by the Playwright walks, and the guard
  mechanism: verdict order, what `force` skips, and a prompt superseded by a
  second navigation.
- `navigation-guard.render.test.tsx` — the confirmation a `prompt` verdict
  raises, and what confirming, cancelling and Escape each do behind it.
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

The redesign journey gate is `npm run acceptance:foks-redesign`. It exercises
account switching, invitation administration, device notification preferences,
Teams → Chat → Files navigation and per-tab restoration at **1280×860** and
**960×860**. The narrow run uses the keyboard throughout and checks that the
channel-info overlay covers the conversation rather than squeezing it. It saves
`redesign-*.png` screenshots alongside the other acceptance artifacts.

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
| `alerts`                                                                               | Accounts, on its attention list           | `lease=lapsed`, so the list has its critical entry                                                      |
| `agent-lost`                                                                           | Full window stop                        | Retry reconnects and refreshes without replay                                                           |
| `groups`                                                                               | Teams                                   | the list, then the collapsed Check-other-servers row; `store=` names the account create/join act as     |
| `people`                                                                               | Accounts                                  | the attention list over one account's panel; `store=` names the account                                 |
| `group-people` · `party` · `federation`                                                | Engineering group page                  | Members tab; `party` opens a member row's menu                                                          |
| `group-channels`                                                                       | Household group page                    | Channels tab, on the group whose server offers chat; `tab=channels` reaches it on any group             |
| `group-files`                                                                          | Engineering group page                  | Files tab; `tab=files` reaches it on any group                                                          |
| `danger`                                                                               | Engineering group page                  | Settings tab                                                                                            |
| `store` · `items`                                                                      | Engineering                             | group vault                                                                                             |
| `add` · `demote` · `remove` · `admit`                                                  | Engineering group page                  | the named Group sheet; `add` and `admit` are the two halves of one sheet                                |
| `invite`                                                                               | Engineering group page                  | the Invite sheet, seeded to the account holding the group                                               |
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
| `settings-account`                                                                     | Accounts                                  | the account panel and its workflows                                                                     |
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

Cross-server collections use the same server-name disambiguation and account
captions. Files roots and Chat team rows show their server visibly; a group
held through multiple accounts on one server also names its holding account.
Duplicate server labels include the underlying profile name. Server details and
setup instructions retain the configured address rather than substituting a
local display name for it.

### Device metadata cache

Accounts and Devices share an in-memory cache for the unlocked shell session.
Device and paper-key metadata is keyed by profile and account store; security-key
enrollments are shared by profile. Fresh entries are reused for one minute.
Returning to either tab paints cached rows immediately; expired entries remain
visible while a read refreshes them. Concurrent metadata reads share requests.

The shared `QueryRepository` owns immutable metadata, subscriptions, freshness,
and one shared read per resource generation. Accounts, Devices and the server's enrollment
page subscribe to the same resources. Pending group operations and invitation
recovery counts use the same repository; invitation payloads are not retained.

A forced full catalog refresh invalidates metadata after installing the new
catalog. Resource-specific actions invalidate their account or profile; cached
rows remain visible during replacement reads. Account/profile identity or access
changes and session concealment replace the repository; lock/unmount clears it.
Generation checks prevent late reads from refilling a cleared or invalidated
resource. Eligible catalog-required read failures share one repair and one read
retry. This mechanism never retries writes or ambiguous results.

Connected-card presence is probed separately on Devices and does not delay
metadata rows. PIN state remains action-specific. The cache never stores recovery
phrases, PINs, passphrases, or other secret inputs, and never persists to disk.
Removing a device first reloads the native device list; cached display metadata
does not replace target validation for the write.

### State coordination

The shell's `CatalogCoordinator` serializes compound snapshot loads. Concurrent
ordinary requests share a load; invalidations during a load coalesce into one
trailing load. Only the newest accepted result is published. Lock, maintenance,
connection loss and unmount retire older replies. Native catalog loads likewise
keep the accepted snapshot until a replacement succeeds. Publication retires
native authorization facts before exposing the new generation; cached display
metadata never supplies write authorization.

Confirmed mutations and subsequent view synchronization have separate outcomes.
A failed refresh does not turn an applied write into a failed write. Automatic
recovery does not replay a mutation. Unknown delivery remains an explicit
reconciliation or durable-operation recovery path.

The agent owns bounded profile admission for foreground requests, periodic work,
retention and chat verification. Conflicting requests queue before occupying a
worker. Independent profiles may run concurrently; explicit multi-profile work
reserves its entire set atomically. Registry changes and dynamic federation
cascades use a conservative state-root barrier. External process locks remain
authoritative, and lock acquisition is retried only before a callback starts.
Long chat polls release profile admission while waiting for remote events.

Opt-in frontend query and catalog diagnostics report aggregate outcomes and
latencies without profile names, item paths, input values or response contents.
The existing profile-work observer reports queue and execution durations. Agent
admission failures use the typed `admission-not-started` reason; execution
failures retain their existing ambiguity classification.

Files and Groups commands prepare missing native catalog state while holding the
mutation reservation, before dispatching one write. Group target validation uses
fresh native facts; file edits retain the caller's exact version. The unlocked
application generation must remain unchanged across preparation and file pickers.
See [state consistency](../../docs/state-consistency.md) for ownership, recovery,
interoperability and regression boundaries.
