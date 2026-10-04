# Desktop frontend architecture and code quality (non-chat)

Area key `fe-arch`. 14 verified findings. Part of the [foks-rs review](../README.md).

## Current state

Outside src/chat, the desktop frontend is about 73k lines of TS/TSX/CSS in 264 files. Several foundations are sound. The bridge has runtime decoders and normalizes mutation errors in bridge/transport.ts. Production builds drop the mock bridge through a dynamic import. CSS uses cascade layers, and a test (styles-geometry) checks token and type-size rules. Overlays use inert. The metadata query repository retires stale reads by generation and epoch, and bridge/ and navigation/ have an import-cycle test.

Structure has not kept up with growth, though. 60 modules sit directly in src/. Features are split across the root, screens/, components/ and app.css: first-run alone spans four places. Non-UI models in screens/ are imported by lower layers. Several files are 1,300-2,200 lines, and FirstRunSession has 24 useState calls and a 16-branch render chain. Local copies of the same thing have started to differ: two sidebar-resize hooks behave differently on cancel and report different ARIA values, and copy-to-clipboard is handled three different ways, four of which ignore failures. Shell services (bridge, snapshot, error handlers) are passed as props through about 60 components each.

Tests: render tests start a Vite server per file and load code through 504 untyped-path ssrLoadModule calls. They also run against a typed mock that skips the production decoders and the mutation-ambiguity handling. The Files list is not accessible as a list: every row is its own role=button tab stop, with buttons nested inside. The bundle is one 1.06 MB chunk.

Every finding below can be done incrementally while keeping Go wire compatibility and the agent-owns-credentials model. Do them in this order: enforce boundaries and fix the test harness first, then move code into feature folders one feature at a time, splitting the large files as each feature moves.

## Index

