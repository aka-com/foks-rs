# FOKS desktop

The web half of `foks-desktop`, with its own trust boundary, release train, and
command surface.

In production, the application selects the Tauri bridge, loads the catalog once,
validates all responses at runtime, and passes opaque store references back unchanged. Show loads one item at its exact version and keeps the value only in the open
details panel until Hide, selection change or window blur. Copy value and
Download stay in Rust. List rows use
`apps/desktop/kit/virtual-list.ts`; card view uses
`src/screens/files-grid.ts` to window complete grid rows at the current viewport
width, so it can scroll through the full loaded catalog. Account-store creates
use must-not-exist; edits, removes and file replacements carry the catalog's
selected dirent identity and exact version.
List rows show Name and Shared readers; the catalog has no write timestamp, so
the UI does not invent a Modified value from its version counter. Folder view
derives a folders-only tree from the filtered catalog, and search temporarily
returns to the flat list without changing the selected New-item destination.
Active authenticated groups can create the same four product kinds as account
stores. Group creates carry explicit read and write roles, with their reader
preview computed from the selected role and live roster. Text edits, streamed
file replacements and removals preserve the catalog roles and carry the selected
dirent identity and exact listed version. A deleted-and-recreated path cannot
redirect an open editor or replacement picker to the new entry. Because v0.1.9
stores files of at most 2,040 bytes in its
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
and Open in Chat, in the Chat column's default order: the general channel
first, then A to Z. Add channel opens the Create channel sheet with this group
selected. Hidden conversations remain listed but are excluded from the unread
count, matching the Create channel sheet. If the server does not support chat,
the page displays an explanation instead of a list. If access lapses while the
tab is open, it displays the same condition as the store access view, evaluated
with the shell's current clock. If synchronization completes with unfinished
work, the page displays a note above the loaded rows. Files links to the group
vault without duplicating the file browser; the row displays a folder mark, the
vault name, the catalog item count, and Open in Files. Both tabs are
addressable as `tab=channels` and `tab=files`. Arrow-key navigation replaces
the current location rather than adding a history entry.
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
be read; Add a group… follows the admitted ones. Each membership action is
its own sheet: Add FOKS user, Add FOKS team, Raise role, Lower role, Remove and
Create a team share their chrome and controls but nothing else. Add FOKS user
states the server (it is the team's), offers role cards that keep an unauthorized role in
place with the reason, and refuses a username the roster already holds before
sending; the agent's own refusal is stated under the field and cleared when it
changes. Add FOKS team lists remote groups already available on this device
because `add_federated_team_member` accepts a store rather than a name and
host. Raise role and Lower role state the member and their present role as a
fact row and offer only higher or lower roles respectively, within the acting
account's authority. Promotion keeps the existing membership and distributes
the newly accessible team keys. Create a team opens on the kind, named team or
ad-hoc share, then the name and the server and account; nothing is behind a
disclosure.
Settings states the group's
name with "Team names cannot be changed after creation." and the static Invite only policy.
A group with incomplete setup displays no tabs. The application navigates to the new
group once created. If creation is interrupted, an incomplete-setup banner
displays a “Finish setup” action above the locally available group details and
uses the same status text as the store access view. Group
creation, discovery and invitations live on the Teams page
itself; joining is the sheet the Teams header opens, and the Members tab no
longer repeats it inline. Joining runs in three steps: the invitation and the
server it is for, the resolved team with who is asking (the account, or a
named team it administers, picked from a list), then the sent request with
its state. Request membership prepares and submits in one press; a request
that stays prepared, or whose submission went unanswered, is finished from
the sheet's Resume a pending request list. A security key PIN is asked for
only when the agent requires one. A group that needs attention says so on its own row,
not a second time under it. On the team page, everything that did not finish
is one band: it counts the incomplete membership changes (interrupted member
additions and role changes, invitations left unsent, approvals cut short) and
says how many of them block other actions. Its Review opens the Unfinished
activity sheet, where the blocking changes lead with their own Resume, ahead
of the invitation and approval rows. Member changes are restricted to unique, locally
manageable usernames.
An inactive or ambiguous federation admission has no extra payload. Active ad-hoc
groups retain read-only roster facts but suppress member and federation actions.
Native clipboard hygiene uses `copy_text`. First run is a location
inside the same main window, with the rail replaced by the setup-step list.
Its versioned local checkpoint contains only nonsecret progress and display facts;
invite, passphrase, entered recovery phrase and prepared recovery phrase values are held
only in their live form or one-time sheet and are never encoded. Reopening
preserves acknowledged-but-unloaded accounts and uncertain account operations.
The v3 checkpoint retains the existing storage key and reads v2 checkpoints;
it adds nonsecret operation identifiers, SSO progress, and a selected group.
Native account-operation records in `account-operation-receipts-v1` tie each
attempt to its client state ID, pinned host, account alias, and operation
inputs.
An acknowledged operation record permits loading the account; a missing or uncertain
operation record never permits replaying creation or treating an existing alias as proof
of success. If an operation record cannot be read but the alias already exists on the
host, setup prompts you to continue with that existing account instead of
linking it automatically. Leaving setup does not cancel an in-flight account
operation.
Reopening
queries the agent's authenticated pending-operation list before showing a
Resume action. Set up again returns to the first question without discarding
completed steps. Creating the first group advances only after a fresh catalog
contains exactly one matching active group with the expected entity kind; its
authenticated name, alias and id become the Done screen's vault target.

Every security-key PIN, PUK and new-PIN input (team, invitation and
membership prompts, rename, bot and admin panels, SSO sign-in and sign-up, and
the device sheets) is the shared `SecurityKeyPinField`. It names a value
outside the `foks_yubi` rule (six to eight printable ASCII characters) before
it is sent, and enrollment retry counts are limited to one to fifteen
attempts. The agent reports a refused or blocked PIN only as `operation-failed`
text, so the field reads the remaining attempts from that text and states
them, or that the PIN is blocked, under the input. A prompt clears the refused
PIN, so the same value is not sent again unchanged.

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
recovery-phrase enrollments, Start/Accept/Finish/Resume pairing and the YubiKey
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
  recovery-phrase enrollment and account-recovery operations — and there the profile comes from
  the selected StoreRef. A StoreRef is opaque and is never drawn: the agent
  answers with its own identifier, which the fixture happens to write as
  `acct:<alias>` and the real agent writes as a JSON object, so a surface that
  needs to name an account draws the username, the alias and the server
  instead. The address is the one place a ref is written, through the
  `?store=` parameter the location codec has always encoded.

Items and folders can be renamed or moved within their current vault from their
context menu; items also expose Rename or move in the details panel. The sheet
offers a Name field and the vault's folder tree, including empty folders, with
New folder creating one in place through `create_folder`; a Path disclosure takes
an exact absolute path instead, and a folder it names that does not exist yet is
created first. A move never overwrites. The source dirent identity and version
remain fixed through concurrent-write retries. Deleting and recreating the same
path cannot redirect a stale selection to the replacement, even when its numeric
version is the same.

Copy to vault…, in the details panel and the row menu, copies a secret or
file into another vault: a personal login into a team, or a file from one team
to another, on the same server or a different one. The sheet takes the
destination vault (vaults this account cannot write to are listed but off), a
folder of its tree, a name, and in a team the read and write roles with the
same readable-by preview as a new item; a destination on another server is
named. `copy_item_to_store` reads the source and writes the copy in the host,
so its content never reaches the renderer. Keeping the original is the
default; Move the original to Trash sends a `trash_item` once the copy is
saved, and if that fails the copy stays and the toast says the original is
still in its vault. Folders are not copied.

Import passwords…, in the New menu, the palette's Actions and the empty
vault's cards, reads a CSV file exported by 1Password, Bitwarden, KeePass,
KeePassXC, LastPass or the Chrome, Edge, Firefox or Safari password manager.
The host opens the file in the native picker and parses it; the sheet
receives the file's name and, for each login, the name and folder it will
take, and lists rows that are left out (a Bitwarden note or card, a LastPass
secure note, a row with no user name, password or website, a value with a
line break) and the
columns that are not read, such as a TOTP column. The sheet takes the vault,
a destination folder (`/logins` unless New would save in another) and in a
team the read and write roles; each login keeps its own folder from the file
below it. Each login is saved as a password in the layout the New password
sheet writes, with its notes as `note:` lines, and a path that is already
taken is left as it is, so importing the same file again saves only what is
missing. The sheet closes with a toast only when every login was saved;
otherwise it stays open and lists each outcome, including each login left
out because its name was taken, since the item already there may be a
different login. A failure stops the run, and Retry sends the logins that
were not reached before those that failed. The export holds every password
in plain text, and the sheet says to delete it afterwards. An account
already used with the FOKS command line needs no export: its items are on its
server, and the empty vault's note points to Settings › Account › Import from
FOKS CLI, which connects that account.

Export passwords…, in a vault's folder menu at its top level and the
palette's Actions, writes the vault's passwords (what Files lists as a
Password, outside Trash) to a CSV file with the columns of a Bitwarden
export, which Import passwords and most password managers read. The sheet
states that the file is plain text, says to open it in a password manager
rather than a spreadsheet app (which can run a value beginning with `=`,
`+`, `-` or `@` as a formula), and is sent only once the vault's name is
typed; the host reads each password and writes the file named in the native
save dialog. Values are written as stored, without a formula-escaping
prefix, so that passwords read back unchanged. Documents and files are not
exported.

Several rows of the list can be chosen with ⌘-click, Shift-click, ⌘A,
⌘/Shift with Enter or Space, and cleared with Escape; a bar above the list then
offers Move to… and Move to Trash, and the row menu and the Delete key act on
all of them. Dragging rows (with pointer events, since the window's native drop
handler is reserved for files from the operating system) onto a folder of the
same vault in the tree or the list moves them there. A group in which this
account can change only some items still moves: the move sheet names the
items that stay, and the progress sheet lists each as not moved. Several items
are sent one `move_item` or `trash_item` at a time: each write retires the
host's catalog of its store, so that store's profile catalog is read again
before each item, and an item whose entry was replaced meanwhile is not sent.
A progress sheet lists each outcome; it closes with a toast when everything
moved, and otherwise offers Retry failed for the items whose request certainly
did not run. Clearing the chosen rows, or moving all of them away, returns
focus to the anchor row or to the row now in the first one's place.

The list is a `grid` (rows under the Name, Kind, Location and Size column
headers, with `aria-rowcount` and `aria-rowindex` covering unmounted rows),
and the card view is a `grid` of lines with `aria-colcount`. Each is one tab
stop (a roving tabindex; `src/screens/files-keyboard.ts` maps keys to
targets): Up, Down, Home, End, PageUp and PageDown move focus, scrolling the
virtual window to mount the target first. In the list, Right and Left move
between a row and its Copy, Reveal or Download buttons; in the card view
they move between cards, whose actions are in the row menu (Shift+F10 or the
ContextMenu key; list rows name it in `aria-keyshortcuts`). Enter opens
details (on a folder row, the folder), ⌘C copies a Password's password or a
text Document's value and ⇧⌘C a Password's user name (the commands the row's
Copy buttons call), F2 opens Rename or move, and Delete or ⌘Backspace moves
the row to Trash. The focused row stays mounted, one line beyond the
window's edge, when the list scrolls away from it, and focus returning to it
scrolls the list back to its place. A store's Trash list is the same kind of
`grid`, whose Right and Left reach Restore and Delete permanently; Enter
shows where the entry is stored, and Delete or ⌘Backspace deletes it
permanently. The vaults and the teams in the folder tree are each a `tree`
with one tab stop: Up and Down move between drawn rows, Right opens a folder
or moves into it, Left closes it or moves to its parent, Enter selects, and
Shift+F10 or the ContextMenu key opens the folder's menu. A store is a
parent when it draws Trash, even with no folders. A team's settings are in
its row menu as well as behind the gear its row shows on hover.

Delete moves an item, or a folder with everything in it, into the vault's
Trash: `/.trash/<role-tag>/` in the same store, one folder per read role,
chosen by the most restrictive read role on the item's path, through
`MoveKv`. It asks for no confirmation; its toast offers Undo for eight
seconds, and ⌘Z does the same. Several rows moved to Trash together are
offered Undo by their toast, or by the progress sheet when some stayed, which
restores each entry to its original path one `restore_trash_item` at a time;
an entry whose original path is taken again stays in Trash. ⌘Z undoes only a
single row's move. Each vault's Trash row lists its entries,
restores them (as a `(restored)` copy when the original path is taken) and
deletes them permanently, which the host allows only inside `/.trash`. The
exception is a team where no owner has created `/.trash` yet: a member who is
not an owner is offered permanent deletion of a file after confirmation, and
cannot delete folders. Empty Trash removes the entries this account can
change; Settings › Preferences can empty Trash automatically after 30 or 90
days on one device. Go clients list `.trash` as an ordinary folder. See the
Trash section of `book/20-desktop.qmd` for the layout and entry names.

**Unsupported operations:** Invitation revocation, expiry and requester
withdrawal remain unsupported.
The live bridge now loads each group roster and federation
admission, so Sharing and every readable-by count are computed from agent
responses rather than fixture facts.

## Running it

```sh
npm run dev:frontend # Vite dev server on http://127.0.0.1:1421 (strict)
npm run build:frontend # production bundle into apps/desktop/dist/
npm run test:foks-ui      # serial node:test via tsx — model, location, invariants, render
tsc --noEmit -p apps/desktop/tsconfig.json
```

`npm run build:frontend` fails when the startup JavaScript (the entry chunk
and the chunks it imports statically) exceeds the budget in
`bundle-budget.ts`. Code that only one of the screens in
`src/app/deferred-screens.tsx` needs loads with that screen instead.

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
  Vite removes the mock and fixture graph from that bundle. To preview the mock
  explicitly:

  ```sh
  VITE_FOKS_MOCK=1 npm run dev:frontend
  ```

  The mock chat (`src/mock/chat-mock.ts`) seeds each team from
  `src/mock/chat-mock-scenarios.ts`. By default a team has one unnamed channel
  holding one message from another roster member, the Owner unless the local
  account owns the team, or from the local account when it is the only member.
  The mock-only `chat-scenario=` parameter selects another seed: `busy` (four
  channels, one admin-only, and 120 messages in #general with 42 unread, from
  roster members, the local account and one UID absent from the roster),
  `failures` (unsent and unconfirmed ledger operations, one whose attempt the
  host rejects with status 12006, and a channel whose saved messages cannot be
  loaded because the local saved-message store is invalid) or `integrity` (a
  page with missing predecessors, a degraded inbox, and a stopped, a muted and
  a hidden channel). For example,
  `?state=chat&store=team%3Ahousehold&chat-scenario=busy` opens Household, the
  team whose fixture server offers chat. Render and unit tests import the same
  seeds and IDs. Every mock chat reply passes the same decoder
  (`decodeChatResult` in `src/chat-contract.ts`) that checks native replies;
  its scope check compares against the team the fixture store ID stands for,
  since fixture IDs such as `team:eng` are not the native JSON store
  references. As the agent does, the mock cuts notification history text to
  `CHAT_NOTIFICATION_TEXT_CHARS` (256) characters and reports an inbox
  preview's first line whole when it fits in 512 bytes, otherwise cut to
  make room for an ellipsis. Tests that fake
  `chat` directly can decode their replies the same way with
  `decodedChatBridge` (`tests/lib/decoded-chat-bridge.ts`).

  `src/mock/mock-bridge.ts` uses `src/mock/fixture.ts` with no network or timers;
  it loads only the app's versioned nonsecret first-run checkpoint so a mocked
  quit/resume walk includes completed setup facts. It assembles one module per
  `src/bridge/commands-*.ts` factory (`accounts.ts`, `core.ts`,
  `enrollment.ts`, `servers.ts`, `vault.ts` and `yubikey.ts` in `src/mock/`),
  which share the fixture's records through `src/mock/state.ts`. Its vault,
  enrollment and YubiKey commands, the account listing, the team roster and
  federation changes, and every server command except `list_servers` answer
  at the wire level: `createBridge` runs the same command factories over a
  transport whose host is `mockHost` (`src/mock/host.ts`), so their JSON
  responses pass the native decoders, and their CommandError rejections get
  the same origin and mutation-ambiguity handling as the native host's.
  `MOCK_HOST_MEMBERS` in
  `mock-bridge.ts` lists the members outside the vault factory that answer
  there, and `tests/mock-host.test.ts` checks that the host answers exactly
  the commands those members send. The other commands still return decoded
  values directly, among them the server, roster and federation listings,
  whose fixture records carry facts the native responses do not. It is
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
`copy_item_value`, `open_item_url` and `download_file` take
`{storeId, path, version}`, and `copy_item_field` takes the same plus
`field` (`password`, `username`, `url` or `value`). Both item copies answer
`{ok, clearAfterSeconds}`: the seconds after which the host clears the
clipboard if it still holds the copied value (`AUTO_CLEAR_SECONDS` in
`src-tauri/src/clipboard.rs`: 15 on Linux, 30 elsewhere), which the copy toast
states, for example "Password copied · clears in 30 s", and closes by then.
`store_usage` takes `{storeId}` and answers `{files, bytes}`: the vault's file
count and total ciphertext bytes from the agent's existing data-read path
(`BindDataAccount`, then `ReadData` with `usage`, narrowed to the team for a
team vault). Team info's Files section and the Settings account page's Storage
row show it as "About 2.3 GiB stored (encrypted) · 1,204 files". The server
walks the vault to compute it, so `src/store-usage.ts` reads it only when one
of those views opens, keeps it for the session, and reads it again only after
the vault's listed files change, on an explicit Refresh, or after access is
reset; the metadata reconciliation cadence skips it, including a retry after a
failed read.
The figure has no denominator: the server reports no limit, a Go host sets its
own, and the quota a write is refused against also counts directories,
metadata and envelopes, so a fraction or an early upload refusal would be
wrong.
`tests/bridge.test.ts`
pins the decoder side and the Rust command tests pin serialization so a drift
fails on one side or the other. `tests/command-contract.test.ts` checks the
command name and argument keys of every native call in `src/` against
`src-tauri/command-signatures.json`, which a Rust test derives from the
registered commands; an argument the Rust command requires must not be
optional or possibly `undefined` on the TypeScript side. A reference to
`invoke` or a transport wrapper that is not the callee of a checked call, such
as `checked.call(...)` or a wrapper passed as a value, fails the test. The
same file also fails when a `Bridge` method other than the few it lists has no
reference outside its `src/bridge/commands-*.ts` wrapper and the mock bridge,
so a registered command does not stay reachable from the renderer without a
feature that calls it.

`createBridge` in `src/bridge/create.ts` assembles the `Bridge` from the
`src/bridge/commands-*.ts` factories over a `Transport`
(`src/bridge/transport.ts`). The transport decodes each response, records
whether a failure arose before or after the host responded, and marks a
mutation's failure with an unknown outcome as ambiguous; `tauriBridge` uses
the transport over Tauri's IPC. The invitation actions and YubiKey commands
that only read are listed once, in `INVITATION_READ_ACTIONS`
(`src/invitation-contract.ts`) and `YUBI_READ_COMMANDS`
(`src/bridge/commands-yubikey.ts`); every other one is sent as a mutation.
The read-only invitation action `inbox-count` is not in the list and is also
sent as a mutation.

The per-action properties of `chat_request` are one table, `CHAT_ACTIONS` in
`src/chat-contract.ts`: the result kind each action is answered with, whether
the agent treats it as a mutation (which selects `checkedMutation`), and the
send service's local and coalescing rules. Two shared fixtures in
`crates/foks-agent-proto/tests/fixtures/` hold both languages to it:
`chat-actions.json` lists every action's canonical request, mutation flag and
result kind, and `chat-replies.json` holds accepted and rejected replies with
at least one accepted reply per action. The native tests in
`crates/foks-desktop/src/chat.rs` check them against `ChatAction`
serialization, `ChatAction::is_mutation` and `validate_chat_reply`;
`tests/chat-contract.test.ts` checks them against `CHAT_ACTIONS` and
`decodeChatReply`.

Phase 3 adds `create_text_item`, `edit_text_item`,
`remove_item`, dropped/native-picker file import and exact-version file
replacement, `resume_group_creation` and `abandon_group_creation`, the local
forgetting of a group whose creation can no longer be finished. Rust-originated
drop events contain
paths only; file bytes never cross into the renderer. The shell polls
`take_agent_connection_loss`; Retry calls `retry_agent_connection` and then
reloads the catalog, never replaying the interrupted write.

`trash_item` and `restore_trash_item` take `{storeId, path, version,
direntId}`, the second also a `destination` outside Trash, and answer
`{applied, path}` with the path the entry now has. `trash_item` fails with
`trash-unavailable` when a team has no `/.trash` and the attempt to create
it is refused by role (any other failure keeps its own code), and with `trash-name-taken` when the same path was moved to Trash
within the same second. `remove_item` takes the same selection and
`outsideTrash`; it removes an entry in `/.trash`, recursively for a folder,
or with `outsideTrash` a file in a team that has no `/.trash`.
`empty_trash` takes `{storeId, entries}`, each entry `{path, version,
direntId}`, and removes them all in one mutation.

`copy_item_to_store` takes the source selection `{storeId, path, version,
direntId}`, a `targetStoreId` and `targetPath`, and in a team target both
`readRole` and `writeRole`. It reads the source at that version and dirent,
refuses folders, links and the source's own store, and creates the copy
with a `Create` precondition, so an occupied path fails with
`already-exists`. A large file is read whole before the upload, since the
agent admits an upload exclusively per profile; until then its chunks are
held in an unlinked file in the system temporary directory, each sealed with
ChaCha20-Poly1305 under a key held only in memory. That directory needs free
space for the whole file (on a Linux system where `/tmp` is a tmpfs, this is
memory or swap); a file larger than its free space fails with `copy-buffer`
before it is read. The mutation reservation covers the source's profile as
well when the two stores are on different servers, and after an ambiguous
copy across servers the sheet refreshes every server, since a refresh of one
profile neither reads the destination nor releases that reservation.

`begin_password_import` opens the native picker for a CSV file of at most
16 MB and 10,000 logins, parses it in the host and holds the logins under a
random id, replacing any import held before; it answers `null` when the
picker is cancelled, or `{id, fileName, entries, skipped, ignoredColumns}`,
where each entry is `{name, folder, document}` (encoded path components;
`document` when the login is longer than 2,040 bytes and is stored as a
file) and each skipped row is `{row, name?, reason}`. A file that is not
UTF-8 or has no password column fails with `import-file`.
`commit_password_import` takes the `importId`, the `entries` to save (indexes
into the preview), a `storeId`, a destination `folder` outside Trash and, in
a team, both roles, and an `onProgress` channel. It writes each login with a
`Create` precondition and `mkdir_p` and answers with one `{index, status,
message?}` per entry, also sent on the channel as it is known: `created`;
`exists` for an occupied path; `failed` or `uncertain` for the write that
ended the run; `not-started` after it. An id the host no longer holds fails
with `import-expired`. `discard_password_import` drops the held logins; the
app lock drops them as well.

`export_logins` takes a `storeId` whose items the catalog read completely,
opens the native save dialog and answers `{saved, exported, skipped}`:
`saved` is false when the dialog was cancelled, and `skipped` counts
passwords whose text is not UTF-8. It reads each password at its listed
version, writes the records to a temporary file beside the destination
(created readable only by its owner on Unix) and moves it into place once
all are written; a vault with no passwords fails with `invalid-request`.

Phase 4 adds `list_accounts`, `create_group`, member add/promote/demote/remove and their
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
both and retain their native Owner defaults. The app creates no protocol
symlinks, and the catalog decoder drops the `Link` entries the agent reports
for existing ones after validating them. Folder creation exists at the native
boundary for protocol completeness but is not exposed as a fifth product kind.
Edit, replacement and removal send no role arguments and preserve the roles
authenticated in the catalog.

## Layout

| Path                           | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/main.tsx`                 | Entry point. One window; mounts `src/app-root.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/app-root.tsx`             | App shell: window, rail, screen, details panel, deep links.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `src/shell-services.tsx`       | The services `VaultShell` provides to the screens and sheets under it, built by `src/app/services-runtime.ts`: the bridge, the lease clock, the command and mutation error handlers and the catalog reloads, each with a stable identity, and the snapshot the shell shows. `useWorkflowAccess` (`src/workflow-context.tsx`) reads that snapshot when it is given none.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/app/`                     | The composition root under `app-root.tsx`: `app-bootstrap.ts` (the launch progress and the first-paint deadline), `vault-shell.tsx` (the unlocked window: rail, topbar, screen, details panel and overlays), `screen-router.tsx` (the screen each location shows), `deferred-screens.tsx` (first-run setup, Chat, Teams, Devices, Settings and team settings, each loaded by dynamic import into a chunk of its own; the shell reaches their state through small modules such as `screens/new-chat-request.ts` and `screens/unrouted-notices.ts`), the `*-runtime.ts` modules `vault-shell.tsx` builds its services from, and the blocking, exit and fault boundaries.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `src/components/`              | Components: Button, Chip, Tag, Badge, KindIcon, Inset, SectionLabel, Notice, Band, SegmentedControl, SplitButton, MenuButton, SearchField, PasswordInput (the login editors' Password row with Show, Generate and strength), SheetDialog (its `icon` draws the server-mark glyph), and the marks and rows several screens share: AccountMark, StoreMark, GroupMark (the one mark a group carries everywhere), RoleChip and PartyRow (a group's member roster and the details panel's Sharing list). `index.ts` exports these primitives. The folder also holds the panels several pages open, which import each primitive from its own module: invitations (`invitation-panel.tsx`, `invite-new-user-sheet.tsx`, `invitation-activity-sheet.tsx`, `membership-requests.tsx`, `unfinished-activity.tsx`), bots, admin, rename, local alias and single sign-on (`sso-panel.tsx`, `sso/`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/shell/`                   | Window chrome: `sidebar.tsx` (the rail), `topbar.tsx` (the crumbs, the back chevron and the shell's own controls), `attention-centre.tsx` (the topbar's attention bell and its popover), `page-header.tsx`, `search-palette.tsx` (⌘K), `shortcut-sheet.tsx` (⌘/), `sync-popover.tsx` (the refresh status behind the refresh button) and `sync-summary.ts` (the per-server summary and copied diagnostics it draws), `toolbar.tsx`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/screens/`                 | Screens: `items-screen.tsx` (list, cards, notices, empties), `groups-screen.tsx` (the group page, its Members and Settings tabs and the add/role/remove/create sheets), `group-tabs.tsx` (the Channels and Files tabs and the unfinished-group page), `store-access.tsx` (the shared unavailable-store takeover and All items summaries), `first-run-screen.tsx`, `servers-screen.tsx` (`ServersSection`, the server list Settings draws, and one server's page; its state, pinned identity and expiry text are in `servers/server-model.ts`), `settings-screen.tsx` (the Settings tab and its sub-navigation), `devices-screen.tsx` (one account's Macs and device keys, recovery phrases and security key enrollments, and one key's own page), `devices/sheets/` (the device, key and passphrase sheets Devices and Settings share, one per file, with the YubiKey actions they run in `devices/credential-actions.ts`), `recovery-phrase-test-sheet.tsx` (Test recovery phrase: 17 fields and the agent's answer), `recovery-phrase-recall.tsx` (the three-token check before a new phrase is saved), `account-switcher.tsx` (the switcher Accounts, Devices and Settings share), `details-panel.tsx`, `item-secrets.ts` (the details panel's revealed value and edit draft, and every transition that clears them), `write-workflows.tsx` (the write overlay's sheets; the workflow type, dropped-file paths and conflict resumption are in `write-workflow-model.ts`), `item-copy.ts` (the copies each item offers, their labels, and the command and toast for each, and Open website), `account-section.tsx` (Settings › Account: the attention list and the account profile: teams, devices, keys and the account rows), `teams-screen.tsx`, `chat-tab.tsx` (the Chat tab: the inbox column, the open conversation and the info panel), `chat-teams.tsx` (the cross-team inbox column), `chat-new.tsx` (the Create channel sheet), `chat-screen.tsx` (one team's conversation), `chat-info.tsx` (the channel info panel), `files-keyboard.ts` (arrow-key movement in the Files list, card grid and folder tree), `item-moves.ts` (the multi-selection, the move, copy and drop checks, and the bulk runner), `folder-picker.tsx` (the folder tree of the move and copy sheets), `copy-item-sheet.tsx` (Copy to vault), `import-passwords-sheet.tsx` (Import passwords: the preview, the destination and each login's outcome), `export-passwords-sheet.tsx` (Export passwords and its typed confirmation), `store-choice.tsx` (the vault choice and role block the new-item and copy sheets share), `bulk-progress-sheet.tsx` (per-item outcomes of a bulk move or move to Trash). |
| `src/screens/first-run/`       | First-run setup behind `first-run-screen.tsx`. `use-first-run-session.ts` builds the setup session (`SetupSession` in `setup-context.ts`): the checkpoint, the server, account, team and CLI workflows, the drafts and `use-setup-secrets.ts`, and one `SetupBusy` owner (`setup-busy.ts`). Each workflow hook takes the `SetupCore` (`setup-core.ts`): the checkpoint controller, the bridge, the session message, the failure report and the account busy flag. `FirstRunSession` provides the session, and `steps.tsx` maps every checkpoint state to the step component that reads it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src/chat/`                    | The Chat tab's services and views below the screens in `src/screens/chat-*.tsx`: the chat client, the inbox, send and notification services and their providers, the composer and message rows.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `src/bridge.ts`                | The compatibility facade that re-exports `src/bridge/index.ts`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `src/bridge/`                  | The only seam to the local agent: the `Bridge` interface (`contract.ts`), the runtime decoders and the command error normalization, the `Transport` that decodes each response and records its timing, and the Tauri transport (`transport.ts`), the `commands-*.ts` factories that `createBridge` (`create.ts`) assembles, the Tauri bridge (`tauri.ts`), the per-profile request queue (`profile-work.ts`), the snapshot load and its projection, and `selection.ts`, which picks the Tauri or the mock bridge.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `src/mock/`                    | The mock bridge behind `VITE_FOKS_MOCK=1` and the tests; the application reaches it only through the dynamic import in `src/bridge/selection.ts`. `mock-bridge.ts` is the same interface, using the fixture, assembled from one module per `bridge/commands-*.ts` factory over the records in `state.ts` (the vault commands, in `vault.ts`, answer at the wire level through `host.ts` and `wire.ts`); `fixture.ts` is the stable desktop fixture, as typed data; `invitation-mock.ts` holds local-only invitation fixtures; the mock chat is `chat-mock.ts`, with its seeds in `chat-mock-scenarios.ts`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src/model/`                   | The pure model — roles, kinds, readers, format, lease, and `login.ts`, the login layout parser that matches the host's; `scope.ts` (what is listed, in what order), `group-model.ts` (the permission rules and reasons the Teams list and the group page share, and the roster order) and `team-members.ts` (the federated teams a group lists). TypeScript only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `src/data/`                    | What the screens read and run against the agent, without views: the catalog (`catalog-coordinator.ts`, `catalog-state.ts`, `catalog-read-gate.ts`), the metadata repository and its hooks (`metadata-repository.ts`, `query-hooks.ts`, `query-read-recovery.ts`; see [Device metadata cache](#device-metadata-cache)), the device and operation resources (`device-cache.ts`, `device-metadata.ts`, `operation-queries.ts`), background reconciliation (`desktop-reconciliation.ts`, `use-desktop-reconciliation.ts`, `refresh-activity.ts`, `roster-staleness.ts`), the read and mutation policy and outcomes (`command-policy.ts`, `operation-outcome.ts`, `mutation-recovery.ts`), `device-model.ts` (the four per-account key calls and the one row model Accounts and Devices share), `device-alert.ts` (each account's recovery-phrase and key-count facts for the session) and `access-lifetime.ts` (the session and per-profile generations whose tickets abort reads and writes when access ends).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `src/location.ts`              | `Location`, `Selection`, `transition`, the `?state=` codec, the store.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `src/navigation/`              | What `src/location.ts` puts together: the routes, the `?state=` and scene codecs, the location store and its React hook, and the transition rules.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `src/first-run-state.ts`       | Pure versioned resumable setup state and its explicit nonsecret checkpoint codec.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `src/platform/`                | `device-storage.ts`: every webview `localStorage` key, registered as device- or profile-scoped, and the only module that reads or writes `localStorage`. `purgeProfiles` removes a profile's entries from the profile-scoped records (setup progress, invitation labels, Recent and Favorites, exposure reviews) after Reset this device or Remove server and credentials. `appearance.ts` applies the stored Light, Dark or System choice and follows system changes.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `src/scheduling/`              | Work ordering against the agent: `profile-work.ts` (one serialized queue per bridge and profile), `reconciliation.ts` (the background jobs and their reports) and `lease-expiry.ts` (lease expiry observations, which a reread or clock rollback cannot reopen).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `src/diagnostics/`             | The bounded in-memory timing log, renderer fault records and the copied diagnostics text. Nothing here persists.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `src/resources/`               | Metadata resources the shell reads through the metadata repository: `application.ts` (the version, the agent socket and the managed profile).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `src/sidebar-prefs.ts`         | The rail's collapse preference and the Chat column's display preferences: the `sideCollapsed`, `chatChannelSort`, `chatPreviews` and `chatFoldedTeams` `localStorage` keys.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `src/recovery-phrase.ts`       | The HESP token grammar for typing a recovery phrase back in: per-field checks, BIP-39 completion (`src/bip39-english.ts`) and the recall positions.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/recovery-phrase-tests.ts` | When each recovery phrase last matched a typed copy on this device, and its snooze: times only, under the `foks.recoveryPhraseTests` `localStorage` key.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/password-generator.ts`    | Password and passphrase generation for the login editors: `crypto.getRandomValues` with rejection sampling, exact entropy for generated values, common-weakness warnings for typed ones, and the last settings under the `passwordGenerator` `localStorage` key.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `src/recent-favorites.ts`      | The ⌘K palette's Recent (the last 10 items opened from it, copied anywhere, or whose website it opened) and Favorites (up to 100), as store refs and dirent ids, never item names or paths, under the `foks.recentFavorites` `localStorage` key; follows a move made in the app to the item's new dirent id.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `src/exposure-review.ts`       | What a removed or lowered member, a removed federated team or a removed device could read, from this account's catalog (a lower bound), and the reviews kept after the change: store refs, dirent ids, versions and marks, plus the party's id and username or device name, never item names or paths, under the `foks.exposureReviews` `localStorage` key. `src/screens/exposure-review.tsx` draws the list in the Remove, Lower role, Remove federated team and Remove device sheets, the band on the team page and Devices, and the review sheet.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src/attention.ts`             | The attention bell's model: one item per entity behind the rail's counts and dots, with the page that resolves it, and the "Get set up" steps, each left out until its fact has been read. See [The topbar](#the-topbar).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `src/shortcuts.ts`             | The keyboard shortcut registry: each command's keys, label and scope, matched by the shortcut listeners and drawn by their hints. See [Keyboard shortcuts](#keyboard-shortcuts).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `src/icons.ts`                 | The mock's 51 icons as structured data.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `src/components/icon.tsx`      | `<Icon name size />`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `src/styles/shell.css`         | `wave6/shell.css` copied in full, minus the shared tokens.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src/styles/app.css`           | Rules for the shared components and the screens without a sheet of their own: the `components` layer.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `tests/`                       | `node:test` via `tsx`: goldens, source invariants, render tests.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |

`layers.json` lists, for each folder, the folders it may not import from.
The kit imports nothing from `src/`. The model imports no data, navigation,
platform, mock or view module. Data, scheduling, diagnostics, navigation,
platform and resources import no view, chat, shell, application or mock
module. The bridge follows the same rule, except that it may import the chat
host contract, `src/chat/local-contract.ts`. The mock imports no view, shell
or application module.
Components import no screen, chat, shell or application module. The shell
and the features (`src/screens/`, `src/chat/`) import nothing from
`src/app/`. No layer outside `src/mock/` imports it, except for the dynamic
import in `src/bridge/selection.ts`.
`eslint.config.mjs` reports a static import specifier that breaks the table,
and `tests/module-boundaries.test.ts` checks the table against resolved
paths, dynamic imports included, with that one edge listed by name.
The same test checks that no folder has a runtime import cycle, apart from
the chat inbox provider's two that it lists, and that no module imports its
own folder's barrel or compatibility facade. The modules still at the `src/` root
belong to no layer yet.

### The rail

`nav.side.rail` is a six-tab rail, 150px wide by default (resizable up to
240px) and blue (`--rail`, `--rail-ink`, `--rail-line`, `--rail-active` in
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
flow) and "Lock" (the command Settings › Device also offers). The account the
window is acting as carries a check at the right end of its row; an account
whose access has stopped is dimmed and carries an amber line under its caption
with the one link that would restore it — "Check in" for a lapsed or missing
check-in, "Server settings" otherwise. An orange dot on the avatar marks the
notices only Settings › Account lists; it is a button, and it opens that
section. The topbar's attention bell lists every item behind the rail's counts
and dots (see [The topbar](#the-topbar)).

The foot holds the agent light and nothing else: a 7px dot and a label in four
states — green "Agent ready", amber pulsing "Starting agent…", red "Agent
stopped" and gray "Locked". It never names an internal step.

The rail collapses to a 56px icon-only track. Collapsing is CSS alone —
`nav.side` gains `is-narrow`, and every row stays in the document — so the
collapsed rail keeps its glyphs, the unread count on the Chat glyph's corner
and the attention dot on the avatar's, with the labels becoming tooltips. The
width is the one the reader chose: a collapsed rail does not expand on hover or
on focus. The toggle is in the topbar, not in the rail, and writes
`sideCollapsed` to `localStorage` through `src/sidebar-prefs.ts`. Item
selection and navigation leave that chosen width unchanged. The open rail,
the Files folder tree and the Chat column share one separator hook,
`usePaneResize` in `src/components/use-pane-resize.tsx`, whose
`PANE_SIZES` table gives each pane's storage key, CSS variable and bounds. A
drag commits on release; Escape, pointer cancellation and lost pointer
capture restore the starting width. A narrow window bounds the displayed
width and `aria-valuemax` without rewriting the stored width, and a
double-click clears it. The rail's separator is rendered into `.app` and
sets `--side-w-open` on `.window`. First run
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
The refresh is one square button: it refreshes on click, carries a spinner
badge while any server is refreshing and an orange dot when one could not be
refreshed, and pointing at it opens the per-server Refresh status popover
(`SyncPopover`), which stays up while the pointer is on the button or on the
popover itself. Its empty parts carry `data-tauri-drag-region`, because the
rail's strip alone does not reach across the window. First run draws no
topbar.

Left of the refresh is the attention bell (`shell/attention-centre.tsx`), with
a count of the items that need the reader. Its popover has three sections,
built by `src/attention.ts` from facts the shell already holds; the model
itself reads nothing from the agent. "Needs attention" restates the rail's
signals one entity at a time, warnings first: an unverified server or a lapsed
check-in, an account with no recovery phrase, an open pairing offer, a
recovery phrase due for a test, a team's join requests and the notices only
Settings › Account lists. It adds each team's interrupted membership changes,
which block later membership changes on that team; the shell reads them from
the same cached journal row a team's page reads, once per server profile that
holds a team (`list_pending_operations`). Each row opens the page that resolves
it. "Get set up" lists the steps still open after first run: a recovery phrase
and a second device or hardware key for each account (from the device lists
the shell preloads; a hardware-card key counts as a device), whether the app
lock is available on this computer (`app_lock_state`), a first password and a
team. A step whose fact has not been read this session is left out; a
per-account step is open as soon as one account lacks it, and done only once
every account has been read. The recovery-phrase step opens Create recovery
phrase on Devices (`open: 'phrase'`), the second-key step the Add a device
chooser. An app lock this computer cannot provide is listed with its reason
and is not counted, since nothing in the app can complete it. The checklist
hides once every counted step is done, or when the reader hides it; Settings
› Preferences › Onboarding tips shows it again (the
`onboarding.dismissed.setup-checklist` `localStorage` key). "Recent" lists the
last 20 toast messages, newest first, from the shell's `ToastController`:
message text and time only, in memory, cleared when the app locks.

Back and Forward traverse committed app history across tabs and folders. The
⌘[/⌘], Alt+Left/Right, browser navigation keys, mouse buttons 3/4, and trackpad
gestures use the same guarded history operations.
Back never means parent or up; breadcrumbs and the folder tree navigate to
those destinations explicitly. A new visit clears Forward; replacements and
view edits preserve it. History restores page state but never transient sheets,
and session reset clears both stacks.

The webview's native history gestures remain off: the URL uses
`history.replaceState` for reload restoration, while app history lives in memory.
Horizontal wheel gestures show an arrow preview in either direction. A 120px
swipe arms navigation, reversing below that threshold cancels it, and navigation
commits only after 120ms without wheel events. Wheel events do not expose a
portable finger-release or momentum phase, so the idle interval approximates
completion and includes momentum. There is no additional cooldown. Modified,
vertical, consumed, and scrollable-pane gestures remain scrolling for the whole
stream; overlays and blocking states disable gestures. Completed swipes use the
same confirmation dialogs and refusal toasts as buttons and shortcuts.

### The search palette

⌘K (Ctrl K elsewhere) opens `shell/search-palette.tsx` over any page. It
searches item and folder names and paths, vaults, teams and shares, roster
usernames, chat channels and the shell's commands; it does not search item
values, and its footer says so. `in:<vault>` keeps the results that are in a
store whose name or alias starts with the value, and `kind:password`,
`kind:document` or `kind:folder` keeps one kind of item; both are parsed by
the pure `parseSearchQuery`. Enter opens the highlighted row: an item is
selected on its store's page, a folder opens in the tree, and an action runs.
On an item, ⌘C copies a Password's password or a text Document's value and
⇧⌘C a Password's user name through `copy_item_field` and `copy_item_value`,
without opening the item; with query text selected, ⌘C copies that text. A
file has no copy. ⌘O on a Password opens its website in the default browser
through `open_item_url`, which reads the `url` line in the host and returns
nothing; a login without a website, or whose website is not an http or https
address, gets a warning toast. ⌘D, or the star in the details header, adds an
item or a folder to Favorites. An empty query lists Recent, every favorite and
Actions (New password and Add a document on a writable Files page, Lock,
Keyboard shortcuts, Settings, Add a device). Add a device navigates to
Devices with `open: 'add'`; the page replaces that address with the plain
list and then opens its Add a device chooser, or shows why the account cannot
add one, as the Teams page does with its sheet intents. A return to the tab
resumes the list, not the chooser.
Favorites holds up to 100 items; adding one past that drops the oldest.
`src/recent-favorites.ts` stores both lists as a `StoreRef` and a dirent id
per item. The `StoreRef` names the server profile, the account and, for a
team, the team's alias and id; no item name or path is written to disk. Rows
are resolved against the live snapshot when drawn, and an id that no longer
resolves is not shown. A dirent id belongs to a path: a move or a rename gives
the item a new id at its destination. The move sheets record each move, and
the shell gives the entry the new id once the refreshed snapshot holds the
item. A move made on another device is not followed, so the entry is no
longer shown, and an item later created at the old path, which takes over
the old id, is shown in its place.

### Keyboard shortcuts

`src/shortcuts.ts` lists every keyboard shortcut once: the command's label,
where it applies (anywhere, the Files list, the Trash list, the folder tree,
the Chat tab and its channel list, messages, composer, @name suggestions and
find bar, the search palette or Settings › Servers) and its key combinations.
The listeners in the shell, the Files screen, the Trash list, the palette,
the Servers page, the Chat column's channel steps and the message rows match
key presses with `matchesShortcut`, and the hints
(the header's search field, the empty vault's cards, the palette's Actions,
row keys and footer) draw keys with `shortcutLabel` or `chordLabel`, which
write `⇧⌘N` on macOS and `Ctrl Shift N` elsewhere. A `mod` chord takes ⌘ or
Ctrl on every platform; Ctrl+Tab, ⌘[ and Alt+Left name one physical key, and
⌘[ is listed on macOS only. Every other modifier must be up, so Shift+Delete
on a row does nothing. The entries for the arrow keys in the Files list, the
Trash list and the folder tree describe the keys
`src/screens/files-keyboard.ts` maps, and `tests/shortcuts.test.ts` checks
that each one moves focus. The chat entries list the keys that
[The Chat tab](#the-chat-tab) describes, without changing them: the
composer's formatting keys and ⌘F take ⌘ on macOS and Ctrl elsewhere, not
either, so each is listed as a macOS chord and a chord for other platforms,
and their handlers keep their own platform test. No key press matches two shortcuts of one scope, and a
chat key matches no shortcut that applies anywhere except Escape, which the
chat handlers take first (`tests/shortcuts.test.ts`). The keys inside a
control (tabs, menus, pickers, the rail's resize handle, the recovery phrase
fields) follow that control's ARIA pattern and are not in the registry.

⌘/ (Ctrl / elsewhere), `?` outside a text field, or Keyboard shortcuts in
the palette's Actions opens `shell/shortcut-sheet.tsx`, which lists the
registry by scope with this platform's keys. ⌘L locks the app from any page,
through the same path as the account menu's Lock. It also locks while a
sheet is open, which the account menu cannot reach, and the lock discards
the sheet's unsaved contents without the sheet's own discard prompt.

The macOS application menu carries Settings… (⌘,), Lock (⌘L), New Password
(⌘N) and Add Document (⇧⌘N), and `tests/shortcuts.test.ts` checks the
accelerators in `src-tauri/src/lib.rs` against the registry. Its Lock arms
the lock in the host and then emits `foks://lock` to the main window, if one
is open. `useAppBootstrap` listens for the event whatever the window shows
(the shell, the loading screen of a boot, or a boot error), reads the lock
state back and shows the lock screen; with no main window open, the next
window starts on it. Where the platform cannot authenticate, the menu item
is disabled and ⌘L reaches the window, whose shell says that the app cannot
lock.

### Blocking states

Three takeovers leave as much of the shell readable as they safely can, and the
rail's light says which one is in force.

- **Starting.** The rail and topbar are displayed before a snapshot exists,
  dimmed and inert. The content area displays a spinner, "Starting the FOKS
  agent…", and "Startup usually takes a few seconds." After 5 seconds without
  catalog progress, the detail line names the Keychain or Secret Service prompt
  an initialized agent can be waiting on. Internal step names remain hidden, and
  maintenance uses the same presentation as the first connection.
- **Stopped** (`restart-required`, `recovery-required`, `restoration-failed`).
  The page remains visible behind a light veil below the topbar (`.stopveil`).
  A centered `.card.stopcard` displays the outcome, details, any copyable
  `foks-rs` recovery commands, and the available actions. `restart-required`
  adds: "Your vaults remain on this Mac and on their configured servers." The
  same card appears at startup over the empty frame.
- **Locked.** A full-window takeover over the rail as well (`.lock-back`,
  blurred), because the rail names accounts and counts unread messages: a round
  accent glyph, "FOKS is locked", the one sentence naming the authentication
  the platform offers, and a full-width Unlock. The boot error card shares the
  same box.

### Render faults

A render error stays inside the nearest error boundary
(`src/app/fault-boundaries.tsx`, `src/app/screen-error-boundary.tsx`):

- **Screen.** The routed screen shows "This page could not be shown" with
  Reload page; the rail and topbar remain.
- **Panels.** The details panel, Team info, the search palette and the write
  workflow sheets each have their own boundary. A failed panel unmounts, which
  discards any value it had revealed, the shell closes it, and a warning toast
  states that it was closed.
- **Window.** A failure anywhere else (the rail, the topbar, first-run setup,
  the boot screens) replaces the window with a full-window card, "This window
  could not be shown", with Reload window, Copy diagnostics and Quit FOKS, and
  focus moves to Reload window. The card uses no shell context, and it renders
  inside the exit guard, so Quit FOKS and the window's own close still go
  through the close guard and its Ask before quitting preference. A failure in the app's boot hooks or in the exit guard
  itself (label `app`) shows the same card without Quit FOKS, and its Copy
  diagnostics holds the renderer's events only.

Each fault is one `renderer.fault` event in the timing log, under the static
label of the boundary that caught it (`screen`, `details`, `team-info`,
`search`, `write-workflow`, `shell`, `app`). `createRoot`'s error callbacks
record what no labelled boundary caught (`uncaught`, `unlabelled`,
`recoverable`), and window listeners record script errors (`window-error`) and
unhandled promise rejections (`unhandled-rejection`); an `error` event without a
thrown value, such as the ResizeObserver loop notice, is not recorded. The
event's `code` is the constructor name of a built-in error type, such as
`TypeError`, and is absent otherwise: minified builds rename components and
custom error classes, and the log never holds error messages. At most 32 faults
are recorded per label in one window load. Copy diagnostics lists them in a
`renderer faults` table by label and code. Unlike the timeline and the other
tables, which cover the last 15 minutes, the table includes every fault the
timing log still holds, so a copy made long after the fault still reports it.
The window card's Copy diagnostics copies the timing log without the status
lines that need the shell.

### What each tab is

Accounts is the list of what needs attention — the page that used to be called
Alerts — over one account. Each attention card keeps its severity — as a
color and as an accessible name — its title and its detail, and displays the
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
account is a profile band — a colored band with the account's mark half over
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
account's — its devices and the recovery-phrase enrollments this Mac holds — while
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
Settings › Preferences, followed by Bot accounts, Web admin panel, Sign in
via SSO, and Import from FOKS CLI, each opening its corresponding panel. Sign
in via SSO is a two-step sheet: the account and whether its keys are on a
security key, then the browser sign-in, with one primary action per step
(open the browser, check the sign-in, finish). Check linkage reads the
account's linkage and, where the account is not yet linked, the primary
becomes Link existing account. First run's Sign up with SSO
shares the flow and the browser step; it opens on where to keep the keys
(this computer or a security key) and ends with Finish sign-up. If
account access is suspended, username changes are disabled with an explanatory
reason while the remaining recovery actions stay available. When the agent
reports that the account's adapter clock refuses new MCP writes (its anchor
disagrees with the computer's clock beyond the elapsed time it measured), the
page draws "Assistant writes paused: confirm this computer's time." with the
computer's current time and a Confirm time action; the native side re-anchors
the clock at the time it reads itself, so the renderer never supplies one.
When the clock is trusted but behind a watermark left by an earlier clock that
ran ahead, the band instead names the time writes resume and offers no action,
because confirming the time cannot bring it forward. An account's mark is the
initial of the username, over a color derived from it, on the account band
and in the switcher: those surfaces name the account by the username the
server knows it by, while a store elsewhere keeps its `GroupMark`. There is no
people search because the bridge has no directory lookup. New people can be
added only by invitation, which is managed in Teams. Chat is one inbox column, listing
every team with chat, beside the open conversation; the tab with no
conversation chosen opens the most recent one, and says how chat gets turned on
when no team has any. The Files tab defaults to All items and displays items
across all stores in a table with sortable Name, Kind, Location, and Size
columns. A permanent tree lists All items followed by vaults, teams, and their
folders, including item counts and team initials badges. Selecting a store or
folder filters the table, omits Location for single-store views, and updates the
toolbar search placeholder to "Search this vault", "Search this team", or
"Search this folder". That field is the width of the topbar's own "Search
everything" trigger rather than the width of the list column, and the kind
filter beside it sizes each of All / Passwords / Documents to its own label
rather than to an equal share of the tree column. The topbar breadcrumb displays the active folder, and Back
navigates up the hierarchy to All items. Teams lists the groups and
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
"2 Macs · 1 key on a card · 1 recovery phrase · 1 enrollment", and Loading… until
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
than removed here), Recovery phrases (each enrollment's name and shortened id,
Revoke…, Add a recovery phrase…, and the row that recovers an account from one) and
Security key enrollments (each enrollment with its Enrolled or Incomplete
chip, its own
Revoke… — off, with the reason, while the enrollment is unfinished — and the
note that the agent reports no serial for one, the serial of the card connected
now with Provision… and PIN status — which asks the card in the port, so a
connected card and an enrollment to name are all it needs — and the
card PIN's link to Settings › Servers). That last section is the one part of
the page that is not about the account above it: enrollments are listed for
the server, and an enrollment's own page is captioned with the server rather
than the account. The mock
draws one flat list of keys;
the page keeps the three sections deliberately, because each kind has actions
a flat list has nowhere to put: Add a recovery phrase…, the recovery menu, and the
connected card and Card PIN rows all belong to one section and to no row.
A phrase you write down is called a
recovery phrase
on these tabs, while the enrollment keeps the name the agent stores for it. The
header's Add a device or recovery phrase opens a chooser — four cards, each with the
glyph of what it adds: pair another Mac, enter a
pairing phrase, recovery phrase, security key — that leads into the pairing sheet in
the direction it names, and into the recovery-phrase and provisioning
sheets. The pairing sheet is the two numbered steps of one exchange, written
for the direction the chooser picked, with Back to the chooser instead of a
control that silently changes what Start would do: getting a phrase is Start
and Resume offer in step 1 and the phrase with Copy, and step 2 is quiet until
there is one; entering a phrase is the other Mac's step 1 and this Mac's
fields in step 2. A phrase that came from Resume says the agent is still
holding that offer, and nothing counts down, because `PairingOffer` carries
only an alias and a phrase. The recovery-phrase sheet reveals the 17 words and
numbers the account's phrase is, one grid, not copyable, then hides it behind
Next and asks for three of its tokens, at random positions with at least one
word and one number, typed from the written copy; Save recovery phrase stays
off until all three match, and Back shows the phrase again. First run's
recovery step asks the same way in place of its "I have written this down"
checkbox. Three tokens do not test the whole copy, so the check is not
recorded as a test: the new phrase reads "Last tested: never" until a full
test matches.
Cancel beside it discards the revealed phrase:
`prepare_owner_backup` generates it and nothing else — it writes no local
state and does not contact the server — so there is nothing to clean up, and
the phrase is gone rather than committed. Esc and a click on the backdrop
leave the same way and discard the same phrase; a revealed phrase is not a
reason to hold the reader in the sheet. Every row on the page also opens that key's own
page — `?state=devices&store=<StoreRef>&device=<key>` — which carries the
kind, the role, the account, the full key id in mono with Copy, and the one
destructive action that kind has: Remove this device… for another Mac, Revoke…
for a recovery phrase or an enrollment, both with the typed confirmation the row
used; this Mac's page says it cannot remove itself and points at Settings, and
a key on a card says it is revoked under its enrollment and carries the action
that goes there — its kind decides that
before its currency does, so the card this Mac is authenticated with now is
not told it cannot remove itself. An enrollment's page is captioned with the
server, not the account, because that is what the agent answered for. The `device=` value is
the key id for a Mac or a recovery phrase and `yubi:<alias>` for an enrollment,
which the agent names by alias and reports no id for. A `device=` that no
longer resolves says the key is not on this account rather than showing an
empty page. The nine card operations that are recovery paths for a key already in
trouble, and creating an account on a hardware key, sit in the enrollment
section's menu, each saying there when it does not apply, and each menu entry
and the sheet it opens read the same label. A stopped account
lists nothing and disables every action with the reason. Each recovery phrase
row, and its own page, carries when a written copy last matched it on this
device ("Last tested: never" until one has) and Test…, which opens a sheet of
17 fields that alternate word and number, complete words from the BIP-39 list,
and name each field's problem as it is typed. Test phrase sends the phrase to
`verify_recovery_phrase`, which the agent answers from this device's verified
copy of the account without contacting the server: a match (recorded on this
device and the fields cleared), a match with a revoked phrase, a near miss that
shares the phrase's first word and number and so differs in positions 3 to 17,
or no match. A phrase never tested here, or last tested more than 180 days ago,
turns on the Devices rail dot, whose label lists each reason, and draws a band
with Test now and Remind me later, which hides the band and that reason for 30
days.
Each section is an ARIA region focused via the `section` query parameter. When
a dialog opens immediately upon loading a scene, keyboard focus is constrained
within the dialog, and the target section receives focus only after the dialog
closes. A scene is entered once: concealing a phrase remounts the tab without
reopening the dialog that the scene opened. Navigating to an account ID that is
no longer present locally displays an account-unavailable screen listing valid
accounts, rather than reporting that no accounts are configured. Settings is a
sub-navigation of three pages. Account ends with one Servers on this device
group, with Add a server… beside its heading. Servers requiring attention stay
at the top of that group; each row has its state chip, a Check on the
never-checked and lapsed rows, and Open for the server's own page, which holds
that server's security keys and its own reset. Passphrase… sits beside Change…
on the Username row (the sheet defaults to Change and switches to Set or Verify
in place; an account whose access has stopped explains why the button is
disabled). Preferences contains Desktop alerts — this device's desktop-alert,
message-preview and app-icon-count preferences, which the desktop persists
locally; a channel's alert mode and mute stay in Chat's info panel, with a link
here, and a team's alert default in its menu in the Chat column; none of these
preferences sync between devices. Its Quitting section sets Ask before quitting:
Always (the default) or When work is pending, with which a window close or
quit stops the agent without asking unless messages are unsent, a change or
data maintenance is in progress, the agent is not one this app launched or
adopted (or cannot be identified), or another agent process uses this app's
state directory (a failed process scan counts as one). When a change or
maintenance is in progress, the question says that shutdown starts none of the
change's remaining agent requests and waits up to 5 seconds for the current
work, after which FOKS stays open and offers Try again or Force stop. The
renderer reports the choice to the native close guard
(`set_quit_confirmation`), which asks every time until it has one. Device: Application (version, Lock now),
Agent (status with Retry connection while it is not ready, socket with Copy),
FOKS data (export, import, verify online, move the data folder) and a danger
zone whose Reset this device runs the per-server reset once per profile, each
with its own one-use preview token and typed profile name; the sheet states a token
lifetime only once every server has reported one, and names the servers, not
the profile ids, when they differ. After each server's reset succeeds, the
webview's records of that profile are removed as well (`purgeProfiles` in
`src/platform/device-storage.ts`): its setup progress, invitation labels,
Recent and Favorites entries and exposure reviews. Older `section=` addresses — `credentials`,
`notifications`, `device`, `about`, `agent`, `security-keys` — resolve to the
page that holds their content.
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
danger zone has one Remove server and credentials action. It deletes the local
profile, credentials, trust history, cache, and unfinished operations without
changing data on the server, and then removes the webview's records of the
profile, as Reset this device does.
A store's abnormal state
is a chip at the end of its row on each of the four lists that draw one —
Files, Teams, Accounts’ Teams you're in, and a server's page —
rather than a caption under its name. One model function, `storeAttentionState`,
decides it for all four, so a group whose roster could not be read draws the
chip rather than printing "Roster unavailable" where the roster summary goes;
`storeDescriptionState`, which knows only whether the store can be reached, is
what dims the row. A group carries one mark everywhere it
is listed — the same initial over the same color on Files, on Teams, in the
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
under one "Teams" label, in the shell's navigation order, ensuring consistent
team ordering across Files, Teams, and Chat for spatial predictability. Every
team is a heading — its mark, its name, its server and its member count — over
its channels, and the whole heading is the control that folds them: it carries
`aria-expanded` and `aria-controls`, and the channels are a `group` labeled by
the team. Folded, the heading carries the total unread count of the channels it
hides, leaving out hidden and muted ones as the rail's count does; a channel
muted on this device, alone or with its team, counts as muted. A search unfolds
every team while it runs and leaves the folds as they were. A heading click on
a team with a single channel also opens that channel; the click still folds or
unfolds the team, and that fold is stored like any other.

Each team's channels keep one order, built by `listChannels`: the general
channel first, then A to Z, with hidden conversations last, dimmed and marked
"Hidden". The order never follows the agent's, which is by inbox version: the
server issues a new version when the reader's own read pointer rises, so
reading a channel would move it above channels that still have unread messages.
The control beside the "Teams" label names the current order ("A–Z" or
"Recent") and opens Sort channels: Alphabetical, or Recent activity, which puts
the channel whose last message arrived most recently first (by the server's
insertion time of the message the inbox previews), the channels with no message
after those A to Z, and hidden conversations last. Reading moves nothing in
either order; a new message, or the reader's own send, moves its channel up
under Recent activity. The same menu carries Show previews. With it on, the
default, a channel row has two lines: the name, a lock for an admins-only
channel and the last message's time (`previewTime`: the clock time today, the
weekday within the past week, the date before that), then the last message
(`previewLine`: "You: …", "sam.ortiz: …", "Team member: …" for a message with
no sender, "Unverifiable message" with no claimed sender) and the unread count.
A channel the inbox states nothing has been said in reads "No messages yet"; a
missing preview alone does not, since a stopped channel's preview is withheld
and a synchronization can leave one unavailable. With it off, rows are one
line: the name, the lock and the count, and only the lone general channel of a
team nothing has been said in reads "No messages yet", beside its name. A
channel with unread messages is drawn with a bolder name and a stronger
preview. The caption beside a name says why a channel is quiet ("Restricted",
"Hidden", "Muted", "Channel stopped") through the same `channelMeta` the group
page's channel list uses. A channel other than the open one that holds an
unsent draft carries a pencil and "Draft" after its name, and its row's label
ends in "· Draft". The column reads which channels hold a draft from the send
service's `draftedChannels`, which changes only when a draft appears or goes,
so typing does not re-render the column. The fold of each team, the order and
the previews switch are display preferences that `src/sidebar-prefs.ts` stores
in guarded `localStorage`, so the column returns as it was left; the fold set
keeps a short hash of each team's store reference rather than the reference,
which names the local profile and aliases, and keeps at most 256 entries. The open
channel's row carries `aria-current="page"`, from the first render that
resolves it: the column and the pane resolve the open channel through the same
`openChannel` helper, so neither marks a row the other did not mount.

A channel's context menu offers Open channel; Mark as read, while the channel's
row draws an unread count (a channel of an unreachable team draws none), which
issues `mark-read` through the send service's request path at the newest
position the inbox states (`lastPosition`) and applies the confirmed pointer
the way the open conversation does; Mute channel or Unmute channel, which
cannot be changed while the team is muted; and Channel info. Renaming and
deleting channels are not offered until channel management ships. Copying a
link to a channel is not offered either: the app registers no URL scheme, and
an in-app location names local store references that mean nothing on another
installation. A team's menu offers Go to team, the team's alert default on this
device as four radio items (Device setting, All new messages, Mentions only,
None), Mute all channels as a checkbox item, and the add-people actions; the
column's background offers New channel. The menu opens at the pointer, or at a
focused channel row or team heading with the ContextMenu key or Shift+F10;
dismissing it, or marking a channel read, returns focus to that row.

A mute is kept on this device, in the native local settings beside the alert
modes (`chat_local.rs`): the pinned server always reports a conversation's
`muted` and `hidden` as false and the protocol has no request that sets them.
The settings hold a set of muted channel and team keys and a team default per
team key, each key a pair of SHA-256 digests (`notificationKey` and `teamKey`
in `local-contract.ts`), so the file names no alias, channel or team; each
holds at most 4,096 entries, as the per-channel alert modes do. A team's key
in the muted set mutes each of its channels, including those created later.
`ChatInboxService` publishes `muted` as the server's flag or this device's
mute, so the row's dimmed look and "Muted" caption, the team and rail counts,
the Alt+Shift steps and the screen-reader announcements follow a mute without
changes of their own; its own state keeps the server's value. A channel's
alert mode is its own when it has one (All new messages, Mentions only or
None), else None while it or its team is muted, else its team's default, else
every message; the native side refuses a desktop alert by the same order, from
the mutes it stores. Each change to these settings, a mute included, clears the
desktop alerts already delivered and restarts the notification consumer, which
takes a fresh baseline for every watched channel. A settings file that holds a
mute, a team default, Mentions only or an app icon count turned off cannot be
read by a desktop build from before these settings existed: that build refuses
the whole file, and its local alert settings fail until the file is removed.

Desktop alerts are shown by Notification Center on macOS (packaged
application only) and by the freedesktop.org notification service
(`org.freedesktop.Notifications` on the session bus) on Linux, which GNOME
Shell, KDE Plasma, dunst and other notification daemons implement
(`chat_local/platform/`). Linux probes the service when a local alert session
begins and when alerts are enabled; Preferences names the service, or says
that none was found and offers Check again, which begins a new session (and so
would close the alerts already shown, which is why it is not offered while a
service is found). The alerts of one channel are grouped: macOS stacks them
under one thread identifier, the channel's settings key, and Linux replaces the
channel's entry in place until the service reports it closed. The entry's text
counts every message it stands for, after the newest message preview while
previews are on: the renderer sends the alerts for messages read in one check of
the channel oldest first, so the newest message's preview is the last one the
entry receives. Once the reader has read every message the channel's earlier
alerts stood for (the inbox's read position, a pending local read included), the
next alert restarts the count in the same entry. Because only an entry's newest
alert can be activated, a channel keeps one route on Linux, and the bound of 64
routes per session, past which every alert shown is closed, counts channels
there and alerts on macOS. A click on an alert, or its Open action on Linux,
opens the channel. Without previews an alert is titled "FOKS" and its text is a
count ("New chat message."). With previews, the title names the channel and team
("#general · Engineering", at most 128 characters on one line) and the text
names the sender from the team roster, or by a short id, followed by the
start of the message, within the 256-character bound the native side admits;
the renderer supplies both, and the native side refuses a title it cannot show
on one line. A service that reads body markup receives the text escaped. The
app icon shows the number in the rail's Chat total, so muted and hidden
channels are left out (`UnreadBadge` in `unread-badge.tsx`, through
`set_unread_badge`); the native side refuses the count while the app is
locked, removes it when the app locks and while "Show unread count on app
icon" is off. On Linux the count is set on the Unity launcher entry, so it is
drawn only when libunity is installed (`libunity9` on Debian and Ubuntu; the
package does not depend on it) and a dock that reads the entry, such as Ubuntu
Dock, is running; otherwise the setting has no visible effect.

Mentions only alerts for a message whose first 256 characters name the reader
as `@username` where the message renderer would draw a mention: the match runs
over the renderer's own inline tokens (`inlineTokens` in `mentions.ts`), so an
`@username` inside a code span, a Markdown link, a bare URL or a fenced code
block does not count, and names are compared as `mentionKey` compares them. It
is a text match on this device, not a protocol mention, which the pinned
protocol does not have; a sender can write any name.

Alt+Up and Alt+Down (Option on macOS) open the previous and next channel row
the column draws, stopping at either end; channels of a folded team are not
drawn and are skipped, and a search limits the steps to the rows it leaves.
From an open channel that is not drawn, the step starts at its place in the
column. Alt+Shift+Up and Alt+Shift+Down open the previous and next channel
with an unread count that the team's badge counts, so muted and hidden
channels are skipped; that search includes the channels of folded teams and
unfolds the team it opens a channel in, wraps around the column, and a toast
says so when no other channel has unread messages. Alt+Left and Alt+Right stay
back and forward. In the composer the keys move the caret unless it is on the
draft's first line (Up) or last line (Down), since macOS moves the caret to a
paragraph boundary with Option+Up/Down. The keys do nothing while a dialog is
open, as the rail's Ctrl+Tab does.

Messages from others are announced to screen readers while the window has
focus, through a visually hidden `role="status"` element in the Chat tab:
"New message from priya.n in #general", or "3 new messages in #general,
latest from priya.n", with the team's name added when more than one team has
chat. The announcement names the sender and channel only, never the text,
whatever the notification previews setting, and names no sender when the
newest message is not authenticated Basic text. It is read from the inbox
projections the column draws, at most once every five seconds, with arrivals
in between merged into the next announcement; muted, hidden, unreadable and
stopped channels, and the reader's own messages, are not announced. Arrivals
while the window is not focused are left to desktop alerts.

Every row is built from `useSidebarInbox()` — the `TeamInbox` projections
`ChatInboxService` already keeps and polls for each eligible team, the same
ones the rail's badge sums — so previews, channel lists and counts arrive for
every team without a conversation being mounted for any of them. Only a plain
count is interpolated into a row; the states `teamUnread` also reports ("…",
"!", "3+") stay in the badge and its description. A team this Mac cannot reach
right now states its own reason ("Check-in expired") in place of the preview
and carries "!" instead of a count; its rows are dimmed but remain clickable,
opening the pane that displays the lock reason so the user can see why the team
is unavailable. A team whose inbox failed — nothing arrived, or it is
unavailable or blocked — carries that error, except for the open team, whose
pane already states the error and carries the retry, so its row says only
"Channels unavailable". A synchronization that succeeded but could not finish
everything ("Read status will retry.", "Some previews are unavailable.") is not
a failure: the channel rows keep their previews and times, and the team's
heading says what is incomplete in a caption. A heading draws a badge only for
what its channels cannot already say — a state rather than a count, so "…",
"!", "3+" and "5·" appear there and a bare number does not. A hidden
conversation keeps its channel listed, marked "Hidden", dimmed and placed last,
rather than being dropped, which used to make a team whose only conversation
was hidden claim it had no channels. Named teams whose server offers no chat
sit dimmed at the foot under "Chat unavailable" with the reason; shares are not
listed. Unread counts are the rail's orange (`--unread`) wherever they appear.

The window header's search field has two scopes in the Chat tab. While a
readable channel is open, a visible badge names it ("#deploys") and the field's
accessible name is "Find in #deploys": typing there opens that conversation's
find bar with the text and moves focus into it. The badge's ×, or Backspace
in the empty field, drops the badge for the rest of the visit to that channel,
and the field then reads "Search channels" and filters teams and channels by
name across the whole column — a team match keeps all of that team's
channels, a channel match keeps its team's heading and that channel — and says
when nothing matches. A name filter already in the field keeps that scope.
Search match counts are announced via a visually hidden `role="status"`
element so screen reader users are notified when the team list filters. It
searches the "Chat unavailable" teams too, so it is live whenever the column
lists anything, not only when a team has chat.

Find in the conversation (`chat/find-bar.tsx`, `chat/use-chat-find.ts`) opens
under the thread header from ⌘F (Ctrl+F on Windows and Linux) anywhere in the
window, the thread header's "Find in channel" button, or the window header's
field. It matches case-insensitively over the rows the open window draws, which
the agent has already decrypted and verified, and counts from the newest match:
"1 of 7". Enter steps to an older match and Shift+Enter to a newer one,
wrapping at either end, and the current match scrolls to the middle of the list
with its row marked; Esc closes the bar and returns focus to where it was.
Matches are measured in the rendered rows, so a match may span a bold word or a
mention but never two paragraphs, list items or quote lines, a control such as
a code block's Copy, or screen-reader-only text; a link matches on its label,
not its destination. While the query is in the hidden lines of a collapsed code
block, the block is drawn whole, so those matches are counted and can be
stepped to. Every match is tinted and the current one drawn solid
with the CSS Custom Highlight API, which leaves the rows' DOM alone; a webview
without it tints the matching rows instead. With no match the bar reads "No
matches in loaded messages", because older history has not been read, and
offers "Search older messages (loaded back to Sep 30)". That reads up to five
older pages (250 rows) per click, below the window, with the same history
request as Load older messages, and keeps only a reference to each matching
row: its sequence, sender, time and text. Those rows are matched on the text
the list would draw for them (`messagePlainText` in `chat/message-syntax.ts`,
which the renderer's own parsing produces), so a message matches the same
query on an older page as once it is opened, and the listed snippet carries no
Markdown markers. The window itself is not changed, so
a search never pushes it past its row or byte bound. Cancel stops the scan and
drops the page in flight, which the next run reads again; "Search further"
continues from where the last run stopped, and the line states how far back the
search reached, or that it reached the beginning of the channel. Selecting a
found message opens the channel around it with the anchored read an unread
channel opens with, which replaces the window rather than growing it; the
message is marked for two seconds and takes focus. Nothing find reads is
stored: the agent has no message index, and search across channels needs one
the agent owns.

New channel opens the Create channel sheet (`chat-new.tsx`). Every entry
point carries that name — the topbar's New channel button, the column's "+"
and its background menu, and the empty conversation's button — and each opens
the sheet on the open team; with no team open, the only team with chat is
selected, and otherwise the selector reads "Choose a team". The sheet is one
form: the team selector, the channel name and the description share one field
group, and the dialog places focus in the name field. A team that cannot chat
is listed in the selector inert, carrying its reason rather than hidden, and
each team's detail line counts the channels it offers, so a hidden
conversation is not counted. Create waits until the chosen team's channel
list has arrived — an empty name is the general channel, and whether the team
already has one is the difference between creating it and being refused —
behind a "Loading channels…" line. A team that loses chat while the sheet is
open disables Create, with the reason in the body.

Creating is `prepare-channel` then `attempt` on a client of its own, keyed by
one durable submission identifier, so a lost reply is recovered rather than
repeated; the fields are the name, the description and the audience (Everyone on
the team / Admins and owners) the action carries. Both requests are guarded the
way `useChatConversation` guards a write: the availability clock and the
server's access generation are checked before and after each one — off the
snapshot, the clock and the generations the shell holds now, read through refs
rather than the ones the request started with, so a check-in that lapses or an
access generation that moves mid-request is seen. A refusal by that check is
the local `access-changed` error the send service and the conversation's
history reads also raise, not the agent's `chat-access-denied` or the shell's
availability reason, so the inbox does not mark the team unavailable for it; a
refusal before the preparation has left this Mac withdraws the submission,
leaves its fields in the sheet and states that the channel was not created, while
a refusal of a saved preparation states that it can be checked once access
returns. A reply under a scope
other than the one the service holds for the team quarantines the account rather
than being trusted. The preparation is retired only by a settled
`attempt` — until then Recover re-issues that same operation, so an `attempt`
that failed is retried rather than turned into a second channel, and whatever
the agent is still holding is invalidated into the conversation's unsent
chip in case the sheet is closed on it. Once a request has failed
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
apart), and carries the team's unsent-work chip, the Find in channel toggle
(⌘F, Ctrl+F elsewhere) that opens the find bar, and the ⓘ that opens
`chat-info.tsx` — rendered as a third column when space permits, and
as an overlay covering the conversation pane below 1000px tab width to preserve
readability at the 960px minimum window width. The overlay covers the entire
conversation pane to avoid partially obscuring message text and header
controls. The team column remains visible to allow navigating away at any
time. It carries the channel's description; Access, with Who can read and Who
can post ("Admins and owners", "Everyone on the team", "Members at visibility
level 1 and above"); Readers, each with an avatar, the shell's `roleName` (never
the band) and "can post" or "read only"; the per-device alert mode, whose
Default choice states the mode it gives and why ("None while the channel is
muted", "Mentions only, the team default", "All new messages, the device
setting"), and the Mute channel switch, described by the note under it, which is
on and cannot be changed while the team is muted; and "Manage in Teams". A
channel that every team user can read folds its readers behind one All members
row; any other lists them and names the team users who cannot read it.

The audience comes from `channelAudience` in `chat/audience.ts`, on the roster
the team page already loads: the user parties whose role `admits` the read
role, raised to Admin on an admins-only channel because the server applies the
tier's floor first, and among them those whose role also admits the write
role. Team parties and users scoped to another host are not counted, since
basic chat serves direct same-host user members only. The header line under
the channel name, the empty conversation's sentence ("Only the 3 admins and
owners of Engineering can read it.") and the panel all draw from it, so none
of them counts the whole roster for a channel only part of it can read; such a
channel leads its header line with its reader count ("3 readers"). While the
roster has not arrived, or after its load failed, no count is drawn, and the
sentence states the read role alone ("Only the admins and owners of
Engineering can read it."). Leaving,
renaming and deleting a channel have no `ChatAction`, so the panel does not
draw them; the mute it offers is this device's.

The composer exposes text entry and delivery actions. Attachments, an emoji
picker and message expiry are omitted until supported. A hint line under the
field, drawn while the composer has focus, reads "Enter to send · Shift+Enter
new line · `**bold**` `*italic*` `` `code` ``"; Alt+Enter also inserts a line
break. The ? at the end of the line opens Formatting and keys, which lists each
form `message-text.tsx` renders with the rendering drawn by `MessageText`
itself, so the sheet cannot list a form messages do not get, and the keys.
Ctrl+B, Ctrl+I and Ctrl+E (⌘ on macOS) wrap the selection in `**`, `*` and
`` ` ``, or remove those markers when the selection is already one span in
them or the caret sits between an empty pair. A span holds no character of
its markers, as `message-text.tsx` reads it, so `**a** b **c**` is two spans
and a caret in `*a**b*` sits between two spans, not in an empty pair; the key
adds markers to these. Inline styles do not cross lines, so a selection across
lines has each non-blank line wrapped after any `- `, `* `, `1. ` or `> `
marker that starts it, keeping a list a list, and pressing the key again
removes the markers when every such line is one span; lines in a code fence
are left as typed. Ctrl+Shift+E (⇧⌘E) puts the selected lines in a code
fence. None of these is an accelerator of the macOS application menu, and the
code-block chord avoids Ctrl+Shift+C, which opens the element inspector in
Chromium-based webviews when developer tools are enabled. The edits go through
`insertText` where the webview supports it, so Ctrl+Z (⌘Z) undoes them
(`composer-format.ts`).

Typing `@` and the start of a name, or of a part after a `.`, `_` or `-`, lists
up to eight team members with their role (`matchMembers` in `mentions.ts`):
the roster's direct same-host users other than the reader, filtered by the
role a reader needs (`directUser` and `channelRoster` with `channelReadRole`
from `audience.ts`, the party set and rule the info panel's reader list
follows), so an admins-only channel offers only admins and
owners, whatever read role a Go client wrote on it, and the list says how many
members cannot read the channel. Up and Down choose,
Enter or Tab inserts `@username` and a space, and Esc closes the list until
another `@` is typed. The result is Basic text; nothing is sent to the server
about it.

From 90% of the 65,536-byte text limit the hint line states the space left in
bytes ("3.3 KB left", or "536 bytes left" below a kilobyte), and past the limit
how far over it is, in red, with Send disabled. The count is in bytes because
the limit is: a character takes one to four UTF-8 bytes, so no character count
follows from it. Screen readers hear when the message passes 90% and the
limit, not each keystroke.

With the field empty, Up moves the channel's newest unsent message whose row
offers Edit (not sent, or queued behind another) back into the field, as Edit
does, and the hint line says so. Drafts are held in memory by the send service
and are discarded on lock or quit; keeping them across a lock needs agent
actions that store them in the protected store, which do not exist yet.

`message-text.tsx` renders a Markdown subset (`**bold**`, `*italics*`,
`` `code` ``, fenced code blocks, `> quote`, `- list`, `1. list`,
`[label](https://…)`) without recursion: markup inside a token stays text. A
bare `http://` or `https://` URL that does not continue a word is a link once
trailing sentence punctuation (`.`, `,`, `;`, `:`, `!`, `?`, `'`, `"` and the
CJK `。`, `、`, `，`, `．`, `！`, `？`, `：`, `；`), a closing bracket that closes
nothing opened in the URL, and a `_` when a `_` precedes the URL are left as
text (`trimBareUrl`); it then passes `safeChatLink` and is drawn like a
`[label](url)` link whose label is the URL text. One or more blank or
whitespace-only lines between blocks start a new paragraph (a 12px gap);
single line breaks do not, and CRLF line ends read as LF.
Lists are indented inside the text column. A fenced code block is drawn on a
surface with a copy button that appears while the block is hovered or holds
focus; a block of more than 20 lines shows its first 12 and Show all N lines.
An `@name` is drawn as a mention when the name, compared as FOKS v0.1.9
username normalization compares it (lower case, `ł`, `ø`, `æ`, `ß` and the
other `flatten_unicode` letters replaced, combining marks removed, `.` and `-`
read as `_`), is a username on the team's roster, and as a highlighted mention
with offscreen text "mentions you" when it is the reader's own
(`mentions.ts`). This is styling of Basic text only: a mention raises no
notification and is not counted, and mentions as a protocol feature remain
deferred (ISSUES.md).

A link in a message carries its full URL as a tooltip, and unless
the label is the URL itself (up to the case of ASCII letters and a final `/`)
and names no other host, it is followed by the destination host in parentheses,
as `new URL(url).hostname` gives it, so an internationalized host appears in its
`xn--` form. A host longer than the line wraps rather than running out of view,
so its last labels stay visible. The link's accessible name ends with
", opens" and that host, and the visible copy is hidden from assistive
technology. `labelHosts` in `message-text.tsx` reads every host name in the
label, including both sides of an `@`, names wrapped in quotes, brackets or
punctuation, and a name, with or without a scheme, after a `?@` or `#@` that
ends a URL's authority or a leading host name
(`https://bank.example#@evil.example`,
`bank.example/?@https://evil.example`), in time linear in the label's length. It
reads the label with invisible characters removed (format characters such as
U+200B, and other default ignorable code points such as U+034F and the variation
selectors, but not the Hangul fillers, which many fonts draw as a blank), and it
takes U+3002, U+FF0E, U+FF61, U+FE52 and U+2024 as full stops, so none of these
can hide a host. The label is drawn without its bidirectional formatting
characters (U+061C, U+200E, U+200F, U+202A..U+202E, U+2066..U+2069), so the
order the reader sees is the order that was compared, and the host after it is
isolated left to right. A path-less name ending in a file extension that is not
also a top-level domain (`index.html`, `Node.js`, also `vite.config.ts` and
`paypal.com.js`) is not read as a host. A name ending in `java`, `md`, `mov`,
`py`, `rs`, `sh` or `zip`, extensions that are also top-level domains, is read
as a file name only when the text before the extension holds an upper-case
letter or an underscore after a letter or digit (`README.md`, `setup_env.sh`),
with the extension compared without regard to case; `docs.rs`, `get.sh` and
`_docs.rs_` are read as hosts. A label that writes a host in that form, such as
`Docs.rs` or `PayPal.zip` for a link to another host, therefore raises no
warning; the destination host after the link is the remaining indication of
where it opens. A name whose first label is `www` (`www.PayPal.zip`,
`www.index.html`) is always read as a host. When any of those hosts differs
from the destination host, ignoring one leading `www.`, the link is drawn with a
warning underline and icon, and a click raises Check this link, which names both
hosts and the full URL and offers Copy link, Open anyway and Cancel, with Cancel
focused. Other links open directly. The check runs in the renderer only:
`open_chat_link` receives the URL and never the label, so it validates the URL
but cannot compare the two.

The message list is one tab stop: its current row, which is the row focus last
entered while focus stays in the list or in a menu or popover a row opened;
otherwise, while the list is scrolled away from its bottom, the first row on
screen, so Tab into the list keeps the reading position; and otherwise the
newest row. The current row is followed in tab order by its links and
code-block buttons and then its toolbar; other rows' links leave the tab order
until their row is current. On a focused row, Up and Down move to the previous
and next row, Home and End to the first and last row the list holds, Enter
opens the row's actions as a menu, and Esc moves focus to the composer. The
list itself takes focus only while it draws no row, or when a control in it
goes away.

Each message row carries its actions (`message-actions.tsx`) in a toolbar over
its top corner, shown while the row is hovered or holds focus, and named after
the message's author and time. Only the current row's toolbar is in the tab
order. A toolbar is one tab stop, and the arrow keys, Home and End move along
it. A right-click on the row, the ContextMenu key or Shift+F10 on anything
focused in it, or Enter on the row itself opens the same actions as a menu;
Shift+F10 and the toolbar are the keyboard routes in WKWebView on macOS, which
has no ContextMenu key. Copy message copies the message's
Markdown source. On an unsent or pending message it is joined only by Message
details, once the agent has prepared an operation for it. Quote
reply, offered only where the reader can send, writes a quote as Basic text at
the composer's caret, on a line of its own: a `>` line with the author in bold
(as `@name` when the name is on the team's roster) and the message's clock
time, then its first two non-blank lines as `>` lines without fence lines, each
cut near 100 characters, an ellipsis where text was left out, and a blank line
for the reply. Consecutive `>` lines render as one quote. No Reply message type
is sent, and the quote is ordinary draft text: no chip names the quoted author
or removes the quote as a unit. Message details lists the sender ("Claimed
sender" for a message that could not be decrypted or verified, or whose type
the client does not open, since only Basic text is authenticated), the server's
receive time, the sender's clock and the sequence number, and the verification
of the history page that loaded the message: the agent reports
`missing_predecessors` per page, so the wording names that page and claims no
result for the one message. Edit, Delete, Reaction and Reply are reserved
message types that the server does not accept, so no action is drawn for them.
Copy link waits for a message identifier that means the same message to another
member: a store reference is local to this Mac, and no URI scheme routes a link
back into the app.

Your own unsent work is drawn as your message, with avatar and author, whether
it is an outgoing message of this session or a tracked operation restored from
the agent's ledger; a restored operation has no time, because `ChatOperation`
carries none. `delivery.ts` holds the one state vocabulary all of these rows
use: Sending, Queued, Not sent, Checking delivery, Couldn't confirm delivery,
Couldn't post (Couldn't create for a channel), Cancelled and Waiting for
access. The state follows the header's time, and a line under the text says
what happened, with only the actions the state allows. A rejection reads as
its reason ("New messages arrived in #general first."), mapped from the closed
set of codes `definite_rejection` in foks-client treats as refused before
commit (1013, 1030, 12002, 12003, 12005 and 12006). The numeric code, its
generated status name, the full operation ID with Copy ID, and the ledger
state appear only in Message details. A failure the agent returns with a
`chat-*` code is shown with the copy table in `error-copy.ts` rather than the
agent's message, which names the failing check in client-library terms; the
agent's text stays on the error object and the code in the diagnostic timing
log. Failures the desktop raises itself keep their own message.

A channel with unread messages opens at the first of them, with the NEW divider
30% down the list and the ten messages before it above, rather than at the
newest message. A message counts as read once its row has been at least half on
screen, or has covered half the list, for half a second while the window has
focus, so a channel left halfway stays unread from where the reader stopped.
When more is unread than a page holds, the list ends in "Load newer messages",
which also loads as it scrolls into view. While the first new message is below
the list, a banner gives the unread count with Jump to first new and Mark as
read, and Esc marks the channel read when the composer is empty (on a focused
message row, Esc moves to the composer first). The jump control at the foot of
a scrolled-up list counts the messages from others that arrived since the
reader scrolled up ("2 new messages"), and reads "Jump to latest" otherwise.
Older messages load as the reader scrolls toward the top of the list; "Load
older messages" stays in the tab order for the keyboard and appears when it
has focus. A conversation holds at most 400 messages, fewer
when they are long: paging further back drops the newest ones, after which the
list ends in "Load newer messages", and new messages drop the oldest. Switching
to another channel or tab and back reopens a channel at the message the reader
left it on, or at the first unread message when more than a page of messages
arrived in between. `book/20-desktop.qmd` describes the history reads behind
this.

Saved work the open thread does not draw (channel creations, and messages in
other channels or in a channel the reader can no longer read or that is
stopped) is counted by a chip in the conversation header, or in a bar of its
own when no channel is open. The count covers the states that leave work for
the reader (Not sent, Checking delivery, Couldn't confirm delivery, Couldn't
post, Waiting for access) and reads "Saved work" when none of them remains.
The chip opens a popover that lists each operation with its channel, saved
text, state and verbs, and Jump for a message in a channel the reader can
open; Esc closes it and returns focus to the chip. Once the last of it is
accounted for the chip says "All caught up" rather than disappearing without a
word, and the popover closes, so the next saved operation does not reopen it.
A channel creation's failures use their own copy where a message's would name
the wrong thing (`chatErrorText(error, 'channel-creation')`), both in the
creation sheet and when a Retry or Cancel from the unsent list fails. A
channel whose content could not be verified shows "Channel stopped" under its
own header, with "Recheck channel" and "Copy diagnostic details". Recheck removes the
channel from the blocked set the desktop sends with `sync-inbox`
(`ChatInboxService.unblockChannel`) and synchronizes the team again: the agent
reports the channel in `blocked_channels` again if it still detects the
conflict, and otherwise the thread reads its history and verifies it from the
start, where hard-state anchors still catch an equivocating server. A
synchronization already running when the reader rechecks was sent the old set,
which the agent echoes back; those echoes are ignored. A channel stopped again
after a recheck says when that happened, until a synchronization requested
since the recheck and a history read since the recheck have both left it
unblocked (`chat/recheck.ts`). The inbox numbers its `sync-inbox` requests
across teams (`syncSequence`) and `unblockChannel` returns the last number
sent, so the reply to a request still running at the recheck does not count. The
diagnostic details carry the failure that stopped the channel, its code and
the agent's description of the failed check, or name `sync-inbox` when the
channel was stopped because the synchronization listed it. "Chat stopped", the
account-wide quarantine, still needs a new chat session, which the next unlock
starts; it offers Copy diagnostic details, which carries the error code and
the agent's own description of the failed check that the pane does not show.
A page whose predecessors could not all be fetched still raises one channel-wide
band. A header chip that scrolls to the first such loaded page needs only
today's data and is not built yet; marking the individual rows needs a
per-message `predecessor_unchecked` field from the agent. When the team's server
check-in has lapsed the column stays and the pane carries the notice, with
"Check in" opening that server in Settings › Servers; every other unavailable
state is stated without an action, because no button in Chat resolves it, and
"Open another team" is offered only when another team can actually be opened. A
location naming a team that has no chat says so in the pane rather than waiting
for an inbox that will not arrive. A location naming a channel the team does not
list _yet_ is the opposite case: while that team has been invalidated and its
next synchronization has not landed, the pane says it is loading rather than
calling the channel unavailable, which is what a channel created a moment ago
would otherwise read as. A synchronization that failed carries its error
instead, so the wait always has something behind it.

Four things the chat redesign drafts showed are deliberately not built, because
no `ChatAction` reaches them: reactions, "Retry all" and "Discard" over every
pending operation at once (one attempt per operation is the rule the recovery
model keeps), Leave / Delete / Edit description in the ⓘ panel (Mute is
there, as a setting on this device), and a "Join a team" button, which has no
FOKS equivalent — a team arrives through an invitation or a per-server check
for groups, both of which live in Teams, so the column's foot keeps the one
"Create or join a team" button.

The conversation header carries Refresh alongside search, files and ⓘ, because
the history, channel list and saved work can become stale.

### `apps/desktop/kit`

`apps/desktop/kit/` holds reusable presentation primitives that depend on nothing
app-specific: `overlay-primitives.tsx`, `menu-position.ts`, `toasts.tsx`,
`virtual-list.ts`, `icon.tsx`, and `tokens.css`. `apps/desktop/kit/README.md` is the
authority on what is in it and why — including why `ui/src/sheet.tsx` stayed
behind. Modules in `src/` import it by relative path, as they import each
other, so node and `tsx` load them without Vite's resolver; there are no
`/src/*` or `/kit/*` aliases. Application-specific models, fixtures, icons, and shell styles remain in
`src/` to keep application logic separate from reusable UI components.

`toasts.tsx` appends information toasts to a `role="status"` polite live
region and warnings to a `role="alert"` assertive one, so warnings are drawn
below information toasts whichever was shown first. Both regions stay
mounted while empty, because screen readers, VoiceOver in WebKit in
particular, can skip a live region inserted together with its text. A
repeated message replaces its toast's text node, so it is announced again
and its countdown starts over. A toast
is a warning only when its caller passes `tone: 'warning'`. A toast without a
caller-given `durationMs` stays for its reading time (one second plus 60 ms a
character), at least 4 s, 8 s for a warning, and at most 20 s; an actionable
toast without one stays until it is dismissed. The countdown holds while the
pointer is over the toast, while focus is inside it and while the window is
hidden, and a released toast stays at least one more second. A caller's
`expiresAt` ends the toast even while it is held: the Move to Trash toast
closes when ⌘Z stops undoing, and a copy toast by the time the host clears
the clipboard. Render tests
that look for a page's own status or alert exclude `#toasts` or require text,
since the empty regions match a role query; jsdom reports `document.hidden`
as true, so toasts in render tests do not time out unless a test sets it.

The model logic is implemented exclusively in TypeScript because item kind classification
and reader computations are client-side presentation models without protocol equivalents.

## Design tokens and the theme decision

`src/styles/shell.css` defines the desktop shell styles. Make shell design
changes directly in this stylesheet.

`src/styles/index.css` is the only stylesheet the app imports, from
`src/main.tsx`. It declares the cascade layers `tokens, base, components,
features, overrides` and imports each sheet into one of them:
`kit/tokens.css` into `tokens`, `shell.css` into `base`, `app.css` into
`components`, each feature's sheet into a `features.<name>` sublayer
(`screens/first-run.css`, `screens/files.css`, `screens/chat.css` and
`screens/chat-picker.css`, `screens/team-info.css`,
`shell/search-palette.css`), and `native-window.css` into `overrides`. A
declaration in a later layer outranks one in an earlier layer whatever their
specificity (`!important` reverses the order), so a feature's rules override
the component rules without heavier selectors. `features.first-run` is the
first sublayer, so the rules of the other features still outrank it. Where a
first-run rule must keep a component value, it declares the property as
`revert-layer` rather than copying the value from `app.css`.
`tests/styles-geometry.test.ts` finds every `.css` file under `src/` and
`kit/` and fails unless the entry imports it exactly once into a declared
layer; it also fails if any other sheet contains an `@import`, if a module
other than `src/main.tsx` imports a stylesheet, which would put the sheet
outside every layer, or if `app.css` selects a first-run class.

Layout is a separate concern from color, and its custom properties are
declared on the shell rather than on `:root`: `--side-w-open` (150px) and
`--side-w` on `.window`, and `--side-track`, `--side-pad`, `--side-head-pad`,
`--side-open-content` and `--details-w` (300px) on `.app`. The rail animates
its own width instead of the grid track, which avoids interpolating
`grid-template-columns`, and `.window:has(>.app.side-narrow)` is what sets
the collapsed width (`--side-w: 56px`).

`kit/tokens.css` owns every global custom property: the `:root` tokens, the
`data-rail` sidebar colors and the `:root[data-theme='dark']` overrides.
`index.css` imports it into the `tokens` layer, and
`tests/styles-geometry.test.ts` fails if any other sheet declares a `:root`
block or a theme selector.

**Appearance:** Preferences offers Light, Dark, and System. Light is the default;
the choice is stored locally as `appearance`. `public/theme-init.js` applies the
choice before first paint, and `src/platform/appearance.ts` follows system and storage
changes. Dark overrides live in `kit/tokens.css`, using the Files redesign palette
while preserving the selected sidebar color.

## Navigation guards

Sheets are not addressable by `Location` or a URL. Rail-tab switches retain
approved input forms in the optional `LocationState.sheet` memory: new items,
new channels, member addition, group creation, invitation details, and typed
pairing acceptance. Screens explicitly opt their selectors and fields into
`useTabSheetState`; PINs, passphrases, recovery-phrase reveals, card setup, revocation,
and in-flight writes are not restored. Confirming a discard clears this memory.
Window focus loss does not dismiss these workflows. `useConcealOnInactive`
instead belongs to the plaintext surface it protects: pairing offers and
revealed vault values hide, the first-run recovery phrase stays on its current
step with its words masked, and plaintext item drafts remain mounted but masked
until the reader reveals them again. Displayed recovery phrases and password inputs
keep their workflow state.
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

`navigationVerdict(intent)` runs the same guards without acting on anything.
History navigation, including completed trackpad gestures, applies the guards
through the store so prompts and refusals are presented consistently.

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
A folder result is one move in the same way: `navigateToFolder` opens the
store page with the folder open in its tree.

Four shortcuts sit on the document, where an open dialog cannot intercept
them, so they test `anyDialogOpen()` themselves: Control-Tab in
`shell/sidebar.tsx`, ⌘K in `shell/search-palette.tsx`, Alt+Up/Down and
Alt+Shift+Up/Down in `screens/chat-teams.tsx`, and ⌘F (Ctrl+F) in
`chat/chat-thread.tsx` do nothing while a dialog owns the window.

## Testing

The unit and render-test layers live in `tests/`:

- `model.test.ts` — verifies reader calculations, role arithmetic, and store
  aggregation invariants against expected baseline values.
- `location.test.ts` — pure transitions and the `?state=` round trip, including
  the original design-state names, and the guard
  mechanism: verdict order, what `force` skips, and a prompt superseded by a
  second navigation.
- `navigation-guard.render.test.tsx` — the confirmation a `prompt` verdict
  raises, and what confirming, canceling and Escape each do behind it.
- `module-boundaries.test.ts` — the import layers in `layers.json`, checked
  against resolved paths; no runtime import cycle inside a folder beyond the
  listed chat provider edges, and no import of a folder's own barrel or
  facade from inside it; every chat module
  reachable from `src/main.tsx`.
- `react-boundary.test.ts` — the negative invariants: no raw-HTML sink
  (the mock's whole render layer is `innerHTML`, and none of it came along), no
  `window.__TAURI__`, the Tauri API imported in one file, the model free of the
  DOM, the mock bundled out.
- `styles-geometry.test.ts` — the stylesheet entry and its cascade layers,
  the first-run rules' sheet and layer, the token sheet's ownership of global
  custom properties, light and dark color schemes, the 12px type floor,
  token-only colors and reduced motion, read off the CSS since jsdom computes
  no layout.
- `sync-status.render.test.tsx` — the topbar's refresh control: one button, and
  the Refresh status popover it opens on hover. The popover is portaled, so the
  pointer leaves the button's wrapper on the way into it; the test holds that
  the crossing does not close it and that leaving both does.
- `app-root.render.test.tsx` — jsdom plus Vite `ssrLoadModule`, the same boot
  as `ui/tests/app-root.render.test.tsx`: the app mounts itself into `#root`
  from `src/main.tsx`, exactly as it does in the browser.

Render tests call `installDom` from `tests/lib/dom-harness.ts` and then load
the modules they render with `await import('../src/...')`, so the path is
checked by the type checker and the module is the same instance a static
import of it gives. A module is evaluated once, by the first import that
reaches it. When that is an `await import()` after `installDom`, the module
evaluates against the jsdom globals and the harness's timers. A module that a
static import of the test file already reaches was evaluated before
`installDom`: `src/data/roster-staleness.ts`, for example, registers its window
listener only if `window` exists at evaluation. A render test's static value
imports from `src/` and `kit/` must therefore reach only modules that read no
DOM global or timer at evaluation; a test that needs no DOM can import modules
statically. `module-boundaries.test.ts` walks the static imports of every test
that calls `installDom` and fails if they reach a module that names `window`,
`document`, another DOM global or a timer outside a function body. `import.meta.env` is undefined outside Vite, so `selectBridge()`
throws instead of returning the mock bridge, and a render test passes its
bridge explicitly. A screen or sheet that reads the shell services renders
under the `TestShell` that `loadTestShell()` in `tests/lib/test-shell.ts`
returns: it provides the given snapshot and services, and fills the rest with
the mock bridge, handlers that do nothing and reloads that resolve with the
snapshot. `workflowScope()` in `tests/lib/workflow-scope.ts` mounts the same
`TestShell` over a snapshot whose servers are all verified and available.
With `timers: true` the harness unreferences the global
timers and keeps the event loop open for at most two minutes from the start of
each test; a test still waiting on an unsettled promise after that is
cancelled, with the rest of its file.
tsx compiles `src/` with the options of `apps/desktop/tsconfig.json` through
the repository's root `tsconfig.json`, which extends it; without it tsx would
compile JSX for the classic runtime. Only `app-root.render.test.tsx` starts
Vite, because `src/main.tsx` imports the stylesheet; it also loads
`src/app/deferred-screens.tsx` through Vite, since it needs the instance
`src/main.tsx` loaded. `module-boundaries.test.ts` fails if another test
imports `vite`.

The Rust command layer covers runtime-dependent behavior. App lock, clipboard
concealment/clearing, native picker/drop streaming and webview resident-set
behavior are tested there or manually.

## Deep links

The current screen, selection, and presentation are encoded in
the address bar, so every state can be reloaded and inspected. `?state=` is the mock's own vocabulary (`01-vault.html`'s `STATES`),
kept so deep links defined in the design specification resolve to this location.

| `?state=`                                                                            | Opens                                   | What else it fixes                                                                                            |
| ------------------------------------------------------------------------------------ | --------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `all`                                                                                | All items                               | —                                                                                                             |
| `personal`                                                                           | Personal (`acct:personal`)              | —                                                                                                             |
| `work` · `household` · `homelab`                                                     | that store                              | —                                                                                                             |
| `password`                                                                           | All items                               | selects the masked GitHub login                                                                               |
| `show`                                                                               | All items                               | loads and reveals GitHub version 9 once                                                                       |
| `resource`                                                                           | All items                               | selects the masked Anthropic API key                                                                          |
| `file`                                                                               | All items                               | selects Household's emergency PDF                                                                             |
| `link`                                                                               | All items                               | no Link items are shown, so it selects the same group password as `group`                                     |
| `group`                                                                              | Household (`team:household`)            | selects the Wi-Fi password                                                                                    |
| `new`                                                                                | All items                               | Password sheet in Household with explicit group roles and computed reader preview                             |
| `new-group`                                                                          | All items                               | Resource sheet in Engineering with explicit roles and computed reader preview                                 |
| `new-resource`                                                                       | All items                               | Resource sheet in Personal                                                                                    |
| `new-file`                                                                           | All items                               | File sheet in Household; renderer receives paths, never bytes                                                 |
| `group-new-text` · `group-new-file`                                                  | Engineering                             | Phase 7 group create acceptance scenes for text and streamed File writes                                      |
| `exists`                                                                             | All items                               | must-not-exist refusal; Open refreshes the invalidated catalog first                                          |
| `conflict`                                                                           | All items                               | exact-version refusal with retained draft and Refresh and review                                              |
| `grid`                                                                               | All items                               | `view=grid`                                                                                                   |
| `folders`                                                                            | All items                               | `view=folders`; store roots and folders are derived from catalog paths                                        |
| `lease`                                                                              | Work (Acme)                             | `lease=lapsed` — the whole snapshot, not a place                                                              |
| `inactive`                                                                           | Homelab                                 | group reports inactive; Resume creation uses its resumable operation                                          |
| `alerts`                                                                             | Accounts, on its attention list         | `lease=lapsed`, so the list has its critical entry                                                            |
| `agent-lost`                                                                         | Full window stop                        | Retry reconnects and refreshes without replay                                                                 |
| `groups`                                                                             | Teams                                   | the list, then the collapsed Check-other-servers row; `store=` names the account create/join act as           |
| `people`                                                                             | Accounts                                | the attention list over one account's panel; `store=` names the account                                       |
| `group-people` · `party` · `federation`                                              | Engineering group page                  | Members tab; `party` opens a member row's menu                                                                |
| `group-channels`                                                                     | Household group page                    | Channels tab, on the group whose server offers chat; `tab=channels` reaches it on any group                   |
| `group-files`                                                                        | Engineering group page                  | Files tab; `tab=files` reaches it on any group                                                                |
| `danger`                                                                             | Engineering group page                  | Settings tab                                                                                                  |
| `store` · `items`                                                                    | Engineering                             | group vault                                                                                                   |
| `add` · `promote` · `demote` · `remove` · `admit`                                    | Engineering group page                  | the named Group sheet; `add` and `admit` are the two halves of one sheet                                      |
| `invite`                                                                             | Engineering group page                  | the Invite sheet, seeded to the account holding the group                                                     |
| `create`                                                                             | Teams                                   | named/ad-hoc Create group sheet on the acting account; `store=` names it, else this Mac's first               |
| `groups-lease` · `groups-inactive`                                                   | Engineering group page or Homelab vault | distinct lease/inactive takeovers                                                                             |
| `manage`                                                                             | Household group page                    | Members tab, without a Manage overlay                                                                         |
| `party-remove`                                                                       | Engineering group page                  | the non-local removal refusal                                                                                 |
| `join`                                                                               | Teams                                   | account-specific discovery and invite choices                                                                 |
| `join-invite`                                                                        | Teams                                   | invite sheet opened on the exact `acct:work` fixture store                                                    |
| `boot` · `who` · `address` · `no-address` · `checked` · `compare` · `error`          | First run, steps 0–2                    | `path=invited` or `path=own` selects the setup route                                                          |
| `account` · `existing` · `protect` · `phrase`                                        | First run, steps 3–4                    | account creation/recovery and the one-time backup sheet                                                       |
| `waiting` · `added`                                                                  | First run, steps 5–6                    | invited group discovery and completion                                                                        |
| `checklist-invited` · `checklist-own`                                                | Get started inside the ordinary shell   | resumable nonsecret progress summary                                                                          |
| `first-run&step=<step>&path=<path>`                                                  | the resumable first-run location codec  | used after the first in-app transition and across reload                                                      |
| `servers-list` · `servers-server` · `servers-lapsed` · `servers-rollback`            | Settings › Servers                      | the list, then one server's page; `profile=` opens it                                                         |
| `servers-reset` · `servers-add` · `servers-unprobed` · `servers-check`               | Settings › Servers                      | typed removal, add/check and explicit result states                                                           |
| `settings&section=servers` · `settings&section=preferences` · `settings&section=mac` | Settings                                | the three pages of the sub-navigation; retired section names resolve to the page that holds them              |
| `settings-macs` · `settings-macs-work` · `settings-phrase`                           | Devices                                 | Macs, pairing and the one-time recovery-phrase reveal; `settings-macs-work` names the exact `acct:work` store |
| `settings-keys` · `settings-enrol`                                                   | Devices                                 | the Security key enrollments section and the hardware-key account sheet                                       |
| `devices&store=<StoreRef>&device=<key>`                                              | Devices › one key                       | that key's own page; `device=` is the key id, or `yubi:<alias>` for an enrollment                             |
| `settings-account`                                                                   | Accounts                                | the account panel and its workflows                                                                           |
| `settings-agent` · `settings-about`                                                  | Settings › Device                       | the application version and lock, the agent status and socket                                                 |

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
Device and recovery-phrase metadata is keyed by profile and account store; security-key
enrollments are shared by profile. Fresh entries are reused for one minute.
Returning to either tab paints cached rows immediately; expired entries remain
visible while a read refreshes them. Concurrent metadata reads share requests.

The shared `MetadataRepository` owns immutable metadata, subscriptions, freshness,
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
