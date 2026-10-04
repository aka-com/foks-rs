# FOKS mockup kit

Shared CSS, JavaScript and sample data for standalone HTML mockups of the
FOKS desktop app (`apps/desktop`). The kit reproduces the app's tokens, shell
and components, and adds the chrome a review mockup needs: a title bar, state
tabs, a theme switch and a notes panel.

| File | Purpose |
| --- | --- |
| `kit.css` | Tokens copied from `apps/desktop/kit/tokens.css`, a port of the app's shell and components, mockup chrome, responsive and motion rules. |
| `kit.js` | No dependencies. Exposes `window.Kit`: theme, states, notes, toasts, menus, popovers, dialogs, drawers, icons, sample data and render helpers. |
| `inline.mjs` | Copies the current `kit.css` and `kit.js` into a page between markers. |
| `icons.mjs` | Regenerates the kit's icon set from the repository's `lucide` package, or prints extra icons for one page. |
| `template.html` | A complete example: today's Chat and Files screens, and a gallery of every component. Start from it. |

Open `template.html#gallery` to see every component with its class names.

## Quick start

```sh
cp docs/review/mockups/_kit/template.html docs/review/mockups/<topic>.html
# edit the page, then:
node docs/review/mockups/_kit/inline.mjs docs/review/mockups/<topic>.html
```

Mockups are single files. The kit is always inlined, never linked: do not add
`<link>` or `<script src>` references to `_kit/`. Re-run `inline.mjs` after the
kit changes; `node inline.mjs --check page.html` exits 1 when a page carries a
stale copy. Never edit the inlined copy; edit `kit.css` or `kit.js` and
re-inline.

## Page skeleton

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Channel management</title>
<style>
/* KIT:CSS:BEGIN */
/* KIT:CSS:END */
/* Page styles: only what this mockup adds, with tokens only. */
</style>
<script>
/* KIT:JS:BEGIN */
/* KIT:JS:END */
</script>
</head>
<body>
<header class="kit-bar">
  <div class="kit-id">
    <span class="kit-badge">Mockup</span>
    <h1 class="kit-title">Channel management</h1>
    <p class="kit-desc">One line: what this mockup proposes.</p>
  </div>
  <div class="kit-tools">
    <span class="seg txt" role="group" aria-label="Theme">
      <button type="button" data-theme-choice="light">Light</button>
      <button type="button" data-theme-choice="dark">Dark</button>
      <button type="button" data-theme-choice="system">System</button>
    </span>
    <button type="button" class="btn cap kit-notes-btn" data-notes-toggle aria-controls="kit-notes"><i data-icon="file-text"></i>Notes</button>
  </div>
  <nav class="kit-tabs" role="tablist" aria-label="Mockup states">
    <button type="button" role="tab" class="kit-tab" data-state-tab="today">Today</button>
    <button type="button" role="tab" class="kit-tab" data-state-tab="proposed">Proposed</button>
  </nav>
</header>
<main class="kit-stage">
  <div class="window" id="app-window" role="group" aria-label="FOKS desktop app (mockup)">
    <div class="app">
      <nav class="side rail" aria-label="Main navigation">…</nav>
      <main class="main">
        <div class="topbar">…</div>
        …screen…
      </main>
    </div>
    <!-- dialogs, menus and popovers of this window, hidden -->
  </div>
  <aside class="kit-notes" id="kit-notes" aria-label="Review notes">
    <div class="kit-notes-head"><h2>Notes</h2></div>
    <section class="kit-note" data-notes-for="today">…</section>
    <section class="kit-note" data-notes-for="proposed">…</section>
  </aside>
</main>
<script>
  // Page script: render from Kit.data here, synchronously.
