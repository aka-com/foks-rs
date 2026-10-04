# Cross-cutting gaps (completeness critic)

Area key `gap`. 11 verified findings. Part of the [foks-rs review](../README.md).

## Current state

This completeness pass looked for gaps the eleven area reviews did not cover. It checked the crates and tools nobody indexed (foks-snowpack, foks-merkle-store, foks-keystore, foks-yubi, foks-compat-artifact, foks-go-interop, tools/chat-v2, packaging/) and cross-cutting concerns: time and clock trust, offline behaviour, account recovery, fork consistency, fault capture, text scaling, notifications, storage visibility, and validation rules duplicated between Rust and TypeScript. The small support crates and the systemd units are in reasonable shape, and I found nothing high-value in them. The two most serious problems are in the client stack. First, the MCP data-write clock pairs wall time with std::time::Instant, which stops while the machine sleeps. Any laptop sleep longer than five minutes therefore blocks every new MCP write until the agent restarts, and a gap of more than a day needs a repair command that exists only in the CLI. Second, SyncAccount and SyncTeam run the KV sync in Content mode. That mode downloads every file, including all large files, and writes the decrypted bytes into the soft-state SQLite file, yet production code reads them back only to compute sizes. The other findings are product and UX gaps where the protocol already holds what is needed, so none needs a wire change: (1) checking a recovery phrase locally against the enrolled backup key IDs, (2) showing and comparing the signed Merkle checkpoints, as book chapter 13 recommends, (3) an explicit read-only offline mode, (4) storage usage the agent already returns, (5) one shared security-key PIN field. Two renderer-quality items complete the list. Render errors outside the routed screen blank the whole window and leave nothing in Copy diagnostics. Text cannot be enlarged, and toasts disappear before warnings can be read.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [MCP write admission treats every laptop sleep over five minutes as an untrusted clock, and a day off needs a CLI-only repair](#gap-mcp-adapter-clock-suspend) | correctness-risk | high | M |
| [Recovery phrases are never checked after they are written down; add a local 'Test recovery phrase' and a recall step](#gap-recovery-phrase-test) | missing-feature | high | M |
| [Text cannot be enlarged: no zoom, no text-size setting, and pixel font sizes throughout](#gap-a11y-text-size) | feature-refinement | medium | M |
| [Let users compare signed checkpoints, the out-of-band root comparison book chapter 13 recommends](#gap-fork-consistency-compare) | missing-feature | medium | M |
| [No read access to vault items while the server is unreachable; add an explicit read-only offline mode with sealed key material](#gap-offline-read-only-vault) | missing-feature | medium | L |
| [A render error outside the routed screen blanks the whole window and leaves no trace in Copy diagnostics](#gap-renderer-fault-capture) | correctness-risk | medium | S |
| [Eleven hand-rolled security-key PIN fields disagree with the 6-8 character rule and never show remaining attempts](#gap-security-key-pin-field) | code-quality | medium | S |
| [SyncAccount and SyncTeam download every file and store its plaintext in soft state, but production code reads it back only for file sizes](#gap-sync-persists-unused-plaintext) | security | medium | S |
| [A wrong system clock surfaces as an opaque check-in or compatibility failure](#gap-clock-skew-diagnosis) | correctness-risk | low | S |
| [Show storage used per vault; the agent already returns usage, but the desktop learns of limits only from a quota refusal](#gap-storage-usage-display) | missing-feature | low | S |
| [Toasts vanish after 2.6 s with no pause, carry the only explanation for refused actions, and are announced unreliably](#gap-toast-timing-announcements) | feature-refinement | low | S |

### gap-mcp-adapter-clock-suspend

**MCP write admission treats every laptop sleep over five minutes as an untrusted clock, and a day off needs a CLI-only repair**

- Type: correctness-risk
- Priority: high
- Effort: M
- Layers: client-lib, agent, mcp, desktop-ui, cli, docs
- Verification: adjusted

SystemAdapterClock pairs SystemTime with std::time::Instant. Instant reads CLOCK_MONOTONIC on Linux and CLOCK_UPTIME_RAW on macOS, and neither advances while the machine sleeps. Within one process, validate() requires wall time to stay within 5 minutes of anchor_wall plus monotonic elapsed. So after any sleep longer than 5 minutes, the resident agent rejects every new data-write submission (MCP writes) and maintenance pruning with clock-untrusted. The failed check is never written back, so the offset persists until the agent process restarts. After a restart, a wall time more than 24 h past the stored floor (a weekend) is also refused. The only recovery is a two-step reanchor run by the CLI, which opens state directly. Book chapter 21 describes wall-clock jumps and restart gaps as intended, but nothing models sleep, and the adapter tests never simulate it.

**Evidence**

- [`crates/foks-client-app/src/adapter_clock.rs:17`](../../../crates/foks-client-app/src/adapter_clock.rs#L17): Process anchor is std::time::Instant. monotonic_seconds = started.elapsed() (line 33) excludes suspend (CLOCK_MONOTONIC on Linux, CLOCK_UPTIME_RAW on Darwin).
- [`crates/foks-client-db/src/repositories/adapter/clock.rs:78`](../../../crates/foks-client-db/src/repositories/adapter/clock.rs#L78): Same-process check against the fixed anchor_wall + elapsed with SKEW_SECONDS = 300 (line 8). On success only validated_time_floor advances (line 82); on error nothing is written.
- [`crates/foks-client-db/src/repositories/adapter/clock.rs:85`](../../../crates/foks-client-db/src/repositories/adapter/clock.rs#L85): New process: wall_seconds > validated_time_floor + RESTART_GAP_SECONDS (24 h, line 9) -> AdapterClockUntrusted
- [`crates/foks-client-db/src/repositories/adapter.rs:183`](../../../crates/foks-client-db/src/repositories/adapter.rs#L183): check_adapter_admission writes clock state only after validate succeeds (183-185); record_adapter_submission is the same (216-235)
- [`crates/foks-client-app/src/kv/data_write.rs:82`](../../../crates/foks-client-app/src/kv/data_write.rs#L82): prepare_data_write samples the adapter clock and calls check_adapter_admission on every MCP data write
- [`crates/foks-client-app/src/adapter_maintenance.rs:128`](../../../crates/foks-client-app/src/adapter_maintenance.rs#L128): Maintenance pruning hits the same error and only sets report.clock_untrusted
- [`crates/foks-client-app/src/adapter_clock.rs:95`](../../../crates/foks-client-app/src/adapter_clock.rs#L95): reanchor_adapter_clock is documented as deliberately CLI-only
- [`crates/foks-cli/src/retention.rs:44`](../../../crates/foks-cli/src/retention.rs#L44): Repair is the two-step reanchor through a direct ProfileSession; there is no agent operation or desktop surface
- [`crates/foks-server/src/web_admin/clock.rs:8`](../../../crates/foks-server/src/web_admin/clock.rs#L8): An existing SuspendClock already reads CLOCK_BOOTTIME (Linux) / CLOCK_MONOTONIC_RAW (macOS); reuse this pattern
- [`book/21-mcp.qmd:190`](../../../book/21-mcp.qmd#L190): Clock discipline documents a 5-minute jump and a 24-hour restart gap; sleep is not considered (also crates/foks-mcp/README.md:114)

**Recommendation**

1) Replace Instant in SystemAdapterClock with a suspend-inclusive monotonic source, following crates/foks-server/src/web_admin/clock.rs SuspendClock: clock_gettime(CLOCK_BOOTTIME) on Linux, CLOCK_MONOTONIC or CLOCK_MONOTONIC_RAW on macOS, and an Instant fallback elsewhere. Keep the process-id and anchor logic. 2) Add clock.rs tests with sleep-shaped samples: (wall+3600, boottime+3600) is accepted in the same process. (wall+3600, monotonic+0) and backward jumps are still rejected. 3) For the restart gap, record a boot identifier (Linux /proc/sys/kernel/random/boot_id; macOS kern.boottime) with the anchor. A restarted agent in the same boot can then validate elapsed time from the boot-time clock instead of applying the 24 h rule. Only a reboot plus a long gap should need repair. Do not rely on compatibility-artifact generated_at (up to 7 days old) or on submitted-link Merkle roots (MCP data writes submit none) as fresh host time. 4) Add agent operations AdapterClockStatus and RepairAdapterClock{preview|confirm} that wrap adapter_clock_preview/reanchor_adapter_clock. Have foks_status report the paused state. Show 'Assistant writes paused: confirm this computer's time' in the desktop's assistant settings. Keep the repair operation unreachable from the MCP tool surface, preserving the intent of the CLI-only comment at adapter_clock.rs:95, and route the CLI command through the agent so it no longer opens state directly. 5) Update book/21-mcp.qmd:190-197 and crates/foks-mcp/README.md:114-125.

**Already tracked:** book/21-mcp.qmd:190-197 documents the 5-minute jump and 24-hour restart rules; it does not say that ordinary sleep triggers them or that repair is CLI-only. Not in ISSUES.md.

<details><summary>Verifier note</summary>

The core claim holds. SystemAdapterClock anchors on std::time::Instant (adapter_clock.rs:17,22,33). Same-process validate() compares wall time against anchor_wall plus monotonic elapsed with a 300 s tolerance (clock.rs:69-80), and anchor_wall is never advanced on success: only validated_time_floor moves (line 82). After a suspend longer than five minutes, every later sample in that process fails. Errors are not written back: check_adapter_admission and record_adapter_submission write only after validate succeeds (adapter.rs:183-185, 216-235). The restart gap rule (clock.rs:85) refuses a wall time more than 24 h past the floor, which a weekend powered off produces. Repair is CLI-only (retention.rs:44-57), and adapter_clock.rs:95 says this is deliberate ('Only the local CLI exposes this operation'). The book (21-mcp.qmd:190-197), the MCP README (lines 114-125) and ISSUES.md do not mention sleep. Three points need correcting. (a) The repo already has a suspend-inclusive clock in crates/foks-server/src/web_admin/clock.rs:8-31: CLOCK_BOOTTIME on Linux and CLOCK_MONOTONIC_RAW on macOS, read through nix. That is the pattern to reuse, and foks-client-app already depends on libc. (b) The step 3 time sources are weak in practice. A compatibility artifact's generated_at may be up to 7 days old (foks-compat-artifact/src/lib.rs:72-73), and MCP data writes submit no sigchain links, so 'a root that includes a link this device just submitted' rarely exists. (c) Step 4 has to keep the deliberate CLI-only restriction in spirit: a repair operation must not be reachable from the MCP adapter's tool surface.

</details>

### gap-recovery-phrase-test

**Recovery phrases are never checked after they are written down; add a local 'Test recovery phrase' and a recall step**

- Type: missing-feature
- Priority: high
- Effort: M
- Layers: agent, desktop-ui, desktop-native, cli
- Verification: confirmed

Setup only asks the user to tick 'I have written down all 17 words'. HESP has no checksum, so an in-dictionary typo (cage for cave) or a wrong in-range number goes unnoticed until recovery, when the derived key fails to open the backup parcel. At that point every device is already lost, so the account cannot be recovered. Nothing checks the phrase later either: Devices offers Verify for passphrases but not for recovery phrases, and the CLI recovery commands are Revoke, Enroll, Recover and Resume. The check can run entirely on the device. BackupKey::from_phrase and public_material derive the backup key ID. The agent already returns that ID in BackupEnrollmentSummary.backup_id_hex. The first word and number are the backup key's public device name, so a near-miss can be localised.

**Evidence**

- [`apps/desktop/src/screens/first-run-recovery-step.tsx:106`](../../../apps/desktop/src/screens/first-run-recovery-step.tsx#L106): Only confirmation is a checkbox 'I have written down all 17 words' (also the dialog toggle at 236-248)
- [`book/06-hesp.qmd:61`](../../../book/06-hesp.qmd#L61): No checksum; a valid-word typo is discovered only at recovery time (lines 61-79)
- [`crates/foks-crypto/src/backup.rs:103`](../../../crates/foks-crypto/src/backup.rs#L103): from_phrase parses strictly; public_material (185) derives the enrolled key's public material; device_name (162) = first word + number
- [`crates/foks-agent-proto/src/message.rs:251`](../../../crates/foks-agent-proto/src/message.rs#L251): BackupEnrollmentSummary carries backup_id_hex the desktop already lists
- [`apps/desktop/src/screens/devices-screen.tsx:141`](../../../apps/desktop/src/screens/devices-screen.tsx#L141): Passphrase section has set/change/verify; recovery phrase rows offer only create and revoke
- [`crates/foks-cli/src/main.rs:296`](../../../crates/foks-cli/src/main.rs#L296): RecoveryCommand: Revoke, Enroll, Recover, Resume; no verify
- [`crates/foks-agent/src/main.rs:2258`](../../../crates/foks-agent/src/main.rs#L2258): recovery_phrase_sentence already produces per-token parse errors to reuse

**Recommendation**

Add an agent operation VerifyRecoveryPhrase{profile, account_alias, phrase: SecretString} that returns {matched_backup_id | null, matched_name, revoked}. It parses with BackupKey::from_owned_phrase, derives the entity ID from public_material, and compares it with the backup keys in the verified user chain rather than only the local list. It runs without network access and zeroizes the phrase. Parse failures reuse recovery_phrase_sentence. When the first word/number pair matches an enrolled name but the key does not, say that the error lies in tokens 3-17. In the desktop, add 'Test…' to each recovery-phrase row in Devices. It opens a 17-field entry sheet that alternates word and number fields, with BIP-39 prefix completion and per-token errors. Store the last-tested time in device-local metadata, and show a Devices alert dot when a phrase was never tested or was last tested more than 180 days ago. At creation, replace the checkbox with a recall step that asks for three random positions from the in-memory phrase before Done is enabled. Add `foks-rs recovery verify --phrase-file`.

**Already tracked:** Not tracked. book/06-hesp.qmd:61-79 names the late-detection problem but proposes no remedy.

**Mockup:** [Test your recovery phrase](../mockups/devices-recovery-phrase-test.html)

<details><summary>Verifier note</summary>

All cited facts hold. First-run shows only an 'I have written down all 17 words.' checkbox (first-run-recovery-step.tsx:106-113) and the dialog toggle (236-246). HESP has no checksum, and an in-dictionary typo is found only at recovery (book/06-hesp.qmd:61-79). BackupKey::from_phrase (backup.rs:103), device_name (162) and public_material (185) support a local derive-and-compare. BackupEnrollmentSummary carries backup_id_hex (message.rs:251-255). Devices offers Revoke for recovery-phrase rows (devices-screen.tsx:445-457). The set/change/verify passphrase group at 141 is for security-key cards, which the finding describes loosely but the contrast stands. RecoveryCommand has only Revoke, Enroll, Recover and Resume. recovery_phrase_sentence exists (agent main.rs:2258). A search of the desktop, agent, client-app and CLI found no verify or test feature for recovery phrases, and ISSUES.md does not track one. The recommendation is local-only, needs no wire change, and keeps the phrase inside the agent.

</details>

### gap-a11y-text-size

**Text cannot be enlarged: no zoom, no text-size setting, and pixel font sizes throughout**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui, desktop-native
- Verification: adjusted

Text cannot be enlarged. The window does not enable zoom hotkeys, the macOS menu is Menu::default with no zoom items, nothing calls set_zoom, and Appearance offers only Theme and Sidebar color. Of the 386 font-size declarations in the desktop CSS, 371 are px. The rest are inherit, 0 or var(--mark-type) (10px), and font shorthands also hard-code px. No rem is used anywhere. The body is 14px and most secondary text is 12-13px. Neither OS text-size preferences nor a root font-size change scales the UI, which fails WCAG 1.4.4 (Resize Text). There are no prefers-contrast or forced-colors rules. The only media queries are reduced-motion and four max-width breakpoints.

**Evidence**

- [`apps/desktop/src-tauri/tauri.conf.json:13`](../../../apps/desktop/src-tauri/tauri.conf.json#L13): Main window config: no zoomHotkeysEnabled; minWidth 860
- [`apps/desktop/src-tauri/capabilities/default.json:6`](../../../apps/desktop/src-tauri/capabilities/default.json#L6): Minimal renderer capability. The zoomHotkeysEnabled polyfill would require adding the webview set-zoom permission.
- [`apps/desktop/src-tauri/src/lib.rs:78`](../../../apps/desktop/src-tauri/src/lib.rs#L78): install_macos_menu uses Menu::default and adds only Settings and File items; no View > Zoom
- [`apps/desktop/src/screens/settings-screen.tsx:287`](../../../apps/desktop/src/screens/settings-screen.tsx#L287): Appearance offers Theme (light/dark/system) and Sidebar color only
- [`apps/desktop/src/styles/shell.css:6`](../../../apps/desktop/src/styles/shell.css#L6): body font: 14px/1.45. 197 of 206 font-size declarations in this file are px.
- [`apps/desktop/kit/tokens.css:96`](../../../apps/desktop/kit/tokens.css#L96): --mark-type: 10px. Even the tokenised sizes are px.
- [`apps/desktop/src/styles/shell.css:891`](../../../apps/desktop/src/styles/shell.css#L891): Fourth max-width breakpoint (also shell.css:19, app.css:1614, chat.css:1240); no prefers-contrast rules

**Recommendation**

Short term: add a per-device 'Text size' setting (90-200%) applied natively with WebviewWindow::set_zoom when the main window is created and when the setting changes. Add View menu items Zoom In, Zoom Out and Actual Size (Cmd +, -, 0) whose handlers call set_zoom natively. Do not set zoomHotkeysEnabled: its macOS/Linux polyfill requires granting the renderer the webview set-zoom permission, which widens the minimal capabilities/default.json. Zoom narrows the effective CSS viewport (an 860px window at 150% is about 573 CSS px), so add render or screenshot checks at that size and fix what breaks (rail auto-collapse, details panel as overlay). Longer term: define font-size tokens in rem in kit/tokens.css, including --mark-type. Migrate font-size declarations and font shorthands to them, and add a stylelint unit allow-list for font-size and font to stop new px sizes. Add @media (prefers-contrast: more) overrides for borders, muted ink and focus rings.

**Already tracked:** Not tracked. fe-product-keyboard-a11y, chat-ux-keyboard-a11y and fe-arch-files-list-a11y cover keyboard and ARIA semantics only.

<details><summary>Verifier note</summary>

Core claim holds. tauri.conf.json sets no zoomHotkeysEnabled (Tauri 2.11.5 defaults it to false). Menu::default on macOS has no zoom items, and install_macos_menu adds only Settings and the File items. Appearance offers only Theme and Sidebar color (settings-screen.tsx:287-334). There are no rem units in src/ or kit/ CSS and no prefers-contrast or forced-colors rules. The body is 14px/1.45 (shell.css:6). Nothing sets the webview zoom. Corrections. Of the 386 font-size declarations, 371 use px; the other 15 are inherit, 0 or var(--mark-type), which is itself 10px in kit/tokens.css. shell.css has 206 font-size declarations, of which 197 are px, not 206. There are four max-width breakpoints, not three: shell.css:19, shell.css:891, app.css:1614 and chat.css:1240. Some font shorthands (font: 12px ...) also hard-code px and are not counted. Recommendation problem: in Tauri 2.11.5, zoomHotkeysEnabled on macOS/Linux injects a renderer polyfill that requires the webview set-zoom permission. That widens capabilities/default.json, which is deliberately minimal (event listen/unlisten and start-dragging only). Native View menu accelerators that call WebviewWindow::set_zoom give the same result without granting the renderer anything.

</details>

### gap-fork-consistency-compare

**Let users compare signed checkpoints, the out-of-band root comparison book chapter 13 recommends**

- Type: missing-feature
- Priority: medium
- Effort: M
- Layers: agent, client-lib, client-db, desktop-ui, cli, docs
- Verification: adjusted

Book chapter 13 says clients never compare Merkle roots with each other or with an auditor, so a host could show different users permanently separate histories without detection. It recommends that users compare root hashes for an epoch out of band, noting that the objects needed are already signed and stored. Hard state does store the root hash and its signed evidence, but the agent's StoredHostStatus exposes only the host ID, chain length and epoch. The Servers page shows 'Checkpoint {epoch}' with no hash, so two users have nothing to compare.

**Evidence**

- [`book/13-client-verification.qmd:177`](../../../book/13-client-verification.qmd#L177): No root comparison between clients; 'The missing piece is a route… compare root hashes for an epoch out of band' (lines 177-207)
- [`crates/foks-client-db/src/schema.rs:156`](../../../crates/foks-client-db/src/schema.rs#L156): merkle_heads stores root_hash, evidence_kind and evidence_bytes per host
- [`crates/foks-client-db/src/lib.rs:1932`](../../../crates/foks-client-db/src/lib.rs#L1932): encode_evidence: the signed root is embedded in evidence_bytes; how it is encoded depends on the evidence kind
- [`crates/foks-agent-proto/src/message.rs:290`](../../../crates/foks-agent-proto/src/message.rs#L290): StoredHostStatus: lookup_name, canonical_name, host_id_hex, host_chain_sequence, merkle_epoch; no root hash
- [`apps/desktop/src/screens/servers-screen.tsx:984`](../../../apps/desktop/src/screens/servers-screen.tsx#L984): Identity and trust shows 'Verified · N entries · Checkpoint {epoch}' without a root hash
- [`crates/foks-agent/src/main.rs:2390`](../../../crates/foks-agent/src/main.rs#L2390): MerkleFork only maps to a RollbackDetected error response; no fork evidence is persisted
- [`crates/foks-proto/src/host.rs:276`](../../../crates/foks-proto/src/host.rs#L276): MerkleRoot carries epoch and a host-signed time, suitable for a portable checkpoint

**Recommendation**

Step 1, display only: add root_hash_hex, root_time and signed_root (base64 of the host-signed root extracted from merkle_heads.evidence_bytes) to StoredHostStatus. Show 'Checkpoint 1,204 · 7f3a 9c… · 3 Oct' and a 'Copy checkpoint' action in Servers > Identity and trust, and print the same from a CLI host status command. Step 2: add an agent operation CompareCheckpoint{profile, signed_root}. It verifies the host signature against the pinned hostchain. At the same epoch, a different root is a fork. Persist both signed roots in a new hard-state fork-evidence table so the result survives restarts and can be exported, then return RollbackDetected-class status. Today MerkleFork is only returned as an error and never stored. At a different epoch, fetch and verify the skip path between the two roots with the existing cited-root code. Results are Consistent, Linked, Fork or Unverifiable (another host or malformed input). Add `foks-rs host checkpoint [--compare FILE]`. Step 3, optional and Rust-only with no wire change: members publish their latest signed checkpoint to a reserved team KV path and compare teammates' entries. Document that the server can withhold these entries but cannot forge a host-signed root, and that the reserved path is visible to other FOKS clients.

**Already tracked:** book/13-client-verification.qmd:177-207 describes the gap and the manual comparison, but nothing implements it and ISSUES.md does not track it. fe-product-signed-activity covers sigchain history display, not root comparison.

<details><summary>Verifier note</summary>

The core claim holds. Book 13:177-207 says clients never compare roots and suggests comparing root hashes out of band. merkle_heads stores root_hash and evidence_bytes (schema.rs:156-164), and every evidence variant except the in-memory SkipPath placeholder carries the signed root (foks-verify merkle.rs:11-35; encode_evidence lib.rs:1932). StoredHostStatus exposes lookup_name, canonical_name, host_id_hex, host_chain_sequence and merkle_epoch, with no root hash (message.rs:290-296). No root hash appears in the desktop, CLI or agent output. Corrections: the UI line is servers-screen.tsx:984, not 983. The recommendation says a same-epoch mismatch is 'persisted like the existing fork errors', but MerkleFork is not persisted anywhere. The agent maps it to an ErrorCode::RollbackDetected response (main.rs:2109-2124, 2390-2420), and no stored fork record or 'server blocked' state exists. Step 2 and the 'server marked blocked' mockup state therefore need a new persisted fork-evidence record. Extracting the signed root requires decoding evidence_bytes per evidence_kind.

</details>

### gap-offline-read-only-vault

**No read access to vault items while the server is unreachable; add an explicit read-only offline mode with sealed key material**

- Type: missing-feature
- Priority: medium
- Effort: L
- Layers: agent, client-lib, desktop-ui, mcp
- Verification: confirmed

Every reveal or copy authenticates the user and fetches the node from the server. Agent caches are short-lived candidates, not answers. On a plane or during a server outage the desktop still lists items from the cached catalog but cannot reveal or copy any of them, which matters for a password manager. The soft cache already keeps each small file's ciphertext node_bytes under rollback anchors. What is missing is the PUK-derived key material to open it, which is fetched from the server on every read and never stored locally.

**Evidence**

- [`crates/foks-client-app/src/kv.rs:80`](../../../crates/foks-client-app/src/kv.rs#L80): read_kv_entry: resolved_user_path (authenticated) then client.read_user_kv_node from the server, no cache fallback
- [`book/19-agent.qmd:270`](../../../book/19-agent.qmd#L270): Read caches are 30-second candidates re-validated by the server
- [`crates/foks-client/src/kv/sync.rs:1074`](../../../crates/foks-client/src/kv/sync.rs#L1074): Small-file ciphertext node_bytes are stored in soft state in both sync modes
- [`crates/foks-client-db/src/soft_schema.rs:88`](../../../crates/foks-client-db/src/soft_schema.rs#L88): kv_entries.node_bytes column
- [`apps/desktop/src/model/lease.ts:80`](../../../apps/desktop/src/model/lease.ts#L80): Signed lease bounds access by local time; a natural cap for offline reads

**Recommendation**

Sequence this after gap-sync-persists-unused-plaintext so no plaintext lands in soft state. 1) After each successful online authentication, the agent seals the KV key material for the current generations into the existing encrypted protected store under a key derived from the keystore master key, scoped to (host, party, generation). It is dropped on access revocation, device removal and reset. 2) ReadKv and the data reads gain an opt-in offline flag. Only when the failure is a transport-level ServerUnavailable, never a rollback, fork, schema or permission error, the agent opens cached node_bytes at the last verified directory version and returns freshness {source: 'cache', verified_at}. 3) Offline reads stop once the signed lease lapses or a shorter per-device window passes; writes stay refused. 4) Desktop: an offline banner, a per-item 'cached copy' chip, and disabled edit, move and delete. MCP tool results carry the freshness field.

**Already tracked:** Not tracked. book/19-agent.qmd:270-282 describes server-revalidated caches by design. fe-product-version-history reuses locally seen versions but does not address unreachable servers.

<details><summary>Verifier note</summary>

The cited code shows no offline read path. read_kv_entry calls resolved_user_path, which authenticates and resolves against the server, then calls client.read_user_kv_node (kv.rs:79-97), with no cache fallback. Large-file reads also fetch every chunk from the server (kv.rs:786-799). Book 19-agent.qmd:270-282 describes the read caches as server-revalidated candidates. node_bytes is kept for small files in both modes (sync.rs:1074-1077; soft_schema.rs:88). The KV key material comes from PUKs obtained through online authentication (authenticated.puks), so cached ciphertext cannot be opened offline. The only agent-side catalog cache is short-lived and in memory (main.rs:2612-2645). No offline mode exists in the desktop, agent or client-app, and ISSUES.md does not track one. lease.ts:1-5 confirms that a lapsed compatibility lease already gates access per server, so using it as the offline cap is consistent. The recommendation is feasible under the constraints: no wire change, the agent keeps the key material, and writes stay online-only. One detail for implementation: the sealed key material must cover every PUK generation that cached entries were sealed under, not only the current one.

</details>

### gap-renderer-fault-capture

**A render error outside the routed screen blanks the whole window and leaves no trace in Copy diagnostics**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: adjusted

The only error boundary wraps ScreenRouter (vault-shell.tsx:712-744). The first-run flow, sidebar, topbar, TeamInfoPanel, DetailsPanel (1,332 lines, holds reveal state), ShellSearch, ShellOverlays and BootstrapScreen render outside it. A throw in any of them unmounts the React root, ExitGuard included, and leaves a blank window. The user can only leave by repeating Cmd-Q or the window close, which reaches the native question. The existing boundary shows the error message in a band and logs to the console, but records nothing. createRoot has no React 19 error callbacks, and there is no window error or unhandledrejection listener. The diagnostic log forbids error messages, so Copy diagnostics carries no trace of the fault.

**Evidence**

- [`apps/desktop/src/app/vault-shell.tsx:712`](../../../apps/desktop/src/app/vault-shell.tsx#L712): ScreenErrorBoundary wraps only ScreenRouter (712-744). FirstRunExperience 582, Sidebar 608, Topbar 649, TeamInfoPanel 752, DetailsPanel 768, ShellSearch 797 and ShellOverlays 811 are outside it.
- [`apps/desktop/src/app-root.tsx:47`](../../../apps/desktop/src/app-root.tsx#L47): BootstrapScreen/VaultShell render under ExitGuard (line 68) with no boundary
- [`apps/desktop/src/app/screen-error-boundary.tsx:82`](../../../apps/desktop/src/app/screen-error-boundary.tsx#L82): componentDidCatch only calls console.error. render() shows the message in a Band (lines 86-111).
- [`apps/desktop/src/main.tsx:20`](../../../apps/desktop/src/main.tsx#L20): createRoot(root) has no error callbacks. React ^19.2.8 is declared in the repository root package.json.
- [`apps/desktop/src/diagnostics/log.ts:10`](../../../apps/desktop/src/diagnostics/log.ts#L10): Never error messages. diagnosticLog singleton at line 256.
- [`apps/desktop/vite.config.ts:23`](../../../apps/desktop/vite.config.ts#L23): Default minified build without keepNames, so componentStack names are mangled in production
- [`apps/desktop/src-tauri/src/close_guard.rs:106`](../../../apps/desktop/src-tauri/src/close_guard.rs#L106): Repeated Cmd-Q or close reaches native_question. CloseRequested goes through the same guard (295-299).

**Recommendation**

Add a root boundary inside ExitGuard that renders a minimal fault screen with Reload window, Copy diagnostics (reading the module-level diagnosticLog singleton) and Quit, and that depends on no shell context. Wrap DetailsPanel, TeamInfoPanel, ShellSearch and ShellOverlays in panel boundaries that close the panel. Unmounting the subtree discards any revealed value. Pass onUncaughtError, onCaughtError and onRecoverableError to createRoot, and add window error and unhandledrejection listeners. Each records a TimingEvent named 'renderer.fault' with attrs {boundary} set to the catching boundary's static label. Set code to the built-in error constructor name only for standard errors (TypeError, RangeError, etc.), because production Vite/esbuild minification mangles component and custom class names in componentStack. Alternatively, set esbuild keepNames. Add a Faults table to diagnostics/format.ts. Add a render test that throws inside DetailsPanel and asserts the rail survives, the panel closes and the fault is recorded.

**Already tracked:** desktop-native-agent-log-and-crash-markers covers native crash markers and agent.log only; nothing covers renderer faults.

<details><summary>Verifier note</summary>

Core claim holds. vault-shell.tsx:712-744 is the only ScreenErrorBoundary in apps/desktop (grep finds no other boundary, no onUncaughtError/onCaughtError/onRecoverableError and no window error or unhandledrejection listener). FirstRunExperience (582), Sidebar (608), Topbar (649), TeamInfoPanel (752), DetailsPanel (768, 1,332 lines, holds revealRequest state), ShellSearch (797) and ShellOverlays (811) are all outside it. app-root.tsx renders BootstrapScreen/VaultShell inside ExitGuard with no boundary, and main.tsx:20 calls createRoot(root) with no options (React ^19.2.8 in the root package.json, not apps/desktop/package.json, which does not exist). In React 19 an uncaught render error unmounts the whole root. CloseRequested and Cmd-Q both go through CloseGuard::begin, and only a repeated request reaches native_question (close_guard.rs:106-110, 295-299). diagnostics/log.ts:10-11 forbids error messages, and the module-level diagnosticLog singleton (log.ts:256) can be used from a context-free fault screen. ISSUES.md and book/20-desktop.qmd do not track this. Two corrections. First, the boundary does not only log: it also renders normalizeCommandError(failure.error).message in a crit Band. Second, the plan to take attrs.component from the first name in componentStack does not work in shipped builds. vite.config.ts uses Vite's default esbuild minification without keepNames, so production component names and custom Error class names are mangled. Use each boundary's own static label instead, or enable esbuild keepNames.

</details>

### gap-security-key-pin-field

**Eleven hand-rolled security-key PIN fields disagree with the 6-8 character rule and never show remaining attempts**

- Type: code-quality
- Priority: medium
- Effort: S
- Layers: desktop-ui, agent, agent-proto
- Verification: adjusted

Hardware-key PIN entry is hand-rolled in at least fourteen places: eleven 'Security key PIN' inputs plus the Card PIN, PUK and New PIN inputs in device-sheets.tsx. They disagree with foks-yubi's 6-8 printable-ASCII rule: nine set maxLength={32}, and the SSO sign-up and device-sheet inputs set no limit. The retry-count inputs allow 1-255, but PinRetryConfiguration accepts 1-15. PinRejected { remaining } and PinBlocked have no ErrorCode and reach the desktop only as OperationFailed message text. In the invitation and membership flows that text is then thrown away: hardwareUnlockRequested matches /hardware key/i and replaces it with 'Enter the PIN for your security key to continue.' A user with one attempt left, or with a blocked PIN, is asked for the PIN again with no warning. No prompt shows the remaining attempts before submission.

**Evidence**

- [`crates/foks-yubi/src/provider.rs:79`](../../../crates/foks-yubi/src/provider.rs#L79): Pin::new requires 6..=8 ASCII-graphic bytes; PinRetryConfiguration::new at line 110 accepts only 1..=15 attempts
- [`apps/desktop/src/components/invite-new-user-sheet.tsx:268`](../../../apps/desktop/src/components/invite-new-user-sheet.tsx#L268): maxLength={32}; same at invitation-activity-sheet.tsx:472, rename-banner.tsx:262, rename-panel.tsx:123, bot-panel.tsx:123, invitation-panel.tsx:400, membership-requests.tsx:476, sso/sign-in-sheet.tsx:46, admin-panel.tsx:115
- [`apps/desktop/src/components/sso/sign-up-panel.tsx:152`](../../../apps/desktop/src/components/sso/sign-up-panel.tsx#L152): PIN input with no length limit (also at 224)
- [`apps/desktop/src/screens/device-sheets.tsx:1293`](../../../apps/desktop/src/screens/device-sheets.tsx#L1293): Card PIN, PUK and New PIN inputs have no length rule and no autoComplete=off; Card PIN Fields at 954 and 1138 likewise; the 'PIN tries'/'PUK tries' inputs (~984-1000) allow max={255} against the Rust limit of 15
- [`apps/desktop/src/invitation-writes.ts:42`](../../../apps/desktop/src/invitation-writes.ts#L42): hardwareUnlockRequested tests /unlock the account key|security key|hardware key/i against the message, so 'Hardware key PIN was rejected; N attempt(s) remain' and 'Hardware key PIN is blocked' both count as an unlock request
- [`apps/desktop/src/components/invitation-panel.tsx:178`](../../../apps/desktop/src/components/invitation-panel.tsx#L178): on a hardwareUnlockRequested match, the agent message is replaced with 'Enter the PIN for your security key to continue.'; same at invite-new-user-sheet.tsx:155, invitation-activity-sheet.tsx:136, membership-requests.tsx:99 and :173
- [`crates/foks-agent/src/main.rs:2131`](../../../crates/foks-agent/src/main.rs#L2131): unmatched errors, including all foks_yubi PIN errors, become ErrorCode::OperationFailed with the leaf message; ErrorCode (foks-agent-proto/src/message.rs:2359) and ErrorFields (:2417) have no PIN code or remaining field
- [`apps/desktop/src/screens/devices/credential-workflow.ts:30`](../../../apps/desktop/src/screens/devices/credential-workflow.ts#L30): yubi_pin_status (args {profile, alias}) is called only by the Devices 'Card PIN status' action, not by any PIN prompt; the team and invitation prompts hold no alias

**Recommendation**

1. Add a shared SecurityKeyPinField (and a PUK variant) in apps/desktop/src/components. It validates 6-8 printable ASCII characters against constants covered by a Rust/TS parity test, sets autoComplete=off, and replaces every hand-rolled PIN, PUK and New PIN input, including those in device-sheets.tsx.
2. Clamp the PIN/PUK retry inputs to 1-15.
3. In the agent, map foks_yubi PinRejected to a new ErrorCode::PinRejected with ErrorFields.remaining, and map PinBlocked to ErrorCode::PinBlocked. Map them in desktop transport.rs, and have the field show 'Incorrect PIN, N attempts left'. When remaining is 1, warn before submitting. When the PIN is blocked, link to Devices > Unblock with PUK.
4. Change hardwareUnlockRequested to key on error codes (PinRejected and PinBlocked are not unlock requests) rather than on /hardware key/ in the message. This stops the invitation and membership flows from discarding these errors.
5. Optionally, when the enrolled alias can be resolved from the device cache, call yubi_pin_status before the first submission to show attempts up front.

**Already tracked:** desktop-native-error-code-registry covers the general error-code registry; the PIN rule divergence and attempts display are not covered.

<details><summary>Verifier note</summary>

The main claim holds. Nine 'Security key PIN' inputs set maxLength={32}, at exactly the cited lines. sso/sign-up-panel.tsx:152 and :224 set no limit. Pin::new (provider.rs:79-86) accepts only 6..=8 ASCII-graphic bytes. ErrorCode (message.rs:2359) has no PIN variant and ErrorFields has no `remaining` field. foks_yubi PinRejected and PinBlocked are referenced nowhere outside foks-yubi, so they fall through to OperationFailed with the leaf message (foks-agent main.rs ~2131). yubi_pin_status is called only from the Devices 'Card PIN status' action (credential-workflow.ts:30, devices-screen.tsx:1365) and never from a PIN prompt. ISSUES.md, the book and docs do not track any of this.

Three parts need correction.
(1) 'No warning' is overstated in one direction and understated in another. The agent's leaf message is 'Hardware key PIN was rejected; N attempt(s) remain', and transport.rs:117 passes it through unchanged, so admin-panel, bot-panel, rename-panel and the SSO sheets do show the count after a wrong entry. The real defect is in four other components: invite-new-user-sheet.tsx:155, invitation-activity-sheet.tsx:136, invitation-panel.tsx:178 and membership-requests.tsx:99/173. Each passes the failure to hardwareUnlockRequested (invitation-writes.ts:42-47), which tests /security key|hardware key/i against the message. PinRejected, PinBlocked and the Pin::new policy error all match, so the message is replaced with 'Enter the PIN for your security key to continue.' The remaining-attempt count and the blocked state are discarded, and a blocked or nearly blocked key is shown as an ordinary PIN request.
(2) There are more than eleven hand-rolled PIN inputs. device-sheets.tsx has 'Card PIN' at 954 and 1138 and the credential-sheet PIN/PUK/New PIN inputs at about 1293-1310. None has a length rule or autoComplete=off, and the New PIN fields are where a user sets a PIN. The 'PIN tries'/'PUK tries' inputs (device-sheets.tsx ~984-1000) allow max={255}, but PinRetryConfiguration::new (provider.rs:110) accepts only 1..=15.
(3) The desktop has no 'kit' module; shared inputs live in apps/desktop/src/components (field.tsx, index.ts). Also, yubi_pin_status takes {profile, alias}, but the team, invitation, bot and rename PIN prompts hold no key alias, because the agent selects the enrolled key. Calling it on mount needs alias resolution from the device cache. A more direct fix is to return `remaining` with the rejection.

Priority goes up to medium because the overwrite in the invitation and membership flows hides a 1-attempt-left or blocked PIN. Every recommended change is in the agent IPC protocol and the desktop, so wire compatibility with Go FOKS and the server are unaffected.

</details>

### gap-sync-persists-unused-plaintext

**SyncAccount and SyncTeam download every file and store its plaintext in soft state, but production code reads it back only for file sizes**

- Type: security
- Priority: medium
- Effort: S
- Layers: client-lib, client-db, agent, cli, docs
- Verification: adjusted

sync_kv_with_fetch always runs in KvSyncMode::Content. That mode stores decrypted small-file contents (passwords, notes) and symlink targets in kv_entries. It also downloads every large file and appends its decrypted chunks to kv_large_file_chunks. Content mode is reached through authenticated_tree (list_kv and sync_account) and sync_team, so the agent's SyncAccount and SyncTeam, `foks-rs account sync` and signup resume all leave the whole vault in plaintext in a SQLite file. That file is protected only by 0600 permissions, file backups copy it, and app lock does not touch it. Nothing consumes this plaintext: the only production reader computes entry sizes, and write_large_file has no non-test caller. The desktop catalog already uses metadata mode, so this cost (a full download of every large file, up to the 512 MiB inline budget per sync) only buys plaintext at rest.

**Evidence**

- [`crates/foks-client/src/kv/sync.rs:763`](../../../crates/foks-client/src/kv/sync.rs#L763): sync_kv_with_fetch passes KvSyncMode::Content unconditionally
- [`crates/foks-client/src/kv/sync.rs:1087`](../../../crates/foks-client/src/kv/sync.rs#L1087): Content mode keeps decrypted small-file plaintext in projected.content; the symlink target is kept at 1128-1133
- [`crates/foks-client/src/kv/sync.rs:1191`](../../../crates/foks-client/src/kv/sync.rs#L1191): Every large file's decrypted chunks are appended to the soft store (bounded per file by MAX_FILE_CHUNKS = 4096 at line 843, not by the 512 MiB inline limit at line 844)
- [`crates/foks-client/src/kv/sync.rs:1347`](../../../crates/foks-client/src/kv/sync.rs#L1347): Content-mode results are persisted and returned through store.tree, so the plaintext is read back from SQLite
- [`crates/foks-client-db/src/soft.rs:863`](../../../crates/foks-client-db/src/soft.rs#L863): INSERT INTO kv_large_file_chunks (file_id, offset, content) with plaintext
- [`crates/foks-client-db/src/soft_schema.rs:89`](../../../crates/foks-client-db/src/soft_schema.rs#L89): kv_entries.content and symlink BLOB columns hold plaintext
- [`crates/foks-client-app/src/kv.rs:1240`](../../../crates/foks-client-app/src/kv.rs#L1240): authenticated_tree calls sync_user_kv (content mode); used by list_kv (line 37) and sync_account (account.rs:999)
- [`crates/foks-client-app/src/yubi.rs:1369`](../../../crates/foks-client-app/src/yubi.rs#L1369): Security-key account sync also uses content mode through sync_user_kv_yubi
- [`crates/foks-client-app/src/team.rs:417`](../../../crates/foks-client-app/src/team.rs#L417): sync_team calls sync_team_kv (content mode)
- [`crates/foks-client-app/src/kv.rs:1920`](../../../crates/foks-client-app/src/kv.rs#L1920): Only production reader of the stored content: size = content.len()
- [`crates/foks-client-db/src/soft.rs:1708`](../../../crates/foks-client-db/src/soft.rs#L1708): A soft schema version mismatch is refused, not migrated
- [`crates/foks-agent/src/main.rs:4763`](../../../crates/foks-agent/src/main.rs#L4763): The desktop catalog path already uses list_kv_metadata

**Recommendation**

Switch authenticated_tree, list_kv, sync_account, sync_unlocked_yubi_account and sync_team to KvSyncMode::Metadata. For small-file sizes, record the plaintext length in a new nullable column when the box is opened. For large files without custom size metadata, report the size as unknown, or measure it during an explicit download. Delete Content mode, or keep it only behind an explicit export API, and migrate the server-testkit callers to the read APIs. Because initialize() refuses version mismatches (soft.rs:1708), add an in-place scrub at open rather than a bare version bump: set kv_entries.content and symlink to NULL and delete kv_large_file_chunks and completed kv_large_files under the existing secure_delete setting, then update user_version in the same transaction. Add an integration test asserting that kv_entries.content and symlink are NULL and no chunks remain after SyncAccount and SyncTeam. If a later feature needs cached plaintext, seal it under a key derived from the keystore master key, as the protected mutation store does (account.rs:126-129), with associated data binding host, party, dirent and version. Update book/19-agent.qmd:154-185 and 215-220, which describe soft state as a plaintext cache with large-file staging.

**Already tracked:** book/19-agent.qmd:154-185 documents soft state as a plaintext cache protected by 0600 and secure_delete. This finding adds that the plaintext and the large-file downloads have no consumer, so the smallest fix is to stop producing them. client-libs-soft-stage-reclaim covers only orphaned-stage reclamation.

<details><summary>Verifier note</summary>

The core claim holds. sync_kv_with_fetch hard-codes KvSyncMode::Content (sync.rs:763). Content mode keeps small-file plaintext and symlink targets (1087-1092, 1128-1133) and downloads every large file into soft-state chunks (1191-1238, soft.rs:862-869). The projection is persisted and re-read through store.tree (sync.rs:1347-1348). Production callers are authenticated_tree (kv.rs:1240, used by list_kv for the CLI and by sync_account through agent SyncAccount, CLI account sync and signup resume at account.rs:134), sync_team (team.rs:417, agent and CLI), and also yubi.rs:1369 sync_user_kv_yubi, which the finding omits. Reads always refetch from the server (kv.rs:89-96, 770-799). The only consumer of the content is the size computation at kv.rs:1920/1922, and write_large_file / read_large_file_chunk have no non-test callers. The desktop catalog uses list_kv_metadata (agent main.rs:4763). Four corrections. (a) The 512 MiB MAX_TOTAL_CONTENT_BYTES (sync.rs:844) bounds only small-file and symlink inline content. Large files are bounded per file by MAX_FILE_CHUNKS = 4096 (sync.rs:843), so one sync can download more than 512 MiB. (b) Soft state refuses any user_version mismatch with UnsupportedSoftSchema (soft.rs:1708-1713), so 'bump the schema' without an in-place migration forces manual deletion. (c) Metadata mode reports no size for large files without custom size metadata (foks-crypto kv.rs:164-166), such as files written by other clients, so `kv list` would lose those sizes. (d) Many server-testkit tests call sync_user_kv/sync_team_kv and must be migrated if Content mode is removed.

</details>

### gap-clock-skew-diagnosis

**A wrong system clock surfaces as an opaque check-in or compatibility failure**

- Type: correctness-risk
- Priority: low
- Effort: S
- Layers: client-lib, agent, desktop-ui
- Verification: adjusted

A wrong system clock surfaces as an opaque check-in or compatibility failure. The desktop decides lease freshness from the local clock and reports a lapsed lease as 'The signed server check-in has expired'. In the agent, a compatibility artifact whose signed generated_at is more than 5 minutes ahead of local time (local clock slow), or whose expiry is not after local time (local clock fast), fails with the same InvalidProfile text as a target mismatch. connectivity.rs returns it as ErrorCode::CompatibilityRejected. A user with a skewed clock is told the server or compatibility is at fault, and nothing estimates the skew, even though host-signed times are verified during normal operation.

**Evidence**

- [`crates/foks-client-app/src/registry.rs:377`](../../../crates/foks-client-app/src/registry.rs#L377): target mismatch, generated_at > now+300 or expires_at <= now all fail with one InvalidProfile message
- [`crates/foks-agent/src/connectivity.rs:406`](../../../crates/foks-agent/src/connectivity.rs#L406): InvalidProfile maps to ErrorCode::CompatibilityRejected with the bounded message. fields.reason is already used on the Incompatible branch (392-403).
- [`apps/desktop/src/model/lease.ts:82`](../../../apps/desktop/src/model/lease.ts#L82): signedLeaseState compares expiresAt with Date.now()
- [`apps/desktop/src/bridge/snapshot-notifications.ts:49`](../../../apps/desktop/src/bridge/snapshot-notifications.ts#L49): Lapsed lease text blames an expired check-in with no clock hint
- [`crates/foks-proto/src/host.rs:278`](../../../crates/foks-proto/src/host.rs#L278): MerkleRoot.time is host-signed and can give an advisory skew estimate
- [`apps/desktop/src-tauri/src/agent/transport.rs:74`](../../../apps/desktop/src-tauri/src/agent/transport.rs#L74): ClockUntrusted is already mapped to non-retryable 'clock-untrusted', which today names the adapter ledger refusal

**Recommendation**

In Profile::apply_compatibility_artifact, distinguish the time-window failure from the target mismatch with a dedicated client-app error variant. In connectivity.rs, report it either as CompatibilityRejected with ErrorFields.reason 'clock-behind' or 'clock-ahead', which the Incompatible branch already populates, or as ClockUntrusted. ClockUntrusted today names the MCP adapter ledger refusal, so check its desktop mapping first. Include the skew in seconds. Whenever the agent verifies a fresh host- or artifact-signed time in this process, record an advisory skew estimate: a just-fetched artifact's generated_at, or the MerkleRoot.time of a root that includes this device's just-submitted link. Expose clock_skew_seconds in DescribeServerStatus. Use the estimate only for messaging, never to adjust validity or freshness checks, because a host can misreport time. When the skew exceeds 5 minutes, the desktop's server notice should say 'Your computer's clock is about 3 hours behind' instead of the expired check-in text. Share the skew source with gap-mcp-adapter-clock-suspend.

<details><summary>Verifier note</summary>

Core claim holds. registry.rs:377-379 rejects an artifact when generated_at > now+300 or expires_at <= now, using the same generic InvalidProfile text as a target mismatch. The agent maps InvalidProfile to ErrorCode::CompatibilityRejected with that text (connectivity.rs:406-410), so the user sees a compatibility rejection. lease.ts:82-88 classifies lease freshness from Date.now(), and snapshot-notifications.ts:49-52 reports 'The signed server check-in has expired' with no clock hint. Nothing in the agent, client or desktop estimates skew; the only SKEW constants are in the server session and the MCP adapter ledger. MerkleRoot.time exists (host.rs:278) and is server-clock milliseconds (book/12-epochs.qmd:27). ErrorCode::ClockUntrusted and ErrorFields.reason exist. Two corrections. ClockUntrusted currently names the MCP adapter ledger's clock refusal (main.rs:2060, desktop transport.rs:74 maps it to non-retryable 'clock-untrusted'). The lower-impact option is the existing CompatibilityRejected path, which already fills fields.reason for the Incompatible case. Also, any host-derived skew estimate must stay advisory: a host can misreport time, so the estimate may only shape the message and must never feed validity or freshness checks.

</details>

### gap-storage-usage-display

**Show storage used per vault; the agent already returns usage, but the desktop learns of limits only from a quota refusal**

- Type: missing-feature
- Priority: low
- Effort: S
- Layers: desktop-ui, desktop-native
- Verification: adjusted

The desktop never shows how much a vault stores. DataRead::Usage returns encrypted file and byte totals, and the MCP server exposes it, but the desktop has no command for it. Team info's Files section shows only an item count, and the first sign of a limit is a 'quota-exceeded' error from a write. The server's limit is host configuration (the Rust server defaults to 16 GiB and 1,000,000 objects; Go hosts set their own). KvUsage carries no limit and counts less than the server's quota accounting, so the client can show usage but not a reliable fraction of the quota. The server computes usage by decoding every small node, so the read should be on demand, not polled.

**Evidence**

- [`crates/foks-agent-proto/src/data.rs:37`](../../../crates/foks-agent-proto/src/data.rs#L37): DataRead::Usage and DataUsage {files, bytes} (44-47); DataScope requires bound host_id/user_id (8-14)
- [`crates/foks-agent/src/data.rs:240`](../../../crates/foks-agent/src/data.rs#L240): Agent serves Usage via data_usage after check_data_identity
- [`crates/foks-mcp/src/agent.rs:217`](../../../crates/foks-mcp/src/agent.rs#L217): MCP exposes usage; the desktop bridge has no equivalent
- [`apps/desktop/src/screens/team-info.tsx:321`](../../../apps/desktop/src/screens/team-info.tsx#L321): Files section shows only an item count
- [`crates/foks-server-db/src/kv.rs:1256`](../../../crates/foks-server-db/src/kv.rs#L1256): kv_usage decodes every small node per call and counts only file ciphertext
- [`crates/foks-server-db/src/kv.rs:1364`](../../../crates/foks-server-db/src/kv.rs#L1364): ensure_kv_capacity counts all exact encodings and all rows, a different measure from KvUsage
- [`crates/foks-server-db/src/config.rs:85`](../../../crates/foks-server-db/src/config.rs#L85): 16 GiB and 1,000,000 objects are server Config defaults, not protocol constants

**Recommendation**

Add a store-addressed desktop usage command, either a new agent operation keyed like ListKv/ListTeamKv or ReadData{Usage} after BindDataAccount. Call it only when Team info or the personal vault header opens, cache the result for the session and invalidate it after writes. Show 'About 2.3 GiB stored (encrypted) · 1,204 files' with loading and unavailable states. Do not show a denominator, a percentage band or an upload preflight refusal unless the server advertises its limit through a signed or negotiated field. KvUsage does not carry one, Go v0.1.9 hosts enforce their own quotas, and KvUsage under-counts relative to ensure_kv_capacity. When a write fails with quota-exceeded, keep the existing refusal and show the last-known usage beside it. Rely on server-storage-usage-ledger to make the server-side read cheap; do not poll.

**Already tracked:** server-storage-usage-ledger and server-operator-capacity-console cover server-side accounting and operator views; the user-facing display is not tracked.

<details><summary>Verifier note</summary>

Core claim holds. DataRead::Usage and DataUsage {files, bytes} exist (agent-proto data.rs:37, 44-47). The agent serves them (agent data.rs:240-244), and MCP exposes them (foks-mcp agent.rs:217-224). apps/desktop/src-tauri has no ReadData or usage call, and team-info.tsx:321-328 shows only an item count. kv_usage (server kv.rs:1256) decodes every small node per call. ISSUES.md does not track a user-facing display. The recommendation is not sound as written. (1) The 16 GiB and 1,000,000-object figures are foks-server-db Config defaults (config.rs:85-86), not protocol constants. KvUsage carries no limit, and a Go v0.1.9 host enforces its own quotas, so the client cannot know the denominator. (2) The server enforces the quota with ensure_kv_capacity (kv.rs:1364-1401), which sums every exact encoding (roots, directories, nodes, dirents, upload metadata, chunk ciphertext plus chunk envelopes, locks) and counts every row. KvUsage reports only small-file ciphertext and large-file chunk ciphertext, and its files count is not the objects count. 'X of 16 GiB', an 80% band and an upload preflight that refuses early would therefore be inaccurate, and could refuse uploads the server would accept. (3) snapshot-projection.ts:51 is just a recoverable-code list for group-detail failures, not a quota surface. (4) ReadData takes a DataScope that needs BindDataAccount's host/user binding (the MCP read path), so a store-addressed desktop operation may fit the desktop bridge better.

</details>

### gap-toast-timing-announcements

**Toasts vanish after 2.6 s with no pause, carry the only explanation for refused actions, and are announced unreliably**

- Type: feature-refinement
- Priority: low
- Effort: S
- Layers: desktop-ui, desktop-native
- Verification: confirmed

Toasts default to 2,600 ms, and the timers start on mount with no pause on hover, focus or a hidden window. Some toasts are the only explanation for an action that did nothing: action refusals in Files, navigation refusals and upload problems. Each toast is a newly inserted element that already carries role=status or role=alert with its text, while the #toasts container is not a live region; screen readers, VoiceOver in WebKit in particular, often skip such insertions. Warning tone falls back to detecting a leading '⚠', which no caller produces. Copying a password says 'Password copied' but not that the native side clears the clipboard after 30 s (15 s on Linux).

**Evidence**

- [`apps/desktop/kit/toasts.tsx:12`](../../../apps/desktop/kit/toasts.tsx#L12): DEFAULT_DURATION_MS = 2_600
- [`apps/desktop/kit/toasts.tsx:96`](../../../apps/desktop/kit/toasts.tsx#L96): Hide and remove timers start on mount; no pause on hover, focus or visibility
- [`apps/desktop/kit/toasts.tsx:118`](../../../apps/desktop/kit/toasts.tsx#L118): role set per inserted toast; container at line 189 has no live-region role
- [`apps/desktop/kit/toasts.tsx:89`](../../../apps/desktop/kit/toasts.tsx#L89): Warning tone inferred from a leading '⚠' that no caller emits
- [`apps/desktop/src/screens/items-screen.tsx:1031`](../../../apps/desktop/src/screens/items-screen.tsx#L1031): Action refusal reason shown only as a warning toast
- [`apps/desktop/src/app/navigation-runtime.ts:61`](../../../apps/desktop/src/app/navigation-runtime.ts#L61): Navigation refusal reason shown only as a toast
- [`apps/desktop/src-tauri/src/clipboard.rs:17`](../../../apps/desktop/src-tauri/src/clipboard.rs#L17): AUTO_CLEAR_SECONDS 15 (Linux) / 30, never shown to the user

**Recommendation**

Make #toasts a persistent role=status aria-live=polite region, with a separate assertive region for warnings, and append messages into them. Pause timers on hover, on focus within the toast and while document.hidden. Scale duration with message length: at least 4 s, warnings 8 s. Remove the '⚠' heuristic. Expose AUTO_CLEAR_SECONDS through a native command so copy toasts can read 'Password copied · clears in 30 s'. Add a render test with fake timers covering pause and announcement.

**Mockup:** [Login items: field copy, generator, quick copy](../mockups/vault-login-copy-and-generator.html)

<details><summary>Verifier note</summary>

Verified in kit/toasts.tsx. DEFAULT_DURATION_MS = 2_600 (line 12). The hide and remove timers start in the mount effect (lines 100-105) with no pause on hover, focus or visibility. Only toasts with an action and no durationMs persist. No caller in src passes durationMs. Each ToastItem sets role alert/status itself (line 118), and the #toasts container (line 191, not 189) has no live-region role. The '⚠' fallback (lines 90-92) has no caller, per a grep over src and kit. Refusal reasons reach the user only as warning toasts in three places: items-screen.tsx:1031 (allowAction on row actions), navigation-runtime.ts:60-61 (setRefusalHandler) and the file-drop warnings at items-screen.tsx:569. The copy toast says only 'Password copied' (items-screen.tsx:1037-1039). AUTO_CLEAR_SECONDS is 15 on Linux and 30 elsewhere (clipboard.rs:17-20) and is never surfaced; the only clipboard copy is the exit overlay's 'Clearing sensitive clipboard data'. ISSUES.md and the book do not track this. The recommendation is feasible and stays in desktop-ui/desktop-native.

</details>
