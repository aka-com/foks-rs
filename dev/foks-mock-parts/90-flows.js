  /* ---------------------------------------------------------- 90-flows.js
     The walkthroughs. Each `M.flow` is an ordered list of steps; a step names
     a registered page, a `set` patch and the one sentence the deck prints
     under the step list. Nothing here renders anything: every state a step
     lands on is a page + view-state combination another part already
     registered, so a step is only ever a coordinate.

       app keys      bare       ('agent', 'acme', 'lock', 'boot', 'managed')
       view keys     'v.'-prefixed

     `group` puts a flow under one of the deck's four headings — `First run`,
     `Main app`, `Groups`, `Settings & servers`. The core renders the headings
     in first-declared order; a flow without one falls into `Other`.

     Key names are the ones the owning part DECLARES, never a synonym: the
     shared vocabulary is `applied` (a mutation already run), `typed`/`inspect`/
     `written`/`advanced` = `'1'`, `empty` for an unreachable empty list, and
     the per-part keys README.md lists. A step that names a key no page
     declares is silently dropped on the next navigation, so it must match.

     `applyStep` (10-core.js:205) clears every view key before it patches,
     so a step is self-contained and `?flow=<id>&step=<n>` reproduces it on a
     cold load. App-wide keys survive a step, so `base()` below puts them back
     to their defaults on the steps that want the ordinary world, and the two
     flows that live in another world (lease-lapsed, agent-lost-and-lock) name
     them on every step instead.

     `reset:false` is used only where a step must inherit the previous step's
     view state; nothing here needs it, because every step names its own.

     Sources: map/first-run.md §6 (a–f), map/groups.md §6 (F1–F8),
     map/settings.md §7 (F1–F17), map/shell.md §9 (1–8).                    */

  (function () {
    /* The ordinary world: agent ready, Acme ok, unlocked, booted, unmanaged.
       Spread first so a step can override any of them. */
    function base(extra) {
      var out = { agent: 'ready', acme: 'ok', lock: 'off', boot: 'ok', managed: 'none', refreshing: 'no' };
      Object.keys(extra || {}).forEach(function (k) { out[k] = extra[k]; });
      return out;
    }
    /* Travel Mac's device id as the Macs pane lists it (70-settings.js:211:
       '04' + id_hex.slice(2)) — the Remove sheet is keyed by it. */
    var TRAVEL_MAC = '04c1e08d5f3a94b7d21e6f0c8a3b5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809';

    /* ==================================================== first run (a) */
    M.flow({
      id: 'first-run-own',
      group: 'First run',
      title: 'First run — your own account',
      steps: [
        { label: 'Initializing this Mac', page: 'fr-who', set: base({ agent: 'starting', 'v.init': 'initializing', 'v.path': 'own' }),
          note: 'First run opens on the fork while the local client state initialises in the background: Continue reads Initializing... and the pill says Agent starting. There is no separate preflight step any more.' },
        { label: 'How are you joining?', page: 'fr-who', set: base({ 'v.choice': 'own' }),
          note: 'Picking “I’m starting on my own” reveals what happens next and arms Continue.' },
        { label: 'Select a server', page: 'fr-address', set: base({ 'v.result': 'new' }),
          note: 'The address is prefilled from the fixture; nothing about you has been sent yet.' },
        { label: 'Server did not answer (branch)', page: 'fr-error', set: base({ 'v.path': 'own', 'v.refusal': 'bad' }),
          note: 'Optional branch: a typo’d address fails the probe and nothing was saved — Check again returns to the field.' },
        { label: 'Server checked', page: 'fr-checked', set: base({ 'v.result': 'new', 'v.disclosure': 'open' }),
          note: 'The host ID is pinned, and the Details block lists what this check returned — lookup name, confirmed name, chain sequence, Merkle epoch and the host ID itself.' },
        { label: 'Create an account', page: 'fr-account', set: base({}),
          note: 'Keys are made on this Mac; only their public halves are registered as rae on foks.example.net.' },
        { label: 'Save recovery phrase', page: 'fr-protect', set: base({}),
          note: 'Three ways back in, in one step: a passphrase you type, a YubiKey you can add later, and the 17-word phrase you write down now.' },
        { label: 'Write these 17 words down', page: 'fr-phrase', set: base({ 'v.path': 'own' }),
          note: 'The phrase sheet: Done stays disabled until “I have written these 17 words down” is ticked, and there is no copy button.' },
        { label: 'Phrase written down', page: 'fr-protect', set: base({ 'v.written': '1' }),
          note: 'Back on protect the phrase is committed but still held in memory, so Show my phrase stays live for another look until this step is left.' },
        { label: 'Create a group', page: 'fr-create-group', set: base({}),
          note: 'A named group can be looked up on the server later; skipping is a first-class answer.' },
        { label: 'You’re in', page: 'fr-done', set: base({}),
          note: 'The group exists, the vault is ready, and the sidebar is the ordinary shell — the STATUS heading is there but bare, because at 5 of 5 first-run-screen.tsx:352 draws no Get started row under it.' },
        { label: 'Get started (bail-out)', page: 'fr-checklist-own', set: base({ 'v.progress': '4' }),
          note: 'Quitting anywhere after the account lands here instead: four of five done, the rest resumable.' },
      ],
    });

    /* ==================================================== first run (b) */
    M.flow({
      id: 'first-run-local',
      group: 'First run',
      title: 'First run — managed local server',
      steps: [
        { label: 'Local server', page: 'frl-local', set: base({ managed: 'local', 'v.result': 'ready' }),
          note: 'A launcher-prepared profile: the sidebar is three steps and the server is already reported ready.' },
        { label: 'Create your account', page: 'frl-account', set: base({ managed: 'local' }),
          note: 'The same account step with the server settled — More options holds the email and invite fields.' },
        { label: 'Recovery', page: 'frl-protect', set: base({ managed: 'local' }),
          note: 'The managed variant folds protect and phrase into one step, collapsed to start.' },
        { label: 'Phrase shown and ticked', page: 'frl-protect', set: base({ managed: 'local', 'v.disclosure': 'open', 'v.written': '1' }),
          note: 'The 17 words inline; Start using FOKS goes live once the box is ticked.' },
        { label: 'Vault ready', page: 'frl-local-done', set: base({ managed: 'local' }),
          note: 'Three sidebar steps instead of seven, and Personal is ready to open.' },
      ],
    });

    /* ==================================================== first run (c) */
    M.flow({
      id: 'first-run-invited',
      group: 'First run',
      title: 'First run — invited to a group',
      steps: [
        { label: 'Initializing this Mac', page: 'fr-who', set: base({ agent: 'starting', 'v.init': 'initializing', 'v.path': 'invited' }),
          note: 'The same fork while the client state initialises; the invited path is picked on the next step.' },
        { label: 'How are you joining?', page: 'fr-who', set: base({ 'v.choice': 'invited' }),
          note: '“Someone invited me to their group” — you will need their server address, not a code or a link.' },
        { label: 'Ask them for the address', page: 'fri-no-address', set: base({}),
          note: 'They sent nothing? A copyable sentence to send them, and your username goes back the same way.' },
        { label: 'Select a server address', page: 'fri-address', set: base({ 'v.result': 'new' }),
          note: 'The admin’s server, foks.acme-corp.com, checked before anything about you is sent.' },
        { label: 'Server checked', page: 'fri-checked', set: base({ 'v.result': 'new' }),
          note: 'The host ID is pinned; the account you make next is the name you send the admin.' },
        { label: 'Create an account', page: 'fri-account', set: base({}),
          note: 'sol on foks.acme-corp.com — the username sam.ortiz must type exactly.' },
        { label: 'Recovery phrase written', page: 'fri-protect', set: base({ 'v.written': '1' }),
          note: 'The same protect step as the own path, with the phrase already committed.' },
        { label: 'Wait to be added', page: 'fri-waiting', set: base({ 'v.disclosure': 'open' }),
          note: 'Nothing is pushed here: Check now asks the server, and Personal is usable meanwhile.' },
        { label: 'Checked — still not listed', page: 'fri-waiting', set: base({ 'v.checked': 'yes', 'v.result': 'ambiguous' }),
          note: 'Check now ran and the admin has not added you yet: the chip reads Checked just now and the server’s answer is echoed under it.' },
        { label: 'You’re in', page: 'fri-added', set: base({ 'v.discovered': 'yes' }),
          note: 'Engineering appears under GROUPS, with the only details panel in first run beside it.' },
        { label: 'Get started', page: 'fri-checklist-invited', set: base({ 'v.progress': '4' }),
          note: '“I’ll come back later” lands here instead, with a note that the group is not listed yet.' },
      ],
    });

    /* ==================================================== first run (d) */
    M.flow({
      id: 'recover-phrase',
      group: 'First run',
      title: 'Recover from a backup phrase',
      steps: [
        { label: 'Add this as a secondary device', page: 'fr-who', set: base({}),
          note: 'The alt link under Continue skips the fork: you already have an account somewhere.' },
        { label: 'Select a server', page: 'fr-address', set: base({ 'v.returning': 'yes', 'v.result': 'new' }),
          note: 'The returning hint says you already have an account on this server, so Continue will route to Add this Mac.' },
        { label: 'Server checked', page: 'fr-checked', set: base({ 'v.returning': 'yes', 'v.result': 'same', 'v.choice': 'rae' }),
          note: 'The pinned history is unchanged; Continue goes to `existing` rather than Create an account.' },
        { label: 'Recover with the phrase', page: 'frx-existing', set: base({ 'v.path': 'own', 'v.card': '', 'v.recovery': 'typed', 'v.back': 'checked' }),
          note: 'The recover card asks for the 17 words and a name for this Mac — nothing else is needed to prove the account is yours.' },
        { label: 'Resume recovery', page: 'frx-existing', set: base({ 'v.path': 'own', 'v.card': '', 'v.pending': 'yes', 'v.back': 'checked' }),
          note: 'A pending account-recovery row relabels the button Resume recovery instead of Recover.' },
        { label: 'Save recovery phrase', page: 'fr-protect', set: base({}),
          note: 'Recovery rejoins the ordinary tail at protect, then create-group or the checklist.' },
      ],
    });

    /* ==================================================== first run (e) */
    M.flow({
      id: 'pair-from-first-run',
      group: 'First run',
      title: 'Pair a second Mac from first run',
      steps: [
        { label: 'Add this as a secondary device', page: 'fr-who', set: base({}),
          note: 'Same entry as recovery — pairing and recovery share the `existing` step.' },
        { label: 'Select a server', page: 'fr-address', set: base({ 'v.returning': 'yes', 'v.result': 'new' }),
          note: 'The server this account already lives on, checked again on this Mac.' },
        { label: 'Server checked', page: 'fr-checked', set: base({ 'v.returning': 'yes', 'v.result': 'same', 'v.choice': 'rae' }),
          note: 'Pinned history unchanged, so nothing about the server has moved since the other Mac saw it.' },
        { label: 'Accept pairing', page: 'frx-existing', set: base({ 'v.path': 'own', 'v.card': 'pair', 'v.phrase': 'typed', 'v.back': 'checked' }),
          note: 'The native bridge’s second card: the short pairing phrase is read off the Mac you already use.' },
        { label: 'Resume acceptance refused', page: 'frx-existing', set: base({ 'v.path': 'own', 'v.card': 'pair', 'v.refusal': 'resume', 'v.back': 'checked' }),
          note: 'Resume acceptance with nothing pending refuses: “That pairing acceptance is no longer pending.”' },
        { label: 'Save recovery phrase', page: 'fr-protect', set: base({}),
          note: 'A paired Mac still needs its own way back in, so it rejoins the tail at protect.' },
      ],
    });

    /* ==================================================== first run (f) */
    M.flow({
      id: 'import-go-cli',
      group: 'First run',
      title: 'Import a FOKS Go CLI profile',
      steps: [
        { label: 'Looking for accounts', page: 'frg-scan', set: base({}),
          note: 'discoverGoProfiles() runs in place of the fork while first run waits.' },
        { label: 'Choose an account', page: 'frg-chooser', set: base({ 'v.choice': '' }),
          note: 'FOKS is already set up on this Mac: three candidates, and Connect selected account is disabled.' },
        { label: 'Candidate picked', page: 'frg-chooser', set: base({ 'v.choice': 'rae' }),
          note: 'Picking a usable candidate arms the primary; “Set up another account” dismisses the chooser.' },
        { label: 'Server prefilled', page: 'fr-address', set: base({ 'v.returning': 'yes', 'v.result': 'new', 'v.choice': 'rae' }),
          note: 'The candidate’s server is filled in and marked returning, because the account already exists there.' },
        { label: 'Server checked', page: 'fr-checked', set: base({ 'v.returning': 'yes', 'v.result': 'same', 'v.choice': 'rae' }),
          note: 'checkAndAddGoProfile binds the candidate’s host ID to the profile this Mac just pinned.' },
        { label: 'Pair from the CLI', page: 'frx-existing', set: base({ 'v.path': 'own', 'v.card': 'cli', 'v.back': 'checked', 'v.choice': 'rae' }),
          note: 'The pairing card is retitled for `foks --simple-ui key assist`, and a copy card appears for a copyable device.' },
        { label: 'Save recovery phrase', page: 'fr-protect', set: base({}),
          note: 'The imported account rejoins the ordinary tail at protect.' },
        { label: 'Later, from Settings', page: 'set-account', set: base({ 'v.sheet': 'go-profile' }),
          note: 'The second entry point: Accounts › Connect from FOKS CLI…, which the mock stops at “not found in its standard location”.' },
      ],
    });

    /* ====================================================== shell §9 (1,3,4) */
    M.flow({
      id: 'browse-and-reveal',
      group: 'Main app',
      title: 'Browse, filter and reveal',
      steps: [
        { label: 'All items', page: 'all', set: base({}),
          note: 'The sidebar’s first row: every readable store at once, 14 rows in the fresh world.' },
        { label: 'Personal', page: 'store-personal', set: base({}),
          note: 'A store page is the same screen scoped to one vault; the heading carries its caption.' },
        { label: 'Passwords only', page: 'store-personal', set: base({ 'v.kind': 'Password' }),
          note: 'The kind filter is one of four independent knobs, all of them in the URL.' },
        { label: 'Sorted by kind', page: 'store-personal', set: base({ 'v.sort': 'kind' }),
          note: 'Sort is separate from the filter: name, kind, grouped by store, or version descending.' },
        { label: 'Cards', page: 'store-personal', set: base({ 'v.view': 'grid' }),
          note: 'The list/cards segmented control is the third knob and survives navigation and reload.' },
        { label: 'A password selected', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown' }),
          note: 'Selecting a row opens the details panel; the password is masked until it is asked for.' },
        { label: 'A resource selected', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/agents/anthropic-api-key', 'v.details': 'shown' }),
          note: 'A Note is a single value — an API key or a recovery code — rather than a set of fields; Resource is the kind’s name in the protocol only.' },
        { label: 'A file selected', page: 'all', set: base({ 'v.sel': 'team:household|/documents/emergency.pdf', 'v.details': 'shown' }),
          note: 'A File shows its size and a download; its bytes are streamed by the local agent, never listed.' },
        { label: 'A link selected', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/latest-key', 'v.details': 'shown' }),
          note: 'A Link is a path pointing at another path in the same store; Read target follows it.' },
        { label: 'Show', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.reveal': '1' }),
          note: 'Show is the only call that returns plaintext, bound to one exact version and dropped on Hide, reselect or agent loss.' },
        { label: 'Read target', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/ssh/id_ed25519', 'v.details': 'shown' }),
          note: 'Read target resolves the link and selects what it points at — the reveal does not carry over.' },
        { label: 'Details hidden', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'hidden' }),
          note: 'The toolbar toggle closes the panel over a still-selected row; deselecting with Escape instead would leave it open and empty.' },
      ],
    });

    /* ======================================================== shell §9 (2) */
    M.flow({
      id: 'search',
      group: 'Main app',
      title: 'Search the vault',
      steps: [
        { label: 'The search field', page: 'all', set: base({}),
          note: '⌘K focuses the field in the page header; it is always there, never a separate mode.' },
        { label: 'A query with results', page: 'all', set: base({ 'v.search': 'logins' }),
          note: 'Typing filters paths, never contents, and matching rows grow their full path.' },
        { label: 'Matching a store name', page: 'all', set: base({ 'v.search': 'Household' }),
          note: 'Store names match too, which is how one group’s items are pulled out of All items.' },
        { label: 'No results', page: 'all', set: base({ 'v.search': 'zzzz' }),
          note: 'The empty state is the search variant, not the “nothing here yet” one.' },
        { label: 'Escape clears it', page: 'all', set: base({ 'v.search': 'logins', 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown' }),
          note: 'A query typed over a selected row: Escape clears the query first and the selection second, so the first press leaves this row still selected.' },
      ],
    });

    /* ======================================================== shell §9 (5) */
    M.flow({
      id: 'create-item',
      group: 'Main app',
      title: 'Create an item',
      steps: [
        { label: 'The New menu', page: 'store-personal', set: base({ 'v.menu': 'new' }),
          note: 'One primary MenuButton — a single New button with a chevron, not a split button — and four kinds behind it: password, note, file, link. Resource is shown as Note wherever a person reads it.' },
        { label: 'New password', page: 'store-personal', set: base({ 'v.sheet': 'new-password', 'v.dest': 'acct:personal' }),
          note: 'In an account store there are no roles to choose: the item is Owner-read and Owner-write.' },
        { label: 'New note', page: 'store-personal', set: base({ 'v.sheet': 'new-resource', 'v.dest': 'acct:personal' }),
          note: 'A single value, with the path derived from the name you type.' },
        { label: 'New file', page: 'store-personal', set: base({ 'v.sheet': 'new-file', 'v.dest': 'acct:personal' }),
          note: 'Until a file is chosen the primary reads “Choose file and create”.' },
        { label: 'New link', page: 'store-personal', set: base({ 'v.sheet': 'new-link', 'v.dest': 'acct:personal' }),
          note: 'A link only needs its own path and the path it points at.' },
        { label: 'Save in', page: 'store-personal', set: base({ 'v.sheet': 'new-password', 'v.dest': 'acct:personal', 'v.menu': 'save-in' }),
          note: 'The Save in card select lists every store you can write to, defaulting to the one you are in.' },
        { label: 'New password in a group', page: 'store-eng', set: base({ 'v.sheet': 'new-password', 'v.dest': 'team:eng' }),
          note: 'A group store adds the two role rows and a preview of who would be able to read it.' },
        { label: 'Reader preview follows the role', page: 'store-eng', set: base({ 'v.sheet': 'new-password', 'v.dest': 'team:eng', 'v.readRole': 'Admin', 'v.advanced': '1' }),
          note: 'Raising the read role to Admin narrows the reader preview; Advanced shows the exact path.' },
        { label: 'Refused: something is already there', page: 'store-personal', set: base({ 'v.sheet': 'exists', 'v.dest': 'acct:personal', 'v.epath': '/logins/github.com' }),
          note: 'An item already exists at this path: nothing was created, nothing overwritten; open the existing item or choose another path.' },
        { label: 'Created', page: 'store-personal', set: base({ 'v.kind': 'Password', 'v.applied': 'created-password' }),
          note: 'Create closes the sheet, toasts “Password created in Personal” and pushes the row into this list — gitlab.com is here because `applied` replays the write; clicking Create in the frame does the same.' },
      ],
    });

    /* ==================================================== shell §9 (6, 7) */
    M.flow({
      id: 'edit-and-remove',
      group: 'Main app',
      title: 'Edit, conflict and remove',
      steps: [
        { label: 'Edit', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.sheet': 'edit' }),
          note: 'Edit turns the details panel into a textarea over the version you are looking at.' },
        { label: 'Conflict: item was updated', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.sheet': 'conflict' }),
          note: 'Saving is bound to the exact version, so a moved version refuses the write rather than merging it.' },
        { label: 'Refresh and review', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.sheet': 'edit', 'v.draft': 'flint-Harbor-19-quay' }),
          note: 'Keep editing brings the retained draft back in the editor over the refreshed version — there is no “save anyway”, and Retry never replays the write.' },
        { label: 'Discard my edit?', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.sheet': 'discard' }),
          note: 'Discarding is the only copy gone; the item itself is untouched at its current version.' },
        { label: 'Back to the item', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown' }),
          note: 'Keep editing returns to the conflict; discarding drops the draft and leaves the panel as it was.' },
        { label: 'Saved', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.applied': 'saved-github' }),
          note: 'Save writes version 10 against the version you were shown, toasts “Saved version 10” and the panel follows the new version.' },
        { label: 'Replace a file', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/documents/passport-scan.pdf', 'v.details': 'shown', 'v.sheet': 'replace' }),
          note: 'A File has no editable text, so its Edit is a Replace row that saves version n+1.' },
        { label: 'Remove', page: 'store-personal', set: base({ 'v.sel': 'acct:personal|/logins/github.com', 'v.details': 'shown', 'v.sheet': 'remove' }),
          note: 'The row trash and the panel Remove both open the same alertdialog.' },
        { label: 'Removed', page: 'store-personal', set: base({ 'v.details': 'shown', 'v.applied': 'removed-github' }),
          note: 'Remove drops the row and toasts “Removed version 9”; the panel stays open and empty — `applied` replays it, and clicking Remove in the frame does the same.' },
      ],
    });

    /* ======================================================== shell §9 (8) */
    M.flow({
      id: 'lease-lapsed',
      group: 'Main app',
      title: 'A lapsed server check-in',
      steps: [
        { label: 'All items band', page: 'all', set: base({ acme: 'lapsed' }),
          note: 'Acme’s check-in expired: All items drops to 10 rows behind a critical band and the Alerts badge goes 2 → 3.' },
        { label: 'Work takeover', page: 'store-work', set: base({ acme: 'lapsed' }),
          note: 'The store page is replaced entirely: FOKS will not list what it cannot prove it may still read.' },
        { label: 'Engineering takeover', page: 'eng-people', set: base({ acme: 'lapsed' }),
          note: 'The group hero keeps its mark and Group settings sub-line; the tabs and roster are replaced.' },
        { label: 'Alerts', page: 'alerts', set: base({ acme: 'lapsed' }),
          note: 'The critical card “foks.acme-corp.com is locked” joins the two standing notes; its button is inert.' },
        { label: 'Needs attention', page: 'srv-list', set: base({ acme: 'lapsed' }),
          note: 'Acme sits in Needs attention in either world — this list reads the profile’s own signed check-in, which is expired in the fixture — so what the lapse moves here is the sidebar and the badge.' },
        { label: 'The server detail', page: 'srv-server', set: base({ acme: 'lapsed', 'v.profile': 'acme' }),
          note: 'Check-in expired with no action to offer: only the agent can renew it, so You and Groups read as locked instead of listing anything.' },
        { label: 'Devices stopped', page: 'set-devices', set: base({ acme: 'lapsed', 'v.account': 'acct:work' }),
          note: 'The same lapse from the account side: a stop band replaces the Macs pane for that account.' },
        { label: 'Accounts', page: 'set-account', set: base({ acme: 'lapsed' }),
          note: 'Work reads “server check-in lapsed” and its passphrase buttons are disabled — the same in the fresh world, because this pane reads the account’s own check-in rather than the catalog.' },
      ],
    });

    /* ============================================== shell §8.1 (alerts) */
    M.flow({
      id: 'alerts-pane',
      group: 'Main app',
      title: 'What needs attention',
      steps: [
        { label: 'Two standing notes', page: 'alerts', set: base({}),
          note: 'The bell in the sidebar carries the count; the pane is one card per note, most severe first.' },
        { label: 'A critical one arrives', page: 'alerts', set: base({ acme: 'lapsed' }),
          note: 'A stopped server adds a crit card above them and the badge goes 2 → 3; its button is inert because only the agent can renew a check-in.' },
        { label: 'Nothing to report', page: 'alerts', set: base({ 'v.empty': 'yes' }),
          note: 'With no notes at all the pane is one line — “Nothing needs attention on this Mac.” — which the fixture alone cannot produce.' },
      ],
    });

    /* =========================================== shell §2.4–2.6 + agent */
    M.flow({
      id: 'agent-lost-and-lock',
      group: 'Main app',
      title: 'Agent loss, lock and boot',
      steps: [
        { label: 'Agent starting', page: 'all', set: base({ agent: 'starting' }),
          note: 'The titlebar pill turns amber while the agent is in Bootstrap; the shell is still usable.' },
        { label: 'Agent lost', page: 'all', set: base({ agent: 'lost' }),
          note: 'A lost socket covers everything below the titlebar: no cached plaintext, no partial screen.' },
        { label: 'Retry', page: 'all', set: base({ agent: 'ready' }),
          note: 'Retry reconnects and toasts “Connected to the local agent”, returning to whatever was showing.' },
        { label: 'App lock', page: 'all', set: base({ lock: 'locked' }),
          note: 'The lock is read before the world loads, so a locked app renders only the Unlock FOKS overlay.' },
        { label: 'Boot loading', page: 'all', set: base({ boot: 'loading' }),
          note: 'Before either of those: “Connecting to the local agent…” in place of the whole shell.' },
        { label: 'Boot error', page: 'all', set: base({ boot: 'error' }),
          note: 'A boot that throws shows “Couldn’t load FOKS” with the normalized message and Retry.' },
      ],
    });

    /* ==================================================== groups F1 / F1b */
    M.flow({
      id: 'group-create',
      group: 'Groups',
      title: 'Create a group (Settings)',
      steps: [
        { label: 'Settings › Groups', page: 'set-groups', set: base({}),
          note: 'Create a group, Needs attention when it is not empty, and Invite someone.' },
        { label: 'Create a group', page: 'set-groups', set: base({ 'v.sheet': 'create' }),
          note: 'Named is preselected and the account defaults to the work store, foks.acme-corp.com / rae.chen.' },
        { label: 'Name it', page: 'set-groups', set: base({ 'v.sheet': 'create', 'v.name': 'Ops' }),
          note: 'The slug footnote and the “Create Ops” label track the field live.' },
        { label: 'The new vault', page: 'store-platform', set: base({ 'v.created': 'platform|Platform|named|acct:work' }),
          note: 'Creating navigates to the group’s own vault: one person, no items yet.' },
        { label: 'Ad-hoc branch', page: 'set-groups', set: base({ 'v.sheet': 'create', 'v.gkind': 'adhoc' }),
          note: 'An ad-hoc group is known only by its id: the name in the field is not sent, so it cannot be looked up on the server or nested in another group. Only the radio moves — the app leaves the alias footnote and the “Create Platform” label exactly as they were (groups-screen.tsx:1620-1622, 1334).' },
      ],
    });

    /* ==================================================== groups F2–F4 */
    M.flow({
      id: 'group-members',
      group: 'Groups',
      title: 'Add, lower and remove members',
      steps: [
        { label: 'Engineering › People', page: 'eng-people', set: base({}),
          note: 'Six parties including a member group and a service account; you are Admin, so every affordance is live.' },
        { label: 'Add someone', page: 'eng-people', set: base({ 'v.sheet': 'add' }),
          note: 'The whole flow is a username — there are no invite links anywhere in FOKS.' },
        { label: 'Admin instead', page: 'eng-people', set: base({ 'v.sheet': 'add', 'v.role': 'Admin' }),
          note: 'Choosing Admin takes the visibility stepper away: it exists only for Member.' },
        { label: 'Visibility', page: 'eng-people', set: base({ 'v.sheet': 'add', 'v.visibility': '-1' }),
          note: 'Back on Member, the stepper is live and cannot exceed your own level minus one.' },
        { label: 'Added', page: 'eng-people', set: base({ 'v.applied': 'add:eng:jules.park:Member:0' }),
          note: 'The roster gains a row and the reader counts, sidebar caption and stacks all move with it.' },
        { label: 'The party panel', page: 'eng-people', set: base({ 'v.panel': 'priya.n' }),
          note: 'The row’s … opens an aside with the party’s roles, generation and id.' },
        { label: 'Lower role', page: 'eng-people', set: base({ 'v.sheet': 'demote', 'v.target': 'priya.n', 'v.role': 'Member' }),
          note: 'Only lowering is possible here; the current role is a disabled card and the sheet says so.' },
        { label: 'Lowered', page: 'eng-people', set: base({ 'v.applied': 'demote:eng:priya.n:Member:0' }),
          note: 'priya.n is now a Member at visibility 0, and what she can read narrows with the role.' },
        { label: 'Remove from the panel', page: 'eng-people', set: base({ 'v.panel': 'dana.okafor', 'v.sheet': 'remove', 'v.target': 'dana.okafor' }),
          note: 'Path A: the party panel’s Remove… — removing rekeys the group, so it is one confirmed action.' },
        { label: 'Danger zone menu', page: 'eng-settings', set: base({ 'v.menu': 'rekey' }),
          note: 'Path B: the Settings tab’s Remove… menu, ordered least-authoritative first.' },
        { label: 'Remove and rekey', page: 'eng-settings', set: base({ 'v.sheet': 'remove', 'v.target': 'dana.okafor' }),
          note: 'The same alertdialog either way; the primary reads Remove and rekey.' },
        { label: 'Removed', page: 'eng-people', set: base({ 'v.applied': 'remove:eng:dana.okafor' }),
          note: 'The row is gone and the group has been rekeyed; copies already read cannot be recalled.' },
        { label: 'Refused: not locally manageable', page: 'eng-people', set: base({ 'v.sheet': 'party-remove' }),
          note: 'A party without a unique locally-managed username — deploy-bot here — shows an amber notice and the confirm stays disabled.' },
      ],
    });

    /* ======================================================== groups F5 */
    M.flow({
      id: 'group-federation',
      group: 'Groups',
      title: 'Share with another group',
      steps: [
        { label: 'Engineering › People', page: 'eng-people', set: base({}),
          note: 'Under the roster, Groups on other servers lists the admissions — homelab’s is Inactive and its roster row is dimmed.' },
        { label: 'Add a group', page: 'eng-people', set: base({ 'v.sheet': 'admit', 'v.remote': 'household' }),
          note: 'A candidate must be a team, active, named, readable and on a different server from this group.' },
        { label: 'Visibility', page: 'eng-people', set: base({ 'v.sheet': 'admit', 'v.remote': 'household', 'v.visibility': '-1' }),
          note: 'Admitting carries a destination role and visibility, exactly as adding a person does.' },
        { label: 'Admitted', page: 'eng-people', set: base({ 'v.applied': 'admit:eng:household:0' }),
          note: 'A new Active federation row and a matching named-team party; there is no un-admit.' },
        { label: 'Restore the lapsed admission', page: 'eng-people', set: base({ 'v.applied': 'admit:eng:household:0,restore:eng:7c14a9f0' }),
          note: 'Restore access reruns homelab’s admission: the chip flips to Active and the dimmed roster row lights up.' },
      ],
    });

    /* ======================================================== groups F6 */
    M.flow({
      id: 'group-resume',
      group: 'Groups',
      title: 'Finish an incomplete group',
      steps: [
        { label: 'The Homelab vault', page: 'store-homelab', set: base({}),
          note: 'Creation stopped part-way, so the vault is replaced by a Setup incomplete notice with Finish setup.' },
        { label: 'The group page', page: 'homelab-people', set: base({}),
          note: 'The same state from Groups: a Setup incomplete band with an inline Finish setup link and no add affordances.' },
        { label: 'Settings › Groups', page: 'set-groups', set: base({}),
          note: 'The Needs attention row is the third entry point; Open lands on the vault takeover.' },
        { label: 'Alerts', page: 'alerts', set: base({}),
          note: 'The fourth: “Homelab is inactive”, whose action is Resume creation.' },
        { label: 'Active ad-hoc vault', page: 'store-homelab', set: base({ 'v.adhoc': 'yes' }),
          note: 'Finish setup toasts “Group creation resumed”: the takeover goes and Homelab is an ordinary, empty ad-hoc vault.' },
        { label: 'Its group page', page: 'homelab-people', set: base({ 'v.adhoc': 'yes' }),
          note: 'The same resumed world from Groups: an active ad-hoc group with 0 people and no Setup-incomplete band.' },
        { label: 'Its Settings tab', page: 'homelab-settings', set: base({ 'v.adhoc': 'yes' }),
          note: 'An ad-hoc group has no name to look up, so the footnote says so and Remove… stays disabled.' },
      ],
    });

    /* ======================================================== groups F7 */
    M.flow({
      id: 'group-invite',
      group: 'Groups',
      title: 'Invite someone to a group',
      steps: [
        { label: 'Settings › Groups', page: 'set-groups', set: base({}),
          note: 'Invite someone sits under Create a group; there is no link to send.' },
        { label: 'The invite dialog', page: 'set-groups', set: base({ 'v.sheet': 'join-invite' }),
          note: 'It writes the sentence for you: your username, your server, and what they should do with it.' },
        { label: 'Whose name goes in it', page: 'set-groups', set: base({ 'v.sheet': 'join-invite', 'v.account': 'acct:personal' }),
          note: 'The account picker chooses the username and server the sentence names; Copy message toasts “Message copied.” and Done closes.' },
        { label: 'Then add them by username', page: 'eng-people', set: base({ 'v.sheet': 'add', 'v.username': 'nils' }),
          note: 'When they reply with their username, the ordinary Add someone sheet finishes the job.' },
      ],
    });

    /* ==================================================== groups F8 + §7.3 */
    M.flow({
      id: 'group-second',
      group: 'Groups',
      title: 'A smaller group you own',
      steps: [
        { label: 'The Household vault', page: 'store-household', set: base({}),
          note: 'The other named group in the fixture: two people, on your own server rather than Acme’s.' },
        { label: 'Household › People', page: 'household-people', set: base({}),
          note: 'Two people and no admissions, so “Groups on other servers” is the empty callout: “No groups from other servers.”' },
        { label: 'Nobody senior to you', page: 'household-settings', set: base({}),
          note: 'You are the Owner, so Leave reads “You can’t remove yourself yet, and no one else here can remove you.” — the danger rows are all present but none of them applies.' },
      ],
    });

    M.flow({
      id: 'group-unreadable',
      group: 'Groups',
      title: 'A group page that will not load',
      steps: [
        { label: 'The roster read failed', page: 'eng-people', set: base({ 'v.failure': 'roster' }),
          note: 'A retryable failure: “Roster unavailable” with a Refresh, the add affordances gone, and the sidebar caption replaced by Roster unavailable.' },
        { label: 'The federation read failed', page: 'eng-people', set: base({ 'v.failure': 'federation' }),
          note: 'A final failure has no Refresh — only the message the agent gave — and the roster above it is unaffected.' },
        { label: 'Both failed', page: 'eng-people', set: base({ 'v.failure': 'both' }),
          note: 'Two independent reads, so both notices stand at once; the sidebar caption reports the roster, the louder of the two.' },
        { label: 'Not a group at all', page: 'group-unavailable', set: base({}),
          note: 'A stale address pointing at an account store: “This group is no longer available”, with the sidebar still there to leave by.' },
      ],
    });

    /* =================================================== settings F1–F2 */
    M.flow({
      id: 'server-add-and-check',
      group: 'Settings & servers',
      title: 'Add and check a server',
      steps: [
        { label: 'Servers', page: 'srv-list', set: base({}),
          note: 'Every server this Mac knows, with Add a server… hanging off the Ready label.' },
        { label: 'Add a server…', page: 'srv-add', set: base({}),
          note: 'The sheet is prefilled partner / foks.partner.dev, which this Mac already has: pressing Add server refuses it with the red pill “That server profile already exists.” — there is no screen for the refusal.' },
        { label: 'A new profile', page: 'srv-add', set: base({ 'v.addid': 'lab', 'v.addaddr': 'foks.lab.test' }),
          note: 'A local profile id and an address; adding toasts “check it before trusting anything on it”.' },
        { label: 'Not checked yet', page: 'srv-server', set: base({ 'v.profile': 'lab', 'v.applied': 'added' }),
          note: 'A new server has no host ID until the first check: the band offers Check now and Identity is empty.' },
        { label: 'Checked', page: 'srv-server', set: base({ 'v.profile': 'lab', 'v.applied': 'added,yes' }),
          note: 'Check now pins the host ID and refreshes the signed check-in; ⌘R repeats it on whichever server is open.' },
        { label: 'Inspect the response', page: 'srv-server', set: base({ 'v.profile': 'lab', 'v.applied': 'added,yes', 'v.inspect': '1' }),
          note: 'The raw answer is always available under a toggle — acceptance, chain length and epoch.' },
      ],
    });

    /* =================================================== settings F4–F5 */
    M.flow({
      id: 'server-forget-reset',
      group: 'Settings & servers',
      title: 'Forget a server, reset local state',
      steps: [
        { label: 'A server', page: 'srv-server', set: base({ 'v.profile': '' }),
          note: 'foks.example.net: Check-in, On this server, Identity and On this Mac, with the danger rows last.' },
        { label: 'Forget…', page: 'srv-forget', set: base({ 'v.profile': '', 'v.typed': '' }),
          note: 'The confirmation is the local profile id, not the address, and the button stays disabled until it matches.' },
        { label: 'Profile id typed', page: 'srv-forget', set: base({ 'v.profile': '', 'v.typed': '1' }),
          note: 'Forget server drops this Mac’s copy and returns to the list; nothing on the server is touched.' },
        { label: 'Reading the reset preview', page: 'srv-reset', set: base({ 'v.profile': '', 'v.preview': 'loading' }),
          note: 'Reset asks the agent for an exact preview first, rather than describing what it might do.' },
        { label: 'The preview and its token', page: 'srv-reset', set: base({ 'v.profile': '', 'v.preview': '' }),
          note: 'The preview names the resumables and artifacts and carries a 60-second, one-use token.' },
        { label: 'Profile id typed again', page: 'srv-reset', set: base({ 'v.profile': '', 'v.preview': '', 'v.typed': '1' }),
          note: 'Reset local state spends the token; reopening Reset fetches a new one.' },
      ],
    });

    /* =================================================== settings F6–F7 */
    M.flow({
      id: 'pair-another-mac',
      group: 'Settings & servers',
      title: 'Pair another Mac',
      steps: [
        { label: 'Your Macs', page: 'set-devices', set: base({}),
          note: 'This account, Your Macs, Pairing and Recovery — the pane both sides of a pairing start from.' },
        { label: 'Offer from this Mac', page: 'set-devices', set: base({ 'v.sheet': 'pair-offer', 'v.seg': 'offer' }),
          note: 'The offering side: Start asks the agent for a short phrase to read out.' },
        { label: 'The phrase is on screen', page: 'set-devices', set: base({ 'v.sheet': 'pair-offer', 'v.seg': 'offer', 'v.started': '1' }),
          note: 'Type it on the other Mac, then Finish — Resume offer re-reveals a pending one instead of starting.' },
        { label: 'Accept on this Mac', page: 'set-devices', set: base({ 'v.sheet': 'pair-accept', 'v.seg': 'accept' }),
          note: 'The accepting side of the same sheet: alias, device name and the phrase the other Mac showed.' },
      ],
    });

    /* ================================================ settings F8–F10 */
    M.flow({
      id: 'passphrase-and-backup',
      group: 'Settings & servers',
      title: 'Passphrase and backup phrase',
      steps: [
        { label: 'Accounts', page: 'set-account', set: base({}),
          note: 'One block per account; the passphrase buttons are offered only while the account is neither stopped nor pending.' },
        { label: 'Set a passphrase', page: 'set-account', set: base({ 'v.sheet': 'passphrase', 'v.seg': 'set' }),
          note: 'Passphrase status is not stored on this Mac, so all three modes live in one sheet.' },
        { label: 'Change it', page: 'set-account', set: base({ 'v.sheet': 'passphrase', 'v.seg': 'change' }),
          note: 'Every non-verify mode shows the same two fields, Passphrase and Confirm; the segmented control switches mode in place rather than opening another sheet.' },
        { label: 'Verify it', page: 'set-account', set: base({ 'v.sheet': 'passphrase', 'v.seg': 'verify' }),
          note: 'Verify only checks, and toasts the generation it confirmed.' },
        { label: 'Name the backup phrase', page: 'set-devices', set: base({ 'v.sheet': 'phrase-prepare' }),
          note: 'Enroll… names the enrollment before anything is generated; a second enrollment is added, never rotated.' },
        { label: 'The 17 words', page: 'set-devices', set: base({ 'v.sheet': 'phrase', 'v.written': '' }),
          note: 'Once the phrase is showing the sheet cannot be dismissed, and there is no copy button.' },
        { label: 'Written down', page: 'set-devices', set: base({ 'v.sheet': 'phrase', 'v.written': '1' }),
          note: 'Done clears the secret from this window and toasts that the phrase was enrolled.' },
        { label: 'Recover here', page: 'set-devices', set: base({ 'v.sheet': 'recover' }),
          note: 'The other half: a local alias, this Mac’s name and the 17 words, submitted for review.' },
      ],
    });

    /* =============================================== settings F11–F14 */
    M.flow({
      id: 'security-keys',
      group: 'Settings & servers',
      title: 'Security keys and YubiKeys',
      steps: [
        { label: 'Security keys', page: 'set-keys', set: base({}),
          note: 'Enrolled keys, what is connected now, and the thirteen everyday and recovery actions.' },
        { label: 'Create on YubiKey', page: 'set-keys', set: base({ 'v.sheet': 'enrol' }),
          note: 'Three warnings before six fields: a card account is not recoverable the way a software one is.' },
        { label: 'Advanced', page: 'set-keys', set: base({ 'v.sheet': 'enrol', 'v.advanced': '1' }),
          note: 'The slots and attempt counts are behind a disclosure and validated before the primary goes live.' },
        { label: 'Provision an existing account', page: 'set-keys', set: base({ 'v.sheet': 'provision' }),
          note: 'The other direction: put an account you already have onto a card.' },
        { label: 'Everyday maintenance', page: 'set-keys', set: base({ 'v.sheet': 'yubi-sync' }),
          note: 'All thirteen rows share one sheet with different fields; each toasts the same completion.' },
        { label: 'Revoke…', page: 'set-keys', set: base({ 'v.sheet': 'revoke', 'v.typed': '' }),
          note: 'Revoking is keyed by alias, because the agent does not report which connected serial belongs to it.' },
        { label: 'Confirmation typed', page: 'set-keys', set: base({ 'v.sheet': 'revoke', 'v.typed': '1' }),
          note: 'Revoke rotates the affected account keys; copies the card already read cannot be recalled.' },
      ],
    });

    /* ==================================================== settings F15 */
    M.flow({
      id: 'remove-device',
      group: 'Settings & servers',
      title: 'Remove a device',
      steps: [
        { label: 'Your Macs', page: 'set-devices', set: base({ 'v.account': 'acct:personal' }),
          note: 'Two devices on this account: MacBook Pro (current) and Travel Mac.' },
        { label: 'Remove…', page: 'set-devices', set: base({ 'v.account': 'acct:personal', 'v.sheet': 'remove-device', 'v.device': TRAVEL_MAC, 'v.typed': '' }),
          note: 'Only a non-current software device can be removed, and the confirmation is its exact name.' },
        { label: 'Confirmation typed', page: 'set-devices', set: base({ 'v.account': 'acct:personal', 'v.sheet': 'remove-device', 'v.device': TRAVEL_MAC, 'v.typed': '1' }),
          note: 'The device loses future access; values it already read are not recalled, so rotate them if it is not yours.' },
        { label: 'Removed', page: 'set-devices', set: base({ 'v.account': 'acct:personal', 'v.applied': '1' }),
          note: 'The row is gone and the toast reads “Removed Travel Mac from this account”.' },
        { label: 'The current-device chip', page: 'set-devices', set: base({ 'v.account': 'acct:personal', 'v.devlist': 'keycurrent' }),
          note: 'The device you are on shows a chip instead of a button — here it is a security key, so the chip says so.' },
        { label: 'Managed under Security keys', page: 'set-devices', set: base({ 'v.account': 'acct:personal', 'v.devlist': 'plus08' }),
          note: 'An 08… device is a YubiKey: it is listed here but must be revoked from Security keys.' },
      ],
    });

    /* =================================================== settings F3 */
    M.flow({
      id: 'server-states',
      group: 'Settings & servers',
      title: 'Servers in trouble',
      steps: [
        { label: 'Never checked', page: 'srv-server', set: base({ 'v.profile': 'partner' }),
          note: 'foks.partner.dev has never been checked: an info band offers Check now and the Host ID row reads “Set by the first check”.' },
        { label: 'Reading status', page: 'srv-server', set: base({ 'v.profile': 'acme', 'v.status': 'pending' }),
          note: 'While no signed check-in has come back the Check-in row says so rather than guessing.' },
        { label: 'Check-in status unknown', page: 'srv-server', set: base({ 'v.profile': 'acme', 'v.status': 'checkin-unavailable' }),
          note: 'The probe itself failed: “Check-in status unknown.” with Check now — unlike an expired check-in, this one you can retry.' },
        { label: 'History changed', page: 'srv-server', set: base({ 'v.profile': 'acme', 'v.status': 'rollback' }),
          note: 'The loudest state in the app: the server’s history no longer matches what this Mac pinned, so Copy is disabled and the raw response is forced open.' },
        { label: 'Access blocked', page: 'srv-server', set: base({ 'v.profile': 'acme', 'v.status': 'blocked' }),
          note: 'A blocked server shares the previous step’s chip and band — servers-screen.tsx:99 labels both “History changed”. What differs is that no snapshot came back at all: Identity is hidden, there is no response to inspect, and even Forget is disabled.' },
      ],
    });

    /* ================================================ settings §8.3 */
    M.flow({
      id: 'about-this-mac',
      group: 'Settings & servers',
      title: 'About and the agent',
      steps: [
        { label: 'About', page: 'set-about', set: base({}),
          note: 'The last section: the app version, the agent socket this window talks to, and where the data lives.' },
        { label: 'Inspect AgentStatus', page: 'set-about', set: base({ 'v.advanced': '1' }),
          note: 'The disclosure prints the status object verbatim — the same one the titlebar pill summarises.' },
        { label: 'The agent is not ready', page: 'set-about', set: base({ agent: 'starting' }),
          note: 'Status reads Bootstrap and the Connection row grows a Retry connection button: “Interrupted changes will not be repeated.”' },
        /* About has NO managed-profile line: app-root reads
           appInfo.managedProfile only to divert first run (app-root.tsx:144-156),
           and first-run-screen names the profile on the `local` step
           (first-run-screen.tsx:1567-1577). So the step that shows what
           `managed` changes has to land there, not here. */
        { label: 'What a managed profile changes', page: 'frl-local', set: base({ managed: 'local', 'v.result': 'ready' }),
          note: 'appInfo.managedProfile changes one thing and it is not on this pane: it diverts first run to the launcher-prepared local server, whose report this step shows as Ready. Settings › About has no managed-profile line.' },
      ],
    });

    /* ==================================================== settings F16 */
    M.flow({
      id: 'settings-unavailable',
      group: 'Settings & servers',
      title: 'A stale Settings link',
      steps: [
        { label: 'The stale link', page: 'set-unavailable', set: base({ 'v.account': 'acct:nope' }),
          note: 'The address names an account this catalog does not list, so the notice replaces the pane. Refresh the catalog reloads the world and toasts — there is no second screen to it — and the section nav never goes away.' },
        { label: 'Or pick a real account', page: 'set-devices', set: base({ 'v.account': 'acct:personal' }),
          note: 'The other recovery is an exact “{alias} · {server}” button, which selects that account and returns to the pane.' },
      ],
    });
  })();