</script>
</body>
</html>
```

Order matters:

- The kit script sits in `<head>`. It applies the saved theme and notes
  preference before first paint, then wires the page on `DOMContentLoaded`.
- Page scripts go at the end of `<body>` and render synchronously. They run
  before the kit wires the page, so icons, states and triggers in their output
  work. Markup injected later needs `Kit.refresh(element)`.
- Put the rail, topbar and every overlay of a window inside that `.window`.
  The window is the positioning context for sheets, menus, toasts and drawers.

Copy app markup from `template.html` or from the live mock build
(`http://localhost:4719/?state=chat|files|teams|devices|settings`), not from
memory. The kit keeps the app's class names, so DOM copied from the running app
renders correctly once its `<svg>` elements are replaced with
`<i data-icon="…">`.

## States and notes

A mockup is a set of states, one per tab. The current state is the
`data-state` attribute on `<body>` and the URL hash (`page.html#proposed`), so
a link can point at one state.

| Attribute | Effect |
| --- | --- |
| `data-state-tab="a"` | A tab. Click or arrow keys (Left/Right/Up/Down, Home, End) select it. |
| `data-show-state="a c"` | Shown only in states `a` and `c` (sets `hidden` otherwise). `*` means every state. |
| `data-hide-state="a"` | Hidden in state `a`. |
| `data-notes-for="a"` | A notes section shown only in state `a`. Sections without the attribute always show. |
| `data-on-state="a"` | Gets class `on` and `aria-current="page"` in state `a` (rail items, channel rows). |
| `data-state-class="a:with-details b,c:other"` | Adds the class after the colon in the listed states. |
| `data-goto-state="a"` | Clicking it switches to state `a` (rail items, links between states). |

Prefer one window whose parts change by state over one window per state. The
template's window switches its crumbs, primary button, content and details
column with these attributes.

Changing state closes open menus, dialogs and drawers. A state that should
show a sheet or menu open as a still picture draws it with
`data-show-state`, without `data-dialog`/`data-menu` triggers.

### Notes content

Each state has a `.kit-note` section: an `h3` naming the state, a short
paragraph saying what the state shows and which app files it touches, then
findings. Start the notes panel with the layer legend from the template.

```html
<ul class="kit-findings">
  <li class="kit-finding">
    <div class="chips"><span class="fid">CHAT-07</span><span class="layer agent">Needs agent/IPC</span></div>
    What changes and why, in one or two sentences.
  </li>
</ul>
```

| Chip | Meaning |
| --- | --- |
| `.layer.data` "Ships on today's data" | Frontend only; the data already exists. |
| `.layer.agent` "Needs agent/IPC" | Needs new agent behaviour or IPC. |
| `.layer.protocol` "Needs protocol/server" | Needs protocol or server work. |
| `.layer.deferred` "Deferred" | Out of scope for now. |
| `.fid` | Monospace finding id, such as `CHAT-07`. Use your topic's prefix and number findings in order. |

Every chip carries its words; colour is never the only signal. Use
`h4` for sub-headings inside a note.

## Class reference

Ported classes keep the app's names and values. Kit-only classes are marked.

**Window and shell**

| Class | Notes |
| --- | --- |
| `.window` | Framed app window, 820px tall on desktop, internal scroll regions. A size container (`kit-window`). `.window.kit-mini` sizes to content for small demos. |
| `.app` | Grid: rail, main. `.app.with-details` adds the 300px details column. `.app.side-narrow` narrows the rail track to 56px. |
| `.side.rail` | Blue rail. Children: `.traffic` (`.lights > .light` ×3, `.side-collapse`), `.rail-brand` (`.mark` with `paw-print`, `.lab`), `.rail-tabs > .nav`, `.side-bottom`, `.rail-foot > .who`. `.is-narrow` collapses it to icons. |
| `.nav` | Rail item: icon (`.rail-files-icon`, `.rail-chat-icon` on those two), `.t` label, optional `.rail-tail.count` (orange count), `.rail-tail.count.warn`, `.rail-tail.dot.warn` (attention dot). Give the tail an `aria-label`. |
| `.who` | Account footer: `.kico.round.group.account.avatar`, `.t > b + small`, `.chev`. |
| `.main` | Main column. |
| `.topbar` | 48px header: `.crumbs` (buttons, `.sep` with `chevron-down`, last button `.cur`), `.grow`, `label.search.no-shortcut.topsearch` (icon, input, `kbd`), `.btn.primary`, `.global-refresh-wrap > .sync` (`.dot`, `.t`). |
| `.kit-drawer-btn` | kit: first topbar button; visible only when the window is narrower than 760px. `data-kit-drawer` makes it open the side column. |
| `.path`, `.path.ruled`, `.loc`, `.loc-copy`, `.header-action` | Page header under the topbar. |
| `.body` | Scrolling page body (20px side padding). |
| `.kit-split > .kit-side + .kit-main` | kit: generic side column plus main, for screens the app does not have yet. `--kit-side-w` sets the side width (264px). |