| Finding | Type | Priority | Effort |
|---|---|---|---|
| [Split FirstRunSession into a setup-session context, a typed step registry and per-step components](#fe-arch-first-run-session) | maintainability | high | L |
| [Use one clipboard-copy action; four first-run copy buttons silently drop failures](#fe-arch-copy-action) | correctness-risk | medium | S |
| [Split app.css by feature, bring team-info.css into the cascade layers, and auto-discover stylesheets in the style test](#fe-arch-css-structure) | maintainability | medium | M |
| [Make DetailsPanel's plaintext lifecycle one reducer instead of four hand-maintained reset paths](#fe-arch-details-panel-plaintext-reducer) | correctness-risk | medium | M |
| [Register webview localStorage keys in one module and purge profile-scoped entries when the device is reset](#fe-arch-device-storage) | correctness-risk | medium | M |
| [Move the flat src/ root and mixed screens/ into feature folders with enforced layering](#fe-arch-feature-folders) | maintainability | medium | L |
| [Give the Files list and folder tree list/tree semantics and a single tab stop](#fe-arch-files-list-a11y) | feature-refinement | medium | M |
| [Run the mock bridge through the production transport and decoders](#fe-arch-mock-bridge-contract) | testing | medium | L |
| [Replace the shell's private sidebar-resize hook with the shared pane-resize hook](#fe-arch-pane-resize-unify) | code-quality | medium | S |
| [Replace per-file Vite servers in render tests with one typed harness and test folders that mirror the features](#fe-arch-render-test-harness) | testing | medium | M |
| [Split the 1,100-1,900-line screen files into model, sheets and view, and deduplicate their local primitives](#fe-arch-screen-split-shared-primitives) | maintainability | medium | L |
| [Provide shell services through context instead of passing bridge, snapshot and error handlers through every screen](#fe-arch-shell-services-context) | maintainability | medium | M |
| [Lazy-load first-run, the chat views and the settings screens to shrink the 1.06 MB startup chunk](#fe-arch-code-splitting) | performance | low | M |
| [Standardize write actions on command-policy with a shared hook instead of 33 hand-rolled busy flags](#fe-arch-mutation-action-hook) | code-quality | low | M |

### fe-arch-first-run-session

**Split FirstRunSession into a setup-session context, a typed step registry and per-step components**

- Type: maintainability
- Priority: high
- Effort: L
- Layers: desktop-ui
- Verification: adjusted

FirstRunSession (first-run-screen.tsx lines 208-2188) holds 24 useState calls. They include the secret inputs invite, passphrase, confirmation, recoveryPhrase and pairingPhrase, which clearSecrets (line 755) resets by hand together with clearBackup() and secretsHeld.

It renders through a 16-branch `content =` if/else chain (lines 1084-1965):
- Since commit 09a1e1e, 8 branches delegate to extracted step components.
- 8 branches remain inline Pane JSX: operation-pending, identity-pending, local, the device scan, the CLI profile chooser, who/boot, the managed-local account, and existing. A trailing else covers added.
- Every step's props are still wired inside this file.
- Dispatch depends on more than checkpoint.state: goChooserOpen, goDiscovery, goScanError, checkpoint.managedLocal and profile.

The sub-hooks in screens/first-run/ receive controller state as Pick<ReturnType<typeof useFirstRunController>, ...> plus raw setters. useAccountOperations takes 13 parameters, including setBusy, setMessage and setConnectionErrors. busy is derived at lines 363-371 from four sources (otherMutationBusy, serverWorkflow.busy, teamDiscovery.busy, phraseOperation), with no named type.

**Evidence**

- [`apps/desktop/src/screens/first-run-screen.tsx:208`](../../../apps/desktop/src/screens/first-run-screen.tsx#L208): FirstRunSession starts here and ends at 2188. It contains 24 useState calls (lines 243-303).
- [`apps/desktop/src/screens/first-run-screen.tsx:1084`](../../../apps/desktop/src/screens/first-run-screen.tsx#L1084): `let content: ReactNode` followed by 16 `content = (` branches up to line 1965. Branches at 1570, 1592, 1794, 1878, 1902, 1917 and 1947 already render extracted step components.
- [`apps/desktop/src/screens/first-run-screen.tsx:1420`](../../../apps/desktop/src/screens/first-run-screen.tsx#L1420): The 'who' state splits into a device scan (goDiscovery and goScanError), the CLI profile chooser (goChooserOpen, line 534) and the method choice. Dispatch is not by state alone.
- [`apps/desktop/src/screens/first-run-screen.tsx:755`](../../../apps/desktop/src/screens/first-run-screen.tsx#L755): clearSecrets calls clearBackup() and resets the five secret states plus secretsHeld by hand.
- [`apps/desktop/src/screens/first-run-screen.tsx:363`](../../../apps/desktop/src/screens/first-run-screen.tsx#L363): mutationBusy, busyOperation and busy are derived from four separate busy sources.
- [`apps/desktop/src/screens/first-run/use-account-operations.ts:37`](../../../apps/desktop/src/screens/first-run/use-account-operations.ts#L37): 13-parameter hook typed via Pick<ReturnType<typeof useFirstRunController>, 'checkpoint'|'checkpointRef'|'mounted'|'commit'> & {setBusy, setMessage, setConnectionErrors, ...}.
- [`apps/desktop/src/screens/first-run/use-setup-session.ts:146`](../../../apps/desktop/src/screens/first-run/use-setup-session.ts#L146): A useSetupSession hook (restart and re-enter actions) already exists, so the proposed context hook name collides with it.
- [`apps/desktop/src/first-run-controller.ts:12`](../../../apps/desktop/src/first-run-controller.ts#L12): Pure controller functions at the src root next to use-first-run-controller.ts.

**Recommendation**

1. Extract the 8 remaining inline branches into step components, continuing the work begun in 09a1e1e. Then dispatch through `const STEPS: { [S in FirstRunStateName]: ComponentType<StepProps> }`, where each component owns its sub-variants: WhoStep covers the device scan, the CLI profile chooser and the method choice; AccountStep covers managedLocal; VerificationStep handles a missing profile. Because the record must cover every state, tsc fails until a new state has a step. Alternatively, use a `stepFor(checkpoint, flags)` resolver that returns a discriminated key. The existing first-run-*.render.test.tsx files protect this extraction.
2. Define a named `SetupSession` interface: checkpoint, commit, send, mounted, bridge, agentReady, a single `busy: 'server-check' | 'discovery' | 'account' | 'phrase' | null` replacing the four busy sources, message and setMessage. Provide it through a context with its own hook, for example useSetupContext(). Do not name it useSetupSession, because screens/first-run/use-setup-session.ts already exports that name; fold that hook's restart and re-enter actions into the context instead. Replace the ReturnType picks and setter parameters with the context hook.
3. Add useSetupSecrets(). It owns the five secret fields, clearBackup(), secretsHeld and clear(). The navigation guard and the Cancel/Restart actions read it.
4. FirstRunSession is left as the provider, sidebar, main frame and readiness blocker.

The checkpoint format and storage key do not change. This can land before or with any feature-folder move.

<details><summary>Verifier note</summary>

Most quoted facts check out:
- FirstRunSession spans lines 208-2188 of a 2,188-line file and contains exactly 24 useState calls.
- The `content =` chain from line 1084 to line 1965 has exactly 16 branches.
- clearSecrets at line 755 resets the five secret states and secretsHeld, and also calls clearBackup().
- useAccountOperations takes exactly 13 parameters, typed as Pick<ReturnType<typeof useFirstRunController>, ...> plus setBusy, setMessage and setConnectionErrors.
- first-run-controller.ts holds pure functions at the root beside use-first-run-controller.ts.
- busy is derived from four sources at lines 363-371.

Corrections:
- The latest commit (09a1e1e, 'Separate onboarding recovery ownership and step views') already extracted step views. 8 of the 16 branches now render ServerAddressStep, ServerVerificationStep, AccountSetupStep, RecoveryStep, LocalCompleteStep, WaitingForTeamStep and SetupChecklistStep. So 'extract protect/phrase' is already done, and the decomposition is in progress.
- Dispatch is not keyed on state alone. 'who' branches on goChooserOpen, goDiscovery and goScanError; 'account' on checkpoint.managedLocal; 'checked'/'compare' on whether profile is set. A plain state-keyed record needs per-state components that handle their sub-variants.
- screens/first-run/use-setup-session.ts already exports a different useSetupSession (restart and re-enter actions), so the proposed name collides.

The core maintainability claim holds and ISSUES.md does not track it.

</details>

### fe-arch-copy-action

**Use one clipboard-copy action; four first-run copy buttons silently drop failures**

- Type: correctness-risk
- Priority: medium
- Effort: S
- Layers: desktop-ui, tooling
- Verification: adjusted

bridge.copyText is called from 16 sites in 13 files with at least six error policies:
- groups-screen.tsx has a private useCopyText hook with try/catch.
- servers, teams, devices and settings use `.then(toast).catch(onError)`.
- team-info, go-profile-connect and blocking-shell use `.catch(onError)` with no confirmation.
- chat/message-text shows its own 'Could not copy' status or a warning toast.
- invite-new-user-sheet sets an inline error.
- Four first-run call sites run `void bridge.copyText(value).then(() => toasts.show(...))` with no rejection handler.

When the native copy fails, through a clipboard write error, text over 1 MB, or an agent or transport error, those four sites produce an unhandled promise rejection and show nothing. The user believes the setup command or message was copied.

The `void` prefix exempts them from the enabled no-floating-promises lint rule. The bridge reports only agent-session errors to readiness listeners, and no global unhandledrejection handler exists.

**Evidence**

- [`apps/desktop/src/screens/first-run-screen.tsx:1180`](../../../apps/desktop/src/screens/first-run-screen.tsx#L1180): onCopyCommand calls copyText(...).then(toast) with no catch. The same pattern appears at 1584-1588, 1939-1943 and 1959-1961.
- [`apps/desktop/src/screens/groups-screen.tsx:145`](../../../apps/desktop/src/screens/groups-screen.tsx#L145): Private useCopyText with try/catch, not shared.
- [`apps/desktop/src/screens/servers-screen.tsx:352`](../../../apps/desktop/src/screens/servers-screen.tsx#L352): Inline copyText().then(toast).catch(onError). devices-screen.tsx:372, teams-screen.tsx:489 and settings-screen.tsx:513 do the same.
- [`apps/desktop/src/chat/message-text.tsx:44`](../../../apps/desktop/src/chat/message-text.tsx#L44): Its own policies: a 'Could not copy' status at 44 and a warning-tone toast at 207.
- [`apps/desktop/src/components/invite-new-user-sheet.tsx:219`](../../../apps/desktop/src/components/invite-new-user-sheet.tsx#L219): Routes copy failure to an inline setError.
- [`apps/desktop/src-tauri/src/commands/vault.rs:1223`](../../../apps/desktop/src-tauri/src/commands/vault.rs#L1223): copy_text requires the main window and rejects text over 1 MB or a failed clipboard write. It does not check the app lock.
- [`eslint.config.mjs:50`](../../../eslint.config.mjs#L50): no-floating-promises is enabled, but `void`-prefixed promises are exempt.

**Recommendation**

Add useCopyAction(onError?) in components/ (later ui/), returning `(text, confirmation) => Promise<boolean>`. On failure it shows a warning-tone toast, matching chat/message-text, or calls the supplied handler.

Replace the 16 bridge.copyText call sites with it, including groups-screen's private useCopyText. Keep chat/message-text's injected Pick<Bridge,'copyText'> for testability by letting the hook accept the copier.

Add an ESLint no-restricted-syntax rule (selector CallExpression[callee.property.name='copyText']) that forbids the call outside that hook and bridge/.

Add a render test that makes the mock copyText reject in the setup checklist step and asserts that a warning toast appears.

<details><summary>Verifier note</summary>

The core claim holds. The four first-run call sites (1179-1181, 1584-1588, 1939-1943, 1959-1961) use `void bridge.copyText(...).then(toast)` with no rejection handler. The `void` operator exempts them from the enabled @typescript-eslint/no-floating-promises rule. checked() reports only agent-session errors to readiness listeners, so a clipboard failure is silent. No unhandledrejection handler exists. groups-screen.tsx:145 has a private useCopyText hook with try/catch.

Corrections:
- There are 16 bridge.copyText call sites in 13 files, not 14.
- There are more than three error policies: a try/catch hook; .then(toast).catch(onError) in servers, teams, devices and settings; .catch(onError) with no confirmation in team-info, go-profile-connect and blocking-shell; a status line or warning toast in chat/message-text; setError in invite-new-user-sheet; a wrapped async operation in invitation-activity-sheet; and no handler in the four first-run sites.
- 'App locked' is not a cause. The native copy_text command does not call require_unlocked. It fails on a clipboard write error, text over 1 MB, a non-main window, or a transport or agent error.

The recommendation is feasible.

</details>

### fe-arch-css-structure

**Split app.css by feature, bring team-info.css into the cascade layers, and auto-discover stylesheets in the style test**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: desktop-ui, tooling
- Verification: confirmed

styles/app.css is 2,153 lines and is declared as the `components` layer, but about 900 of those lines are first-run/setup styles (lines 686-1600), alongside group, roster, settings, account and rail-color rules. styles/shell.css (912 lines) is kept in compressed one-line-per-rule form through .prettierignore, with 35 lines over 200 characters, so diffs to it are unreadable. team-info.tsx imports team-info.css outside any @layer, so its rules override every layered rule, including the `overrides` layer. styles-geometry.test.ts lists stylesheets by hand and leaves team-info.css out, so it also skips the token-only-color and 12 px type-floor checks. One- and two-letter global classes (.t used 83 times, .ic 82, .fn 62, plus .v, .a, .k) are shared across features through descendant selectors.

**Evidence**

- [`apps/desktop/src/styles/app.css:686`](../../../apps/desktop/src/styles/app.css#L686): 'Onboarding flow layout overrides': first-run rules run to about line 1600 inside the components layer
- [`apps/desktop/src/styles/index.css:1`](../../../apps/desktop/src/styles/index.css#L1): Declares @layer tokens, base, components, features, overrides; team-info.css is not imported here
- [`apps/desktop/src/screens/team-info.tsx:32`](../../../apps/desktop/src/screens/team-info.tsx#L32): import './team-info.css' puts the sheet outside the layers, so it outranks overrides
- [`apps/desktop/tests/styles-geometry.test.ts:11`](../../../apps/desktop/tests/styles-geometry.test.ts#L11): Hand-written STYLES list omits team-info.css
- `prettierignore:9`: shell.css excluded to keep its compressed formatting

**Recommendation**

1. Import team-info.css from index.css as `layer(features.teams)`. Change styles-geometry.test.ts to glob src/**/*.css and kit/**/*.css, and to fail on any .css import outside main.tsx and index.css.
2. Move feature rules out of app.css into features/<feature>/<feature>.css, registered as features.first-run, features.teams and features.settings. Keep in app.css only the rules for components/ primitives (menus, card-select, segmented controls, spinner).
3. Reformat shell.css once with prettier in a formatting-only commit, and drop it from .prettierignore.
4. Give classes in new or moved feature CSS a feature prefix (.fr-, .tm-, ...) instead of adding more short global names. Rename existing short classes only when their feature moves.

<details><summary>Verifier note</summary>

Verified. app.css is 2,153 lines and index.css:5 imports it as layer(components). Line 686 begins 'Onboarding flow layout overrides', and setup and first-run sections follow through about 1600. That range also includes account-settings panels (1132), the key list heading (1236) and the button spinner (1493), so 'about 900 lines' of first-run styles is an approximation. shell.css is 912 lines with exactly 35 lines over 200 characters, and .prettierignore:9 excludes it. team-info.tsx:32 imports team-info.css directly. Unlayered normal declarations outrank every @layer, including overrides (native-window.css), so the cascade claim is correct. styles-geometry.test.ts:11-19 lists 7 sheets and omits team-info.css, which therefore skips the token-only-color and 12 px checks; it currently passes both by inspection. Selector counts in the CSS match: .t 83, .ic 82, .fn 62. ISSUES.md does not track this. The recommendation is feasible: native-window.css has no team-info rules, so layering it as features.* does not change current rendering.

</details>

### fe-arch-details-panel-plaintext-reducer

**Make DetailsPanel's plaintext lifecycle one reducer instead of four hand-maintained reset paths**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

DetailsPanel keeps revealed values and edit drafts in 10 useState and 15 useRef calls, and at least six code paths clear different subsets of them:
- the readProblem effect (381): read and editPasswordShown;
- the key/scope/generation effect (457-485): read and editPasswordShown, plus the editor fields, editError and binaryFile unless the draft is still bound to the same scope and generation;
- useConcealOnInactive (495): read and editPasswordShown, and sets editContentConcealed only for non-login text items;
- the accessGeneration effect (512): a read still loading;
- the concealSignal effect (518): read, editValue, baseline, target, editing, editPasswordShown, editContentConcealed and replacementPath, but not editError or binaryFile;
- clearEdit (586): the editor fields, but not read.

editTarget, editScope and editGeneration are assigned together in four places.

The differences are largely intentional, and the fields some paths skip carry no plaintext, so no current leak is shown. The risk is maintenance. Book chapter 20 treats the renderer as untrusted, yet the plaintext-bearing fields (read.value and editValue) are not declared in one place, so a new plaintext field must be added to each path by hand. Render tests cover focus-loss concealment of drafts, but nothing checks the reset sets as a unit.

**Evidence**

- [`apps/desktop/src/screens/details-panel.tsx:303`](../../../apps/desktop/src/screens/details-panel.tsx#L303): State declared at 303-320: read, editing, editValue, editPasswordShown, editContentConcealed, saving, replacementPath, dropHover, editError and binaryFile. Refs at 279-369, including editBaseline, editTarget, editScope and editGeneration.
- [`apps/desktop/src/screens/details-panel.tsx:381`](../../../apps/desktop/src/screens/details-panel.tsx#L381): readProblem effect: clears read and editPasswordShown. This sixth path is not mentioned in the original finding.
- [`apps/desktop/src/screens/details-panel.tsx:457`](../../../apps/desktop/src/screens/details-panel.tsx#L457): key/scope/generation effect: keeps a draft bound to the same scope and generation and shows a 'changed on the server' notice. Otherwise it clears the editor fields, editError and binaryFile.
- [`apps/desktop/src/screens/details-panel.tsx:495`](../../../apps/desktop/src/screens/details-panel.tsx#L495): useConcealOnInactive clears read and editPasswordShown. It sets editContentConcealed only for non-login, non-File, non-binary items.
- [`apps/desktop/src/screens/details-panel.tsx:518`](../../../apps/desktop/src/screens/details-panel.tsx#L518): concealSignal effect clears read, editValue and the editor fields, but leaves editError and binaryFile. Neither holds plaintext.
- [`apps/desktop/src/screens/details-panel.tsx:586`](../../../apps/desktop/src/screens/details-panel.tsx#L586): clearEdit clears the editor fields but not read.
- [`apps/desktop/src/screens/details-panel.tsx:541`](../../../apps/desktop/src/screens/details-panel.tsx#L541): editTarget/editScope/editGeneration are assigned together at 541-543, 711-713, 740-742 and 754-756 (four places).

**Recommendation**

Extract a useItemSecrets() hook built on useReducer.
- State: `{ read: Idle | Loading | Shown(value) | Error, editor: null | { value, baseline, binding: {item, scope, generation}, concealed, passwordShown, replacementPath, binary, error } }`.
- Actions: reveal-start, reveal-done, reveal-fail, read-blocked (the readProblem path), generation-changed, conceal-inactive, conceal-signal, open-editor, restore-draft, close-editor and item-changed.
- item-changed must keep an editor whose binding still matches the new scope and generation, setting the 'changed on the server' notice, and clear it otherwise.
- Make the binding triplet a single object and use one EMPTY constant.

Unit-test the reducer in node, without the DOM harness. The assertions, with a declared PLAINTEXT_FIELDS list, are:
- After conceal-signal, and after an item-changed whose binding no longer matches, no plaintext field holds a non-empty string.
- conceal-inactive keeps unsaved drafts but marks them concealed, or hides the password for login items.
- close-editor clears the editor.

Leave readOnce/assertExactRead and the epoch and generation guards in the hook unchanged. Then split the view into DetailsHeader, ValueView and EditorView components.

<details><summary>Verifier note</summary>

These counts check out:
- 10 useState and 15 useRef calls.
- The concealSignal effect (line 518) omits editError and binaryFile.
- clearEdit (line 586) does not clear read.
- useConcealOnInactive (line 495) clears only read and editPasswordShown, and sets editContentConcealed only for non-login text items.
- Book chapter 20 treats the renderer as untrusted.

Corrections:
- The binding triplet is assigned in 4 places (541-543, 711-713, 740-742, 754-756), not 5.
- The concealSignal effect clears far more than read and editPasswordShown: editValue, baseline, target, editing, editContentConcealed and replacementPath as well.
- There are at least six reset paths, not four. The readProblem effect (381) and the accessGeneration loading effect (512) are missing from the count.
- The fields omitted from the conceal path (editError, a message string, and binaryFile, a boolean) hold no plaintext.
- The divergent reset sets are mostly intentional: focus loss keeps unsaved drafts, and a version-only refresh keeps a still-bound draft and shows 'changed on the server'.

No actual plaintext leak is demonstrated, so this is a maintainability finding, not a correctness risk; I lowered it to medium. The proposed test assertion that no field holds a non-empty string after scope-changed would contradict the intentional retention of a still-bound draft.

</details>

### fe-arch-device-storage

**Register webview localStorage keys in one module and purge profile-scoped entries when the device is reset**

- Type: correctness-risk
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Fourteen modules in src read or write localStorage directly. Each uses its own key naming, and most have their own try/catch; first-run-recovery.ts has none and throws on bad data. Keys include 'appearance' (also hard-coded in public/theme-init.js), 'railColor', 'sidebarWidth', 'filesSidebarWidth', 'chatSidebarWidth', 'sideCollapsed', 'filesView' and 'onboarding.dismissed.*', plus the namespaced keys 'foks.first-run.v2', 'foks.setup-recovery.v1' and 'foks.invitationLabels'. Five modules read the first-run checkpoint key directly. The native reset_server command already forgets the profile's chat-local state (servers.rs:1183). Neither 'Reset this device' nor 'Remove server' removes the webview records keyed by profile: invitation labels keyed `${profile}/${account}/${operationId}` (user-typed invitee labels), and retained setup checkpoints holding host, profile and alias. A retained checkpoint is reconciled again if setup is rerun for the same target. Nothing records which keys are profile-scoped, so neither flow can find them.

**Evidence**

- [`apps/desktop/src/appearance.ts:4`](../../../apps/desktop/src/appearance.ts#L4): KEY = 'appearance'; the same literal is hard-coded in apps/desktop/public/theme-init.js:5
- [`apps/desktop/src/invitation-labels.ts:9`](../../../apps/desktop/src/invitation-labels.ts#L9): 'foks.invitationLabels' stores user-typed labels keyed by profile/account/operation; only forgetInvitationLabel on cancel removes them
- [`apps/desktop/src/first-run-recovery.ts:7`](../../../apps/desktop/src/first-run-recovery.ts#L7): 'foks.setup-recovery.v1' retains setup checkpoints (host, profile, alias); no try/catch; cleared only by clearRetainedSetups in use-setup-session.ts:203
- [`apps/desktop/src/screens/first-run/use-account-operations.ts:146`](../../../apps/desktop/src/screens/first-run/use-account-operations.ts#L146): A retained setup matching host/profile/alias is resurrected via onReplaceSession, even after that profile was reset
- [`apps/desktop/src/screens/settings-screen.tsx:975`](../../../apps/desktop/src/screens/settings-screen.tsx#L975): ResetMacSheet calls resetServer for each server; no local purge follows
- [`apps/desktop/src-tauri/src/commands/servers.rs:1183`](../../../apps/desktop/src-tauri/src/commands/servers.rs#L1183): reset_server already calls chat_local::forget_profile, the native-side counterpart of the missing webview purge
- [`apps/desktop/src/screens/servers-screen.tsx:1230`](../../../apps/desktop/src/screens/servers-screen.tsx#L1230): removeServerAndCredentials is a second profile-removal path that also leaves webview records
- [`apps/desktop/src/app/vault-shell.tsx:105`](../../../apps/desktop/src/app/vault-shell.tsx#L105): One of five direct reads of FIRST_RUN_CHECKPOINT_KEY (others in use-first-run-controller.ts:98, first-run-operations.ts:79, first-run/use-setup-session.ts:102, mock-bridge.ts:321)

**Recommendation**

Add src/platform/device-storage.ts:
- `definePref<T>({ key, scope: 'device' | 'profile', decode, fallback })` and `defineRecord<T>` for JSON maps keyed by profile.
- usePref, built on useSyncExternalStore and the 'storage' event.
- `purgeProfiles(profiles)`.

Migrate the fourteen modules. Move new keys under 'foks.ui.*' and read each legacy key once during migration.

Call purgeProfiles([profile]) after each successful resetServer in ResetMacSheet, and after a successful removeServerAndCredentials in servers-screen. This matches what reset_server already does for chat-local state on the native side. The sheet lists the local data that will be removed.

Add a test that greps src for localStorage access outside device-storage.ts, and a test that asserts theme-init.js uses the registered appearance key.

<details><summary>Verifier note</summary>

The core claim holds. ResetMacSheet (settings-screen.tsx:970-990) calls bridge.resetServer for each server and then only onDone/onError; nothing in src clears 'foks.invitationLabels', 'foks.setup-recovery.v1' or 'foks.first-run.v2'. clearRetainedSetups is called only by 'start setup over' (use-setup-session.ts:203), and forgetInvitationLabel only when a single invitation is cancelled. The native reset_server command already calls chat_local::forget_profile (src-tauri/src/commands/servers.rs:1183), so the project already purges profile-scoped local data on reset, but only on the native side. The stale records matter in two ways. Invitation labels hold user-typed invitee labels that stay on the Mac after 'Reset this device'. A retained setup for the same host/profile/alias is resurrected by use-account-operations.ts:146-156 when setup is run again. Corrections: fourteen src modules touch localStorage directly, not eleven (vault-shell, mock-bridge and use-setup-session are missing from the count). Not every module has its own try/catch: first-run-recovery.ts reads and writes with none and throws. removeServerAndCredentials (servers-screen.tsx:1230) is a second path that needs the same purge. ISSUES.md and the book do not track this. All key names, the five direct readers of FIRST_RUN_CHECKPOINT_KEY, and the duplicated 'appearance' literal in theme-init.js:5 are verified.

</details>

### fe-arch-feature-folders

**Move the flat src/ root and mixed screens/ into feature folders with enforced layering**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: desktop-ui, docs, tooling
- Verification: adjusted

src/ has 61 files at its root (60 modules plus lucide-icons.d.ts) beside 13 subfolders, and each feature is spread across several places.
- First-run spans 8 root modules (first-run-*.ts, use-first-run-controller.ts), 5 screens/first-run-*.tsx files, 17 files in screens/first-run/, and first-run selectors interleaved through roughly app.css lines 686-1640.
- Invitations span 5 root modules plus 5 files in components/: invitation-activity-sheet, invitation-panel (798 lines), invite-new-user-sheet, membership-requests and unfinished-activity.
- chat-contract.ts, chat-limits.ts and chat-mock.ts sit at the root beside src/chat/.
- screens/ mixes views with non-UI models that other layers import. device-cache.ts imports screens/device-model and operation-queries.ts imports screens/group-model. app/, shell/ and chat/ modules (chat-thread.tsx, outgoing-row.tsx) also import screens/.
- components/ mixes generic primitives, exported via index.ts, with domain panels.
- The README Layout table names files-screen.tsx and invite-sheet.tsx, which no longer exist. It omits app/, navigation/, chat/, scheduling/, diagnostics/, commands/ and resources/.

Boundary enforcement is partial:
- module-boundaries.test.ts runs cycle and facade checks only for bridge/ and navigation/, plus a check on which bridge APIs resources/ may call.
- react-boundary.test.ts keeps model/ pure and confines Tauri, mock-bridge and fixture imports.
- Nothing enforces layering between screens/, components/, shell/, app/ and the root modules.
- kit/README.md says the kit must not depend on application state, and no test checks it.

**Evidence**

- [`apps/desktop/src/device-cache.ts:16`](../../../apps/desktop/src/device-cache.ts#L16): Root data module imports DeviceLists and NO_DEVICES from ./screens/device-model.
- [`apps/desktop/src/operation-queries.ts:22`](../../../apps/desktop/src/operation-queries.ts#L22): Root query module imports manageReason from ./screens/group-model.
- [`apps/desktop/src/screens/device-model.ts:13`](../../../apps/desktop/src/screens/device-model.ts#L13): Imports types from '../bridge'. Moving the file into model/ unchanged would fail the model-purity test.
- [`apps/desktop/tests/react-boundary.test.ts:70`](../../../apps/desktop/tests/react-boundary.test.ts#L70): The existing model/ purity test forbids DOM access, react, and imports from ../bridge, ../fixture or ../mock-bridge.
- [`apps/desktop/tests/module-boundaries.test.ts:63`](../../../apps/desktop/tests/module-boundaries.test.ts#L63): Cycle and facade checks run only for ['bridge', 'navigation']. Line 109 restricts the resources/ bridge API. Line 74 ignores non-relative specifiers.
- [`apps/desktop/src/components/index.ts:1`](../../../apps/desktop/src/components/index.ts#L1): The barrel exports only generic primitives. The folder also holds invitation-panel (798 lines), membership-requests, bot-panel, admin-panel and sso/.
- [`apps/desktop/README.md:290`](../../../apps/desktop/README.md#L290): The Layout table names files-screen.tsx and invite-sheet.tsx, which do not exist. app/, navigation/, chat/ and scheduling/ are not listed.
- [`apps/desktop/src/query-repository.ts:1`](../../../apps/desktop/src/query-repository.ts#L1): 2-line alias facade (QueryRepository = MetadataRepository) used only by tests. query-hooks.ts:41-42 carries matching aliases, used only by tests.
- [`eslint.config.mjs:36`](../../../eslint.config.mjs#L36): Desktop lint block. No import-layering rule exists. Core no-restricted-imports supports per-files patterns but has no 'zones' option.

**Recommendation**

Target layout:
- src/app/ and src/shell/: unchanged.
- src/platform/: bridge/, diagnostics/, scheduling/, device-storage and appearance.
- src/model/: pure domain logic, plus group-model, team-members and scope moved out of screens/. device-model goes to data/ unless the bridge types it uses are first re-homed, because the model purity test in react-boundary.test.ts forbids '../bridge' imports, type-only ones included.
- src/data/: metadata-repository, query-hooks, query-read-recovery, catalog-*, device-cache, device-metadata, operation-queries, desktop-reconciliation, refresh-activity, roster-staleness, mutation-recovery, operation-outcome and commands/.
- src/navigation/: absorbs location.ts and navigation-guard.tsx.
- src/ui/: the generic components/ primitives. apps/desktop/kit/ stays outside src as the zero-dependency kit.
- src/features/{first-run, files, teams (with invitations, sso, bots), devices, settings (with servers, account), chat}: each holds its screens, sheets, model.ts and CSS.
- A mocks/ folder for mock-bridge, fixture and invitation-mock. These also back the VITE_FOKS_MOCK web mock, so they are not test-only. Update the hardcoded paths in react-boundary.test.ts and the dynamic-import regex for bridge/selection.ts.

Order:
1. Add layering rules as files-scoped ESLint blocks using core no-restricted-imports patterns, with no new dependency. Core no-restricted-imports has no 'zones' option; that is eslint-plugin-import's no-restricted-paths. The rules:
   - kit cannot import src.
   - model, data and platform cannot import ui, features or app.
   - A feature imports another only through its index.ts.
   Then generalize module-boundaries.test.ts to cover every folder, and teach it to resolve the /src/* and /kit/* aliases, because it currently skips non-relative specifiers.
2. Move the non-UI screens/*.ts modules into model/ and data/. This removes the upward imports and changes no behavior.
3. Move the mocks.
4. Move features one at a time, smallest first: devices, then teams/invitations, then files, then first-run (combined with its decomposition), then chat last, coordinated with the chat owner.
5. Delete query-repository.ts and the QueryRepositoryContext/useQueryRepository aliases, updating the tests that use them. Replace the README layout table with one short README per feature folder.

Steps 1 and 2 carry most of the value and can land before any larger move.

<details><summary>Verifier note</summary>

The core claims hold. device-cache.ts:16-17 imports from ./screens/device-model and operation-queries.ts:22 imports from ./screens/group-model. app/, shell/ and chat/ modules also import from screens/. README Layout row 290 names files-screen.tsx and invite-sheet.tsx, neither of which exists, and the table omits app/, navigation/, chat/ and scheduling/. query-repository.ts is a 2-line alias used only by tests, and query-hooks.ts:41-42 holds the matching aliases. invitation-panel.tsx is 798 lines. The first-run counts (8 root modules, 5 screens/first-run-*.tsx files, 17 files in screens/first-run/) are correct. ISSUES.md does not track any of this.

Corrections needed:
- The root holds 61 files (60 modules plus lucide-icons.d.ts) and there are 13 subfolders, not 14.
- 'Only bridge/ and navigation/ have a boundary test' is false. module-boundaries.test.ts:109 restricts which bridge APIs resources/ may call. react-boundary.test.ts:70 keeps model/ pure, and its tests at :52, :91 and :103 confine Tauri, mock-bridge and fixture imports.
- Following the recommendation literally would break an existing test: device-model.ts imports types from '../bridge', which the model purity regex forbids even for type-only imports.
- kit/ lives at apps/desktop/kit, outside src.
- Core ESLint no-restricted-imports has no 'zones' option; that belongs to eslint-plugin-import's no-restricted-paths. The same rules work as files-scoped config blocks with patterns.
- module-boundaries.test.ts skips non-relative specifiers, so moving cross-feature imports to /src/* would hide them from the cycle check.
- mock-bridge also backs the VITE_FOKS_MOCK web mock, so it is not test-only, and react-boundary.test.ts hardcodes its path.

I lowered priority to medium: this is a large reorganization with no defect behind it, and it would collide with active chat and first-run work.

</details>

### fe-arch-files-list-a11y

**Give the Files list and folder tree list/tree semantics and a single tab stop**

- Type: feature-refinement
- Priority: medium
- Effort: M
- Layers: desktop-ui, tooling
- Verification: confirmed

Every file and folder row is a `<div role="button" tabIndex={0} aria-pressed>`, and file rows contain their own Copy, Reveal and Download buttons. That nests focusable buttons inside a button role, whose children assistive technology treats as presentational. Each row is a separate tab stop with no arrow-key movement, so reaching row 200 takes hundreds of Tab presses. The virtualized container (.virtual-rows) has no list or grid role, so screen readers get no row count or position. The folder tree uses two buttons per node (expand, navigate) with no tree/treeitem roles. eslint.config.mjs has no jsx-a11y rules that would flag any of this.

**Evidence**

- [`apps/desktop/src/screens/items-screen.tsx:261`](../../../apps/desktop/src/screens/items-screen.tsx#L261): File row role=button tabIndex=0 aria-pressed; nested action buttons rendered in .acts at 315
- [`apps/desktop/src/screens/items-screen.tsx:418`](../../../apps/desktop/src/screens/items-screen.tsx#L418): FolderRow role=button tabIndex=0 handles Enter/Space only
- [`apps/desktop/src/screens/items-screen.tsx:1346`](../../../apps/desktop/src/screens/items-screen.tsx#L1346): .virtual-rows container has no role, aria-rowcount or rowindex; grid view at 1354 likewise
- [`eslint.config.mjs:36`](../../../eslint.config.mjs#L36): Desktop rules add react-hooks only; no jsx-a11y plugin

**Recommendation**

List view: model it as role=grid, reusing the existing column headers (Name/Kind/Location/Size from ListHeader).
- Set aria-rowcount to the total item count and aria-rowindex on each mounted row.
- Use a roving tabindex so the list is one tab stop.
- Up, Down, Home, End, PageUp and PageDown move focus and scroll the virtual window; virtual-list.ts already computes offsets.
- Enter opens details. Shift+F10 and the ContextMenu key open the existing menu.
- Row actions become gridcells reached with Left/Right.

Card view: role=grid with aria-colcount taken from gridWindow.columns, and 2-D arrow keys.

Folder tree: role=tree with treeitem, aria-level and aria-expanded. Right/Left expand and collapse; Enter navigates. That leaves one tab stop per tree.

Add eslint-plugin-jsx-a11y (recommended config) for apps/desktop, starting at warn and raising to error once the current hits are fixed.

<details><summary>Verifier note</summary>

Verified in items-screen.tsx. The file row at 252-286 is a div with role=button, tabIndex=0, aria-pressed and aria-haspopup, and it contains real <button> children in .acts (315-324) built by action(). FolderRow at 413-426 has role=button and tabIndex=0, and its handler covers Enter/Space only. TreeRow (480-525) renders separate twist and fselect buttons inside a plain div, under <aside aria-label="Folders"> (1522), with no tree or treeitem roles. .virtual-rows (1346) and .files-grid (1355) have no role, aria-rowcount or rowindex. Items-screen has no arrow-key handling; the only arrow-key handlers are in the sidebar resizers and the search palette. eslint.config.mjs registers only react-hooks for apps/desktop, and no jsx-a11y package is installed. ISSUES.md, the book and the desktop README do not track this. The recommendation fits the existing code: virtualListWindow, gridWindow.columns and ListHeader all exist. Two notes: Shift+F10 and the ContextMenu key already open menus for file rows (272-279) and at container level for folder rows (1400-1407), so the recommendation should keep that behaviour rather than add it. A roving-tabindex grid must also keep the focused row mounted when it scrolls out of the virtual window. Effort is closer to L than M.

</details>

### fe-arch-mock-bridge-contract

**Run the mock bridge through the production transport and decoders**

- Type: testing
- Priority: medium
- Effort: L
- Layers: desktop-ui, desktop-native, ci
- Verification: confirmed

mock-bridge.ts is 1,574 lines in a single closure, imported by 64 test files and used in dev mode. It implements Bridge directly and returns already-typed values. Render tests therefore never run the runtime decoders, or the `checked`/`checkedMutation` step that sets origin, ambiguous and retryable flags on mutation failures. Mock failures are hand-built objects with ambiguous: false, so UI paths for ambiguous mutation outcomes are tested only where a test sets those flags itself. A mock response that the real decoder would reject still passes every render test. The rules for which commands are reads and which are mutations live in inline string arrays (bridge/tauri.ts for invitation actions, commands-yubikey.ts for yubikey commands), and the mock does not reuse them.

**Evidence**

- [`apps/desktop/src/mock-bridge.ts:43`](../../../apps/desktop/src/mock-bridge.ts#L43): mockBridge(snapshot): Bridge, a single closure returning typed values without decoding
- [`apps/desktop/src/mock-bridge.ts:88`](../../../apps/desktop/src/mock-bridge.ts#L88): failure() builds CommandError objects with ambiguous: false, not normalized like the native path
- [`apps/desktop/src/bridge/transport.ts:57`](../../../apps/desktop/src/bridge/transport.ts#L57): checked(): decode, origin tagging, mutation ambiguity and the agent-generation retirement that the mock bypasses
- [`apps/desktop/src/bridge/tauri.ts:28`](../../../apps/desktop/src/bridge/tauri.ts#L28): Read-only invitation actions listed inline
- [`apps/desktop/src/bridge/commands-yubikey.ts:18`](../../../apps/desktop/src/bridge/commands-yubikey.ts#L18): Read-only yubikey commands listed inline
- [`apps/desktop/src-tauri/wire-contract/inventory.json:5`](../../../apps/desktop/src-tauri/wire-contract/inventory.json#L5): Generating DTOs from Rust is recorded as deferred; this recommendation does not depend on it

**Recommendation**

1. Turn commands-*.ts into factories that take a transport `{checked, checkedMutation, invoke}`, with Tauri's invoke as the default.
2. Rewrite the mock as a wire-level `invoke(command, args) => wire JSON` handler, split by domain to mirror bridge/commands-*.ts (testing/mock/vault.ts, groups.ts, ...). It returns wire DTOs and throws wire CommandError objects. Where src-tauri/wire-contract/*.json golden records exist, use them as seed responses.
3. `mockBridge()` becomes `createBridge(mockTransport)`, so every dev-mode and render-test call goes through the real decoders and mutation normalization.
4. Add a node test that parses `tauri::generate_handler![...]` in src-tauri/src/lib.rs. It compares that list with the first-argument literals of checked/checkedMutation/invoke plus the YubiCommand command union, and asserts the two sets are equal. A renamed or unregistered command then fails CI instead of failing at runtime.

**Already tracked:** src-tauri/wire-contract/inventory.json records DTO generation as deferred. This finding adds a smaller step that does not need generation: a wire-level mock and a command-name parity test.

<details><summary>Verifier note</summary>

Verified. mock-bridge.ts is 1,574 lines and exports mockBridge(snapshot): Bridge at line 43 as a single closure. Line 88 builds failures with ambiguous: false; other paths throw plain Error or VersionMismatchError. 64 test files import the mock. selectBridge in bridge/selection.ts uses it in dev and VITE_FOKS_MOCK mode. checked() at transport.ts:57 handles decode, origin, invalid-response, mutation ambiguity and agent-request-retired, and the mock bypasses all of it. Read-only lists are inline arrays at tauri.ts:32-41 and commands-yubikey.ts:18. inventory.json:5-8 records DTO generation as deferred. No test compares tauri::generate_handler! (lib.rs:221) with the frontend command literals; the only Rust registry test covers the three core renderer permissions. Only transport.ts, commands-core.ts and commands-vault.ts import invoke. The only non-literal command name is runYubi's, which the YubiCommand union covers, so the parity test is feasible. One nuance: 'never' slightly overstates the gap. go-handoff.render.test.tsx stubs __TAURI_INTERNALS__.invoke to run one tauriBridge call, and the mock calls normalizeCommandError at line 525. Injecting a transport, as recommended, is the right route rather than stubbing __TAURI_INTERNALS__ in dev mode, because isNativeHost() checks for that global and selectBridge would then choose the native bridge. The recommendation does not affect wire compatibility, since the change is test and dev-mode only.

</details>

### fe-arch-pane-resize-unify

**Replace the shell's private sidebar-resize hook with the shared pane-resize hook**

- Type: code-quality
- Priority: medium
- Effort: S
- Layers: desktop-ui
- Verification: confirmed

The rail uses shell/use-sidebar-resize.tsx, a separate copy of the generic screens/use-sidebar-resize.tsx that backs the Files and Chat panes (through two 13-line wrappers). The copy behaves differently:
- It saves the dragged width on pointercancel and lostpointercapture, where the generic hook reverts.
- It has no Escape-to-cancel.
- It reports a fixed aria-valuemax of 240 while the actual maximum from maxWidth() can be lower.
- It ignores pointerId on move and has no aria-controls or aria-valuetext.
- It never re-clamps the stored width when the window shrinks.
It also finds its host and its CSS-variable target through closest('.app') and closest('.window').

**Evidence**

- [`apps/desktop/src/shell/use-sidebar-resize.tsx:67`](../../../apps/desktop/src/shell/use-sidebar-resize.tsx#L67): finish() always saves; wired to onPointerCancel/onLostPointerCapture at 108-109
- [`apps/desktop/src/shell/use-sidebar-resize.tsx:87`](../../../apps/desktop/src/shell/use-sidebar-resize.tsx#L87): aria-valuemax={MAX_WIDTH} although keyboard and drag clamp to maxWidth() = min(240, window - 420)
- [`apps/desktop/src/screens/use-sidebar-resize.tsx:92`](../../../apps/desktop/src/screens/use-sidebar-resize.tsx#L92): Generic finish(commit) reverts on cancel; Escape cancels at 168; ResizeObserver re-clamps
- [`apps/desktop/src/screens/use-chat-sidebar-resize.tsx:4`](../../../apps/desktop/src/screens/use-chat-sidebar-resize.tsx#L4): Thin wrapper; use-files-sidebar-resize.ts is identical apart from its constants

**Recommendation**

Give the generic hook two options: `target?: () => HTMLElement | null` (the element that receives the CSS variable) and `portal?: () => HTMLElement | null`. Describe all three panes in one table, `PANE_SIZES = { rail: {storageKey:'sidebarWidth', cssVariable:'--side-w-open', min:150, max:240, default:150, compact:150, reserved:420}, files: {...}, chat: {...} }`. Then delete shell/use-sidebar-resize.tsx and both wrappers, and move the hook into ui/.

Turn chat-sidebar-resize.render.test.tsx into a test parameterized over all three panes, covering: cancel reverts, Escape reverts, aria-valuemax after the window narrows, and double-click reset.

<details><summary>Verifier note</summary>

I verified every claim in shell/use-sidebar-resize.tsx (131 lines):
- finish() (line 67) always saves and is wired to onPointerCancel and onLostPointerCapture (108-109).
- It has no Escape handler.
- aria-valuemax is the fixed MAX_WIDTH of 240 (line 87), while drag and key handling clamp to maxWidth() = max(150, min(240, window - 420)).
- onPointerMove ignores pointerId.
- It sets no aria-controls or aria-valuetext.
- The stored width is clamped only against 240, never re-clamped when the window narrows.
- It finds its host and CSS-variable target with closest('.app') and closest('.window').

The generic screens/use-sidebar-resize.tsx (190 lines) has finish(commit) at 92 that reverts on cancel, Escape handling at 168, a ResizeObserver, aria-controls and aria-valuetext. The chat and files wrappers are each 13 lines.

The recommendation is feasible. The generic hook still needs to measure the .window element rather than its own ref, and its 0.55 reserved factor and shrinking-min semantics differ slightly from the rail's. A target option covers this. sidebar-collapse.render.test.tsx:229 would need to be adapted. ISSUES.md does not track this.

</details>

### fe-arch-render-test-harness

**Replace per-file Vite servers in render tests with one typed harness and test folders that mirror the features**

- Type: testing
- Priority: medium
- Effort: M
- Layers: desktop-ui, tooling, ci
- Verification: adjusted

59 test files each start their own Vite dev server in test.before: 55 render tests plus chat-notification, chat-text, freshness-status and search-index. toasts.render.test.tsx takes about 4 s (3.7 s test duration) for 2 tests that load only kit/toasts.tsx. Tests reach source through 504 vite.ssrLoadModule('/src/...') calls, 469 of them cast `as typeof import(...)`, and the path string and the type import can go out of sync without a type error. 14 of these files also import src modules statically, which loads two copies of the same module. For example, freshness-status.test.tsx builds RefreshActivities from the Node-loaded copy and passes it to summarizeSync from the Vite-loaded copy. That is harmless today because summarizeSync duck-types the object, but module-level state such as transport's agent generation can diverge. Two things tie the tests to Vite: the root-absolute /src/* and /kit/* aliases, which tsx does not resolve (verified), and team-info.css, the only stylesheet a component imports. All 144 test files sit flat in tests/, bridge.test.ts alone is 3,324 lines, and the npm script globs only tests/*.test.ts(x).

**Evidence**

- [`apps/desktop/tests/toasts.render.test.tsx:14`](../../../apps/desktop/tests/toasts.render.test.tsx#L14): createServer in test.before only to ssrLoadModule('/kit/toasts.tsx'); measured 3.7 s test duration, 4.1 s wall for 2 tests
- [`apps/desktop/tests/freshness-status.test.tsx:1`](../../../apps/desktop/tests/freshness-status.test.tsx#L1): Static import of RefreshActivities (instantiated at 397), passed to summarizeSync from the copy loaded by ssrLoadModule at 32; one of 14 Vite-using files with static src value imports
- [`apps/desktop/src/components/menus.tsx:9`](../../../apps/desktop/src/components/menus.tsx#L9): Imports '/kit/overlay-primitives'; under tsx 4.23 this fails with ERR_MODULE_NOT_FOUND, so tsx alone cannot replace Vite resolution (28 src files use these aliases)
- [`apps/desktop/src/screens/team-info.tsx:32`](../../../apps/desktop/src/screens/team-info.tsx#L32): Only component-level CSS import in src
- [`package.json:29`](../../../package.json#L29): test:foks-ui:fast globs apps/desktop/tests/*.test.ts and *.test.tsx only
- [`apps/desktop/tests/bridge.test.ts:1`](../../../apps/desktop/tests/bridge.test.ts#L1): 3,324-line test covering every bridge domain

**Recommendation**

1. Move team-info.css into styles/index.css (see fe-arch-css-structure), so no component imports CSS.
2. Make the aliases resolvable outside Vite. Either replace /src/* and /kit/* with package.json subpath imports (#src/*, #kit/*), which Node, TypeScript and Vite all honour, or register a short node module.register resolve hook for the test run. tsx 4.23 does not map these root-absolute tsconfig paths. Then import components statically with real types, removing ssrLoadModule and the duplicate-module hazard.
3. If some tests must keep Vite, add tests/lib/vite-harness.ts with a typed load<T>(path). Sharing one server needs --experimental-test-isolation=none (Node 22); under the default process isolation it is still one server per file.
4. Reorganize into tests/unit/<feature>/ and tests/render/<feature>/. Split bridge.test.ts by bridge/commands-*.ts domain, and change the glob to a quoted 'apps/desktop/tests/**/*.test.{ts,tsx}'.

Do this before any src restructure, so that modules moved later break at type-check time instead of failing on ssrLoadModule path strings at runtime.

<details><summary>Verifier note</summary>

The core claim holds. 59 test files call Vite createServer in test.before, and none share a harness. Measured counts: 504 ssrLoadModule calls in total, 469 of them cast `as typeof import(...)`. Exactly 14 Vite-using files also import values statically from ../src, and freshness-status.test.tsx is one of them. team-info.tsx:32 is the only component-level CSS import. bridge.test.ts is 3,324 lines. package.json:29 globs only tests/*.test.ts(x). Corrections: (a) The 59 files are not all render tests. 55 are; chat-notification, chat-text, freshness-status and search-index also start Vite. Four render tests (desktop-reconciliation, maintenance-ownership, metadata-resources, query-repository) do not start Vite. (b) Measured toasts.render.test.tsx at 3.7 s test duration and 4.1 s wall time. (c) Recommendation step 2 is wrong as written. tsx 4.23.13 does not honour the tsconfig `/src/*` and `/kit/*` paths: importing src/components/menus.tsx under tsx fails with ERR_MODULE_NOT_FOUND for '/kit/overlay-primitives', even with --tsconfig or TSX_TSCONFIG_PATH. 28 src files use these aliases, so a resolve hook or an alias change is required, not optional. (d) Step 3, "one server per process", saves nothing under node --test's default process isolation, which is already one process per file. Sharing a server needs --experimental-test-isolation=none on Node 22, or dropping Vite altogether. (e) summarizeSync reads service.activities.getSnapshot() by duck typing and has no instanceof check, so the dual-module hazard is latent rather than a current failure. Loading @tauri-apps/api under plain tsx already works, because bridge.test.ts and admin.render.test.tsx import src/bridge statically.

</details>

### fe-arch-screen-split-shared-primitives

**Split the 1,100-1,900-line screen files into model, sheets and view, and deduplicate their local primitives**

- Type: maintainability
- Priority: medium
- Effort: L
- Layers: desktop-ui, tooling
- Verification: adjusted

Besides first-run, nine screen files exceed 1,100 lines, and each mixes pure logic, sheets and views:
- groups-screen.tsx: 1,895 lines; GroupSettingsScreen alone is about 735.
- device-sheets.tsx: 1,841 lines holding 11 exported sheets.
- items-screen.tsx: 1,697 lines; ItemsScreen alone is about 1,030.
- write-workflows.tsx: 1,492 lines; workflow-state helpers plus 5 sheets and WriteOverlay.
- devices-screen.tsx (1,424), details-panel.tsx (1,332), servers-screen.tsx (1,278), account-section.tsx (1,125) and settings-screen.tsx (1,114).

shell/sync-popover.tsx keeps the 260-line pure function summarizeSync inside a .tsx view. Local copies have started to differ:
- StoreMark exists in both items-screen.tsx and details-panel.tsx.
- PartyRow exists in details-panel.tsx and groups-screen.tsx, and the two render roles differently (a local roleText vs RoleChip).
- details-panel.tsx defines its own roleText next to groups/sheet-support.tsx's roleText.
- SheetFrame and DeviceSheetFrame differ only in their icon and the dismissible prop.

The groups, devices and first-run folders already follow a per-feature split; the remaining large files have not been moved.

**Evidence**

- [`apps/desktop/src/screens/device-sheets.tsx:213`](../../../apps/desktop/src/screens/device-sheets.tsx#L213): DeviceSheetFrame duplicates servers-screen.tsx:1027 SheetFrame, apart from the icon and the dismissible prop
- [`apps/desktop/src/screens/items-screen.tsx:106`](../../../apps/desktop/src/screens/items-screen.tsx#L106): StoreMark, duplicated at details-panel.tsx:173
- [`apps/desktop/src/screens/details-panel.tsx:195`](../../../apps/desktop/src/screens/details-panel.tsx#L195): PartyRow with local roleText (defined at line 118); groups-screen.tsx:204 PartyRow uses RoleChip from groups/sheet-support.tsx, which also exports its own roleText at line 69
- [`apps/desktop/src/screens/details-panel.tsx:1`](../../../apps/desktop/src/screens/details-panel.tsx#L1): 1,332 lines; omitted from the reviewer's list of oversized screens
- [`apps/desktop/src/shell/sync-popover.tsx:285`](../../../apps/desktop/src/shell/sync-popover.tsx#L285): Pure summarizeSync (about 260 lines) inside a view module; tested via vite.ssrLoadModule in tests/freshness-status.test.tsx:25
- [`apps/desktop/src/screens/write-workflows.tsx:184`](../../../apps/desktop/src/screens/write-workflows.tsx#L184): Pure helpers conflictDraftWorkflow/initialWriteWorkflow/workflowForError/writeBlockReason mixed with NewSheet, NewFolderSheet, ExistsSheet, ConflictSheet, DeleteSheet and WriteOverlay

**Recommendation**

Apply one pattern to each feature as it moves, as screens/groups, screens/devices and screens/first-run have started to do:
- model.ts holds pure functions. summarizeSync moves to shell/sync-summary.ts; the write-workflow helpers to files/write-workflow-model.ts; resolveServerUiState, sortRoster and targetReason to their features' model.ts.
- sheets/<name>-sheet.tsx holds one sheet per file; device-sheets.tsx becomes devices/sheets/*.
- The screen view keeps only the screen.

Move StoreMark, roleText and PartyRow (with a `variant` prop) into components/. Give SheetDialog an `icon` convenience that builds the server-mark glyph, and delete SheetFrame and DeviceSheetFrame.

Add ESLint `max-lines: [warn, 800]` for apps/desktop/src/**/*.tsx so new growth is visible. Unit-test the pure model modules in node, without the DOM harness or ssrLoadModule.

<details><summary>Verifier note</summary>

The core claim holds. Verified sizes: groups-screen 1,895 lines (GroupSettingsScreen at 1160-1895, about 735 lines); device-sheets 1,841 with 11 exported sheets; items-screen 1,697 (ItemsScreen at 670-1697, about 1,030); write-workflows 1,492; devices-screen 1,424; servers-screen 1,278; account-section 1,125; settings-screen 1,114. summarizeSync runs from sync-popover.tsx:285 to about 546 and is loaded in tests through vite.ssrLoadModule (tests/freshness-status.test.tsx:25). StoreMark is duplicated (items-screen.tsx:106 takes a hue prop; details-panel.tsx:173 computes it). PartyRow diverges: details-panel.tsx:195 uses a local roleText, while groups-screen.tsx:204 uses RoleChip. DeviceSheetFrame (device-sheets.tsx:213) and SheetFrame (servers-screen.tsx:1027) differ only in their icon and the dismissible prop. ESLint has no max-lines rule. Corrections: nine screen files besides first-run exceed 1,100 lines, not eight; details-panel.tsx (1,332) was omitted. details-panel.tsx:118 also defines its own roleText(wire) alongside groups/sheet-support.tsx:69 roleText(party), a third duplicate. write-workflows holds 5 sheets plus WriteOverlay, not 6 sheets. The repo has already started this pattern (screens/groups/*-sheet.tsx, screens/devices/*-workflow.ts, screens/first-run/*), so the recommendation extends an existing direction. Not tracked in ISSUES.md.

</details>

### fe-arch-shell-services-context

**Provide shell services through context instead of passing bridge, snapshot and error handlers through every screen**

- Type: maintainability
- Priority: medium
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

Outside src/chat, component prop types declare `bridge: Bridge` about 60 times (36 files), `snapshot: AgentSnapshot` about 80 times, and an onError/onCommandError/onMutationError callback about 55 times. ScreenRouter takes 23 props and assembles a settingsProps bag. Screens pass the same handlers on to their sheets; DetailsPanelProps alone carries snapshot, bridge, accessNow, accessSession, accessTicket, accessGeneration, onCommandError and onMutationError. WorkflowProvider puts the snapshot in context, but only within account-section, devices-screen and profile-keys, so those screens hold it in both context and props. Every new sheet adds another level of passing.

**Evidence**

- [`apps/desktop/src/app/screen-router.tsx:20`](../../../apps/desktop/src/app/screen-router.tsx#L20): ScreenRouter destructures 23 props (lines 20-43, typed at 44-67) and builds settingsProps at 71
- [`apps/desktop/src/screens/details-panel.tsx:223`](../../../apps/desktop/src/screens/details-panel.tsx#L223): DetailsPanelProps carries 8 shell-service props alongside its own
- [`apps/desktop/src/workflow-context.tsx:14`](../../../apps/desktop/src/workflow-context.tsx#L14): WorkflowContext holds the snapshot, but WorkflowProvider is mounted only in account-section.tsx:522, devices-screen.tsx:607 and profile-keys.tsx:41
- [`apps/desktop/src/app/catalog-runtime.ts:95`](../../../apps/desktop/src/app/catalog-runtime.ts#L95): commandErrorRef created here, assigned in shell-runtime.ts:296, read in access-runtime.ts:107/117 and in UI-invoked useCallbacks, so useEffectEvent cannot replace it

**Recommendation**

VaultShell provides a ShellServicesContext with stable identities: { bridge, accessNow, accessTicket(), commandError, mutationError, refresh, refreshSnapshot, navigate, copy }. For commandError, expose the existing ref-backed wrapper (a stable function that calls commandErrorRef.current) rather than the raw handler. Add useShellSnapshot(), which reads the shown snapshot from a small external store through useSyncExternalStore, so leaf components subscribe without props. Lift WorkflowProvider to the shell, so its snapshot and the props snapshot are the same value.

Migrate sheets first (device-sheets, groups/*, write-workflows sheets) and screen props last. Keep purely presentational components prop-driven. Tests wrap components in a TestShell provider in tests/lib.

Do not replace commandErrorRef with useEffectEvent. The ref is shared across catalog-runtime, access-runtime and shell-runtime, and it is called from callbacks that UI handlers invoke. Effect Events may be called only from Effects in the same component. Use useEffectEvent only where a ref copy is read solely inside one component's effects.

<details><summary>Verifier note</summary>

The core holds. Outside src/chat, .tsx prop types declare bridge: Bridge 62 times (in 36 files), snapshot: AgentSnapshot 80 times, and onError/onCommandError/onMutationError 55 times. ScreenRouter builds a settingsProps bag. DetailsPanelProps (details-panel.tsx:223-256) carries snapshot, bridge, accessNow, accessSession, accessTicket, accessGeneration, onCommandError and onMutationError. No bridge or shell-services context exists; the existing contexts cover device-cache, metadata repository, navigation guard, workflow and chat. Corrections: (a) ScreenRouter destructures 23 props, not 25 (lines 20-43, types 44-67). (b) WorkflowProvider is not shell-wide. It is mounted only in account-section.tsx:522, devices-screen.tsx:607 and profile-keys.tsx:41, so the duplication is local to those screens. (c) The useEffectEvent suggestion is infeasible for the cited case. commandErrorRef is created in catalog-runtime.ts:95, assigned in shell-runtime.ts:296, and read in access-runtime.ts:107/117, catalog-runtime.ts:208 and useCallback callbacks such as recoverAgentReadiness. Those callbacks are passed to ScreenRouter and run from UI handlers. React allows an Effect Event to be called only from Effects in the same component, never passed to other hooks or components, and the installed eslint-plugin-react-hooks rule ('can only be called from Effects and Effect Events in the same component') would flag this. The ref also breaks a declaration-order cycle between the catalog and shell runtimes.

</details>

### fe-arch-code-splitting

**Lazy-load first-run, the chat views and the settings screens to shrink the 1.06 MB startup chunk**

- Type: performance
- Priority: low
- Effort: M
- Layers: desktop-ui, tooling
- Verification: adjusted

vite build emits a single 1,057 kB JS chunk (307.6 kB gzipped) plus 143 kB of CSS, and Vite prints its >500 kB chunk warning. The only dynamic import in src is the mock bridge, which production builds remove. Through the source map (in KiB), src/screens is 349, react-dom 174, src/chat 104, the src root 89 and components 76. First-run code, about 270 kB of source, is needed only until setup finishes, yet vault-shell and ScreenRouter import every screen eagerly. The static index.html placeholder paints first, so splitting does not change first paint. It only shortens the time the webview spends evaluating the bundle before React takes over from the loading placeholder. FIRST_PAINT_DEADLINE_MS is unrelated: it is a partial-catalog mount deadline that starts after the agent is ready.

**Evidence**

- [`apps/desktop/vite.config.ts:22`](../../../apps/desktop/vite.config.ts#L22): build has no manualChunks, codeSplitting option or chunk budget; it emits one 1,057 kB chunk
- [`apps/desktop/src/bridge/selection.ts:28`](../../../apps/desktop/src/bridge/selection.ts#L28): The only dynamic import in the app (mock bridge), removed in production builds
- [`apps/desktop/src/app/screen-router.tsx:9`](../../../apps/desktop/src/app/screen-router.tsx#L9): Eager imports of ChatTab, DevicesScreen, GroupSettingsScreen, ItemsScreen, SettingsScreen, TeamsScreen
- [`apps/desktop/src/app/vault-shell.tsx:53`](../../../apps/desktop/src/app/vault-shell.tsx#L53): Eager import of FirstRunExperience
- [`apps/desktop/index.html:10`](../../../apps/desktop/index.html#L10): A static .app-loading placeholder is the first paint, before the module bundle evaluates
- [`apps/desktop/src/app/app-bootstrap.ts:367`](../../../apps/desktop/src/app/app-bootstrap.ts#L367): The FIRST_PAINT_DEADLINE_MS timer starts only after the agent is ready and a partial catalog arrives; it does not cover bundle evaluation

**Recommendation**

First, measure cold start in the packaged app (test:foks-desktop:packaged). Record the time from webview navigation to React's first commit; if module evaluation is only tens of milliseconds, stop.

If splitting pays off:
- Wrap FirstRunExperience, the ChatTab view, DevicesScreen, SettingsScreen (with servers) and GroupSettingsScreen in React.lazy, with a Suspense fallback that reuses the existing loading pane.
- Keep the src/chat providers and the notification consumer eager.
- Prefetch the chat chunk after mount with requestIdleCallback.
- Update the jsdom/ssrLoadModule render tests to await lazy resolution.

The CSP's `script-src 'self'` already allows same-origin chunks. Add a bundle-size budget check to scripts/build-desktop.mjs regardless, so growth is visible.

<details><summary>Verifier note</summary>

Bundle facts verified by building to the scratchpad. There is one JS chunk of 1,056.99 kB (307.59 kB gzipped; the extra 0.04 kB is the sourcemap comment) and 143.07 kB of CSS. Source-map attribution matches the reported figures in KiB: src/screens 357 kB (349 KiB), react-dom 179 kB, src/chat 107 kB, src root 91 kB, components 78 kB. First-run source is about 269 kB. The only dynamic import is the mock bridge (bridge/selection.ts:28), which production builds remove. ScreenRouter imports every screen eagerly, vite.config.ts sets no chunking, and the CSP is script-src 'self' (tauri.conf.json:32). The performance rationale is wrong, though. FIRST_PAINT_DEADLINE_MS (app-bootstrap.ts:27) does not bound bundle evaluation or agent startup. Its doc comment calls it the wait before preloading from a partial catalog. Its setTimeout (app-bootstrap.ts:367) starts only after the agent is ready and a partial catalog arrives, long after the bundle has run. The first paint is the static .app-loading placeholder in index.html, which does not wait for the module. The gain is therefore only a shorter wait before the loading screen hands off to React, which is likely small for a 1 MB local script. Priority stays low. The render tests mount screens synchronously through jsdom and ssrLoadModule, so React.lazy would need Suspense-aware awaits there. Not tracked in ISSUES.md.

</details>

### fe-arch-mutation-action-hook

**Standardize write actions on command-policy with a shared hook instead of 33 hand-rolled busy flags**

- Type: code-quality
- Priority: low
- Effort: M
- Layers: desktop-ui
- Verification: adjusted

commands/command-policy.ts defines attemptMutation and reportMutationOutcome with explicit outcomes (applied, not-started, rejected, unknown). Only the devices, groups and servers controllers use them. Other screens call synchronizeApplied directly, and send errors to the central onMutationError handler, which reconciles using failureOutcome. Their sheets keep UI state by hand: 33 busy setters and 19 `[error, setError]` pairs repeat the same try/catch/finally, and none shows an 'unknown' outcome distinctly. ResetMacSheet has a concrete defect. Its docstring says a partial run reports how far it got. In practice, if the second of three servers fails, `done` is discarded, the error goes to onError, and all previews reload. No message names the servers that were already reset.

**Evidence**

- [`apps/desktop/src/commands/command-policy.ts:21`](../../../apps/desktop/src/commands/command-policy.ts#L21): attemptMutation with outcome classification; imported only by screens/servers/server-workflow.ts, screens/groups-screen.tsx, screens/groups/operation-controller.ts and screens/devices/operation-controller.ts
- [`apps/desktop/src/app/catalog-runtime.ts:280`](../../../apps/desktop/src/app/catalog-runtime.ts#L280): useMutationError centrally reconciles after failures via reconcileMutationFailure/failureOutcome; per-sheet UI state is still hand-rolled
- [`apps/desktop/src/screens/device-sheets.tsx:1`](../../../apps/desktop/src/screens/device-sheets.tsx#L1): 10 of the 33 hand-rolled busy flags in src
- [`apps/desktop/src/screens/settings-screen.tsx:801`](../../../apps/desktop/src/screens/settings-screen.tsx#L801): Docstring: 'A run that fails part way says how far it got rather than pretending the rest happened'
- [`apps/desktop/src/screens/settings-screen.tsx:983`](../../../apps/desktop/src/screens/settings-screen.tsx#L983): The catch discards `done` (computed at 970-980), calls onError and load(); no partial-progress message

**Recommendation**

Fix ResetMacSheet now, independently:
- Record an outcome for each server (reset, failed with reason, not attempted).
- On a partial failure, call onRefreshSnapshot and report, for example, 'Reset 1 of 3 servers; <server> failed: …'.
- Re-preview only the servers that were not reset, so they remain retryable.

Then add useMutationError-compatible useMutationAction({ policy, write, onApplied }), returning { run, busy, outcome, error } and built on attemptMutation. It shows an 'unknown' outcome as its own state with no retry button, and applies the useSheetGuard refusal while busy. Migrate sheets, starting with device-sheets.tsx, as each feature moves to its folder.

<details><summary>Verifier note</summary>

The core claim holds. attemptMutation and reportMutationOutcome (command-policy.ts:21-53) are imported only by servers/server-workflow.ts, groups-screen.tsx, groups/operation-controller.ts and devices/operation-controller.ts. 33 busy-state setters (`setBusy]`) are verified across src, with 10 in device-sheets.tsx alone. The ResetMacSheet defect is confirmed: `done` is computed (settings-screen.tsx:970-980) and discarded in the catch at 983-987, which calls onError and load(). The sheet's own docstring (settings-screen.tsx:801-802) promises 'A run that fails part way says how far it got rather than pretending the rest happened'. The code breaks that promise in a destructive flow. Corrections: there are 19 `[error, setError]` pairs, not 24. Outcome handling is not wholly ad hoc: other screens use synchronizeApplied, and sheet errors go through the central useMutationError/reconcileMutationFailure (catalog-runtime.ts:280, mutation-recovery.ts:16), which applies failureOutcome. The missing piece is the per-sheet UI state for 'unknown' outcomes and progress across multiple steps, not the classification itself. The ResetMacSheet fix is a self-contained correctness bug and should not wait for the hook migration. Not tracked in ISSUES.md.

</details>
