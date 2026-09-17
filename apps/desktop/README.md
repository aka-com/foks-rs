# FOKS desktop

The web half of `foks-desktop`, with its own trust boundary, release train, and
command surface.

In production, the application selects the Tauri bridge, loads the catalog once,
validates all responses at runtime, and passes opaque store references back unchanged. Show and Read target each read one item at one exact version,
hold the returned string only in the open details panel, and drop it on Hide,
selection change or window blur. Copy value, Copy path and Download stay in Rust. List rows use
`apps/desktop/kit/virtual-list.ts`; cards are capped at 200 because the virtual list
does not model a wrapping grid. Account-store creates use must-not-exist;
edits, removes and file replacements carry the catalog's exact version.
Active authenticated groups can create the same four product kinds as account
stores. Group creates carry explicit read and write roles, with their reader
preview computed from the selected role and live roster. Text edits, streamed
file replacements and removals preserve the catalog roles and carry the exact
listed version. Because v0.1.9 stores files of at most 2,040 bytes in its
inline `small-file` encoding, a value that fails the explicit text read is
presented with Download and native Replace actions without changing the wire
or database format.

Group settings is opened from each group’s vault page and loads the agent's roster
and federation facts, computes every people, group and readable-item count, and
exposes the operations the command layer can perform. Group creation, discovery,
attention states and account-specific invite messages live under Settings ›
Groups. Member changes are restricted to unique, locally manageable usernames.
An inactive or ambiguous federation admission has no extra payload. Active ad-hoc
groups retain read-only roster facts but suppress member and federation actions.
Native clipboard hygiene uses `copy_text`. First run is a location
inside the same main window, with its sidebar replaced by the setup-step list.
Its versioned local checkpoint contains only nonsecret progress and display facts;
invite, passphrase, recovery phrase and prepared backup phrase values are held
only in their live form or one-time sheet and are never encoded. Reopening
queries the agent's authenticated pending-operation list before showing a
Resume action. Set up again returns to the first question without discarding
completed steps. Creating the first group advances only after a fresh catalog
contains exactly one matching active group with the expected entity kind; its
authenticated name, alias and id become the Done screen's vault target.

Servers & devices now uses passive signed status plus explicit Check. Its
five-row status list keeps Host facts, Trust and the response inspector behind
a Details disclosure; no checked-at, trusted-since, reachability or latency is
invented. A signed expiry at or before the current clock stops the whole
profile when its pinned protocol requires a compatibility lease. A missing or
unreadable required expiry also fails closed under the distinct “signed
check-in unavailable” state; v0.1.9 profiles explicitly report that no lease is
required rather than manufacturing an expiry.
Reset uses a one-use preview token and names computed resumables and local
artifacts before typed confirmation. Settings exposes authenticated devices,
backup enrollments, Start/Accept/Finish/Resume pairing, account passphrase
operations, and the YubiKey lifecycle. Secret phrases, PINs, unlock codes and
passphrases remain live-form state only and are cleared when dispatched,
cancelled, concealed or unmounted.

**Account identity model:**

- **alias**: Profile-local display label.
- **profile**: Server configuration reference.
- **StoreRef**: Canonical application store identifier (`id` on `Store`, `store` on `Account`). Two profiles
  may both hold an account aliased `personal`, so Settings and Join carry the
  StoreRef — `?state=settings&section=macs&store=<StoreRef>` — and resolve
  exactly against it. A Settings address with no store names this Mac's first
  account and is rewritten to that account's StoreRef; a StoreRef that no longer
  resolves is reported as unavailable rather than silently replaced by another
  account, and reordering the catalog cannot change which account an address
  means. Switching accounts closes any open sheet and drops the device, backup
  and key lists that belonged to the previous one. The shell cancels stale
  asynchronous responses when switching accounts, ensuring responses from a
  previous account do not render in the newly selected account’s view. Alias
  comparisons survive only where the native command takes `(profile, alias)` —
  the backup-phrase and recovery operations — and there the profile comes from
  the selected StoreRef.

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
  `selectBridge`, checks the Rust app lock, and calls `loadWorld` only after an
  armed lock has been authenticated; only an explicitly mocked browser gets the
  fixture world. A native load constructs its world solely from validated
  command responses, loads passive signed status before exposing catalog facts,
  and then loads group rosters and federation admissions only for available
  profiles. Status notices come only from typed catalog failures, typed status
  failures, or the absence of a usable signed lease expiry. Device facts load
  only for available accounts in the relevant Settings surface.

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