**Chat** (`chat.css`)

`.chat-screen` (grid: `--chat-inbox-w` 264px + conversation) ·
`.chat-inbox > .chat-inbox-scroll` · section label
`.sec.chat-inbox-label` · `.chat-team > .chat-team-head` (team mark
`.kico.group`, `.t > b + small`, `.chat-team-chev`) +
`.chat-channel-list > .chat-channel` (`.hash`, `.t > .n`, `.lock`,
`.chat-unread`; states `.on`, `.unread`, `.muted`, `.hidden`, `.off`) ·
unavailable section: `.sec.chat-nochat-label` then
`.chat-team-head.off` · `.chat-conversation` · `.chat-thread-header` (`h2`,
`.chat-thread-sub`, `.chat-thread-spacer`, `.btn.icon.chat-head-act`) ·
`.chat-messages-wrap > .chat-messages` · `.chat-history-edge` ·
`.chat-daysep` · `.chat-divider` (NEW) · `article.chat-message`
(`.grouped` for continuation rows) · `.chat-avatar` · `.chat-sender`
(`.you`) · `.chat-message-text` (`p`, `code`, `pre`, lists, `blockquote`;
the app draws code without a background) · `.chat-composer` (`textarea`,
`.chat-send`) · `.chat-info` panel (`.chat-info-head`,
`.chat-info-identity`, `.chat-info-rows > .chat-info-row`,
`.chat-info-foot`) · `.chat-empty` · `.chat-locked` · `.chat-pending` ·
`.chat-outgoing.sending|.queued`, `.chat-send-status` · create sheet form
`.chat-create`.

Build message lists with `Kit.render.thread()` rather than by hand: it applies
the app's day separators, NEW divider and five-minute grouping.

**Files and details** (`files.css`, `shell.css`)

`.folder-layout > .folder-split` (grid: `--files-tree-w` 232px + list) ·
`.tpane` (`.filt > .seg.txt`, `.tscroll`, `h6` with optional `.plus`) ·
tree row `.fn` (`style="--d:1"` for depth; `.root`, `.on`, `.quiet`;
children `.twist` [`.open`, `.none`], `.fselect` [icon or `.smallmark`, `.nm`],
`.fact`, `.c` count) · `.lpane` (`.lt` with `.where`, `.sub`, `.sp`, `.seg`) ·
`.body.folder-body` · header `.hdr.cols.loc` · rows `.row.one.cols.loc`
(`.name` [`.ic.k` key or `.ic.f` file, `.nm`, `.fpath`], `.cell.kind`,
`.cell.mark`, `.cell.num`, `.acts`; `.sel` for the selected row; use
`Kit.render.itemRow()`) · `.empty`, `.nextup`, `.lnotice`, `.dropzone`.

Details panel: `aside.details` (third child of `.app.with-details`) ·
`.dh` (`.kic.Password|Document`, `.t > h2 + small`, `.x`) · `.scroll` ·
`.detail-actions` · `.sec` · `.prev > .irow` (`.k`, `.v`, `.v.mask`, `.a` with
`button.value-action`; `.irow.rev` puts the value on its own line) ·
`.inset > .fr` (`.v.mark` with `.smallmark`) · `.pillrow > .pill` ·
`.who > .sec + p`.

