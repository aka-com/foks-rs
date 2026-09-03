# dev/app-foks.html — parts

`dev/app-foks.html` is assembled from the files here, in filename order, by
`node dev/foks-mock-tools/assemble.mjs`. Edit the parts, run the assembler,
then load http://localhost:8133/dev/app-foks.html (repo root served over http).

| Part | Owns |
| --- | --- |
| `00-head.html` | page head, stylesheet links, scaffolding CSS, the control deck markup, `<script>` open |
| `10-core.js` | the `M` runtime: state, registry, flows, page picker, state panels, URL, render loop |
| `20-data.js` | fixture world (stores, groups, items, servers, parties, alerts), icons, app-wide state groups |
| `30-shell.js` | `M.renderTitlebar`, `M.renderSidebar`, page header/search/toolbar helpers, component helpers |
| `40-items.js` | vault pages (All items, each store), list/grid, empties, notices; details panel; write workflows (sheets) |
| `50-first-run.js` | first run: both paths, every step, resume, recovery, pairing from first run, Go CLI profile import |
| `60-groups.js` | group settings (People / Settings), party panel, federation, sheets; Settings › Groups |
| `70-settings.js` | Settings sections (devices, phrase, keys, account, about), pairing, passphrase, security keys, reset |
| `71-servers.js` | Servers & devices: list, status rows, check, add, lapsed, forget, reset |
| `72-alerts.js` | Alerts pane |
| `90-flows.js` | the walkthroughs (`M.flow`) that string pages and states together |
| `99-tail.html` | `</script>`, body close |

Tools, in `dev/foks-mock-tools/`: `assemble.mjs` builds the page,
`capture.mjs '<url>' <prefix> [steps.json]` shoots a PNG plus the DOM
(`ROOT=".frame"` to dump the frame alone, which is what diffs against a
`/tmp/foks-mock/cap/**` capture; `FULLPAGE=1 HEIGHT=1600` shoots the whole
page, deck included, which is how the deck itself is reviewed), and
`audit.mjs` walks every flow step and every page in a headless browser and
reports steps that land on the wrong page, keys a step sets that the landing
page does not declare, values it sets that light no chip, URL round-trips and
JS errors. Everything it prints but `chipClash` should be empty.

## The runtime API (10-core.js)

```js
M.app({ key:'lease', label:'Lease', note:'…', values:[{v:'fresh',label:'Fresh'},{v:'lapsed',label:'Lapsed',hint:'…'}] })
M.page({
  id:'all', title:'All items', path:['Vault','All items'],   // path = nesting in the picker; last = leaf label
  nav:'all',                                                  // which sidebar entry is on (read by 30-shell)
  note:'one line under the picker',
  controls:[ { key:'kind', label:'Kind filter', values:[{v:'All',label:'All'},…] }, … ],  // view state this page reads
  render: function (s) { return { main: html, details: html|'' , side?: html, titlebar?: html, takeover?: html, band?: html }; },
  overlay: function (s) { return html; }     // sheets/menus, rendered into the frame's #overlays
});
M.flow({
  id:'first-run-own', group:'First run',                       // group = the deck heading it sits under
  title:'First run — your own account',
  steps:[ { label:'Who', page:'fr-who', set:{ 'agent':'ready', 'v.choice':'own' }, note:'…' }, … ],
});
```

`render` may also return `windowTail` (raw html appended after `.app`, inside
`.window` — where the app hangs the agent-loss `.stopwrap`), `appClass`,
`windowClass` and `band`.

State: `M.s.<appKey>` for app-wide values, `M.s.v.<viewKey>` for view-specific ones.
`M.set('v.sheet','new')`, `M.get('lease')`, `M.go('eng-people', {'v.tab':'people'})`, `M.toast('…')`, `M.hint('…')`.
Markup hooks: `data-act="set" data-key="v.kind" data-val="Password"`, `data-act="go" data-page="…" data-set='{"v.x":"y"}'`,
`data-act="call" data-fn="name" data-arg="…"` (handlers in `M.fns`), `data-act="toast" data-text="…"`,
`data-bind="v.search" data-live` on inputs. Helpers: `M.esc`, `M.h(...)`, `M.icon(name, size)`.
A view key is null until set; declare the default as the first value in `values` and treat null as that.