| Path                       | What it is                                                                                                                                                                                                                                                                                                                                                                  |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.tsx`             | Entry point. One window; mounts `src/app-root.tsx`.                                                                                                                                                                                                                                                                                                                         |
| `src/app-root.tsx`         | App shell: window, sidebar, screen, details panel, deep links.                                                                                                                                                                                                                                                                                                              |
| `src/components/`          | Components: Button, Chip, Tag, Badge, Avatar, Stack, KindIcon, Inset, SectionLabel, Notice, Band, SegmentedControl, SplitButton, MenuButton, SearchField.                                                                                                                                                                                                                   |
| `src/shell/`               | Window chrome: `sidebar.tsx`, `page-header.tsx`, `toolbar.tsx`.                                                                                                                                                                                                                                                                                                             |
| `src/screens/`             | Screens: `items-screen.tsx` (list, cards, notices, empties), `groups-screen.tsx`, `store-access.tsx` (the shared unavailable-store takeover and All items summaries), `first-run-screen.tsx`, `servers-screen.tsx`, `settings-screen.tsx`, `details-panel.tsx`, `write-workflows.tsx`, `edit-value.ts`, `alerts-screen.tsx`, `scope.ts` (what is listed, in what order).     |
| `src/bridge.ts`            | The typed `Bridge` interface and the Tauri implementation.                                                                                                                                                                                                                                                                                                                  |
| `src/mock-bridge.ts`       | The same interface, using the fixture.                                                                                                                                                                                                                                                                                                                                      |
| `src/fixture.ts`           | The stable desktop fixture, as typed data.                                                                                                                                                                                                                                                                                                                                  |
| `src/model/`               | The pure model — roles, kinds, readers, format, lease. TypeScript only.                                                                                                                                                                                                                                                                                                     |
| `src/location.ts`          | `Location`, `Selection`, `transition`, the `?state=` codec, the store.                                                                                                                                                                                                                                                                                                      |
| `src/first-run-state.ts`   | Pure versioned resumable setup state and its explicit nonsecret checkpoint codec.                                                                                                                                                                                                                                                                                           |
| `src/icons.ts`             | The mock's 31 icons as structured data.                                                                                                                                                                                                                                                                                                                                     |
| `src/components/icon.tsx`  | `<Icon name size />`.                                                                                                                                                                                                                                                                                                                                                       |
| `src/styles/shell.css`     | `wave6/shell.css` copied in full, minus the shared tokens.                                                                                                                                                                                                                                                                                                                  |
| `src/styles/app.css`       | Styles that replace the mock browser chrome with the app window.                                                                                                                                                                                                                                                                                                            |
| `tests/`                   | `node:test` via `tsx`: goldens, source invariants, render tests.                                                                                                                                                                                                                                                                                                            |
| `tests/acceptance/run.mjs` | Layer 3: Chromium over the built UI, one load per deep link.                                                                                                                                                                                                                                                                                                                |

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

`src/styles/shell.css` is `dev/foks-desktop/iteration/wave6/shell.css` lifted
whole. The design is authoritative: change it there, then bring the
change here.

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

| `?state=`                                                                   | Opens                                       | What else it fixes                                                                                            |
| --------------------------------------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `all`                                                                       | All items                                   | —                                                                                                             |
| `personal`                                                                  | Personal (`acct:personal`)                  | —                                                                                                             |
| `work` · `household` · `homelab`                                            | that store                                  | —                                                                                                             |
| `password`                                                                  | All items                                   | selects the masked GitHub login                                                                               |
| `show`                                                                      | All items                                   | loads and reveals GitHub version 9 once                                                                       |
| `resource`                                                                  | All items                                   | selects the masked Anthropic API key                                                                          |
| `file`                                                                      | All items                                   | selects Household's emergency PDF                                                                             |
| `link`                                                                      | All items                                   | selects latest-key; its target stays masked until Read target issues an exact-version read                    |
| `group`                                                                     | Household (`team:household`)                | selects the Wi-Fi password                                                                                    |
| `new`                                                                       | All items                                   | Password sheet in Household with explicit group roles and computed reader preview                             |
| `new-group`                                                                 | All items                                   | Resource sheet in Engineering with explicit roles and computed reader preview                                 |
| `new-resource`                                                              | All items                                   | Resource sheet in Personal                                                                                    |
| `new-file`                                                                  | All items                                   | File sheet in Household; renderer receives paths, never bytes                                                 |
| `new-link`                                                                  | All items                                   | Link sheet in Personal                                                                                        |
| `group-new-text` · `group-new-link` · `group-new-file`                      | Engineering                                 | Phase 7 group create acceptance scenes for text, Link/symlink and streamed File writes                        |
| `exists`                                                                    | All items                                   | must-not-exist refusal; Open refreshes the invalidated catalog first                                          |
| `conflict`                                                                  | All items                                   | exact-version refusal with retained draft and Refresh and review                                              |
| `grid`                                                                      | All items                                   | `view=grid`                                                                                                   |
| `lease`                                                                     | Work (Acme)                                 | `lease=lapsed` — the whole world, not a place                                                                 |
| `inactive`                                                                  | Homelab                                     | group reports inactive; Resume creation uses its resumable operation                                          |
| `alerts`                                                                    | Alerts                                      | `lease=lapsed`, so the pane has its critical entry                                                            |
| `agent-lost`                                                                | Full window stop                            | Retry reconnects and refreshes without replay                                                                 |
| `groups`                                                                    | Settings › Groups                           | create, discovery, attention and invite sections                                                              |
| `people` · `party` · `federation`                                           | Engineering Group settings                  | People tab, with federation below the roster, or party panel                                                  |
| `danger`                                                                    | Engineering Group settings                  | Settings tab                                                                                                  |
| `store` · `items`                                                           | Engineering                                 | group vault                                                                                                   |
| `invite` · `add` · `demote` · `remove` · `admit`                            | Engineering Group settings                  | the named Group sheet                                                                                         |
| `create`                                                                    | Settings › Groups                           | named/ad-hoc Create group sheet, defaulting to Work (Acme)                                                    |
| `groups-lease` · `groups-inactive`                                          | Engineering Group settings or Homelab vault | distinct lease/inactive takeovers                                                                             |
| `manage`                                                                    | Household Group settings                    | People tab, without a Manage overlay                                                                          |
| `party-remove`                                                              | Engineering Group settings                  | the non-local removal refusal                                                                                 |
| `join`                                                                      | Settings › Groups                           | account-specific discovery and invite choices                                                                 |
| `join-invite`                                                               | Settings › Groups                           | invite sheet opened on the exact `acct:work` fixture store                                                    |
| `boot` · `who` · `address` · `no-address` · `checked` · `compare` · `error` | First run, steps 0–2                        | `path=invited` or `path=own` selects the setup route                                                          |
| `account` · `existing` · `protect` · `phrase`                               | First run, steps 3–4                        | account creation/recovery and the one-time backup sheet                                                       |
| `waiting` · `added` · `create-group` · `done`                               | First run, steps 5–6                        | invited discovery or own-group completion                                                                     |
| `checklist-invited` · `checklist-own`                                       | Get started inside the ordinary shell       | resumable nonsecret progress summary                                                                          |
| `first-run&step=<step>&path=<path>`                                         | the resumable first-run location codec      | used after the first in-app transition and across reload                                                      |
| `servers-list` · `servers-server` · `servers-lapsed` · `servers-rollback`   | Servers & devices                           | list/detail/stopped states from `04-servers.html`                                                             |
| `servers-reset` · `servers-add` · `servers-unprobed` · `servers-check`      | Servers & devices                           | typed reset, add/check and explicit result states                                                             |
| `settings-macs` · `settings-macs-work` · `settings-phrase`                  | Settings                                    | devices, pairing, recovery and one-time backup reveal; `settings-macs-work` names the exact `acct:work` store |
| `settings-keys` · `settings-enrol` · `settings-account`                     | Settings                                    | YubiKey lifecycle and passphrase/account status                                                               |
| `settings-agent` · `settings-about`                                         | Settings                                    | local agent status, inspect, version; `settings-agent` is an alias of About                                   |

`decodeLocation` returns `null` for display mode, item selection, or lease
modifiers because those properties represent presentation options or environmental
status rather than distinct navigation destinations. `decodeScene` encapsulates
these orthogonal state properties alongside the active location.
Everything is also addressable on its own — `sel=<store>|<path>`, `view=`,
`kind=`, `sort=`, `lease=` — and `sceneHref` writes back only what differs
from the default, so an ordinary `?state=all` stays `?state=all`.

Search query text is intentionally omitted from the URL address state so that
active filter queries persist across store navigation transitions within a session.