**Settings, devices, teams**

`.settingslayout` (160px section nav + page) · `nav.side.subnav-side >
.tabs[role=tablist] > .tab` (`.on`) · `.subnav-page` · `.settings-main`
(`.account-main` spacing) · `.inset.settings-inset` (`.middle`, `.wide`) rows
`.fr` with `.k`, `.v`, `.a` · device rows `.fr.devrow` (`.device-icon`,
`.t`) · server rows `.fr.srow` (`.v.srv`, `.smark.ok|.bad`) ·
`.verified-mark` · `.account-fact-link` · Teams list `.body.nav-rows >
.rowline > .row` (`.kico`, `.name > .tt + small`, `.tail` with `.chip`s and
`.go`; `.row.off`) · roster `.rt > .prow`, `.who2`, `.rowtail` · `.swatches`.

**Controls**

| Class | Notes |
| --- | --- |
| `.btn` | Secondary. `.primary`, `.danger`, `.primary.danger`, `.icon` (32px square; needs `aria-label`), `.cap` (24px pill, the small button), `.on`, `[disabled]`, busy: `<span class="spin">` plus "Creating…". |
| `.lnk` | Text button in accent ink. |
| `.seg` / `.seg.txt` | Segmented control; buttons with `.on` and `aria-pressed`. `data-kit-seg` on the group makes clicks switch it. |
| `.tabs > .tab` | Underlined tabs, with `.n` counts. |
| `.chip` | `.you`, `.warn`, `.ok`, `.bad`. `.tag`, `.tag.warn`, `.badge`, `.pchip`, `.chat-unread`. |
| `.toggle-switch[role=switch]` | kit: the app uses checkboxes; use only where a mockup proposes a switch. Clicks toggle `aria-checked`. Wrap with a label in `.switch-row`. |
| `.select > select + i[data-icon=chevron-down]` | kit: native select styled like the Files sort control; `.select.sm` for the pill role select. |
| `.card-select > .card-select-trigger` | The app's custom select; its list is a `.card-select-menu[role=listbox]` of `.card-select-option[role=option]`, opened with `data-menu`. Choosing an option copies its `.t` into the trigger. |
| `.check` (`.on`) with `.bx` | Checkbox look. |
| `.inset > .fr` | Grouped field rows: `.k` label column (88px), `.v` value or input, `.a` actions. Inside `.chat-create`, inputs are boxed as in the Create channel sheet. |
| `.radios[role=radiogroup] > .radio[role=radio]` | Radio cards: `.rb`, optional `.rico`, `.t > b + small`; `.on`, `.off`. Clicks and arrow keys select. |
| `.sec` | Uppercase section label; `.sec .right` holds a trailing action. |

**Marks**

| Class | Notes |
| --- | --- |
| `.kico.group.account` | Person mark (26px circle). Sizes: `.md`, `.round` (30px), `.big`. |
| `.kico.group` | Team mark (26px rounded square). |
| `.smallmark` | 16px store mark in the Files tree, location cells and details. |
| `.kic.Password|Document|Folder|Store` | Tinted kind tiles; `.kico.Password|Document` solid. |
| `.hue-indigo` … `.hue-brown`, `.hue-none` | Set a mark's colour (the app's eight HUES). Or `style="--mark: var(--hue-red)"`. |

**Overlays**

| Class | Notes |
| --- | --- |
| `.backdrop[role=dialog] > .sheet` | Sheet: `.hd` (glyph, `.t > h2 + .step`), `.sb` body, `.ft` footer, primary action last. `.sheet.mid`, `.sheet.wide`. Use `role="alertdialog"` for destructive confirmations. |
| `.menu[role=menu]` | Menu items are buttons with `role="menuitem"`, optional leading icon, `kbd` shortcut, `.danger`, `aria-disabled="true"` with a `title` saying why; `.menu-separator`. `.rail-account-menu` for the account switcher. |
| `.popover` | kit: non-modal panel surface. The sync status popover is `.sync-popover` with `.sync-row`, `.sync-head`, `.sync-body`, `.sync-job-table`, `.sync-foot`. |
| `.toast` | Drawn by `Kit.toast()`; static copies need `.show`. |
| `.tooltip`, `[data-tip]` | kit: the app uses native `title` tooltips. `.tooltip` draws one statically; `data-tip="…"` shows on hover and keyboard focus. Keep `title` or `aria-label` for the accessible name. |
| `.pal-back > .pal` | Search palette (`.pal-q`, `.pal-scopes`, `.pal-results`, `.pal-grp`, `.pal-hit.sel`, `.pal-foot`). |