## Core behaviours parts rely on (10-core.js)

* **Navigation clears view state.** `M.go(pageId)` to a different page nulls `sel`, `reveal`, `sheet`,
  `menu`, `confirm` (`M.leaveKeys`) and every other view key the destination page's `controls` do not
  declare — the app remounts its screens, so drafts and sheet state do not travel. A page that must
  receive an undeclared key on arrival lists it in `keeps: ['profile']`. A page may also define
  `leave(s, nextId)` for its own teardown.
* **Modal sheets make the app inert.** While a page's `overlay(s)` contains a `.backdrop`, or anything
  inside `#win` carries `aria-modal="true"` (which is how the agent-loss `.stopwrap` reaches it), the
  core sets `inert` + `aria-hidden="true"` on `.app` — the kit's `Dialog` does the same, and the app's
  own capture shows it. `M.appInert = true` forces it for overlays drawn elsewhere.
* **`windowTail`.** After `.app`, inside `.window`, the core appends `out.windowTail` if the page named
  one, else `M.windowTail(s)` (30-shell). That is the app's own place for the agent-loss stop, which
  covers the page rather than replacing it — so no page renderer has to know about `agent=lost`.
* **Toasts** are the kit's markup and the kit's clock (`ui/kit/toasts.tsx`). `M.toast(text)` is info;
  `M.toast(text, {tone:'warning'})` is the red pill with `role="alert"`; `{action:{label, onAction}}`
  adds the action button; `M.dismissToast(id)` removes one now. A toast lives
  `DEFAULT_DURATION_MS` = **2600 ms**, then loses its `.show` class (the CSS fade) and is removed
  `EXIT_DURATION_MS` = **300 ms** later — 2.9 s in the DOM, 2.6 s of it visible. `M.TOAST_MS` /
  `M.TOAST_EXIT_MS` / `M.TOAST_MAX` (5, oldest dropped) are those constants. An actionable toast with
  no explicit `durationMs` never expires (toasts.tsx:88-92), and repeating a message revives the entry
  in place instead of stacking a second one (`dedupeKey`, defaulting to the message).
* **Where `#toasts` sits inside `#overlays` is React mount order, not z-order.** `ToastProvider`
  renders `{children}` before its own portalled host, and a portal's node is appended at commit time,
  so an overlay the screen already had open on the **first paint** (a deep-linked sheet) lands
  *before* `#toasts`, and anything mounted afterwards (a sheet or menu opened by clicking) lands
  *after* it. The app's own captures show all three shapes:

  | capture | `#overlays` children |
  | --- | --- |
  | `cap/shell/new.html`, `exists.html` | `.backdrop`, `#toasts` |
  | `cap/shell/remove-confirm.html`, `new-menu-note.html` | `#toasts`, `.backdrop` |
  | `cap/shell/new-store-open.html` | `.backdrop`, `#toasts`, `.menu-portal` |

  The core keeps that bit in `M.overlayFirst`: true while the overlay the page returns is the one it
  returned on the very first render, false for ever once it has closed. A page overrides it with
  `overlayOrder: 'before' | 'after'` or by returning `overlayFirst` from `render`. A `.menu-portal`
  the page returns from `overlay(s)` (the CardSelect list) is always split off and placed after
  `#toasts`, because a menu only mounts when its trigger is clicked; queued menus (`M.drainMenus`)
  and `M.renderGlobalOverlay` come last, in that order.
* **A takeover suppresses the page's overlay.** `lock=locked` and `boot=loading|error` are what `App`
  returns *instead of* `<VaultShell>`, so the screen underneath never mounted and neither did its
  sheets: when `render` returns `takeover`, the core skips `overlay(s)` (toasts still stack). A page
  needs no guard of its own. `agent=lost` is the opposite case — the app keeps drawing the screen and
  hangs `.stopwrap` over it — and it goes through `windowTail`, so its sheets stay up.
