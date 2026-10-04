# Desktop product features outside chat

Area key `fe-product`. 14 verified findings. Part of the [foks-rs review](../README.md).

## Current state

The non-chat desktop is careful at the security boundary: values are revealed one at a time on request, the host does clipboard writes with digest-only auto-clear, edits and moves are bound to dirent identity and version, and ambiguous mutation outcomes are reconciled, not retried. As a product it is still MVP. There are two client-side kinds (Password and Document), every action works on one item, and there is no trash or undo, version history, password generator, import/export, cross-vault sharing, favourites/recents or activity log. Moving an item means typing an absolute path. Several gaps are correctness or security problems rather than polish. (1) In production, Password classification and the masked field template depend on an `Item.value` that only the mock fixture supplies. A login outside `/logins/` is shown as a Document, and "Copy" puts the whole `user:/password:/url:` text on the clipboard while the toast says "Password copied". (2) No inactivity, sleep or screen-lock auto-lock exists, although book/20-desktop.qmd says one does. (3) The empty-vault card tells users to run a CLI command (`foks kv export`) that does not exist. Teams and Devices cover the administrative actions but not their consequences: there is no list of secrets a removed member or device could read. They also omit history, although verified sigchains already carry signed times, sequences and signers. Most refinements below need no protocol change. They reuse existing agent operations (MoveKv, RemoveKv, CreateFolder, ReadKv, kvGetNode) or data the client already verifies, so they keep Go v0.1.9 interop and the ciphertext-only server.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [No inactivity, sleep or screen-lock auto-lock; add a Security settings section](#fe-product-auto-lock) | security | high | M |
| [Production login classification and Copy act on the whole "user:/password:/url:" text](#fe-product-login-fields-copy) | correctness-risk | high | M |
| [Permanent delete with no undo, and no folder delete; add a role-aware Trash built on MoveKv](#fe-product-trash-undo) | missing-feature | high | M |
| [Exposure review and rotation checklist after removing a member or device](#fe-product-exposure-review) | missing-feature | medium | M |
| [Starter card advertises a nonexistent `foks kv export`; add host-side CSV import and export](#fe-product-import-export) | missing-feature | medium | L |
| [Files keyboard model and ARIA semantics, plus a discoverable shortcut sheet](#fe-product-keyboard-a11y) | feature-refinement | medium | M |
| [Organising: folder-picker move, drag-to-folder, and multi-select bulk actions](#fe-product-organize-bulk) | feature-refinement | medium | L |
| [Password generator, reveal toggle and strength feedback in New and Edit](#fe-product-password-generator) | missing-feature | medium | S |
| [⌘K palette: quick copy actions, recents and favourites, folders and commands](#fe-product-search-quick-actions) | feature-refinement | medium | M |
| [Share or copy an item into a team vault](#fe-product-share-to-team) | missing-feature | medium | M |
| [Show signed device and team history from the verified sigchains](#fe-product-signed-activity) | missing-feature | medium | L |
| [Item version history and restore from versions this device has seen (no protocol change)](#fe-product-version-history) | missing-feature | medium | L |
| [Single attention centre with a post-setup checklist and recent-activity history](#fe-product-attention-centre) | feature-refinement | low | M |
| [Remove or use the unused `copy_item_path` and `create_link` IPC commands](#fe-product-unused-vault-ipc) | code-quality | low | S |

### fe-product-auto-lock

**No inactivity, sleep or screen-lock auto-lock; add a Security settings section**

- Type: security
- Priority: high
- Effort: M
- Layers: desktop-native, desktop-ui, docs
- Verification: adjusted

Locking happens only by hand. The account popover's Lock item and the Settings › Storage 'Lock now' button both call the lock_app command, which is the only caller of AppLock::lock(). No idle timer exists, the app does not lock on system sleep or screen lock, and no keyboard shortcut exists. Once unlocked, the vault stays readable until the user locks it. The book (20-desktop.qmd and the glossary) says the app lock engages after a period of inactivity. The clipboard auto-clear interval is a compile-time constant (15 s on Linux, 30 s elsewhere), is not shown to the user, and lock does not clear the clipboard. Settings has no lock or clipboard policy controls.

**Evidence**

- [`book/20-desktop.qmd:92`](../../../book/20-desktop.qmd#L92): says the app lock engages 'after a period of inactivity or on request'
- [`book/appendix-b-glossary.qmd:15`](../../../book/appendix-b-glossary.qmd#L15): glossary repeats the inactivity claim
- [`apps/desktop/src-tauri/src/applock.rs:146`](../../../apps/desktop/src-tauri/src/applock.rs#L146): lock_app is the only caller of AppLock::lock(); it does not clear the clipboard
- [`apps/desktop/src/app/vault-shell.tsx:460`](../../../apps/desktop/src/app/vault-shell.tsx#L460): lockFromMenu: the account popover trigger
- [`apps/desktop/src/screens/settings-screen.tsx:432`](../../../apps/desktop/src/screens/settings-screen.tsx#L432): Settings › Storage has a manual 'Lock now' row; no policy controls
- [`apps/desktop/src/shell/sidebar.tsx:514`](../../../apps/desktop/src/shell/sidebar.tsx#L514): Lock item in the account popover, with no accelerator
- [`apps/desktop/src-tauri/src/lib.rs:78`](../../../apps/desktop/src-tauri/src/lib.rs#L78): install_macos_menu defines Settings/New accelerators but no Lock
- [`apps/desktop/src-tauri/src/clipboard.rs:17`](../../../apps/desktop/src-tauri/src/clipboard.rs#L17): AUTO_CLEAR_SECONDS is fixed per platform
- [`apps/desktop/src/screens/settings-screen.tsx:247`](../../../apps/desktop/src/screens/settings-screen.tsx#L247): Preferences holds only appearance, alerts and tips

**Recommendation**

Implement the policy in the Rust host, not the renderer, because the renderer is untrusted. Add an `IdlePolicy { lock_after: Option<Duration>, lock_on_sleep: bool, clear_clipboard_on_lock: bool, clipboard_seconds: u64 }` stored in host state (an app-config file), with get/set commands that require main window and unlocked state. Timer: the renderer calls a throttled `note_activity` (at most once per 30 s, on pointer and key events), and the host locks when the deadline passes. A compromised renderer could keep the session alive, but it already holds an unlocked session. OS triggers, handled in the host: on macOS, the `com.apple.screenIsLocked` distributed notification and NSWorkspace will-sleep; on Linux, logind `PrepareForSleep` and the session `Lock` signal over D-Bus (the host already talks to polkit). Make `clipboard_seconds` configurable within [10, 120] and clear on lock. Return `clearsInSeconds` in the copy acknowledgement so the toast can show a countdown. Add a CmdOrCtrl+L 'Lock' accelerator to install_macos_menu and a matching renderer shortcut. Until this ships, correct book/20-desktop.qmd.

**Mockup:** [Auto-lock and clipboard policy](../mockups/security-auto-lock.html)

<details><summary>Verifier note</summary>

The core claim holds. AppLock::lock() is called only from lock_app (applock.rs:146-159). No idle timer, sleep handler or screen-lock handler exists; a search of the renderer and host for idle, inactivity, sleep and screen-lock handling found nothing. install_macos_menu has no Lock accelerator (lib.rs:78-91). AUTO_CLEAR_SECONDS is fixed at 15 s on Linux and 30 s elsewhere (clipboard.rs:17-20). lock_app does not clear the clipboard. book/20-desktop.qmd:92 and book/appendix-b-glossary.qmd:15 both say the lock engages after inactivity. zbus is already a dependency (polkit.rs), so the logind signals are reachable. One piece of evidence is wrong: lockFromMenu is not the only renderer trigger. Settings › Storage (the 'mac' section, DeviceSection) has a 'Lock' row with a 'Lock now' button (settings-screen.tsx:432-449) that also calls onLock and lock_app. Settings therefore already has a manual lock control, but no lock or clipboard policy controls.

</details>

### fe-product-login-fields-copy

**Production login classification and Copy act on the whole "user:/password:/url:" text**

- Type: correctness-risk
- Priority: high
- Effort: M
- Layers: client-lib, agent, desktop-native, desktop-ui
- Verification: adjusted

New passwords are stored as one small file `user: X\npassword: Y\nurl: Z`. kindOf recognises a Password either from a `password:` line in Item.value or from a `/logins/` path. The production catalog never carries value (snapshot-projection strips it in native mode), so only the path rule applies there. NewSheet builds the path from the selected folder (e.g. /work/github.com), and such a login is shown as a Document. copy_item_value copies the entire decrypted text, so the list-row and details Copy actions put the username and URL lines on the clipboard together with the password while the toast says 'Password copied'. The mock merges fixture items that carry masked value templates, and its copy command is a no-op, so UI tests exercise a classification and field layout that production cannot reproduce.

**Evidence**

- [`apps/desktop/src/model/kinds.ts:51`](../../../apps/desktop/src/model/kinds.ts#L51): PASSWORD_LINE tests item.value; otherwise only the /logins/ prefix makes an item a Password
- [`apps/desktop/src/bridge/snapshot-projection.ts:926`](../../../apps/desktop/src/bridge/snapshot-projection.ts#L926): native mode uses ItemDto fields only and deletes value; the mock merges base fixture items that carry value
- [`apps/desktop/src-tauri/src/commands/vault.rs:54`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L54): ItemDto has no value or shape field
- [`apps/desktop/src/screens/write-workflows.tsx:489`](../../../apps/desktop/src/screens/write-workflows.tsx#L489): Password path = namedPath(site, '/logins', initialFolder); the selected folder replaces /logins
- [`apps/desktop/src/screens/write-workflows.tsx:645`](../../../apps/desktop/src/screens/write-workflows.tsx#L645): logins written as `user: …\npassword: …\nurl: …`
- [`apps/desktop/src-tauri/src/commands/vault.rs:1183`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1183): copy_item_value copies read.value (the whole small file)
- [`apps/desktop/src/screens/items-screen.tsx:1034`](../../../apps/desktop/src/screens/items-screen.tsx#L1034): list-row Copy uses copyItemValue and toasts 'Password copied'
- [`apps/desktop/src/screens/details-panel.tsx:699`](../../../apps/desktop/src/screens/details-panel.tsx#L699): details toast 'Password copied' for the multi-line copy
- [`apps/desktop/src/screens/details-panel.tsx:143`](../../../apps/desktop/src/screens/details-panel.tsx#L143): passwordFields(undefined, null) shows only a masked password row in production
- [`crates/foks-client/src/kv/sync.rs:1080`](../../../crates/foks-client/src/kv/sync.rs#L1080): metadata sync already opens each small-file plaintext and then zeroizes it; this is the place to compute a shape bit
- [`apps/desktop/src/mock-bridge.ts:601`](../../../apps/desktop/src/mock-bridge.ts#L601): mock copyItemValue is a no-op

**Recommendation**

1) Add a host command copy_item_field(storeId, path, version, field: 'password'|'username'|'url'|'value') in src-tauri/commands/vault.rs next to read_text. It parses the decrypted small file in Rust, accepting the same aliases as newItemDraft (user/username, url/website), and passes only the selected value to clipboard::copy_with_hygiene. A missing field returns a typed field-missing error. Point the list-row Copy, the details Copy and the palette quick action at field 'password' for Password items. 2) Add one non-secret bit, e.g. shape: 'login'|'text'|null, to the catalog. Compute it in foks-client's metadata sync, which already opens each small-file node before zeroizing it (crates/foks-client/src/kv/sync.rs:1080-1094). Carry it through the soft projection, KvCatalogReport, the agent's KvEntryMetadata (agent IPC only, not Go wire) and its cached-catalog pages, then ItemDto and decodeItem. Field values stay out of the catalog. 3) In kindOf, use shape when present and keep the /logins/ prefix only as a fallback. Extend the inline login edit form, currently gated on isLogin, to cover these items. 4) Make the mock project items exactly as production does: move the masked templates out of Item.value into a mock-only plaintext map, and record copied fields so tests can assert them. No protocol change; the stored format stays a v0.1.9 small file.

**Mockup:** [Login items: field copy, generator, quick copy](../mockups/vault-login-copy-and-generator.html)

<details><summary>Verifier note</summary>

The core claim holds. kindOf (kinds.ts:51-61) needs either item.value or a /logins/ prefix. ItemDto (vault.rs:54-64) and decodeItem (vault-catalog.ts:219-235) carry no value, and snapshot-projection.ts:926-948 strips value in native mode while the mock merges in fixture items that have values. NewSheet builds paths with namedPath(site, '/logins', initialFolder) (write-workflows.tsx:149-158, 489-498), and both vault-shell.tsx:486 and screen-router.tsx:93 pass the selected folder, so a login created in /work becomes /work/<site> and is classified as a Document. copy_item_value (vault.rs:1183-1203) copies the whole read.value. The list-row Copy (items-screen.tsx:1034-1043) and every details Copy button (details-panel.tsx:697-699, 934-970) use it, and the toast says 'Password copied'. The mock's copyItemValue (mock-bridge.ts:601) is a no-op. ISSUES.md and the book do not track this. Recommendation step 2 is wrong about where the work happens. foks-desktop only receives KvEntryMetadata from the agent's ListKv/ListTeamKv and has no small-file content at catalog load. The listing runs foks-client's metadata sync (KvSyncMode::Metadata), which already fetches and opens every small-file node and then zeroizes the plaintext (sync.rs:1080-1094). The shape bit has to be computed there and carried through KvCatalogReport, the agent's KvEntryMetadata and its cached-catalog pages, so the client-lib and agent layers are also involved. The stored login format uses 'user:' and 'url:', so the host field parser must accept the same aliases that newItemDraft accepts (write-workflows.tsx:190-206).

</details>

### fe-product-trash-undo

**Permanent delete with no undo, and no folder delete; add a role-aware Trash built on MoveKv**

- Type: missing-feature
- Priority: high
- Effort: M
- Layers: client-lib, agent, desktop-native, desktop-ui, docs
- Verification: adjusted

Deleting an item writes a tombstone through RemoveKv, and the sheet says 'This cannot be undone'. The folder context menu shows 'Delete folder' permanently disabled, and the host rejects directories. For a secrets vault, one mis-click destroying a credential with no recovery path is the most likely data-loss route. The agent already supports MoveKv, which moves a whole folder subtree as one atomic namespace mutation (a source tombstone plus a destination dirent), and MkdirKv. A Trash built from these needs no protocol change. Moving a folder into Trash also avoids recursive delete, which cannot prove a directory is empty while peers write to it: a concurrently added child moves with the folder instead of being destroyed.

**Evidence**

- [`apps/desktop/src/screens/write-workflows.tsx:1489`](../../../apps/desktop/src/screens/write-workflows.tsx#L1489): delete sheet: 'This cannot be undone.'
- [`apps/desktop/src/screens/items-screen.tsx:1469`](../../../apps/desktop/src/screens/items-screen.tsx#L1469): 'Deleting folders is not supported by the desktop agent.'
- [`apps/desktop/src-tauri/src/commands/vault.rs:707`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L707): remove_item_operation rejects directory nodes
- [`crates/foks-agent-proto/src/message.rs:956`](../../../crates/foks-agent-proto/src/message.rs#L956): MoveKv and RemoveKv{recursive} exist
- [`crates/foks-client/src/kv/write.rs:655`](../../../crates/foks-client/src/kv/write.rs#L655): a move is a source tombstone plus a destination dirent; a new name gets a fresh dirent ID (line 1238)
- [`apps/desktop/src/bridge/vault-catalog.ts:363`](../../../apps/desktop/src/bridge/vault-catalog.ts#L363): mutation response carries only `applied`, not the destination dirent ID or version
- [`apps/desktop/kit/toasts.tsx:16`](../../../apps/desktop/kit/toasts.tsx#L16): toasts support an action button usable for Undo
- [`crates/foks-server-db/src/schema/kv.sql:60`](../../../crates/foks-server-db/src/schema/kv.sql#L60): kv_dirents keyed by version; every version is retained

**Recommendation**

Make Delete mean 'move to /.trash' within the same store. (1) Trash location: dirent names are sealed under the destination directory's keyset, so use one trash folder per read role, /.trash/<role-tag>/ (owner, admin, member-<visibility>). Create it with mkdir_p, that read role, and a write role low enough for everyone who can delete items of that role. (2) Entry name: <UTC yyyymmddThhmmss>~<percent-encoded original path>. If the name would exceed 255 bytes, fall back to a hashed name plus a sidecar small file. (3) Undo: a moved entry gets a new dirent ID, and move_item currently returns only {applied}. Either extend KvWriteReport, the MoveKv response and MutationResponse to return the destination dirent ID and version, or have Undo reload the catalog and select the trashed entry by path before issuing MoveKv back. The host's selected_item requires the entry to be in the catalog. (4) Trash view: a per-store Trash row in the tree, hidden from normal folders and search. Restore handles an occupied path with 'Restore as copy' or Cancel. 'Delete permanently' and 'Empty Trash' use RemoveKv (recursive for folders) only inside /.trash. (5) Enable 'Move folder to Trash', choosing the trash by the folder's own directory read role (children's names stay sealed under the folder's own key). (6) Reword the permanent-delete copy to 'Removes it for everyone. Earlier encrypted versions may remain on the server.' Go clients see .trash as an ordinary folder; document this in book/20-desktop.qmd. Order of work: single-item trash with Undo, then folders, then optional auto-empty.

**Already tracked:** book/20-desktop.qmd:337 states 'Folder deletion is not supported in the desktop.' This finding adds a design (folder move into role-scoped trash) that removes the reason for that restriction.

**Mockup:** [Trash, undo and bulk organising](../mockups/vault-trash-and-undo.html)

<details><summary>Verifier note</summary>

The core claims hold. The delete sheet says 'This cannot be undone.' (write-workflows.tsx:1489). The folder menu shows 'Delete folder' permanently disabled (items-screen.tsx:1466-1472). remove_item_operation rejects directories (vault.rs:707-714). RemoveKv has a recursive flag and MoveKv exists (message.rs:956-970). The toast kit supports an action button (toasts.tsx:16-24). The server never deletes from kv_dirents or kv_nodes. No trash, undo or restore feature exists anywhere in the desktop. book/20-desktop.qmd:337 records only the folder restriction. dirent names are sealed under the destination directory's seed, so the role-scoped trash rationale is correct, and the 255-byte component limit is confirmed (support.rs:75-87). The Undo mechanics need correcting. A move is not 'one dirent rewrite': it is a source tombstone plus a destination dirent in one namespace mutation (crates/foks-client/src/kv/write.rs:655-735). A new destination name gets a fresh random dirent ID (write.rs:1228-1250). KvWriteReport returns only path and version, and the desktop's move_item response is only {applied} (vault-catalog.ts:363-366). The host's selected_item also requires the entry to be in the current catalog (context.rs:1208). As written, 'MoveKv back using the dirent id and version returned by the move' cannot work without extending the result or reloading the catalog.

</details>

### fe-product-exposure-review

**Exposure review and rotation checklist after removing a member or device**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Removing a member rekeys the team, and the sheet notes that the person 'may retain a local copy'. Removing a device tells the user to 'change any secrets it could read'. Neither flow says which secrets those are or helps rotate them. A rekey protects future KV reads, but credentials stored in the vault (API tokens, Wi-Fi and streaming passwords) stay valid in the outside world until rotated. The app already computes which items a party can read (`readsOf`, shown as 'reads N of M' in the Remove menu), so the data for a checklist is available on the client.

**Evidence**

- [`apps/desktop/src/screens/groups-screen.tsx:118`](../../../apps/desktop/src/screens/groups-screen.tsx#L118): readsOf(snapshot, party) lists the items a member can read
- [`apps/desktop/src/screens/groups-screen.tsx:977`](../../../apps/desktop/src/screens/groups-screen.tsx#L977): the Remove menu shows only the count 'reads N of M'
- [`apps/desktop/src/screens/groups/remove-member-sheet.tsx:69`](../../../apps/desktop/src/screens/groups/remove-member-sheet.tsx#L69): generic warning, no item list
- [`apps/desktop/src/screens/device-sheets.tsx:1568`](../../../apps/desktop/src/screens/device-sheets.tsx#L1568): 'change any secrets it could read' with no list or follow-up

**Recommendation**

Before the write, wait for catalog and roster readiness. Then snapshot `readsOf(snapshot, target)` as `[{store, direntId, versionAtRemoval}]`. For a device, take every item in every store the account can read, since the device held the account's user key. Show a summary in the sheet, Passwords first, labelled as the items you can see that they could read. After a successful removal, store a review record locally keyed by team and removed party. Persist only opaque ids and versions, never paths or names, and resolve names from the live snapshot when drawing. Show a band on the team page. The review page marks an item 'Changed since removal' once its version rises above versionAtRemoval, and the user confirms it as rotated or marks it 'Not needed'. 'Edit & rotate' opens the login editor with the generator (fe-product-password-generator). Apply the same flow to LowerRoleSheet (demote), which narrows future reads but not past knowledge. No agent or protocol change.

<details><summary>Verifier note</summary>

Core claims hold. readsOf is at groups-screen.tsx:118. The Remove menu shows only 'reads N of M' (976-977). RemoveMemberSheet's text is generic ('may retain a local copy', remove-member-sheet.tsx:68-71), and the device sheet says 'change any secrets it could read' with no list (device-sheets.tsx:1567-1570). No checklist or review feature exists, and ISSUES.md does not track one. The recommendation has three soundness problems. (1) Marking an item 'Rotated' when its version exceeds versionAtRemoval is a heuristic. Any rewrite bumps the version (an unrelated edit, or replacing the same value), so the state should read 'Changed since removal' and need confirmation, not assert rotation. (2) readsOf runs over the remover's catalog, so items the remover cannot read are missing, and readersOf returning null for an unloaded roster silently drops items. The list is a lower bound and the sheet must say so. It must also wait for itemsReadiness and roster readiness, which matters most for the device variant across all stores. (3) Storing item paths in localStorage would write decrypted path metadata to disk outside the agent. Storing store ref, dirent id and versionAtRemoval, and resolving names from the live snapshot, gives the same function. No agent or protocol change is needed, which is consistent with the constraints.

</details>

### fe-product-import-export

**Starter card advertises a nonexistent `foks kv export`; add host-side CSV import and export**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: desktop-native, desktop-ui, cli
- Verification: adjusted

The empty-vault starter card advertises `foks kv export …`, which exists in neither the Rust CLI nor Go FOKS v0.1.9. The real 'Import from FOKS CLI' action pairs or copies a Go account profile and does not import items. Users moving from 1Password, Bitwarden or KeePass must retype every entry. No vault-item export exists either; the existing portability export is an encrypted client-state archive.

**Evidence**

- [`apps/desktop/src/screens/items-screen.tsx:1218`](../../../apps/desktop/src/screens/items-screen.tsx#L1218): 'Import from the CLI' card shows `foks kv export …`; the comment at 1212 wrongly says the CLI has this command
- [`crates/foks-cli/src/main.rs:172`](../../../crates/foks-cli/src/main.rs#L172): KvCommand: List, Get, Put, Mkdir, Remove only; Go FOKS v0.1.9 `foks kv` also has no export subcommand
- [`apps/desktop/src/screens/go-profile-connect.tsx:234`](../../../apps/desktop/src/screens/go-profile-connect.tsx#L234): the real 'Import from FOKS CLI' pairs or copies a Go account profile; that account's server-side items then appear
- [`apps/desktop/src-tauri/src/commands/context.rs:1878`](../../../apps/desktop/src-tauri/src/commands/context.rs#L1878): record_picked_path returns the absolute path to the renderer as an authorization key; it is not a host-only handle
- [`apps/desktop/src-tauri/src/commands/portability.rs:230`](../../../apps/desktop/src-tauri/src/commands/portability.rs#L230): the existing export is an encrypted client-state archive, not a vault-item export

**Recommendation**

Immediate (S): replace the card's command text. Either point it to Settings › Account › Import from FOKS CLI ('Connect an existing FOKS account; its items come with it') or to the new password import, and fix the comment at items-screen.tsx:1212. Import (desktop-native): begin_password_import(format) opens the native picker in the host and keeps the path in host state. Unlike pick_import_file, it returns only an opaque preview id plus non-secret counts, names and folders. It parses 1Password, Bitwarden or KeePass CSV (and Bitwarden JSON) into zeroizing records. commit_password_import(previewId, storeId, folder, duplicatePolicy, roles) writes `user:`/`password:`/`url:` small files under /logins/<site> through the existing agent mutation path, one journaled create at a time, and reports progress over a Channel. Values over the 2,040-byte small-file limit become file nodes, and attachments are skipped with reasons. Export: export_store(storeId, format) writes through the native save dialog after a typed confirmation and a plaintext warning. Prefer an encrypted archive as the default once one exists. Optionally add `foks kv export/import` to the Rust CLI as a Rust-only extension. The renderer only ever sees counts and names.

<details><summary>Verifier note</summary>

The core claim holds. items-screen.tsx:1218-1220 shows 'Import from the CLI' and `foks kv export …`. Its comment at 1212 claims to state 'the one the CLI already has', which is false. The Rust CLI KvCommand (main.rs:172-216) has only List, Get, Put, Mkdir and Remove. Go FOKS v0.1.9 kv has readlink, mv, get-usage, rm, symlink, get, put, ls, mkdir and rest, and no export (checked against the v0.1.9 module source). No CSV, 1Password, Bitwarden or KeePass import and no vault-item export exists. The only export, portability.rs export_archive with foks-client-app/src/portability/export.rs, is an encrypted client-state archive (profiles and credentials), not decrypted items. The 'Import from FOKS CLI' sheet (go-profile-connect.tsx:234) pairs or copies a Go profile. Not tracked in ISSUES.md or the book. One correction: pick_import_file does not keep the path out of the renderer. record_picked_path (context.rs:1878) returns the absolute UTF-8 path to the renderer as the authorization key, so 'the path never crosses IPC, as in pick_import_file' is wrong. The import should return an opaque preview id instead and not copy that pattern. Also, KV items live on the server, so pairing an existing Go profile already brings an existing FOKS install's items. The immediate fix should point the card there. Password items are small files with a `password:` line or a path under /logins/ (model/kinds.ts). The 2,040-byte small-file limit means long notes must become file nodes.

</details>

### fe-product-keyboard-a11y

**Files keyboard model and ARIA semantics, plus a discoverable shortcut sheet**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui, desktop-native
- Verification: adjusted

Each Files row is its own `role="button"` tab stop with `aria-pressed`. Arrow keys do nothing, so a keyboard user must Tab through every row. In flat mode (search or All items) the list is virtualised, and a focused row scrolled out of the window is unmounted and loses focus. The folder tree is buttons, with aria-expanded on the twist and aria-current on the active node, but has no `tree`/`treeitem` roles, levels or arrow-key navigation. Screen readers get selection only as toggle-button 'pressed' state and never get a row's position. Shortcuts (⌘K, ⌘N/⇧⌘N, Ctrl+Tab, ⌘[ / ⌘] and Alt+←/→, Esc, ⌘R on Servers, ⌘, on macOS only) are bound in at least seven separate modules with no list. The starter-card hints already disagree with platform (⌘N shown on Linux). There is no shortcut for Lock, Copy, Delete or Rename.

**Evidence**

- [`apps/desktop/src/screens/items-screen.tsx:260`](../../../apps/desktop/src/screens/items-screen.tsx#L260): each row is role=button, tabIndex=0, aria-pressed; handlers cover Enter, Space, ContextMenu and Shift+F10 only
- [`apps/desktop/src/screens/items-screen.tsx:440`](../../../apps/desktop/src/screens/items-screen.tsx#L440): TreeRow renders divs and buttons without tree semantics (aria-expanded on twist, aria-current on select only)
- [`apps/desktop/src/screens/items-screen.tsx:1005`](../../../apps/desktop/src/screens/items-screen.tsx#L1005): virtualListWindow applies only in flat mode; folder views render all children
- [`apps/desktop/src/shell/search-palette.tsx:360`](../../../apps/desktop/src/shell/search-palette.tsx#L360): ⌘K handler
- [`apps/desktop/src/app/vault-shell.tsx:498`](../../../apps/desktop/src/app/vault-shell.tsx#L498): ⌘N and ⇧⌘N handler
- [`apps/desktop/src/shell/sidebar.tsx:723`](../../../apps/desktop/src/shell/sidebar.tsx#L723): Ctrl+Tab section cycling
- [`apps/desktop/src/shell/back-inputs.ts:9`](../../../apps/desktop/src/shell/back-inputs.ts#L9): ⌘[ / ⌘] and Alt+←/→ history shortcuts
- [`apps/desktop/src/app/navigation-runtime.ts:86`](../../../apps/desktop/src/app/navigation-runtime.ts#L86): Escape clears search, then selection
- [`apps/desktop/src/screens/servers-screen.tsx:280`](../../../apps/desktop/src/screens/servers-screen.tsx#L280): ⌘R checks the selected server
- [`apps/desktop/src/screens/items-screen.tsx:1198`](../../../apps/desktop/src/screens/items-screen.tsx#L1198): starter cards hardcode ⌘N / ⇧⌘N regardless of platform
- [`apps/desktop/src-tauri/src/lib.rs:78`](../../../apps/desktop/src-tauri/src/lib.rs#L78): macOS-only menu accelerators (⌘, ⌘N ⇧⌘N); no Lock entry

**Recommendation**

(1) Make the list a single tab stop: `role="grid"` (or listbox for the grid view) on the scroll container, with `aria-activedescendant`. Arrow keys and Home/End move the active row; Enter opens details; Space reveals; ⌘C copies through copy_item_value, or copy_item_field for login passwords; Delete or Backspace starts the delete or trash workflow; F2 opens Rename. Add a pinned-index parameter to the pure virtualListWindow so the active row is never unmounted in flat mode. Set `aria-rowcount` and `aria-rowindex`, and replace aria-pressed with aria-selected. (2) Make the folder pane `role="tree"` with `treeitem`, `aria-level` and `aria-expanded`, and Left/Right to collapse and expand. (3) Add a `shortcuts.ts` registry (id, keys per platform, label, scope). Make every existing listener, the starter-card hints, the palette footer, the macOS menu labels and a new shortcut sheet (opened with `?` or ⌘/) read from it, which also fixes the hardcoded ⌘N on Linux. (4) Add CmdOrCtrl+L Lock as a renderer handler that calls the existing lockFromMenu path (vault-shell.tsx:460), and add it as a macOS menu item that emits to the renderer.

<details><summary>Verifier note</summary>

Core claims hold. Rows are role=button, tabIndex=0 and aria-pressed (items-screen.tsx:260-284), and their only key handling is Enter, Space, ContextMenu and Shift+F10. TreeRow (440) has no tree or treeitem roles, levels or arrow keys. A grep found no arrow-key handling on the Files list or tree, and no shortcut registry or shortcut sheet. Lock has no accelerator. The summary has four errors. (1) Selection is exposed: as aria-pressed toggle-button state on rows and aria-current=location on the active tree node, and twist buttons carry aria-expanded. What is missing is listbox/grid selection semantics and position (row N of M). (2) Virtualisation applies only in flat mode, meaning search or All items (items-screen.tsx:987-1010). Folder views mount every direct child, so focus loss on scroll affects only flat mode. (3) Shortcuts are spread across at least seven modules, not four. The finding missed back-inputs.ts (⌘[ / ⌘], Alt+←/→), navigation-runtime.ts:86 (Esc), servers-screen.tsx:280 (⌘R). (4) Concrete drift already exists: the starter cards hardcode ⌘N and ⇧⌘N on every platform (items-screen.tsx:1198, 1210), while SearchField picks the label with isMac(). Cited lines are slightly off: the ⌘K handler is at 360 and the macOS menu at 78-91. The native menu is installed only on macOS, so Lock needs a renderer handler as well.

</details>

### fe-product-organize-bulk

**Organising: folder-picker move, drag-to-folder, and multi-select bulk actions**

- Type: feature-refinement
- Priority: medium
- Effort: L
- Layers: desktop-ui
- Verification: adjusted

Moving an item means typing a full absolute destination path into a free-text field, and the destination folder must already exist. There is no folder picker and no 'New folder' in the move flow. Rows cannot be dragged onto tree folders. Selection is a single `{store,path}` and every write workflow takes one item, so tidying an imported or legacy vault means repeating Move or Delete per item.

**Evidence**

- [`apps/desktop/src/screens/move-item-sheet.tsx:113`](../../../apps/desktop/src/screens/move-item-sheet.tsx#L113): 'Enter the full destination path… The destination folder must already exist.'
- [`apps/desktop/src/navigation/types.ts:136`](../../../apps/desktop/src/navigation/types.ts#L136): Selection is one item or null
- [`apps/desktop/src/screens/write-workflows.tsx:128`](../../../apps/desktop/src/screens/write-workflows.tsx#L128): WriteWorkflow move and delete carry a single Item
- [`apps/desktop/src/screens/scope.ts:67`](../../../apps/desktop/src/screens/scope.ts#L67): folderTree already builds the per-store tree a picker could reuse
- [`crates/foks-desktop/src/lib.rs:1528`](../../../crates/foks-desktop/src/lib.rs#L1528): create_kv_directory_operation already sets mkdir_p: true
- [`apps/desktop/src/screens/items-screen.tsx:490`](../../../apps/desktop/src/screens/items-screen.tsx#L490): tree rows carry data-folder-store / data-folder-path, usable as drop targets

**Recommendation**

(1) Replace the path field with a FolderPicker built from `folderTree` for the item's store. It has a tree, a 'New folder' inline action that calls the existing createFolder (already mkdir -p), and a Name field defaulting to the current name, with a path preview. Keep the free-text path behind a 'Path' disclosure, as NewSheet does. (2) Add row-to-tree-folder drag within the renderer using pointer events (pointerdown, pointermove, pointerup with elementFromPoint on [data-folder-store]) rather than HTML5 dataTransfer. The window's native dragDropEnabled handler stays reserved for OS file drops through record_drop_paths. (3) Add `MultiSelection` (ordered keys plus an anchor) with ⌘-click and shift-click, and a selection toolbar. Add `move-many` and `delete-many` (later `trash-many`) workflows that run MoveKv or RemoveKv sequentially; the host already takes a per-profile mutation permit. Report per-item outcomes, reusing the ambiguous/uncertain handling, and offer 'Retry failed'. Do not batch dirents into one kvPut until a measured need appears.

**Mockup:** [Trash, undo and bulk organising](../mockups/vault-trash-and-undo.html)

<details><summary>Verifier note</summary>

Core claims hold. move-item-sheet.tsx:112-114 says 'Move … within this vault. Enter the full destination path… The destination folder must already exist.' and offers only a free-text Field. Selection is `{store,path} | null` (navigation/types.ts:136). WriteWorkflow move and delete each carry one Item (write-workflows.tsx:128-129). Greps for draggable, onDragStart and dataTransfer found no in-app drag between rows and folders. Nothing in ISSUES.md or the book tracks this. The feasibility claims check out. createFolder already uses mkdir_p: true (crates/foks-desktop/src/lib.rs:1528-1539). NewSheet has a Path disclosure (write-workflows.tsx:773). Tree and folder rows already carry data-folder-store and data-folder-path, usable as drop targets. Evidence fix: folderTree is at scope.ts:67, not line 1. Recommendation fix: tauri.conf.json sets dragDropEnabled: true for the OS file drop. That native handler is known to suppress HTML5 dataTransfer drag inside the webview on Windows, so row-to-folder drag is more robust with pointer events than with HTML5 DnD.

</details>

### fe-product-password-generator

**Password generator, reveal toggle and strength feedback in New and Edit**

- Type: missing-feature
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

The New password sheet's Password field is a plain masked input with no reveal toggle, no generator and no strength feedback. Users of a password manager expect to create a strong credential at the moment they save it. The Edit flow has a reveal toggle but nothing else. The same generator is needed for the rotation flows in fe-product-exposure-review.

**Evidence**

- [`apps/desktop/src/screens/write-workflows.tsx:830`](../../../apps/desktop/src/screens/write-workflows.tsx#L830): Password is a type=password Field with no action button; field() never passes Field's action prop (line 719)
- [`apps/desktop/src/screens/write-workflows.tsx:645`](../../../apps/desktop/src/screens/write-workflows.tsx#L645): value is composed directly from the typed fields
- [`apps/desktop/src/screens/details-panel.tsx:846`](../../../apps/desktop/src/screens/details-panel.tsx#L846): the inline login edit form (shown only when isLogin) has a show/hide action but no generate action
- [`apps/desktop/src/components/field.tsx:39`](../../../apps/desktop/src/components/field.tsx#L39): Field already accepts an action slot that a PasswordInput can build on

**Recommendation**

Add a `PasswordInput` component (apps/desktop/src/components) with Reveal and Generate actions, used by NewSheet and the login edit form. Generate in the renderer with `crypto.getRandomValues` and rejection sampling (no modulo bias). This is acceptable because the renderer already holds the typed value in this sheet. Policies: random characters (length 8–64, default 24, character classes, avoid ambiguous characters) and passphrase (an embedded EFF short wordlist, 4–8 words, separator). Remember the last policy per device in localStorage. Strength: compute entropy exactly for generated values; for typed values use a bundled offline estimator such as zxcvbn-ts, loaded lazily, and show warnings only, never a block. Defer reuse detection, which would need agent-side comparison of plaintexts.

**Mockup:** [Login items: field copy, generator, quick copy](../mockups/vault-login-copy-and-generator.html)

<details><summary>Verifier note</summary>

The core claim holds. The New sheet's Password field is a plain field(..., 'password') with no action (write-workflows.tsx:830-837). field() never passes Field's existing action prop (write-workflows.tsx:719-735). The renderer has no generator, getRandomValues use, strength meter or passphrase support. The edit form's show/hide toggle is at details-panel.tsx:846-870, not line 478, which is inside a selection-reset useEffect. That inline login edit form appears only for isLogin items (paths under /logins/). Generating in the renderer is consistent with the book: the renderer already holds the typed value in this sheet.

</details>

### fe-product-search-quick-actions

**⌘K palette: quick copy actions, recents and favourites, folders and commands**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

The palette can only navigate. Choosing an item opens its store and selects it, so search-then-copy needs a second interaction in the details panel. An empty query lists the first six entries of each group in index order instead of recent or pinned items. Folders are excluded from the index, and only names and paths are searched. No favourites or recents exist anywhere in the app. The palette has no commands group, although New has shortcuts (⌘N/⇧⌘N) and Lock exists as a rail-menu and Settings action (with no shortcut).

**Evidence**

- [`apps/desktop/src/shell/search-palette.tsx:150`](../../../apps/desktop/src/shell/search-palette.tsx#L150): folders are skipped when building the item index
- [`apps/desktop/src/shell/search-palette.tsx:163`](../../../apps/desktop/src/shell/search-palette.tsx#L163): terms are only the name and the path
- [`apps/desktop/src/shell/search-palette.tsx:331`](../../../apps/desktop/src/shell/search-palette.tsx#L331): an empty needle ranks everything 0, so results appear in index order
- [`apps/desktop/src/shell/search-palette.tsx:511`](../../../apps/desktop/src/shell/search-palette.tsx#L511): open() only navigates or selects; no other action exists
- [`apps/desktop/src-tauri/src/commands/vault.rs:1183`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1183): copy_item_value already copies a text item's value through clipboard hygiene without returning it to the renderer
- [`apps/desktop/src/shell/sidebar.tsx:514`](../../../apps/desktop/src/shell/sidebar.tsx#L514): Lock exists only as a rail-menu action; no keyboard shortcut is bound anywhere

**Recommendation**

Extend SearchEntry with `actions` and handle modifier keys in the palette's keydown handler. Enter opens. ⌘C copies through the existing `copy_item_value` for text items now, and through `copy_item_field` (fe-product-login-fields-copy) for a login's password once that command exists. Files get no copy action. Intercept ⌘C only when the query input's selection is collapsed, so copying query text still works. ⇧⌘C copies the username once field copy exists, and ⌘O opens the URL. Add a per-device `recents` list (last 10 opened or copied items) and `favorites` (star toggle in the details header and ⌘D in the palette). Persist only opaque identifiers (store ref and dirent id) and resolve names from the live snapshot at render time, dropping ids that no longer resolve. Do not write item names or paths to localStorage, because that would put decrypted metadata on disk beyond the lock. Wrap storage access in try/catch and clear it on sign-out. An empty query shows Recent, Favorites, then Actions (New password, Add document, Lock through the existing lockFromMenu path, Settings, Add a device). Index folders as results that navigate with locations.setFolder. Parse `in:<vault>` and `kind:password|document` tokens in the pure `searchResults` so they can be unit-tested. State in the palette footer that values are not searched.

**Mockup:** [Login items: field copy, generator, quick copy](../mockups/vault-login-copy-and-generator.html)

<details><summary>Verifier note</summary>

Core claims hold. search-palette.tsx:150 skips folders, item terms are only [name, path] (line 163, not 160), an empty needle ranks every entry 0 so results come back in index order (line 331, not 327), and open() only navigates or selects (506-533). A grep for favourite, recent, starred and pinned found no such feature, and ISSUES.md and the book do not track one. One summary claim is wrong: Lock has no keyboard shortcut. It exists only as a rail-menu item (sidebar.tsx:514-523) and in Settings (settings-screen.tsx:432-449). The native menu (lib.rs:78-91) has no Lock entry, and no renderer handler binds a Lock key. The recommendation also ignores an existing host command: copy_item_value (vault.rs:1183) already does clipboard-hygienic copy for text items, so palette copy for non-login items needs no new command. Copy is offered only for non-File items (items-screen.tsx:316). Two feasibility details are missing. ⌘C in the palette's text input conflicts with copying selected query text. Persisting item names in plaintext localStorage would write decrypted vault metadata to disk beyond the lock, which the current renderer storage does not do: it holds only UI preferences and invitation labels.

</details>

### fe-product-share-to-team

**Share or copy an item into a team vault**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: desktop-native, desktop-ui
- Verification: adjusted

An item cannot leave the vault it was created in: Move works only within the current vault, and no host command copies across stores. To share a personal credential with a team, the user copies the value and pastes it into a new team item, so the plaintext passes through the renderer's form. For a file, the user downloads it in plaintext to disk and re-uploads it. Either way the original must be deleted by hand. The new-item sheet already has an AccessBlock that previews who can read and change an item at a given role, which a share flow can reuse.

**Evidence**

- [`apps/desktop/src/screens/move-item-sheet.tsx:113`](../../../apps/desktop/src/screens/move-item-sheet.tsx#L113): 'Move … within this vault.'
- [`crates/foks-desktop/src/lib.rs:1542`](../../../crates/foks-desktop/src/lib.rs#L1542): move_kv_operation builds MoveKv bound to one store
- [`apps/desktop/src/screens/write-workflows.tsx:312`](../../../apps/desktop/src/screens/write-workflows.tsx#L312): AccessBlock computes readers and changers for a candidate role
- [`apps/desktop/src-tauri/src/commands/vault.rs:1183`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1183): copy_item_value shows the host already reads plaintext in core without returning it to the renderer
- [`apps/desktop/src-tauri/src/commands/vault.rs:1240`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1240): download_file is the only way to move a file's bytes today, via plaintext on disk

**Recommendation**

Add a host command `copy_item_to_store({source: storeId,path,version,direntId}, target: {storeId,path,readRole,writeRole}, removeSource: bool)` that resolves both profiles with for_store. Small items: read_text, then build the create_text_item mutation in the host. Large files: adapt apply_file_upload to take a reader fed by ReadKv chunks in zeroizing buffers instead of a source path. Plaintext never reaches the renderer or disk. Apply it as two separate mutations: create in the target with a Create precondition, then RemoveKv with ExactVersion and the expected dirent id on the source if removeSource is set. If the second fails, report 'Copied, original kept' rather than retrying. In the UI, add 'Share to team…' and 'Copy to vault…' to the details actions and the context menu, reusing CardSelect for the store, a FolderPicker (fe-product-organize-bulk) and AccessBlock for roles. Disable destinations where writeBlockReason is non-null. Label the destination server when it differs from the source.

<details><summary>Verifier note</summary>

Core claim holds. The move sheet says 'within this vault' (move-item-sheet.tsx:113). move_kv_operation (crates/foks-desktop/src/lib.rs:1542) builds MoveKv with one store, and no host command copies across stores (the vault.rs command list has none). Greps for share, copy to and another vault in the renderer found nothing, and ISSUES.md does not track it. AccessBlock starts at write-workflows.tsx:312; line 330 is inside it. The recommendation is feasible. The host already reads plaintext in the core process without returning it to the renderer (copy_item_value at 1183, download_file at 1240). File creation streams through apply_file_upload from a source, which can be adapted to take a reader. remove_kv_operation already uses ExactVersion with an expected dirent id. AppState::for_store resolves per profile, so a cross-profile command needs two resolutions. The summary overstates the current workaround. A text secret can be copied with the existing hygienic Copy action and pasted, with no reveal or retyping, but the paste still passes the plaintext through the renderer's new-item form. A file has to be downloaded to disk in plaintext and re-uploaded, which is the stronger argument for this feature. 'Journaled' is loose: the host has per-profile mutation permits and ambiguous-outcome handling, not a journal spanning two operations.

</details>

### fe-product-signed-activity

**Show signed device and team history from the verified sigchains**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: client-lib, agent, desktop-native, desktop-ui
- Verification: adjusted

Device rows show only a name, a kind and a 'Current' chip, and `DeviceSummary` carries only id, name, role and current. Users cannot tell when a device was added, by which device, or whether an unfamiliar key appeared recently, which is the main question a device list should answer. Every user and team sigchain link already carries a signed `time`, `sequence` and `signer`, and the agent verifies them on each read. A security activity timeline (device adds and revocations, recovery-phrase and hardware-key enrollments, key rotations, membership changes with approver) could therefore be shown with no server or protocol change.

**Evidence**

- [`apps/desktop/src/screens/devices-screen.tsx:775`](../../../apps/desktop/src/screens/devices-screen.tsx#L775): row shows only <b>{entry.name}</b> and kindLabel(entry.kind); the detail view (around 1100-1140) also has no added time or signer
- [`crates/foks-agent-proto/src/message.rs:165`](../../../crates/foks-agent-proto/src/message.rs#L165): DeviceSummary: id_hex, name, role, current
- [`crates/foks-client-app/src/account.rs:186`](../../../crates/foks-client-app/src/account.rs#L186): list_devices maps verified().devices() to the four summary fields only
- [`crates/foks-verify/src/user.rs:228`](../../../crates/foks-verify/src/user.rs#L228): VerifiedDevice keeps id, role, hepk, subkey; no provenance
- [`crates/foks-verify/src/user.rs:318`](../../../crates/foks-verify/src/user.rs#L318): device_history (and device_signing_bookends at 439) already replay retained authenticated evidence, so provenance can be derived without changing verification
- [`crates/foks-proto/src/identity/chain.rs:1389`](../../../crates/foks-proto/src/identity/chain.rs#L1389): UserGroupChange carries seqno, time, signer and the committed root (TreeRoot); UserEldest at 1455 likewise
- [`crates/foks-proto/src/host.rs:276`](../../../crates/foks-proto/src/host.rs#L276): MerkleRoot carries a server-signed time that bounds a link's signer-asserted time from below

**Recommendation**

(1) In foks-verify, add a read-only accessor over the retained authenticated evidence (the same loop device_history and device_signing_bookends use). It yields per-link {sequence, signer-asserted time, signer, committed root epoch, kind of change}. Do not change VerifiedDevice's role in verification. (2) Add optional added_sequence, added_time and added_by to DeviceSummary in both foks-client-app and foks-agent-proto, and bump PROTOCOL_VERSION. (3) Add an agent op ListChainEvents {profile, subject: account | team, before_sequence, limit} that returns summaries of verified links, resolving signer names through device_display_name (names of revoked devices are kept) and team signer owners through the verified team state. (4) Desktop: render a paginated timeline under Settings › Account › Activity and Team settings › Activity, and label times 'signed by <device>'. For the 'recently added' highlight, use the later of the signer time and the server-signed time of the link's committed Merkle root, or mark it as signer-asserted, so a backdated link cannot hide a new device. No server or wire change is needed.

<details><summary>Verifier note</summary>

The core claim holds. devices-screen.tsx:774-775 shows only the name and kindLabel; the detail view (lines 1100-1140) adds Account, Role, Kind, key id and Status, with no provenance. DeviceSummary (agent-proto message.rs:165) and the client-app DeviceSummary (account.rs:1175) carry only id_hex, name, role and current. The agent maps those four fields straight through (foks-agent/src/main.rs:3964-3979). VerifiedDevice (user.rs:228) has no sequence, time or signer. No ChainEvent, activity or history feature exists in the TS, the agent or the server. ISSUES.md and the book do not track it. Links really carry signed times: the Rust client sets time: now_milliseconds() on every builder. Two evidence and feasibility corrections. (a) chain.rs:19 is the derive on DecodedMembershipLink, the user-side team-membership generic link, not the device-add link. Device additions and removals are UserGroupChange (chain.rs:1389) and UserEldest (chain.rs:1455), which both carry seqno, time and signer. (b) VerifiedUserState already keeps the authenticated chain and evidence (authenticated_chain_bytes, evidence_bytes) and replays them after verification: authenticated_link at user.rs:268, device_history at 318, device_signing_bookends at 439. Device display names are kept for devices that were later revoked (authenticated_device_display_names at 1272). So provenance can be derived from the retained, already-verified evidence with no change to the verification replay. One more problem: a 'new this week' highlight based only on signer-asserted time can be dodged by a compromised device that backdates its link. Each link commits to a Merkle root epoch, and MerkleRoot (proto host.rs:276) has a server-signed time, which gives a lower bound.

</details>

### fe-product-version-history

**Item version history and restore from versions this device has seen (no protocol change)**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: client-lib, agent, desktop-native, desktop-ui
- Verification: confirmed

The details panel shows a bare 'Version 9' with no way to see or restore earlier values. An overwrite by a teammate, or by the user, is final from the app's point of view. The server keeps every dirent version and every node. `kvGetNode` returns any node by ID, authorised only by the caller's read-key role, not by whether a current dirent references it. The client's soft-state `kv_entry_history` keeps only the last accepted version as a rollback anchor. The catalog metadata also omits the dirent's server-set `creation_time`, so the app cannot show when an item changed.

**Evidence**

- [`apps/desktop/src/screens/details-panel.tsx:1227`](../../../apps/desktop/src/screens/details-panel.tsx#L1227): Version is displayed as a number only
- [`crates/foks-server-db/src/schema/kv.sql:60`](../../../crates/foks-server-db/src/schema/kv.sql#L60): kv_dirents primary key includes version; all versions are retained
- [`crates/foks-server/src/services/kv.rs:598`](../../../crates/foks-server/src/services/kv.rs#L598): load_node fetches by node id and checks only require_read_key
- [`book/14-kv-store.qmd:333`](../../../book/14-kv-store.qmd#L333): kv_entry_history stores only the last accepted version per entry
- [`crates/foks-agent-proto/src/message.rs:322`](../../../crates/foks-agent-proto/src/message.rs#L322): KvEntryMetadata has no creation_time
- [`crates/foks-proto/src/kv.rs:350`](../../../crates/foks-proto/src/kv.rs#L350): dirents carry a server-set creation_time

**Recommendation**

Step 1: add `server_time` (the dirent creation_time) to KvEntryMetadata and ItemDto. Show it as 'Changed' in the details panel and as an optional list column, labelled as server-reported because the binding MAC does not cover it. Step 2: in foks-client-db, turn the per-entry rollback anchor into a bounded ring (for example the last 20 versions per dirent with a total byte cap) of exact dirent bytes as accepted during sync. These are ciphertext only, consistent with the soft-cache rules. Step 3: add agent operations `ListKvVersions{store,path}` and `ReadKvVersion{store,path,dirent_id,version}`. The second re-verifies the stored dirent's binding MAC and name under the directory keyset, then fetches the node through the existing kvGetNode path (chunks for large files). Step 4: Restore runs the existing edit mutation with an ExactVersion precondition on the current version, writing the old bytes as a new version. Clearly state the limit: history covers only versions this device observed. A server-side 'list dirent versions' RPC would be a negotiated extension and should be deferred. Reveal of an old version goes through the same one-value-at-a-time host command as today.

<details><summary>Verifier note</summary>

Every cited item checks out. The details panel shows only item.version (details-panel.tsx:1227). kv_dirents is keyed by (uid, parent, dirent, version), and the server never deletes dirents or nodes. load_node (services/kv.rs:598) authorizes only with require_read_key and does not check that a current dirent references the node. The server sets creation_time to the write time for each new dirent version (services/kv.rs:547-553). The client computes binding_mac before the server sets creation_time, so the MAC cannot cover it. KvEntryMetadata (message.rs:322) lacks creation_time, even though the soft cache's kv_entries already stores it, so step 1 is cheap. kv_entry_history keeps one row per entry (soft_schema.rs:111; book/14-kv-store.qmd:333). No history or restore UI exists. The recommendation states that history covers only versions this device observed and defers any new RPC. One caveat: retrieving unreferenced old nodes through kvGetNode is verified only for the Rust server. Behaviour against a Go v0.1.9 host should be tested, and the brief's 'no longer available' error state covers a refusal.

</details>

### fe-product-attention-centre

**Single attention centre with a post-setup checklist and recent-activity history**

- Type: feature-refinement
- Priority: low
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Items that need the user are spread across rail badges and dots, the account-avatar dot, Teams banners, per-team UnfinishedActivity bands, the Devices 'No recovery key' band, lease notices in Files and the sync popover. Confirmations exist only as toasts that vanish. A missing recovery phrase is already flagged through the Devices dot. Nothing flags an account with only one device and no hardware key, and once first-run setup completes no checklist remains beyond the 'add-password' tip.

**Evidence**

- [`apps/desktop/src/shell/sidebar.tsx:704`](../../../apps/desktop/src/shell/sidebar.tsx#L704): rail tails (counts and dots) are the only cross-section signal
- [`apps/desktop/src/shell/sidebar.tsx:384`](../../../apps/desktop/src/shell/sidebar.tsx#L384): unrouted notice count is a dot on the account avatar that opens Settings › Account
- [`apps/desktop/src/screens/device-alert.ts:86`](../../../apps/desktop/src/screens/device-alert.ts#L86): a missing recovery phrase is already tracked and shown as a Devices dot; a second device or hardware key is not tracked
- [`apps/desktop/src/components/unfinished-activity.tsx:17`](../../../apps/desktop/src/components/unfinished-activity.tsx#L17): incomplete membership changes shown per team only
- [`apps/desktop/src/onboarding-tips.ts:3`](../../../apps/desktop/src/onboarding-tips.ts#L3): single post-setup tip ('add-password')
- [`apps/desktop/kit/toasts.tsx:12`](../../../apps/desktop/kit/toasts.tsx#L12): timed toasts (2.6 s default), no history

**Recommendation**

Add an attention.ts model AttentionItem {id, source, severity, title, detail, deepLink: Location, dismissible}, built from data the shell already holds: team request counts, deviceAlertRegistry (open offers, missing recovery phrase), unroutedNotices, settingsAlertSummary (lapsed check-in, unverified server), expired leases and pending operations. Add a topbar popover listing these. Add a 'Get set up' checklist from existing facts: recovery phrase (already tracked), second device or hardware key (from the preloaded device lists), app lock availability, first password, team joined. Omit 'CLI/MCP connected' unless the agent first exposes a non-secret 'clients seen' fact. Add a 'Recent' list: an in-memory ring of the last 20 toast messages, cleared on app lock, and never holding values. Keep the rail badges and make them open the relevant filtered view.

<details><summary>Verifier note</summary>

Mostly holds. Cross-section signals are rail tails (sidebar.tsx:697-718). unroutedNotices is passed as `attention` (vault-shell.tsx:612) and drawn as a dot on the account avatar (sidebar.tsx:384-393) and on the Settings tab. UnfinishedActivity is per team. The only onboarding tip is 'add-password' (onboarding-tips.ts:3). Toasts are timed and keep no history (kit/toasts.tsx). No bell, attention popover or notification centre exists. Not tracked in ISSUES.md or the book. One correction: the summary says nothing tracks whether a recovery phrase is set up, which is wrong. devicesAlertSummary (screens/device-alert.ts:86-103) checks recoveryPhrases per account and shows a Devices-rail dot ('An account has no recovery phrase') plus the Devices band. The first-run SetupCard (first-run-state.ts:811-833) also tracks recovery during setup. What is untracked is a second device or hardware key and any checklist after setup. Device lists are preloaded by the shell (app-bootstrap.ts, metadata-runtime.ts), so the 'single device' item can be derived. The 'CLI/MCP agent seen' item cannot: neither the snapshot nor agent-proto exposes client usage, so it needs an agent addition or should be dropped.

</details>

### fe-product-unused-vault-ipc

**Remove or use the unused `copy_item_path` and `create_link` IPC commands**

- Type: code-quality
- Priority: low
- Effort: S
- Layers: desktop-native, desktop-ui
- Verification: adjusted

`copy_item_path` and `create_link` are registered Tauri commands with bridge wrappers and mock implementations, but no screen calls them. `create_link` creates symlink nodes that the renderer filters out at decode, so any links it created would never be shown. The project treats renderer-reachable commands as attack surface (the three-core-permission design and the ACL test), so commands with no caller should go, or be wired into a visible feature.

**Evidence**

- [`apps/desktop/src-tauri/src/lib.rs:317`](../../../apps/desktop/src-tauri/src/lib.rs#L317): commands::vault::copy_item_path registered
- [`apps/desktop/src-tauri/src/lib.rs:321`](../../../apps/desktop/src-tauri/src/lib.rs#L321): commands::vault::create_link registered
- [`apps/desktop/src-tauri/src/commands/vault.rs:1311`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1311): create_link builds a KV symlink via foks_desktop::create_kv_symlink_operation
- [`apps/desktop/src/bridge/vault-catalog.ts:332`](../../../apps/desktop/src/bridge/vault-catalog.ts#L332): Link entries are filtered out of every catalog
- [`apps/desktop/src/bridge/commands-vault.ts:72`](../../../apps/desktop/src/bridge/commands-vault.ts#L72): copyItemPath (72) and createLink (88) wrappers; no UI callers, only tests/bridge.test.ts
- [`apps/desktop/src-tauri/src/commands/vault.rs:1223`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1223): copy_text also uses copy_with_hygiene, so it auto-clears too

**Recommendation**

Delete create_link from invoke_handler, contract.ts, commands-vault.ts, mock-bridge.ts and the createLink test at bridge.test.ts:2466. Drop the now test-only create_kv_symlink_operation and its src-tauri test use. Keep 'Link' only as an accepted wire kind in the catalog decoder, and narrow the post-filter item type so it cannot hold 'Link'. Delete copy_item_path and its stubs as well: the renderer already holds the path and copy_text exists. If a 'Copy path' menu item is wanted, call copy_text and keep the clipboard hygiene, since vault paths can reveal which services an account uses. Add a test asserting that every command in invoke_handler has a non-test bridge caller, next to the existing ACL test.

<details><summary>Verifier note</summary>

The core holds. copy_item_path and create_link are registered (lib.rs:317 and 321) and implemented (vault.rs:1206 and 1311), with bridge wrappers (commands-vault.ts:72 and 88, contract.ts:194 and 197) and mocks (mock-bridge.ts:605 and 629). No screen or component calls either one. The only callers are bridge.test.ts stubs and line 2466. The catalog decoder drops 'Link' entries (vault-catalog.ts:331-333), so a created link would never show. Tauri app commands are not ACL-gated (capabilities/default.json grants only three core permissions), so every registered command is reachable from the renderer. One correction: the copy_item_path advice is internally inconsistent. copy_text (vault.rs:1223) also goes through copy_with_hygiene (clipboard.rs:39), so routing 'Copy path' through copy_text does not avoid the auto-clear. Auto-clear is also defensible for vault paths: /logins/<site> shows which services a person has accounts with. The removal also touches test code the recommendation does not list: bridge.test.ts (copyItemPath stubs at 1499, 1675, 1740, 1833 and createLink at 2466), src-tauri/src/commands/tests/vault.rs:569, and foks_desktop::create_kv_symlink_operation (crates/foks-desktop/src/lib.rs:1502), which would then be used only by tests. NodeKind 'Link' is still needed as an accepted wire kind in the decoder (vault-catalog.ts:222); only the post-filter item type can drop it.

</details>