**Notices**

`.notice` (warning), `.notice.info`, `.notice.stop` (danger): icon, `.t` with
optional `.who` eyebrow, `h2`, `p`, `.fn`; actions in `.acts2`. Single-line
bands: `.band`, `.band.info`, `.band.stop`, `.band.ok`, with `.t` and
actions in `.a`. Use `circle-alert` for warning and danger, `info` for info,
`circle-check` for ok. Empty states: `.empty` (`.big`, `h2`, `p`, a button),
`.chat-empty`, `.chat-inbox-none`, `.callout`.

**Mockup chrome**

`.kit-bar` (`.kit-id` > `.kit-badge`, `.kit-title`, `.kit-desc`; `.kit-tools`;
`.kit-tabs > .kit-tab`) · `.kit-stage` · `.kit-notes` (`.kit-notes-head`,
`.kit-legend`, `.kit-note`, `.kit-findings > .kit-finding`, `.layer`, `.fid`) ·
gallery helpers `.kit-gallery`, `.kit-g`, `.kit-g-row`, `.kit-g-grid`,
`.kit-g-cell`, `.kit-g-cap`, `.kit-g-surface`, `.kit-g-stack`, `.kit-g-pair`.

## JavaScript

### Declarative attributes

| Attribute | Behaviour |
| --- | --- |
| `data-theme-choice="light|dark|system"` | Theme buttons. |
| `data-notes-toggle` | Shows or hides the notes panel. |
| `data-menu="id"` | Click toggles the menu `#id`; Arrow Down/Up on the trigger opens it at the first/last item. Sets `aria-haspopup`, `aria-expanded`, `aria-controls`. `data-align="end"` aligns the menu's right edge with the trigger. |
| `data-context-menu="id"` | Right-click, Shift+F10 or the ContextMenu key on the focused element opens `#id` at the pointer or the element. Make the target focusable. |
| `data-popover="id"` | Click toggles the non-modal panel `#id`; focus moves into it. |
| `data-dialog="id"` | Opens the dialog `#id` (a `.backdrop` inside the window). |
| `data-dialog-close` | Closes the dialog that contains it. |
| `data-autofocus` | Receives focus when its dialog or popover opens. |
| `data-dismissible="false"` | On a dialog: Esc and backdrop clicks do not close it. |
| `data-keep-open` | On a menu item: clicking it leaves the menu open. |
| `data-kit-drawer` | Opens or closes the window's side column as a drawer (narrow windows). |
| `data-drawer-close` | Closes the drawer. Channel rows, tree rows and section tabs inside a drawer close it too. |
| `data-kit-disclosure` + `aria-controls="id"` | Toggles `#id`'s `hidden`, the button's `aria-expanded` and `.open` (twists), and `.collapsed` on an enclosing `.chat-team`. |
| `data-kit-close="id"` | Hides a panel opened by a disclosure and resets its controls. |
| `data-kit-seg` | On a `.seg`: clicks move `.on`/`aria-pressed`. |
| `data-toast="text"` | Click shows a toast in the element's window. `data-toast-action="Undo"`, `data-toast-tone="warning"`. |
| `<i data-icon="name" data-size="14" class="extra">` | Replaced by the icon's SVG with class `ic extra`. `aria-label` makes it a labelled image. |
| `<kbd data-kit-kbd="K">` | Reads "⌘K" on macOS and "Ctrl K" elsewhere. |