* **Menus portal themselves.** `M.ui.menuButton` / `M.ui.splitButton` return only the trigger
  (`.menuwrap > button[data-menu]`); when `open` they queue the menu on `M.pendingMenus`, and the core
  appends `M.drainMenus()` after the page overlay and calls `M.placeMenus()` once it is in the DOM.
  That is the app's tree — `#overlays > .menu-portal > .menu[role=menu]` — with nothing asked of the
  page. A menu whose trigger is not a `menuButton` still renders `M.ui.menuPortal` from `overlay(s)`
  and positions it with `M.placeMenuPortal({trigger:'…', width:false})` from `after()`.
* **The deck** lists the current page's `controls` live and, behind the *Show N groups other pages
  read* toggle in the panel's title (open by default, remembered in `localStorage.foksMockDim`), every
  other page's groups dimmed and `disabled`, deduped by key + label; a key the current page declares
  hides other pages' groups of the same key. Chips write `v.<key>`; a value of `''` clears the key.
  A value no chip carries lights a mono `custom` chip instead of nothing, and any key the state holds
  that the page does not declare is named in a line under the groups. `audit.mjs` reports a flow step
  that sets such a value; a group whose values are only examples of free text (a draft, a typed name)
  says `freeText:true` and is exempt.
* **Chip labels are prose.** The group label is the sentence's subject and the chip is its value —
  `Sheet target` · `sam.ortiz`, not `target sam.ortiz`. Labels no longer have to differ from in-frame
  button text: `capture.mjs`'s `clickText` searches inside `#frame` first and only falls back to the
  whole page (`"deck": true` aims at the deck on purpose), and `audit.mjs`'s `chipClash` is advisory.
  The sibling quick-chips still carry their page id (`Personal store-personal`) because that id is what
  a deep link needs.
* **Walkthroughs are grouped.** `M.flow({group})` puts a flow under one of four headings — `First run`,
  `Main app`, `Groups`, `Settings & servers` — rendered in first-declared order; no `group` means
  `Other`. The steps are **numbered circles in a row and nothing else** — a dozen titles and page
  paths stacked vertically pushed the app frame off the screen. Each circle carries
  `<n> · <label> — <page path>` as its `title` and `aria-label`; the chosen step's `note` is printed
  once, beside ‹ Previous / Next ›, and ← / → step the flow from anywhere outside a text field. A
  step's `label` is therefore a tooltip and a screen-reader name, never a visible row: keep it short
  and put the sentence in `note`.
* **The deck's panel titles are bare.** `App-wide state` and `View state` carry no subtitle; the only
  thing beside a title is the *Show N groups other pages read* toggle. The page picker's tree is
  **fully expanded** — every `<details>` is `open` — with the current page's leaf highlighted and
  scrolled into view when the popover opens; a collapsed branch hid pages a reader had no other way
  of knowing about.
* **Backdrops dismiss on mousedown**, as the kit's `DismissibleDialog` does: a `.backdrop` carrying a
  `data-act` runs it on left mousedown when the backdrop itself is the target, and the following click
  is ignored. A drag that starts inside a sheet and ends over the backdrop does not close it.
* **Bound inputs** (`data-bind="v.key"`) keep an empty string as `''` — a cleared field is not an
  unset one — so read `s.v.key == null ? default : s.v.key`. Only deck chips fold `''` to null.
* **Flows**: `M.flow({id,title,steps})`; `M.currentFlow` / `M.step` are the running state; each step
  resets the view state unless `reset:false`, then applies `set`, then navigates.

## Data contract and shell helpers

`20-data.js` and `30-shell.js` are what every page builder calls. Nothing below
re-derives anything: if a page needs a caption, a count, a notice or a band, it
comes from here.

### `M.icons` / `M.icon`

`M.icons[name]` is the **inner** SVG body of each of the 37 icons in
`foks-ui/src/icons.ts` — element order and attribute order preserved, so
`M.icon(name)` reproduces `components/icon.tsx` byte for byte.
`M.iconNames` is the list in declaration order. `M.icon(name, size, {cls})`
appends `cls` to `class="ic …"`, which is how the chevron
(`<Icon className="chevron" />`) is drawn.

### App-wide keys (`M.app`, in this order)