Menus and popovers move into their trigger's window while open, so they are
clipped by the window like the app's portal, and return to their place on
close. One is open at a time. Esc closes and returns focus to the trigger; a
click outside closes; in menus and listboxes Up, Down, Home, End and the first
letter move focus. Dialogs trap Tab and Shift+Tab and return focus to their
trigger.

### `Kit` API

| Member | Description |
| --- | --- |
| `Kit.setState(name, {focus, hash})`, `Kit.state()` | Switch or read the state. |
| `Kit.setTheme('light'|'dark'|'system')`, `Kit.theme()`, `Kit.resolvedTheme()` | Theme; persisted in `localStorage` when storage is available. `?theme=dark` in the URL overrides for one load. |
| `Kit.setNotes(open)` | Notes panel; persisted. `?notes=closed` overrides for one load. |
| `Kit.toast(text, {action: {label, onClick}, timeout, tone, within})` | Shows a toast at the bottom of the window. `timeout` in ms (default 5000, 8000 with an action; 0 keeps it). `tone: 'warning'` for failures. Returns `{element, close}`. |
| `Kit.openMenu(idOrEl, trigger, {point, align, last})`, `Kit.openPopover(…)`, `Kit.closeMenu({focus})` | Programmatic menus and popovers. |
| `Kit.openDialog(idOrEl, trigger)`, `Kit.closeDialog(idOrEl)` | Programmatic dialogs. |
| `Kit.setDrawer(windowEl, open)` | Open or close a window's drawer. |
| `Kit.icon(name, size, extraClass)` | SVG string. `size` in px; without it the icon is 1em, so `font-size` sizes it, as in the app. |
| `Kit.icons()`, `Kit.registerIcons({name: innerSvg})` | List icons; add page-local ones (see Icons). |
| `Kit.refresh(element)` | Hydrate icons, apply state visibility and prepare triggers in markup added after load. |
| `Kit.on(name, fn)` | Events: `state` `{state, previous}`, `theme`, `notes`, `open`, `close`, `dialog`, `ready`. |
| `Kit.hue(name)` | The app's `hue()`: one of `indigo orange teal red green purple blue brown`. Use as `hue-${Kit.hue(name)}`. |
| `Kit.initial(name)`, `Kit.initials(name)` | `S`; `SO` for `sam.ortiz`. |
| `Kit.fmt.time(iso)`, `.day(iso)`, `.stamp(iso)`, `.plural(n, word)` | "9:12 AM"; "Today", "Yesterday", weekday within a week, else "November 14, 2023"; "Oct 4, 2026, 9:12 AM". |
| `Kit.esc(text)` | HTML-escape. |
| `Kit.render.thread(key or messages, {newFrom, you, edge})` | Message list HTML for `.chat-messages`. |
| `Kit.render.message(msg, {grouped, you})`, `Kit.render.messageText(text)` | One message; message text markup. |
| `Kit.render.avatar(name, {size, className, hue})`, `.teamMark(team, {size})`, `.storeMark(vault)` | Marks. |
| `Kit.render.itemRow(item, {selected, location, path, contextMenu})` | A Files list row. |

### Icons

`kit.js` carries 94 Lucide icons (stroke width 1.7, as in the app): see the
gallery's Icons section or `Kit.icons()`. The app's own mapping is in
`apps/desktop/src/icons.ts`: Files `folder`, Chat `message-square`, Teams
`users`, Devices `shield-check`, Settings `settings`, brand `paw-print`,
passwords `key-round`, documents `file`, crumb and twist chevrons
`chevron-down` (rotated by CSS).

For an icon the kit lacks, generate it from the repository's package and
register it in your page script; do not edit `kit.js` from a mockup:

```sh
node docs/review/mockups/_kit/icons.mjs calendar-clock
# prints Kit.registerIcons({ 'calendar-clock': '<path …/>…' });
```

Register before the kit wires the page (in the end-of-body page script).
To add icons to the kit itself, edit `KIT_ICONS` in `icons.mjs` and run
`node icons.mjs --kit`.

## Data

Every mockup uses `Kit.data`, so names, counts and dates agree across pages.
Today is Sunday 4 October 2026 (`Kit.data.today`); messages are dated
Saturday 3 and Sunday 4 October.

```text
me        { username: 'satoshi', host: 'foks.example.net', server: 'Personal server', alias, hue: 'red', initial: 'S' }
accounts  [{ username, alias, host, server, current, hue }]   satoshi (personal), vitalik (work, Acme)
servers   [{ host, label, verified }]                          foks.example.net "Personal server", acme.example "Acme"
people    { [username]: { username, host, hue, initial, you?, bot? } }
            satoshi, hal, priya.n, vitalik, sam.ortiz, lee, deploy-bot (bot)
teams     [{ id, name, host, server, hue, fileHue, initial, yourRole,
             members: [{ username, role: 'owner'|'admin'|'member', bot? }],
             chat: { enabled, reason? },
             channels: [{ id, name, description, audience: 'everyone'|'admins', readers?, unread, newFrom? }] }]
            household  Personal server, 2 members, #general "A place for the whole team.", #groceries
            engineering Acme, 6 members, #general, #deploys "Release and rollout announcements",
                        #incidents (admins; readers satoshi, priya.n, vitalik), #random
            homelab    Personal server, chat.enabled false: "Chat is not enabled on Personal server"
requests  { engineering: 2, household: 2 }                      outstanding join requests (Teams count 4)
vaults    [{ id, name, kind: 'account'|'team', account?|team?, host, fileHue, initial, folders }]
            Personal (agents, documents, env, env/prod, logins, ssh), Work (Acme),
            Household (documents, streaming, wifi), Homelab, Engineering (deploy, onboarding, release)
items     [{ id, name, kind: 'Password'|'Document', vault, folder, size, bytes, version?, fields?, access?, sharing? }]
            13 items, as in Files › All items; github.com has the full details
devices   [{ id, name, kind, label, icon, current? }]           MacBook Pro (this device), Travel Mac, YubiKey 5C, cage 32
messages  { 'team/channel': [{ id, author, at: 'YYYY-MM-DDTHH:MM', text }] }
```

Helpers: `Kit.data.team(id)`, `.vault(id)`, `.item(id)`, `.device(id)`,
`.person(username)`, `.channel('household/general')` (team, channel,
messages), `.itemsIn(vaultId, folder)`, `.unreadTotal()`.

Message text uses the app's markup: `` `code` ``, `**bold**`, `*italic*`,
`[label](https://…)`, fenced code blocks, `- ` and `1. ` lists, `> ` quotes.

Two colours exist per team, as in the app: `hue` is `hue(name)`, used by
team marks in Teams and Chat; `fileHue` is the store colour the Files tree,
location cells and details use. People are coloured by `Kit.hue(username)`.
Account vaults are marked with the account's initial (`S`, `V`).

Add to the data in your page when a mockup needs more (a new channel, a
pending invitation), and say so in the notes. Do not rename or recolour the
canonical entries.

## Theming

- Colours come from tokens only: `var(--ink)`, `var(--muted)`, `var(--line)`,
  `var(--accent)`, `var(--surface)`, `var(--danger-wash)` and the rest of
  `kit.css` section 1. No hex, `rgb()` or named colours in page styles or
  inline styles; `--mark: var(--hue-…)` for marks.
- Every token has a dark value. Check each state in Light, Dark and System;
  System follows the OS and an explicit choice wins in both directions.
- The accent is the rail colour (`--rail`, `--accent`, `--accent-ink`,
  `--accent-wash`, `--accent-wash-2`, `--accent-border`). Text on solid
  accent is `--ink-inverse`.
- Status colours: warning `--amber-*`, success `--ok-*`, danger `--danger-*`,
  unread `--unread`. Keep the app's type scale (12px floor, 13–14px body,
  `--sans`, `--mono` for ids and code) and radii (`--radius-*`).