| key | values (first = default) | what it changes |
| --- | --- | --- |
| `agent` | `ready`, `starting`, `lost` | the titlebar pill (`Agent ready` / `.agent.warn` `Agent starting`); `lost` hangs the `.stopwrap` off `.window` **over** the page (`M.windowTail`), and marks `.app` inert |
| `acme` | `ok`, `lapsed`, `checkin-unavailable`, `unavailable`, `unprobed`, `blocked` | the state of foks.acme-corp.com — see the mapping table below |
| `lock` | `off`, `locked` | the `.app-lock` card **instead of** the shell — `App` reads the lock before `loadWorld`, so there is no title bar under it |
| `boot` | `ok`, `loading`, `error` | `app-loading` / the boot-error card instead of the shell |
| `managed` | `none`, `local` | `appInfo.managedProfile`: the 3-step first run only. It is read in exactly two places — `app-root.tsx:144-156`, which diverts first run to `local` when the named profile's server is `ok`, and `first-run-screen.tsx:1567-1577`, which names the profile on that step. **Settings › About has no managed-profile line**, so no page under `set-*` may draw one and no flow step may claim one. |
| `refreshing` | `no`, `yes` | the titlebar refresh button disabled, labelled `Refreshing vaults and groups` |

`acme` maps onto the model like this — all six `ServerState`s, all six chips.
Only `ok` and `lapsed` exist in the running app (`applyLease` is the one world
knob it has, and its `?state=` presets never touch a server's state); the other
four are worlds this file builds, which is why `M.data.world` also builds the
crit Alerts note `bridge.ts:1706-1745` would have attached to a stopped server.

| `acme` | server.state | store state | takeover title |
| --- | --- | --- | --- |
| `ok` | `ok` | `normal` | — |
| `lapsed` | `lease-lapsed` | `lease-lapsed` | Check-in expired |
| `checkin-unavailable` | `lease-unavailable` | `lease-unavailable` | Server status unavailable |
| `unavailable` | `ok` | `catalog-unavailable` (via `unavailableStores`) | Store connection failed |
| `unprobed` | `never-probed` | `never-probed` | Server not checked yet |
| `blocked` | `blocked` | `blocked` | Server access blocked |

Alerts follow `notificationsOf`: every stopped server state adds one crit note first (badge 3 for
`lapsed`, `checkin-unavailable`, `unprobed`, `blocked`); `unavailable` leaves the server `ok` but adds
one `warn` `catalog-store-<i>` note per unlistable store with a live `Retry` (badge 4).

### `M.data` — the fixture as typed data

`servers` · `hostIds` · `accounts` · `stores` · `unavailableStores` · `items`
(all 15, including the `/ssh` Folder) · `plaintext` · `parties` (keyed by
StoreRef, in `sortRoster` order — which for this fixture is also fixture order —
each row carrying `name`, `initials`, `hue`) · `partyList` · `federation` ·
`groupDetailFailures` (+ `groupDetailFailure(ref, source)`) · `devices` ·
`DEVICE_ID_PREFIX` / `deviceIdFor(idHex)` / `syntheticDevice` /
`backupEnrollments` (the per-account Macs-pane facts, map/settings.md §6) ·
`yubi` (each account carries `state:'complete'`) · `backupPhrase` / `backupPhraseWords`
· `alerts` · `appInfo` · `appLock` · `COPY` · `captions` (the §3.4 table, fresh
and lapsed) · `goldens` (readers 3 / 5 / 5 / 2, `5 people · 1 group`,
`2 people`, badge 2 / 3 — computed, not typed).

Ids are the fixture's: `acct:personal`, `acct:work`, `team:eng`,
`team:household`, `team:homelab`; servers `personal`, `acme`, `partner`.
`M.data.items` is the mutable copy — a create pushes to it, a remove splices it,
and `M.data.world(s)` re-derives on the next render.

Model helpers, same names and same answers as `foks-ui/src/model/*` and
`screens/scope.ts`: `fmtSize`, `initials`, `hue`, `HUES`, `shortId`, `plural`;
`KINDS`, `KIND_LIST`, `kindLabel`, `kindOf`, `isLogin`, `nameOf`, `prefixOf`,
`rtype`, `rtypeWords`; `parseRole`, `roleRank`, `visibilityOf`, `admits`,
`formatRole`, `ROLES`; `storeOf`, `partiesOf`, `partyName`, `peopleLabel`,
`peopleGroups`, `admissionActive`, `readersOf`, `readableBy`,
`actionableGroupMember`; `storeNavigationOrder`, `storeDisplayOrder`;
`whereOf`, `itemsIn`, `itemAt`, `itemKey`, `plaintextOf`.

### `M.data.world(s)`

The fixture adjusted for `s.acme` (and `s.agent`) — `applyLease`,
`storeDescriptionState`, `storeDescription`, `storeHeadingDescription`,
`storeReadable`, `catalog`, `storeAccessBands` and `notesNow`, all applied.
Pages call this instead of re-deriving the lease world.

```js
{
  acme, leaseState,            // 'fresh' | 'lapsed' | 'unavailable'
  agent: { phase },            // 'Ready' | 'Bootstrap'
  servers, serverById, unavailableStores,
  stores: [{ …store,
    state,        // 'normal'|'blocked'|'lease-unavailable'|'lease-lapsed'
                  // |'never-probed'|'catalog-unavailable'|'inactive'
    description,  // the sidebar caption — including 'Roster unavailable' /
                  // 'Federation unavailable' when M.data.groupDetailFailures
                  // holds one for this store (60-groups writes that array
                  // from its `failure` key before calling world())
    heading,      // the page-header <small> (blank on any problem, a detail
                  // failure included)
    failure,      // null|'roster'|'federation'|'both'
    readable, serverName, parties,
    notice,       // null | { severity, title, detail, action, actionKind, profile }
  }],
  storeById, navOrder, vaults, groups, shares,
  items,                       // catalog(): 14 fresh, 10 lapsed
  bands,                       // [{ key, text }] — no severity: `.band`, not `.band.stop`
  alerts, alertCount,          // notesNow(): 2 fresh, 3 lapsed
}
```

### `M.ui.*`

Component helpers that emit the app's own DOM (world.md §5). Content slots are
**raw HTML**; attribute options are escaped; every helper takes `attrs`, a raw
attribute string for the `data-act` hook. Full signatures and one example each
are in the comment block at the top of `30-shell.js`:

`btn` · `chip` · `tag` · `badge` · `avatar` · `stack` · `kindIcon` ·
`kindGlyph` · `inset` · `insetRow` · `field` · `sectionLabel` · `notice` ·
`band` · `segmented` · `menuButton` · `splitButton` · `searchField` ·
`searchPlaceholder` · `sheet` · `tabs` · `toggle` · `radioGroup` / `radioCard` ·
`cardSelect` / `cardSelectMenu` (+ `M.placeMenuPortal(o)`) · `menuPortal` ·
`copyBox` · `pageHeader` · `toolbar` / `spacer` / `body` / `main` ·
`listHeader` · `listRow` · `emptyState` / `emptySearch`.

### Shell composites

* `M.renderTitlebar(s, page)` — the core calls it when a page does not supply
  `titlebar`. Refresh runs `M.fns.refresh`, which is `refreshAll`: ignored while
  one is in flight, `refreshing=yes` for the duration (button disabled, both its
  labels `Refreshing vaults and groups`), then the toast `Vaults and groups
  refreshed`.
* `M.renderSidebar(s, page)` — the core calls it when a page does not supply
  `side`. `page.nav` selects the row: a StoreRef, or `'all'` / `'alerts'` /
  `'settings'` / `'setup'`. An optional `page.status` (string or `fn(s)`) is
  rendered between the store lists and the footer — that is where first run puts
  its checklist. `M.navRow(o)` is exported for it.
* **Sidebar page ids** — every row navigates with `data-act="go"`:

  | row | page id |
  | --- | --- |
  | All items | `all` |
  | Personal / Work (Acme) | `store-personal` / `store-work` |
  | Engineering / Household | `store-eng` / `store-household` |
  | Homelab | `store-homelab` |
  | Alerts | `alerts` |
  | Settings | `set-devices` |
  | Set up new vault | `fr-who` |

  `M.storePages` is that StoreRef → page-id map, for anything else that links
  to a store.
* `M.globalTakeover(s)` — **every page renderer must start with**

  ```js
  render: function (s) { var t = M.globalTakeover(s); if (t) return t; … }
  ```

  It returns a render result for `lock=locked`, `boot=error` and `boot=loading`
  — the three screens `App` returns *instead of* `<VaultShell>` — and `null`
  otherwise. It does **not** cover `agent=lost`: the app keeps drawing the whole
  screen and hangs `.stopwrap` (`inset:44px 0 0 0`) off `.window` beside `.app`,
  so that is `M.windowTail(s)`, appended by the core, and no page renderer has
  to do anything. Retry sets `agent` back to `ready`, flickers `refreshing` and
  toasts `Connected to the local agent`. `M.stopwrap()` is exported separately.
* `M.renderGlobalOverlay(s)` — a hook for a frame-wide overlay, called by the
  core after the page's own `overlay(s)`. It is empty: the app lock lives in
  `globalTakeover`, where the app puts it.
* `M.fns.escape` — app-root's rule: close `v.sheet`, else `v.menu`, else
  `v.confirm`, else clear `v.search`, else drop `v.sel`. A page with a different
  rule replaces it from its own `after()`.


## Key-naming conventions

A view key belongs to the part that declares it, and a step in `90-flows.js`
must use that exact name — `M.go` drops any key the destination page does not
declare, and the deck cannot light a chip for a key it has never heard of.
`node dev/foks-mock-tools/audit.mjs` reports every mismatch.

**The shared vocabulary.** Where two parts mean the same thing they use the
same key and the same values, so the deck dedupes them into one group:

| key | values | meaning |
| --- | --- | --- |
| `applied` | `''` \| a token, comma-joinable | the mutation this pane's sheet performs has already run, replayed onto the fixture on every render. Tokens are the part's own: `created-password` / `saved-github` / `removed-github` (40-items), `add:eng:jules.park:Member:0` / `remove:eng:dana.okafor` (60-groups), `added` / `yes` (71-servers), `1` (70-settings) |
| `typed` | `''` \| `1` (\| `wrong`) | the typed confirmation matches the expected string, so the danger button goes live |
| `inspect` | `''` \| `1` | the "Inspect … response" disclosure is open |
| `written` | `''` \| `1` | "I have written these down" is ticked |
| `advanced` | `''` \| `1` | the Advanced / Path disclosure is open |
| `disclosure` | `''` \| `open` (\| `inspect`) | first run's `<details>` blocks (50-first-run keeps one name for all of them) |
| `sheet` | `''` \| a sheet id | which overlay is up; `''` means "the page decides" |
| `menu` | `''` \| a menu id | which menu is open |
| `empty` | `''` \| `yes` \| … | an empty-list state the fixture cannot reach |
| `account` | a StoreRef | the selected account (70-settings and 60-groups share it) |
| `adhoc` | `''` \| `yes` | `resumeGroupCreation` landed — a world change, so every vault page declares it too |
| `failure` | `''` \| `roster` \| `federation` \| `both` | `world.groupDetailFailures`; `M.data.world` reads the array, so the sidebar caption moves with it |

**Per-part keys.** `40-items`: `view`, `kind`, `sort`, `search`, `sel`,
`details`, `reveal`, `dest`, `readRole`/`readVis`, `writeRole`/`writeVis`, plus
the free-text draft fields (`site`, `user`, `pw`, `web`, `val`, `rname`,
`target`, `dpath`, `draft`, `ekind`, `epath`). `50-first-run`: `path`, `choice`,
`result`, `returning`, `refusal`, `pending`, `made`, `card`, `back`, `recovery`,
`phrase`, `progress`, `checked`, `discovered`, `pass`, `conf`. `60-groups`:
`tab`, `panel`, `target`, `role`, `visibility`, `remote`, `gkind`, `name`,
`username`, `created`. `70-settings`: `seg`, `started`, `devlist`, `yubistate`,
`device`, and the sheets' free-text fields (`alias`, `dev`, `tokens`, `pin`,
`puk`, `other`, `confirm`, `pw`, `pw2`, `serial`, `slot1`, `slot2`, `tries1`,
`tries2`, `invite`). `71-servers`: `profile`, `status`, `preview`, `addid`,
`addaddr`. `72-alerts`: `empty`.

A key a page **reads but does not declare** (first run's `path` on the steps
after `who`, the free-text draft fields) still deep-links and still survives a
flow step, because a step sets `M.s.page` directly rather than navigating — but
the deck draws it dimmed, which reads as "this page ignores it". Declare a key
in `controls` whenever the page actually reads it.