## Responsive rules

- The page never scrolls sideways and keeps 16px side gutters (plus safe-area
  insets). Long ids and code in notes wrap.
- 1180px and wider: the window and notes sit side by side; the window is 820px
  tall with internal scroll regions. Below 1180px the notes stack under the
  window. Below 760px the window is between 640px and 820px tall.
- Inside the window, breakpoints are container queries on the window's width
  (`@container kit-window`), so the window behaves as the app would at that
  window size:
  - under 1180px, `.app.with-details` overlays the details panel on the list
    (the app's rule);
  - under 860px, chat paddings shrink (the app's rule);
  - under 760px (kit): the rail becomes an icon strip, the side column
    (`.chat-inbox`, `.folder-split > .tpane`, `.subnav-side`, `.kit-side`)
    becomes a drawer opened by the topbar's `.kit-drawer-btn`, the search
    field collapses to an icon, topbar buttons drop their `.lab` text, and
    settings rows wrap.
- Wrap topbar and header button text in `<span class="lab">` and give the
  button an `aria-label`, so it can collapse to an icon.
- Write page-level responsive rules with `@container kit-window (…)` for
  anything inside a window and `@media` for page chrome.
- Check 400px wide: `document.documentElement.scrollWidth <= innerWidth` in
  every state.

## Accessibility

- Buttons are `<button type="button">`; icon-only buttons have `aria-label`
  and `title`. Decorative icons and marks are `aria-hidden` (the kit's icons
  are by default).
- Focus is always visible: the kit draws a 2px accent ring (rail: white).
  Do not remove outlines without a replacement.
- Menus use `role="menu"`/`menuitem`; disabled items use
  `aria-disabled="true"` and a `title` with the reason, so they stay
  reachable. Listboxes use `role="listbox"`/`option`. Dialogs use
  `role="dialog"` (`alertdialog` for destructive confirmations), `aria-modal`
  and `aria-labelledby`. Radio cards use `role="radiogroup"`/`radio` with
  `aria-checked`. Switches use `role="switch"`/`aria-checked`.
- Every control reachable by pointer is reachable by keyboard: context menus
  open with Shift+F10 or the ContextMenu key, so context targets need
  `tabindex="0"` if they are not buttons.
- State is never colour alone: counts carry numbers and `aria-label`s, dots
  carry `aria-label`/`title`, chips carry words.
- Motion: the kit honours `prefers-reduced-motion` (the app's global rule).
  Do not add motion that conveys state on its own.
- Keep the 12px type floor; `--mark-type` (10px) is only for initials in
  16px store marks.

## Checking a page

Playwright is available at `/opt/node-tools/node_modules/playwright`. A
minimal check, run from a scratch directory:

```js
import { chromium } from '/opt/node-tools/node_modules/playwright/index.mjs';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 400, height: 860 } });
page.on('console', (m) => m.type() === 'error' && console.log(m.text()));
page.on('pageerror', (e) => console.log(e.message));
await page.goto('file:///…/docs/review/mockups/<topic>.html?theme=dark#proposed');
console.log(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
await page.screenshot({ path: 'proposed-dark-400.png', fullPage: true });
await browser.close();
```

Compare desktop screenshots against the live mock build
(`http://localhost:4719/?state=…`; set
`document.documentElement.dataset.theme = 'dark'` there for dark). Use
`?notes=closed` to give the window the full page width.

## Known differences from the app

- The window is framed (radius, border, shadow) like the app's web mock
  `.window`, not full-bleed like the native window.
- The template's rail indicators follow `Kit.data`: Chat counts the 2 unread
  in `#deploys`, Teams the 4 join requests, Devices flags the work account
  without a recovery phrase; Settings has no dot because both servers are
  verified.
- Toggle switch, styled select, popover surface, tooltip, `.kit-split`,
  drawers and icon-only topbar controls do not exist in the app; mark their
  use as a proposal in the notes.
