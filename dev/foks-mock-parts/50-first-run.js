  /* ------------------------------------------------------ 50-first-run.js
     First run: everything a new Mac sees before the vault shell.

       fr-*   the own path            (boot who address error checked account
                                       protect phrase create-group done
                                       checklist-own)
       fri-*  the invited path        (address no-address checked account
                                       protect waiting added checklist-invited)
       frl-*  the managed-local path  (local account protect local-done)
       frx-*  recovery / pairing      (existing)
       frg-*  the FOKS Go CLI import  (scan, chooser)

     Structure comes from /tmp/foks-mock/cap/first-run/*.html; copy that
     depends on state comes from foks-ui/src/screens/first-run-screen.tsx,
     go-profile-chooser.tsx and first-run-state.ts.

     Two shapes of left column (first-run-screen.tsx:3120-3157):
       * setup mode — <nav class="side setup-side"> with the seven step
         labels, or the three managed-local ones. Used everywhere except…
       * app mode — the ordinary M.renderSidebar plus a `Status` slot, for
         `added`, `done`, `checklist-invited` and `checklist-own`.
     `Pane` never draws its `scope` chip (every caller that passes one also
     passes header={false}), so no `.scope` node exists anywhere in first run;
     only the four app-mode states draw a PageHeader. */

  (function () {
    var D = M.data;
    var UI = M.ui;
    var esc = M.esc;
    /* app-root's Escape rule, so the phrase sheet can hand it back. */
    var shellEscape = M.fns.escape;

    /* ----------------------------------------------- fixture facts (§5)
       mock-bridge.ts:122-167 — firstRunFixture, per path. `admin` is absent
       on the own path, so `admin` falls back to "the group admin" and its
       short form is the first dotted segment ("the"). */
    var FACTS = {
      own: {
        path: 'own',
        profile: 'personal',
        accountAlias: 'personal',
        server: 'foks.example.net',
        typo: 'foks.example.ne',
        username: 'rae',
        deviceName: 'MacBook Pro',
        admin: null,
        groupName: 'Household',
        groupAlias: 'household',
        store: 'team:household',
        report: {
          profile: 'personal',
          acceptance: 'inserted',
          lookupName: 'foks.example.net',
          canonicalName: 'foks.example.net',
          hostId: '0231c2aa07e4b1d86c3f52a09e7d41c8b6f0a2d3e95c17b48f6a0d2e3c5b719a4f',
          chain: 12,
          epoch: 4821,
        },
      },
      invited: {
        path: 'invited',
        profile: 'acme',
        accountAlias: 'sol',
        server: 'foks.acme-corp.com',
        typo: 'foks.acme-corp.co',
        username: 'sol',
        deviceName: "Sol's MacBook Air",
        admin: 'sam.ortiz',
        groupName: 'Engineering',
        groupAlias: 'engineering',
        store: 'team:eng',
        report: {
          profile: 'acme',
          acceptance: 'inserted',
          lookupName: 'foks.acme-corp.com',
          canonicalName: 'foks.acme-corp.com',
          hostId: '024d17e390a5c2f8e16d7b39c04a8e5f2d1c6b7a9038e4f5d6c1a2b3e7f0948d1c',
          chain: 33,
          epoch: 90417,
        },
      },
    };
    /* The managed local profile: appInfo().managedProfile plus the probe the
       shared server status reports (first-run-screen.tsx:1565-1577). */
    var LOCAL = { probe: 'localhost:4430', username: 'rae', deviceName: 'MacBook Pro', alias: 'personal' };

    /* ------------------------------------------- the Go CLI candidates (§6f)
       Two plausible usable candidates and one that is not: a keychain-backed
       owner device that can be paired or copied, and a passphrase-backed one
       that can only be paired and whose profile carries no server hint.
       Shapes from bridge.ts:317-335; the chooser that lists them is
       `frg-chooser`, at the bottom of this file.

       The first candidate is deliberately the own path's own server and host
       ID, because that is the pair the fixture's `checkAndAddGoProfile` will
       accept (mock-bridge.ts:738-747); the second one's host ID is the other
       server's, so probing foks.example.net with it selected is refused with
       "did not match". */
    var GO_CANDIDATES = [
      {
        candidateId: 'rae', username: 'rae', serverHint: 'foks.example.net',
        hostId: '0231c2aa07e4b1d86c3f52a09e7d41c8b6f0a2d3e95c17b48f6a0d2e3c5b719a4f',
        userId: '0138a4c95e77b21d6f0a3c8b5d2e9f14a7b0c3d6e9f2a5b8c1d4e7f0a3b6c9d2e5',
        deviceId: '02a779c40674942e2d0fc18aa8d59b2ee4fa95ea7a652310e3f13d2d317f170b22',
        role: 'Owner', storageKind: 'macos-keychain', hidden: false, provisional: false, pairable: true, copyable: true,
      },
      {
        candidateId: 'ops', username: 'ops-runner', serverHint: null,
        hostId: '024d17e390a5c2f8e16d7b39c04a8e5f2d1c6b7a9038e4f5d6c1a2b3e7f0948d1c',
        userId: '01c4f70b2ae35d1986c0f4a72b5d8e13c6a9f0b3d6e9c2f5a8b1d4e7f0a3c6b9d2',
        deviceId: '02c1e08d5f3a94b7d21e6f0c8a3b5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809',
        role: 'Owner', storageKind: 'passphrase', hidden: false, provisional: false, pairable: true, copyable: false,
      },
      {
        candidateId: 'draft', username: null, serverHint: 'foks.partner.dev',
        hostId: '0204c8b19e3f6a2d75c081b4e9f3a6d2c5b8e1f4a7d0c3b6e9f2a5d8c1b4e7f0a3',
        userId: '019e2b7f4c1a8d05e3b6f9c2a5d8e1b4f7a0c3d6e9b2f5a8c1d4e7b0f3a6c9d2e5',
        deviceId: '0217c4e9b2f5a8d1c4e7f0a3b6c9d2e5f8a1b4c7d0e3f6a9b2c5d8e1f4a7b0c3d6',
        role: 'Member', storageKind: 'noise-file', hidden: false, provisional: true, pairable: false, copyable: false,
      },
    ];
    function goShort(v) { return v.slice(0, 8) + '…' + v.slice(-8); }
    /* `Connect selected account` slugs the candidate's username into
       `recoveryAlias` (first-run-screen.tsx:1779-1783) — no edge trimming,
       and `personal` when the candidate has no username. */
    function goSlug(name) {
      return name == null ? 'personal' : name.toLowerCase().replace(/[^a-z0-9_-]+/g, '-');
    }
    /* Which candidate the chooser handed on. `choice` is the `who` screen's
       pick either way — a path on the fork, a candidateId on the chooser — so
       only a value that names a candidate means one was imported. */
    function candidateOf(s) {
      var id = s.v.choice;
      if (!id) return null;
      return GO_CANDIDATES.filter(function (c) { return c.candidateId === id; })[0] || null;
    }
    /* A field the deck must be able to type empty: a chip cannot write '' (the
       core folds it back to null), so the "(cleared)" chips carry this token
       and the panes read it as the empty string. Same convention as
       60-groups.js. */
    var EMPTY = '(empty)';
    function vtext(s, key, dflt) {
      var val = s.v[key];
      if (val === EMPTY) return '';
      return val == null ? dflt : val;
    }
    /* `managedReport` — Boolean(sharedServerStatus's report). The mock bridge
       names no managedProfile, so it is absent unless the app-wide `managed`
       key (or the page's own knob) says otherwise. It gates `Continue` on
       `local` and the sidebar's `Recover account`. */
    function localReady(s) { return s.v.result ? s.v.result === 'ready' : s.managed === 'local'; }

    var PERSONAL_FIXED =
      'Your Personal vault belongs only to your account and has no members. Put items in a group to share them.';
    var SERVER_LEAD =
      'Everything in FOKS lives on a server — a machine someone runs, where your account, your groups and their encrypted stores are kept.';

    function adminOf(f) { return f.admin || 'the group admin'; }
    function adminShortOf(f) { return adminOf(f).split('.')[0]; }

    /* React's useId, near enough: the captures number every labelled row
       `_r_0_`, `_r_1_`, … in render order. Reset per render. */
    var rid = 0;
    function rr() { rid += 1; return '_r_' + (rid - 1) + '_'; }

    function at(name, value) {
      return value === undefined || value === null || value === false ? '' : ' ' + name + '="' + esc(value) + '"';
    }
    /* Every controlled <input> in the captures carries style="" — React
       writes the attribute for the style object the field components pass. */
    function input(o) {
      return '<input' + at('aria-label', o.ariaLabel) + at('placeholder', o.placeholder) +
        (o.spellcheck === false ? ' spellcheck="false"' : '') +
        (o.id ? ' id="' + esc(o.id) + '"' : '') +
        (o.type ? ' type="' + esc(o.type) + '"' : '') +
        (o.disabled ? ' disabled=""' : '') +
        ' value="' + esc(o.value == null ? '' : o.value) + '" style=""' +
        /* A field the pane reads back: the app keeps it in React state and
           re-renders on every keystroke, which is what enables or disables
           the button beside it. */
        (o.bind ? ' data-bind="' + esc(o.bind) + '" data-live' : '') + '>';
    }
    /* A `<button class="lnk">` — the bare link button, which (unlike Button)
       carries no type attribute in the app's markup. */
    function lnk(label, attrs, cls) {
      return '<button class="' + (cls ? 'lnk ' + cls : 'lnk') + '"' + (attrs ? ' ' + attrs : '') + '>' + esc(label) + '</button>';
    }

    /* ============================================== the setup-mode sidebar
       first-run-screen.tsx:210-326. `current` is stepOf/localStepOf; `path`
       null draws step 6 as the neutral, pending `Group`. */
    function navBtn(icon, label, attrs, disabled) {
      return '<button type="button" class="nav"' + (disabled ? ' disabled=""' : '') +
        (attrs ? ' ' + attrs : '') + '>' + M.icon(icon) + '<span class="t">' + esc(label) + '</span></button>';
    }
    function setupSide(o) {
      o = o || {};
      var labels = o.local
        ? ['Local server', 'Create your account', 'Recovery']
        : ['Preparing this Mac', 'How are you joining?', 'Select a server', 'Create your account',
           'Save recovery phrase',
           o.path == null ? 'Group' : (o.path === 'invited' ? 'Wait to be added' : 'Create a group'),
           'You’re in'];
      var pending = (!o.local && o.path == null) ? 5 : -1;
      var steps = labels.map(function (label, i) {
        var cls = 'setup-step' + (i === o.current ? ' on' : '') + (i < o.current ? ' done' : '') +
          (i === pending ? ' pending' : '');
        return '<div class="' + cls + '"' + (i === o.current ? ' aria-current="step"' : '') + '>' +
          '<span class="setup-mark">' + (i < o.current ? '✓' : (i + 1)) + '</span>' +
          '<span class="t">' + esc(label) + '</span></div>';
      }).join('');
      var foot = '';
      if (o.managedNoAccount) {
        foot += navBtn('server', 'Use another server', 'data-act="go" data-page="fr-address"');
        foot += navBtn('person', 'Recover account',
          'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"local","v.path":"own","v.recovery":"","v.phrase":"","v.refusal":""}\'', !o.recoverEnabled);
      }
      /* `Cancel setup` only when an account store already exists — true in
         the fixture, so it is always drawn (first-run-screen.tsx:3126). */
      foot += navBtn('x', 'Cancel setup', 'data-act="go" data-page="all"');
      return '<nav class="side setup-side" aria-label="Setting up">' +
        '<div class="setup-steps">' + steps + '</div>' +
        '<div class="foot">' + foot + '</div></nav>';
    }

    /* ============================================== the app-mode sidebar
       FirstRunAppSidebar (first-run-screen.tsx:328-387): the ordinary
       Sidebar with a `Status` slot, and an alerts count first run supplies
       itself (1 while invited and not yet added, else 0) rather than the
       world's — M.renderSidebar's `alerts` option. */
    function statusSlot(o) {
      return UI.sectionLabel('Status', { as: 'side' }) +
        (o.left < 5
          ? M.navRow({
              active: true, glyph: M.icon('flag'), name: 'Get started',
              tail: '<span class="badge">' + o.left + ' of 5</span>',
              attrs: 'data-act="go" data-page="' + esc(o.checklist) + '"',
            })
          : '') +
        (o.note ? '<p class="side-note">' + esc(o.note) + '</p>' : '');
    }
    function appSide(s, page, alerts) {
      return M.renderSidebar(s, page, { alerts: alerts });
    }

    /* ================================================== the Pane wrapper
       first-run-screen.tsx:496-529. `header` is only ever true for the four
       app-mode states; `foot` is the .pfoot sibling of .body. */
    function pane(o) {
      return '<main class="main first-run-main">' +
        (o.header ? UI.pageHeader({ title: o.header.title, subtitle: o.header.subtitle }) : '') +
        '<div class="body pb"><div class="pane' + (o.wide ? ' wide' : '') + '">' + (o.body || '') + '</div></div>' +
        (o.foot || '') + '</main>';
    }
    function foot(o) {
      o = o || {};
      return '<div class="pfoot">' +
        (o.back ? UI.btn('‹ Back', { className: 'lnk', attrs: o.back }) : '') +
        (o.note ? '<span class="note">' + o.note + '</span>' : '') +
        '<span class="spacer"></span>' + (o.children || '') + '</div>';
    }
    /* <details class="dd"> whose openness is view state, so a click on the
       summary survives the re-render. */
    function dd(o) {
      var open = !!o.open;
      return '<details class="dd"' + (open ? ' open=""' : '') + '>' +
        '<summary data-act="set" data-key="' + esc(o.key || 'v.disclosure') + '" data-val="' +
        esc(open ? '' : (o.val || 'open')) + '">' + esc(o.summary) + '</summary>' + o.body + '</details>';
    }
    /* One `.inset.checklist` row: a glyph or mark in `.k`, title + hint in
       `.v`, an optional `.a`. */
    function checkRow(mark, title, hint, action, extra) {
      return UI.insetRow({
        label: mark,
        value: '<b>' + title + '</b><span class="hint">' + hint + '</span>' + (extra || ''),
        action: action,
      });
    }

    /* ==================================================================== */
    /* boot — Preparing this Mac                                            */
    /* ==================================================================== */
    M.page({
      id: 'fr-boot',
      title: 'Preparing this Mac',
      path: ['First run', 'Your own account', '1 · Preparing this Mac'],
      note: 'state=boot. The agent is still bootstrapping, so the titlebar pill reads Agent starting. Continue stays disabled until the client state is initialised — and the initialize event moves the step on by itself (to `who`, or to `local` when a managed profile is ready), so nobody ever clicks it. Step through with the walkthrough.',
      controls: [
        { key: 'path', label: 'Path', note: 'Which route the checkpoint holds — it only changes the sixth sidebar label here.', values: [{ v: 'own', label: 'Own' }, { v: 'invited', label: 'Invited' }] },
        { key: 'disclosure', label: 'Disclosure', note: 'The <details class="dd"> under the checklist.', values: [{ v: '', label: 'Closed' }, { v: 'open', label: 'Details open' }] },
        { key: 'result', label: 'Client state', note: 'checkpoint.initialized. The mock bridge is not native, so `initializeClientState` never runs and the web app is stuck at `Initialising` — but the same screen redraws its lead, its last two marks and its Continue once the initialize event lands (first-run-screen.tsx:1609-1650). A failure puts the agent’s own sentence in a `.crit` under the checklist and turns the footer button into `Try again` (:1598-1608, :1651).', values: [{ v: '', label: 'Initialising' }, { v: 'ready', label: 'Initialised' }, { v: 'error', label: 'Initialisation failed' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var open = s.v.disclosure === 'open';
        var ready = s.v.result === 'ready';
        /* fail() puts the normalized command error in `message`; the one
           `initialize_client_state` raises when the agent comes back but is
           not Ready is commands.rs:5023-5026. */
        var failed = s.v.result === 'error';
        var body = '<h1>Preparing this Mac</h1>' +
          '<p class="lead">' + (ready
            ? 'The local <b>foks-agent</b> and encrypted store for your keys are ready.'
            : 'Starting a local <b>foks-agent</b> and initializing an encrypted store for your keys.') + '</p>' +
          UI.inset(
            checkRow('✓', 'Starting macOS private helper', 'Runs only on this Mac and bound to this window.') +
            checkRow(ready ? '✓' : '◌', 'Creating encrypted vault', 'Your keys will be stored in an end-to-end encrypted vault.') +
            checkRow(ready ? '✓' : '3', 'Ready', 'Continue to the next step.'),
            { className: 'checklist' }) +
          (failed ? '<p class="crit">The agent did not become ready after initialization.</p>' : '') +
          dd({
            open: open, summary: 'Details',
            body: '<p>Agent status reports Bootstrap then Ready. Initialisation creates the native credential and rollback boundary once per Mac.</p>',
          });
        return {
          /* app-root.tsx:330-344 rewrites the agent to Bootstrap while the
             boot step is showing, whatever the world says. */
          titlebar: M.renderTitlebar({ agent: 'starting', refreshing: s.refreshing }),
          side: setupSide({ current: 0, path: s.v.path || 'own' }),
          main: pane({
            body: body,
            /* `checkpoint.initialized` is false on this step, so the app draws
               a disabled Continue (first-run-screen.tsx:1614-1617). The step
               advances from the initialize event, not from this button — but
               once it has landed the button is live and goes to `who`. */
            foot: foot({
              children: failed
                /* `Try again` clears the message and re-runs
                   initializeClientState (first-run-screen.tsx:1599-1607). */
                ? UI.btn('Try again', {
                    variant: 'primary',
                    attrs: 'data-act="set" data-key="v.result" data-val=""',
                  })
                : UI.btn('Continue', {
                    variant: 'primary', disabled: !ready,
                    attrs: ready ? 'data-act="go" data-page="fr-who"' : '',
                  }),
            }),
          }),
        };
      },
    });

    /* ==================================================================== */
    /* who — How are you joining?                                           */
    /* ==================================================================== */
    var JOINING = [
      { path: 'own', icon: 'person', title: 'I’m starting on my own', detail: 'For yourself, or to start a group that others will join.', need: 'You’ll need a server address' },
      { path: 'invited', icon: 'people', title: 'Someone invited me to their group', detail: 'They said something like “install this and I’ll add you”.', need: 'You’ll need their server address' },
    ];
    var NEXT_STEPS = {
      invited: [
        'Enter <b>their server address</b> and check it is the right one.',
        'Pick a <b>username</b> and save your recovery phrase.',
        '<b>Wait to be added.</b> Joining a server requires admin approval.',
      ],
      own: [
        'Enter a <b>server address</b>: yours, or one you were given.',
        'Pick a <b>username</b> and save your recovery phrase.',
        '<b>Create a group</b> for others to join, or skip it and keep things personal.',
      ],
    };
    M.page({
      id: 'fr-who',
      title: 'How are you joining?',
      path: ['First run', 'Your own account', '2 · How are you joining?'],
      note: 'state=who. Nothing is chosen until Continue, so step 6 stays neutral and pending until an option is picked.',
      controls: [
        { key: 'choice', label: 'Picked', note: 'pendingPath — screen state, not the checkpoint. `who` is the one step that ignores checkpoint.path entirely: SetupSidebar reads the pick instead (first-run-screen.tsx:281-283), so this page declares no `path`.', values: [{ v: '', label: 'Nothing' }, { v: 'own', label: 'On my own' }, { v: 'invited', label: 'Invited' }] },
        { key: 'refusal', label: 'Go CLI scan', note: 'discoverGoProfiles() rejected, so the fork is drawn with the scan error above the options rather than the chooser (first-run-screen.tsx:1806-1810). Native only — the mock bridge answers {installed:false}, so no capture has it.', values: [{ v: '', label: 'No error' }, { v: 'scan', label: 'Scan failed' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var picked = s.v.choice || null;
        var opts = JOINING.map(function (o, i) {
          var on = picked === o.path;
          return '<button type="button" role="radio" aria-checked="' + (on ? 'true' : 'false') + '" tabindex="' +
            ((on || (picked === null && i === 0)) ? '0' : '-1') + '" class="' + (on ? 'opt on' : 'opt') + '"' +
            ' data-act="set" data-key="v.choice" data-val="' + o.path + '">' +
            '<span class="pip" aria-hidden="true"></span>' + M.icon(o.icon, null, { cls: 'ic' }) +
            '<span class="txt"><span class="ptitle">' + esc(o.title) + '</span>' +
            '<span class="bd">' + esc(o.detail) + '</span></span>' +
            '<span class="need">' + M.icon('server') + esc(o.need) + '</span></button>';
        }).join('');
        var next = picked ? (picked === 'invited'
          ? '<section class="next" aria-live="polite"><h3>What happens next</h3><ol>' +
            NEXT_STEPS.invited.map(function (x) { return '<li><span>' + x + '</span></li>'; }).join('') + '</ol></section>'
          : '<section class="next" aria-live="polite"><h3>What happens next</h3><ol>' +
            NEXT_STEPS.own.map(function (x) { return '<li><span>' + x + '</span></li>'; }).join('') + '</ol></section>') : '';
        var body = '<h1>How are you joining?</h1>' +
          '<p class="lead">Pick the one that fits. Everything else is figured out in the next few steps.</p>' +
          /* The scan error sits between the lead and the options; the message
             is whatever discover_go_profiles refused with (bridge.ts:1376). */
          (s.v.refusal === 'scan'
            ? '<p class="crit">Existing CLI profiles could not be inspected: discover_go_profiles returned duplicate candidates</p>'
            : '') +
          '<div class="opts" role="radiogroup" aria-label="How are you joining">' + opts + '</div>' + next +
          '<div class="actions">' +
          UI.btn('Continue', {
            variant: 'primary', disabled: !picked,
            /* `choose` resets the checkpoint, so nothing the fork was holding
               — a Go candidate included — travels (first-run-state.ts:118). */
            attrs: picked
              ? 'data-act="go" data-page="' + (picked === 'invited' ? 'fri-address' : 'fr-address') + '" data-set=\'{"v.path":"' + picked + '","v.choice":"","v.returning":"","v.card":"","v.refusal":"","v.made":"","v.recovery":"","v.phrase":""}\''
              : '',
          }) +
          '<span class="alt">Already use FOKS on another device? ' +
          '<button type="button" class="lnk" data-act="go" data-page="fr-address" data-set=\'{"v.path":"own","v.choice":"","v.returning":"yes","v.card":"","v.refusal":"","v.made":"","v.recovery":"","v.phrase":""}\'>Add this as a secondary device</button></span>' +
          '</div>';
        return {
          side: setupSide({ current: 1, path: picked }),
          main: pane({ wide: true, body: body }),
        };
      },
    });

    /* ==================================================================== */
    /* address / no-address / error — Select a server                       */
    /* ==================================================================== */
    function addressPane(s, path, state) {
      var f = FACTS[path];
      var invited = path === 'invited';
      var refusal = s.v.refusal || '';
      var empty = refusal === 'empty';
      var err = state === 'error';
      /* The address the pane shows back is whatever was refused: the fixture
         typo on a deep link, or the value typed into the field. */
      var probe = s.v.probe || '';
      /* `Connect selected account` prefills the field from the candidate's
         serverHint — and leaves it EMPTY when the profile carries none
         (first-run-screen.tsx:1777). Otherwise it is the fixture address. */
      var cand = invited ? null : candidateOf(s);
      var prefill = cand ? (cand.serverHint || '') : f.server;
      var address = empty ? '' : (err ? (probe || f.typo) : prefill);
      var message = err
        ? (refusal === 'bad'
            ? (probe || f.typo) + ' did not answer, so nothing was saved.'
            : refusal === 'mismatch'
              /* checkAndAddGoProfile refuses when the address answers for a
                 different host ID than the candidate's (mock-bridge.ts:746). */
              ? (probe || f.server) + ' did not match, so nothing was saved.'
              : (address || 'The empty address') + ' did not answer, so nothing was saved')
        : null;
      var body = '<h1>' + (invited ? 'Select a server address' : 'Select a server') + '</h1>' +
        '<p class="lead">' + SERVER_LEAD + ' ' + (invited ? 'Enter the server address from ' + esc(adminShortOf(f)) + '.' : '') + '</p>' +
        UI.sectionLabel('Server') +
        UI.inset(
          '<label class="fr server-address-row"><span class="k">Address</span>' +
          input({ ariaLabel: 'Server address', placeholder: 'e.g. foks.example.net, localhost, etc.', value: address }) +
          '</label>',
          { className: (err || empty) ? 'err' : undefined }) +
        (s.v.returning === 'yes'
          ? '<p class="hint"><b>You already have an account on this server.</b> Nothing new is registered: after the check, this Mac is added to the account you already have.</p>'
          : '') +
        (message ? '<div class="crit"><b>' + esc(message) + '</b>FOKS could not connect to this address. Check the address and your network connection, then try again. No changes were saved.</div>' : '') +
        (invited && state === 'no-address'
          ? '<div class="pcard"><h3>Ask ' + esc(adminShortOf(f)) + ' this</h3>' +
            '<p>Any way you normally talk to them. It is the only thing you need from them right now.</p>' +
            UI.copyBox({
              display: '“What’s the address of the FOKS server our group is on?”',
              attrs: 'data-act="toast" data-text="Sentence copied."',
            }) +
            '<p>Your username is next — you choose it here and send it to them after. They add you from their side; you never need a code or a link.</p></div>'
          : invited
            ? '<p class="hint">' + lnk('They sent nothing?', 'data-act="go" data-page="fri-no-address"') + '</p>'
            : '');
      return {
        side: setupSide({ current: 2, path: path }),
        main: pane({
          body: body,
          foot: foot({
            back: 'data-act="go" data-page="fr-who"',
            children: UI.btn(err ? 'Check again' : 'Check the server', {
              variant: 'primary',
              attrs: 'data-act="call" data-fn="frCheck" data-arg="' + path + '"',
            }),
          }),
        }),
      };
    }
    /* checkServer() (first-run-screen.tsx:1140-1175): an empty address is
       refused on this Mac — `addressInvalid`, no bridge call, and from `error`
       it also drops back to `address`. Otherwise the probe answers, and a
       failure carries the bridge's own sentence (which ends in a full stop).
       The field is read from the DOM at click time, exactly as the app reads
       its `address` state, so clearing it and pressing the button refuses.

       checkAndAddProfile (mock-bridge.ts:727-737) answers for exactly the two
       fixture addresses and refuses everything else, so `Check again` on the
       `error` pane — whose field still holds the typo — fails again rather
       than succeeding. The probe that was refused rides to `error` in
       `v.probe`, which is what the `.crit` headline names. */
    var KNOWN_SERVERS = { 'foks.example.net': 'own', 'foks.acme-corp.com': 'invited' };
    M.fns.frCheck = function (arg) {
      var path = arg === 'invited' ? 'invited' : 'own';
      var field = document.querySelector('#frame input[aria-label="Server address"], #frame .checked-address input');
      var value = field ? field.value.trim() : '';
      if (!value) {
        M.go(path === 'invited' ? 'fri-address' : 'fr-address', { 'v.path': path, 'v.refusal': 'empty' });
        return;
      }
      var answered = KNOWN_SERVERS[value];
      if (M.s.v.result === 'error' || !answered) {
        M.go('fr-error', { 'v.path': path, 'v.refusal': 'bad', 'v.probe': value });
        return;
      }
      /* With a Go candidate selected the check is `checkAndAddGoProfile`,
         which also demands the answering host ID be the candidate's. */
      var cand = candidateOf(M.s);
      if (cand && FACTS[answered].report.hostId !== cand.hostId) {
        M.go('fr-error', { 'v.path': path, 'v.refusal': 'mismatch', 'v.probe': value });
        return;
      }
      /* The probe rides on: `checked` shows the report of whichever server
         answered, which is not always the one this path's fixture names. */
      M.go(path === 'invited' ? 'fri-checked' : 'fr-checked', { 'v.refusal': '', 'v.probe': value });
    };
    /* The candidate `who`'s chooser handed on, on the three panes that read it
       (address → checked → existing). Only the two usable candidates are
       chips: the third cannot be selected. */
    var CANDIDATE_CONTROL = {
      key: 'choice', label: 'Go CLI candidate',
      note: '`Connect selected account` carried this candidate over: the address is its serverHint (blank when it has none), the recovery alias is its slugged username, and the check must find its host ID. Native only.',
      values: [
        { v: '', label: 'None' },
        { v: 'rae', label: 'rae · foks.example.net' },
        { v: 'ops', label: 'ops-runner · no hint' },
      ],
    };
    var ADDRESS_CONTROLS = [
      { key: 'refusal', label: 'Refusal', note: 'The client-side refusal an empty address raises: .inset.err with no .crit, still on `address`. Clearing the field and pressing the button reaches it too.', values: [{ v: '', label: 'None' }, { v: 'empty', label: 'Empty address' }] },
      { key: 'returning', label: 'Returning', note: 'checkpoint.returning — the "You already have an account on this server." hint, and Continue routes to `existing`.', values: [{ v: '', label: 'First time' }, { v: 'yes', label: 'Returning' }] },
      { key: 'result', label: 'Check result', note: 'What the next check answers: acceptance on `checked`, or a failure that lands on `error`.', values: [{ v: 'new', label: 'Inserted' }, { v: 'same', label: 'Unchanged' }, { v: 'advanced', label: 'Advanced' }, { v: 'error', label: 'Fails' }] },
    ];
    M.page({
      id: 'fr-address',
      title: 'Select a server',
      path: ['First run', 'Your own account', '3 · Select a server'],
      note: 'state=address, path=own. The address is prefilled from the fixture — or from the Go CLI candidate, which may have no server hint at all; Check the server pins the host ID.',
      /* A Go CLI candidate picked on `who` stays selected all the way to
         `existing`, where it retitles the pairing card. */
      keeps: ['card'],
      controls: ADDRESS_CONTROLS.concat([CANDIDATE_CONTROL]),
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return addressPane(s, 'own', 'address');
      },
    });
    M.page({
      id: 'fr-error',
      title: 'Server did not answer',
      path: ['First run', 'Your own account', '3 · Server did not answer'],
      note: 'state=error. The seeded address is the fixture typo; with no bridge message the fallback sentence has no full stop. `Check again` re-probes whatever the field holds — the typo fails again, the real address goes through.',
      /* `probe` is the address the check refused, carried in from `frCheck`
         so the field and the headline name what was actually typed. */
      keeps: ['card', 'probe'],
      controls: [
        { key: 'path', label: 'Path', values: [{ v: 'own', label: 'Own' }, { v: 'invited', label: 'Invited' }] },
        { key: 'refusal', label: 'Message', note: 'The .crit headline: the client fallback, or one of the two mock bridge refusals (both of which end in a full stop) — `did not answer` from an ordinary check, `did not match` when a Go candidate’s host ID is not the one that answered.', values: [{ v: '', label: 'Fallback' }, { v: 'bad', label: 'Did not answer' }, { v: 'mismatch', label: 'Did not match' }] },
        { key: 'returning', label: 'Returning', values: [{ v: '', label: 'First time' }, { v: 'yes', label: 'Returning' }] },
        CANDIDATE_CONTROL,
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return addressPane(s, s.v.path === 'invited' ? 'invited' : 'own', 'error');
      },
    });
    M.page({
      id: 'fri-address',
      title: 'Select a server address',
      path: ['First run', 'Invited to a group', '3 · Select a server address'],
      note: 'state=address, path=invited: the lead names the admin and a "They sent nothing?" link sits under the field.',
      controls: ADDRESS_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return addressPane(s, 'invited', 'address');
      },
    });
    M.page({
      id: 'fri-no-address',
      title: 'Ask them for the address',
      path: ['First run', 'Invited to a group', '3 · Ask them for the address'],
      note: 'state=no-address, path=invited: the link is replaced by the "Ask sam this" card and its CopyBox.',
      controls: ADDRESS_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return addressPane(s, 'invited', 'no-address');
      },
    });

    /* ==================================================================== */
    /* checked / compare — Server checked                                   */
    /* ==================================================================== */
    var ACCEPTANCE = { new: 'inserted', same: 'unchanged', advanced: 'advanced', error: 'inserted' };
    function checkedPane(s, path) {
      var f = FACTS[path];
      /* The report is the answering server's, not the path's: a Go CLI
         candidate can put an own-path checkpoint in front of the other
         fixture's server. The path still decides the sidebar and where
         Continue goes, because that is the checkpoint, not the probe. */
      var answered = FACTS[KNOWN_SERVERS[s.v.probe] || path] || f;
      var report = {
        profile: answered.report.profile,
        acceptance: ACCEPTANCE[s.v.result || 'new'] || 'inserted',
        lookupName: answered.report.lookupName,
        canonicalName: answered.report.canonicalName,
        hostId: answered.report.hostId,
        chain: answered.report.chain,
        epoch: answered.report.epoch,
      };
      var open = s.v.disclosure === 'open' || s.v.disclosure === 'inspect';
      var addrId = rr();
      var facts_ =
        '<div class="facts">' +
        '<div><span class="k">Lookup name</span><code>' + esc(report.lookupName) + '</code></div>' +
        '<div><span class="k">Confirmed name</span><code>' + esc(report.canonicalName) + '</code></div>' +
        '<div><span class="k">Chain sequence</span><span>' + report.chain + '</span></div>' +
        '<div><span class="k">Merkle epoch</span><span>' + report.epoch.toLocaleString('en-US') + '</span></div>' +
        '<div class="wide"><span class="k">Host ID</span><code>' + report.hostId.match(/.{1,4}/g).join(' ') + '</code></div>' +
        '</div>';
      var dbody = facts_ +
        '<p class="hint">These values came from this check. The certificate established the first connection; future checks use the pinned host ID.</p>' +
        UI.toggle({
          id: rr(), label: 'Inspect response', open: s.v.disclosure === 'inspect',
          body: '<pre>' + esc(JSON.stringify(report, null, 1)) + '</pre>',
          attrs: 'data-act="set" data-key="v.disclosure" data-val="' + (s.v.disclosure === 'inspect' ? 'open' : 'inspect') + '"',
        });
      var body = '<h1>Select a server</h1>' +
        '<p class="lead">' + SERVER_LEAD + '</p>' +
        UI.inset(
          UI.insetRow({
            label: 'Address', forId: addrId,
            value: input({ placeholder: 'e.g. foks.example.net, localhost, etc.', spellcheck: false, id: addrId, value: answered.server }),
            /* checkServer() again — the same probe, so it lands back here
               unless the `Check result` knob says the address fails. */
            action: UI.btn('Check again', { attrs: 'data-act="call" data-fn="frCheck" data-arg="' + path + '"' }),
          }),
          { className: 'checked-address' }) +
        '<div class="pcard"><h3>' + M.icon('server') + ' ' + esc(report.canonicalName) + ' answered ' +
        UI.chip('Pinned on this Mac') + '</h3>' +
        '<p>Saved as <b>' + esc(report.canonicalName) + '</b>. The host ID is pinned on this Mac.</p>' +
        '<details class="dd"' + (open ? ' open=""' : '') + '>' +
        '<summary data-act="set" data-key="v.disclosure" data-val="' + (open ? '' : 'open') + '">Details</summary>' +
        '<div class="dbody">' + dbody + '</div></details></div>';
      var cont = s.v.returning === 'yes'
        ? 'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"checked","v.path":"' + path +
          '","v.recovery":"","v.phrase":"","v.refusal":""}\''
        : 'data-act="go" data-page="' + (path === 'invited' ? 'fri-account' : 'fr-account') + '"';
      return {
        side: setupSide({ current: 2, path: path }),
        main: pane({
          body: body,
          foot: foot({
            back: 'data-act="go" data-page="' + (path === 'invited' ? 'fri-address' : 'fr-address') + '"',
            children: UI.btn('Continue', { variant: 'primary', attrs: cont }),
          }),
        }),
      };
    }
    var CHECKED_CONTROLS = [
      { key: 'disclosure', label: 'Disclosure', note: 'Closed = `checked`; open = `compare` (the same screen with the Details block open); inspect also opens the raw response.', values: [{ v: '', label: 'Closed' }, { v: 'open', label: 'Compare' }, { v: 'inspect', label: 'Inspect' }] },
      { key: 'result', label: 'Acceptance', note: 'The `acceptance` the probe returned, visible in the inspected response.', values: [{ v: 'new', label: 'Inserted' }, { v: 'same', label: 'Unchanged' }, { v: 'advanced', label: 'Advanced' }] },
      { key: 'returning', label: 'Returning', note: 'Continue goes to `existing` instead of `account`.', values: [{ v: '', label: 'First time' }, { v: 'yes', label: 'Returning' }] },
    ];
    M.page({
      id: 'fr-checked',
      title: 'Server checked',
      path: ['First run', 'Your own account', '3 · Server checked'],
      note: 'state=checked (disclosure closed) / compare (open). Nothing about you has been sent yet.',
      /* `probe` is the address that answered, so the report on show is that
         server's; `choice` only rides through to `existing`. */
      keeps: ['card', 'choice', 'probe'],
      controls: CHECKED_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return checkedPane(s, 'own');
      },
    });
    M.page({
      id: 'fri-checked',
      title: 'Server checked',
      path: ['First run', 'Invited to a group', '3 · Server checked'],
      note: 'state=checked, path=invited — the same screen, the admin’s server.',
      keeps: ['probe'],
      controls: CHECKED_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return checkedPane(s, 'invited');
      },
    });

    /* ==================================================================== */
    /* account — Create an account                                          */
    /* ==================================================================== */
    function accountPane(s, path) {
      var f = FACTS[path];
      /* `‹ Back` from Save recovery phrase lands here with the account
         already created: the primary becomes `Continue` and only moves on
         (first-run-screen.tsx:2118-2131). The fields stay editable — only the
         managed-local pane disables them. */
      var made = s.v.made === 'yes';
      /* disabled={busy || (!checkpoint.account && (!username.trim() ||
         !deviceName.trim()))} (first-run-screen.tsx:2115-2121): the two
         required fields are read back on every keystroke, so emptying either
         one takes `Create my account` away. Once the account exists the
         button is `Continue` and neither field can hold it back. */
      var uname = vtext(s, 'username', f.username);
      var dname = vtext(s, 'dev', f.deviceName);
      var live = made || (uname.trim() && dname.trim());
      var u = rr(), dev = rr(), mail = rr(), inv = rr();
      var body = '<h1>Create an account</h1>' +
        '<p class="lead">FOKS creates your account keys on this Mac and registers only the public keys with the server.</p>' +
        '<div class="two account-form">' +
        '<div>' + UI.sectionLabel('You') +
        UI.inset(
          UI.insetRow({ label: 'Username', forId: u, value: input({ id: u, value: uname, bind: 'v.username' }) }) +
          UI.insetRow({ label: 'This Mac’s name', forId: dev, value: input({ id: dev, value: dname, bind: 'v.dev' }) })) +
        '</div>' +
        '<div>' + UI.sectionLabel('Optional') +
        UI.inset(
          UI.insetRow({ label: 'Email (optional)', forId: mail, value: input({ id: mail, placeholder: 'you@example.net', value: '' }) }) +
          UI.insetRow({ label: 'Invite (optional)', forId: inv, value: input({ id: inv, value: '' }) })) +
        '</div></div>' +
        /* `{message ? <p className="crit">{message}</p> : null}`, between the
           form and the link (first-run-screen.tsx:2186) — the bridge's own
           refusal when a resumed signup is no longer pending. */
        (s.v.refusal === 'resume' ? '<p class="crit">That account setup is no longer pending.</p>' : '') +
        lnk('I already have an account on this server',
          'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"account","v.path":"' + path +
          '","v.recovery":"","v.phrase":"","v.refusal":""}\'');
      var protectPage = path === 'invited' ? 'fri-protect' : 'fr-protect';
      return {
        side: setupSide({ current: 3, path: path }),
        main: pane({
          wide: true, body: body,
          foot: foot({
            back: 'data-act="go" data-page="' + (path === 'invited' ? 'fri-checked' : 'fr-checked') + '"',
            children: UI.btn(made ? 'Continue' : (s.v.pending === 'yes' ? 'Resume account setup' : 'Create my account'), {
              variant: 'primary', disabled: !live,
              attrs: live ? 'data-act="go" data-page="' + protectPage + '" data-set=\'{"v.written":"","v.refusal":"","v.made":"yes"}\'' : '',
            }),
          }),
        }),
      };
    }
    var ACCOUNT_CONTROLS = [
      { key: 'pending', label: 'Pending operation', note: 'listPendingOperations reports a matching account-signup row with no target: the primary becomes `Resume account setup`.', values: [{ v: '', label: 'None' }, { v: 'yes', label: 'Resumable' }] },
      { key: 'made', label: 'Account exists', note: 'checkpoint.account — set once the account is created, so `‹ Back` from Save recovery phrase returns to a pane whose primary reads `Continue` and does nothing but go on. The managed-local pane also disables its fields.', values: [{ v: '', label: 'Not yet' }, { v: 'yes', label: 'Already created' }] },
      { key: 'refusal', label: 'Refusal', note: 'The `.crit` this pane can carry: the bridge’s answer when a resumed account signup is no longer pending (mock-bridge.ts:761-765).', values: [{ v: '', label: 'None' }, { v: 'resume', label: 'Not pending' }] },
      { key: 'username', label: 'Username', freeText: true, note: 'What is typed into the Username field. `Create my account` is refused while it or This Mac’s name is empty — type the field empty, or use the “(cleared)” chip, which is how a chip writes an empty string.', values: [{ v: '', label: 'the fixture username' }, { v: 'rae.chen', label: 'rae.chen' }, { v: EMPTY, label: '(cleared)' }] },
      { key: 'dev', label: 'This Mac’s name', freeText: true, note: 'The device-name field, the pane’s other required one.', values: [{ v: '', label: 'the fixture name' }, { v: 'Studio Mac', label: 'Studio Mac' }, { v: EMPTY, label: '(cleared)' }] },
    ];
    M.page({
      id: 'fr-account',
      title: 'Create an account',
      path: ['First run', 'Your own account', '4 · Create an account'],
      note: 'state=account, path=own. Keys are made on this Mac; only their public halves are registered.',
      controls: ACCOUNT_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return accountPane(s, 'own');
      },
    });
    M.page({
      id: 'fri-account',
      title: 'Create an account',
      path: ['First run', 'Invited to a group', '4 · Create an account'],
      note: 'state=account, path=invited — the username you send the admin afterwards.',
      controls: ACCOUNT_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return accountPane(s, 'invited');
      },
    });

    /* ==================================================================== */
    /* protect / phrase — Save recovery phrase                              */
    /* ==================================================================== */
    function protectPane(s, path, sheetOpen) {
      var f = FACTS[path];
      /* While the sheet is open, `written` is only the checkbox: the phrase
         is not committed until Done, so the card behind still reads Not yet. */
      var written = !sheetOpen && (s.v.written === '1');
      /* disabled={passphrase !== confirmation} (first-run-screen.tsx:2600) —
         both empty is the default, and equal, so Continue starts live. */
      var pass = rr(), conf = rr();
      var passValue = s.v.pass || '', confValue = s.v.conf || '';
      var mismatch = passValue !== confValue;
      var body = '<h1>Save recovery phrase</h1>' +
        '<p class="lead">Right now this Mac holds the only key to ' + esc(f.username) +
        '. Add at least one recovery method now. You can add more later under <b>Recovery devices</b> or <b>Security keys</b>.</p>' +
        '<div class="two">' +
        '<div class="pcard"><h3>Passphrase</h3>' +
        '<p>Encrypts the keys this Mac keeps for you. It never leaves this Mac and the server never sees it.</p>' +
        UI.inset(
          UI.insetRow({ label: 'Passphrase', forId: pass, value: input({ ariaLabel: 'Passphrase', placeholder: '••••••••••••', id: pass, type: 'password', value: passValue, bind: 'v.pass' }) }) +
          UI.insetRow({ label: 'Confirm', forId: conf, value: input({ ariaLabel: 'Confirm passphrase', placeholder: '••••••••••••', id: conf, type: 'password', value: confValue, bind: 'v.conf' }) })) +
        '<p>Optional. Leave both empty to go without one.</p></div>' +
        '<div class="pcard"><h3>YubiKey ' + UI.chip('Later') + '</h3>' +
        '<p>Enroll a hardware key later from <b>Settings › Security keys</b>.</p>' +
        dd({
          open: s.v.disclosure === 'open', summary: 'What to know first',
          body: '<div class="dbody"><ol>' +
            '<li><b>Preparing the card cannot be split or resumed.</b> If it stops part way, recovery requires resetting its PIV applet, which erases everything on it.</li>' +
            '<li><b>It must still hold its factory management key.</b> A card that has already been managed will refuse.</li>' +
            '<li><b>The unlock code is yours to choose and keep.</b> Nothing generates one for you or shows it later.</li>' +
            '</ol></div>',
        }) +
        UI.btn('Enroll a YubiKey…', { attrs: 'data-act="set" data-key="v.refusal" data-val="yubikey"' }) +
        '</div>' +
        '<div class="pcard"><h3>Backup phrase ' +
        (written ? UI.chip('Written down') : UI.chip('Not yet', { tone: 'warn' })) + '</h3>' +
        '<p>Write down these 17 tokens to recover your account if every device is lost. FOKS shows them once.</p>' +
        UI.btn(written ? 'Shown once — done' : 'Show my phrase', {
          disabled: written,
          attrs: written ? '' : 'data-act="go" data-page="fr-phrase" data-set=\'{"v.path":"' + path + '"}\'',
        }) +
        '</div></div>' +
        /* `{message ? <p className="crit">{message}</p> : null}` under the
           cards (first-run-screen.tsx:2639): the YubiKey notice, or whatever
           the last bridge call refused with — `commitOwnerBackup` is the one
           reachable from here, from the sheet's `Done`. */
        (s.v.refusal === 'yubikey' ? '<p class="crit">A YubiKey is enrolled later from Settings › Security keys.</p>'
          : s.v.refusal === 'commit' ? '<p class="crit">The prepared backup phrase no longer matches.</p>' : '');
      return {
        side: setupSide({ current: 4, path: path }),
        main: pane({
          wide: true, body: body,
          foot: foot({
            /* `go('account')` — and the account exists by now, so that pane
               comes back with `Continue` rather than `Create my account`. */
            back: 'data-act="go" data-page="' + (path === 'invited' ? 'fri-account' : 'fr-account') + '" data-set=\'{"v.made":"yes"}\'',
            children: UI.btn('Continue', {
              variant: 'primary', disabled: mismatch,
              attrs: mismatch ? '' : 'data-act="go" data-page="' +
                (path === 'invited' ? 'fri-waiting' : 'fr-create-group') +
                '" data-set=\'{"v.disclosure":"","v.refusal":"","v.pass":"","v.conf":""}\'',
            }),
          }),
        }),
      };
    }
    var PROTECT_CONTROLS = [
      { key: 'written', label: 'Backup phrase', note: 'checkpoint.backupCommitted: the chip reads `Written down` and the reveal button is spent.', values: [{ v: '', label: 'Not written' }, { v: '1', label: 'Committed' }] },
      { key: 'disclosure', label: 'Disclosure', note: 'The YubiKey card’s `What to know first`.', values: [{ v: '', label: 'Closed' }, { v: 'open', label: 'What to know' }] },
      { key: 'refusal', label: 'Message', note: '`Enroll a YubiKey…` sets a message rather than doing anything; the sheet’s `Done` can also land one here, when commitOwnerBackup refuses the phrase it was handed (mock-bridge.ts:774-779).', values: [{ v: '', label: 'None' }, { v: 'yubikey', label: 'YubiKey notice' }, { v: 'commit', label: 'Phrase no longer matches' }] },
      { key: 'pass', label: 'Passphrase', note: 'What is typed in the Passphrase field. Continue is disabled while it and Confirm disagree; both empty is the app\u2019s default and goes on without one.', values: [{ v: '', label: 'Empty' }, { v: 'hunter2', label: 'Typed' }] },
      { key: 'conf', label: 'Confirm', note: 'The Confirm field. Set one and not the other to see the refusal.', values: [{ v: '', label: 'Empty' }, { v: 'hunter2', label: 'Typed' }] },
    ];
    M.page({
      id: 'fr-protect',
      title: 'Save recovery phrase',
      path: ['First run', 'Your own account', '5 · Save recovery phrase'],
      note: 'state=protect, path=own. Continue is refused only when the two passphrase fields disagree.',
      controls: PROTECT_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return protectPane(s, 'own');
      },
    });
    M.page({
      id: 'fri-protect',
      title: 'Save recovery phrase',
      path: ['First run', 'Invited to a group', '5 · Save recovery phrase'],
      note: 'state=protect, path=invited — Continue leads to `waiting` rather than `create-group`.',
      controls: PROTECT_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return protectPane(s, 'invited');
      },
    });

    /* ---------------------------------------- the one-time phrase sheet */
    M.page({
      id: 'fr-phrase',
      title: 'Write these tokens down',
      path: ['First run', 'Your own account', '5 · Write these tokens down'],
      note: 'state=phrase: the wide sheet over the protect pane. The phrase is prepared once and never survives a reload.',
      controls: [
        { key: 'written', label: 'Backup phrase', note: 'phraseWritten — `Done` stays disabled until the box is ticked. Ticking it is not committing: the card behind still reads `Not yet` until Done runs.', values: [{ v: '', label: 'Not ticked' }, { v: '1', label: 'Written down' }] },
        { key: 'result', label: 'Prepared phrase', note: 'prepareOwnerBackup runs in an effect when the sheet opens; until it answers there are no tokens to show and `Done` is refused whatever the checkbox says (first-run-screen.tsx:2665-2674).', values: [{ v: '', label: 'Prepared' }, { v: 'preparing', label: 'Still preparing' }] },
        { key: 'path', label: 'Path', values: [{ v: 'own', label: 'Own' }, { v: 'invited', label: 'Invited' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return protectPane(s, s.v.path === 'invited' ? 'invited' : 'own', true);
      },
      overlay: function (s) {
        /* A lost agent conceals the phrase, exactly as concealSignal does
           (first-run-screen.tsx:998-1015); the app lock and the boot screens
           replace the whole shell (app-root.tsx:172-254), so nothing of first
           run — the sheet included — is drawn under them. */
        if (s.agent === 'lost' || s.lock === 'locked' || (s.boot && s.boot !== 'ok')) return '';
        var path = s.v.path === 'invited' ? 'invited' : 'own';
        var back = path === 'invited' ? 'fri-protect' : 'fr-protect';
        var ticked = s.v.written === '1';
        /* `backupPhrase` is null until prepareOwnerBackup answers; the sheet
           says so in place of the grid, and `Done` stays refused. */
        var prepared = s.v.result !== 'preparing';
        var close = 'data-act="go" data-page="' + back + '" data-set=\'{"v.written":""}\'';
        var words = prepared
          ? '<div class="words">' + D.backupPhraseWords.map(function (w, i) {
              return '<div class="word"><i>' + (i + 1) + '</i>' + esc(w) + '</div>';
            }).join('') + '</div>'
          : '<p>Preparing the one-time phrase…</p>';
        return UI.sheet({
          width: 'wide',
          glyph: M.icon('key'),
          title: 'Write these tokens down',
          subtitle: 'Shown once and cannot be copied',
          /* The sheet passes `onClose`, so it is a DismissibleDialog: a
             mousedown on the backdrop itself, or Escape, is `go('protect')`
             and the prepared phrase is discarded (sheet.tsx:92-118,
             kit/overlay-primitives.tsx:231-247). UI.sheet writes the
             backdrop's own aria-modal/tabindex. */
          backdropAttrs: close,
          body: '<p>Anyone with these tokens can access your account. Store them somewhere other than this Mac.</p>' +
            words +
            '<button type="button" aria-label="I have written these 17 tokens down" class="check' + (ticked ? ' on' : '') + '"' +
            ' data-act="set" data-key="v.written" data-val="' + (ticked ? '' : '1') + '">' +
            '<span class="bx">' + (ticked ? '✓' : '') + '</span>I have written these 17 tokens down</button>',
          footer: UI.btn('Not now', { attrs: close }) +
            UI.btn('Done', {
              variant: 'primary', disabled: !ticked || !prepared,
              attrs: (ticked && prepared) ? 'data-act="go" data-page="' + back + '" data-set=\'{"v.written":"1"}\'' : '',
            }),
        });
      },
      after: function () {
        /* The sheet's own Escape: `go('protect')`, discarding the phrase.
           Off this page the shell's rule stands again. */
        M.fns.escape = function () {
          if (M.s.page !== 'fr-phrase') return shellEscape && shellEscape();
          M.go(M.s.v.path === 'invited' ? 'fri-protect' : 'fr-protect', { 'v.written': null });
        };
      },
    });

    /* ==================================================================== */
    /* waiting — Waiting for {admin} to add {username}                      */
    /* ==================================================================== */
    M.page({
      id: 'fri-waiting',
      title: 'Wait to be added',
      path: ['First run', 'Invited to a group', '6 · Wait to be added'],
      note: 'state=waiting. Nothing is pushed here: the group appears only when this Mac asks for it.',
      controls: [
        { key: 'disclosure', label: 'Disclosure', note: 'The `Details` block under the Check now card.', values: [{ v: '', label: 'Closed' }, { v: 'open', label: 'Details open' }] },
        { key: 'result', label: 'Check answers', note: 'What the next `Check now` finds: the group (→ `added`), or an answer discover() will not act on, which only sets a message (first-run-screen.tsx:1443-1473).', values: [{ v: '', label: 'Engineering' }, { v: 'ambiguous', label: 'Not listed yet' }, { v: 'no-store', label: 'Store not ready' }, { v: 'other-account', label: 'Different account' }] },
        { key: 'checked', label: 'Checked', note: 'Whether a check has already run — the chip reads `Checked just now` and the `.res` block echoes the message.', values: [{ v: '', label: 'Not checked yet' }, { v: 'yes', label: 'Checked' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var f = FACTS.invited;
        var short = adminShortOf(f);
        var WAIT_MESSAGES = {
          ambiguous: 'Checked just now — ' + f.groupName + ' is not listed unambiguously yet.',
          'no-store': 'The authenticated group was found, but its refreshed store is not available yet. Check again.',
          'other-account': 'The authenticated discovery response belongs to a different account. Reopen FOKS before continuing.',
        };
        var message = (s.v.checked === 'yes' && WAIT_MESSAGES[s.v.result]) || null;
        /* discover() finds Engineering unless the knob says otherwise; a
           message means the step stays here. */
        var checkAttrs = WAIT_MESSAGES[s.v.result]
          ? 'data-act="set" data-key="v.checked" data-val="yes"'
          : 'data-act="go" data-page="fri-added" data-set=\'{"v.discovered":"yes"}\'';
        var sentence = 'Add ' + f.username + ' on ' + f.report.canonicalName + ' to ' + f.groupName;
        var left = '<div class="col">' +
          '<div class="pcard"><h3>Send ' + esc(short) + ' this</h3>' +
          UI.copyBox({ display: '“' + esc(sentence) + '”', attrs: 'data-act="toast" data-text="Sentence copied."' }) +
          '<p>' + esc(short) + ' needs your username exactly as written. Nothing else — no code, no link.</p></div>' +
          UI.inset(
            checkRow(M.icon('people'), esc(f.groupName) + ' will appear under GROUPS', 'FOKS lists groups after it checks the server.') +
            checkRow(M.icon('eye'), 'You’ll see what your role lets you read',
              'Roles are <b>Member</b>, <b>Admin</b>, and <b>Owner</b>. Members also have a visibility level. Items above your role or visibility level remain locked.') +
            checkRow(M.icon('door'), 'Quitting is fine',
              'Your account and pinned server remain on this Mac. Reopen FOKS and continue from Get started.'),
            { className: 'checklist' }) +
          '</div>';
        var right = '<div class="col">' +
          '<div class="pcard"><h3>Check now</h3>' +
          '<div class="checkrow">' +
          UI.btn('Check now', { variant: 'primary', attrs: checkAttrs }) +
          '<span class="status">' + UI.chip(message ? 'Checked just now' : 'Not checked yet') +
          (message ? 'Not yet — only Personal is listed.' : '') + '</span></div>' +
          '<p>FOKS checks for the group when it opens and when you select <b>Check now</b>. Checks stop when FOKS is closed.</p>' +
          UI.band({ label: 'Group discovery', text: '<b>Check now</b> searches for groups using your authenticated account. FOKS also checks when it opens.' }) +
          (message ? '<div class="res">' + esc(message) + '</div>' : '') +
          '<p class="note">You can use <b>Personal</b> while you wait. If ' + esc(short) +
          ' has already added you, confirm that they entered <code>' + esc(f.username) + '</code>.</p>' +
          dd({
            open: s.v.disclosure === 'open', summary: 'Details',
            body: '<p>Re-reads ' + esc(f.username) + '’s own signed chain on ' + esc(f.report.canonicalName) +
              ', then lists groups. A group appears only when that authenticated answer and the refreshed catalog agree.</p>',
          }) +
          '</div>' +
          '<div class="pcard"><h3>Use your Personal vault</h3>' +
          '<p>' + PERSONAL_FIXED + ' Put your own logins in <b>Personal</b> now; nothing in it is visible to ' + esc(f.groupName) + '.</p>' +
          /* `Open Personal` does not open the vault: it is go('checklist-invited'),
             the same landing as `I’ll come back later` (first-run-screen.tsx:2805). */
          UI.btn('Open Personal', { attrs: 'data-act="go" data-page="fri-checklist-invited" data-set=\'{"v.progress":"4"}\'' }) +
          '</div></div>';
        var body = '<h1>Waiting for ' + esc(adminOf(f)) + ' to add ' + esc(f.username) + '</h1>' +
          '<p class="lead">Your account ' + esc(f.username) + ' on ' + esc(f.report.canonicalName) + ' exists. ' +
          esc(short) + ' must add you to <b>' + esc(f.groupName) + '</b>. Check again after they do.</p>' +
          '<div class="two">' + left + right + '</div>';
        return {
          side: setupSide({ current: 5, path: 'invited' }),
          main: pane({
            wide: true, body: body,
            foot: foot({
              children: UI.btn('I’ll come back later', {
                attrs: 'data-act="go" data-page="fri-checklist-invited" data-set=\'{"v.progress":"4"}\'',
              }),
            }),
          }),
        };
      },
    });

    /* ==================================================================== */
    /* create-group — Create a group                                        */
    /* ==================================================================== */
    /* createGroup (first-run-screen.tsx:1516-1522, mock-bridge.ts:556-567):
       the alias is the name lowercased with runs of anything outside
       [a-z0-9_-] turned into `-` and the edges trimmed. An empty alias is
       refused here; an alias `team:<alias>` already holds is refused by the
       bridge — and the field is prefilled with `Household`, which is one. */
    var TAKEN_GROUP_ALIASES = ['eng', 'household', 'homelab'];
    M.fns.frCreateGroup = function () {
      var field = document.querySelector('#frame .first-run-main .inset input');
      var name = (field ? field.value : '').trim();
      var alias = name.toLowerCase()
        .replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '');
      if (!alias) { M.set('v.refusal', 'name'); return; }
      if (TAKEN_GROUP_ALIASES.indexOf(alias) >= 0) { M.set('v.refusal', 'alias'); return; }
      /* `group-complete` stores the group it just made, and `done` names it
         everywhere (first-run-screen.tsx:3000-3118) — not the fixture group a
         deep link to `?state=done` seeds. */
      M.go('fr-done', { 'v.refusal': '', 'v.group': name });
    };
    M.page({
      id: 'fr-create-group',
      title: 'Create a group',
      path: ['First run', 'Your own account', '6 · Create a group'],
      note: 'state=create-group. Skipping is a first-class answer — Groups is always in the sidebar. The prefilled `Household` is already a group here, so pressing Create group refuses it: type another name.',
      controls: [
        { key: 'choice', label: 'Group kind', note: 'Named groups can be looked up on the server and nested; ad-hoc ones are known only by id.', values: [{ v: 'named', label: 'Named group' }, { v: 'adhoc', label: 'Ad-hoc group' }] },
        { key: 'refusal', label: 'Refusal', note: 'The client refuses a name with no letters or numbers; the bridge refuses an alias `team:<alias>` already holds — which the prefilled `Household` is.', values: [{ v: '', label: 'None' }, { v: 'alias', label: 'Alias exists' }, { v: 'name', label: 'No letters or numbers' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var f = FACTS.own;
        var kind = s.v.choice === 'adhoc' ? 'adhoc' : 'named';
        var nameId = rr();
        var body = '<h1>Create a group</h1>' +
          '<p class="lead">' + PERSONAL_FIXED + ' Start one now, or skip and do it later from <b>Groups</b>.</p>' +
          UI.sectionLabel('Your first group') +
          UI.inset(
            UI.insetRow({ label: 'Group name', forId: nameId, value: input({ id: nameId, value: f.groupName }) }) +
            UI.radioGroup([
              UI.radioCard({
                title: 'Named', selected: kind === 'named',
                detail: 'Has a name others on ' + esc(f.report.canonicalName) + ' can look up, and can be added to other groups.',
                attrs: 'data-act="set" data-key="v.choice" data-val="named"',
              }),
              UI.radioCard({
                title: 'Ad-hoc', selected: kind === 'adhoc',
                detail: 'Known only by its id — for a quick group that never needs to be found by name.',
                attrs: 'data-act="set" data-key="v.choice" data-val="adhoc"',
              }),
            ], { label: 'Group kind' })) +
          (s.v.refusal === 'alias' ? '<p class="crit">That group alias already exists.</p>'
            : s.v.refusal === 'name' ? '<p class="crit">Enter a group name containing letters or numbers.</p>' : '') +
          '<p class="hint">You can add people by username after creating a group.</p>';
        return {
          side: setupSide({ current: 5, path: 'own' }),
          main: pane({
            body: body,
            foot: foot({
              back: 'data-act="go" data-page="fr-protect"',
              children: lnk('Skip this step', 'data-act="go" data-page="fr-checklist-own" data-set=\'{"v.progress":"4","v.refusal":""}\'') +
                UI.btn('Create group', { variant: 'primary', attrs: 'data-act="call" data-fn="frCreateGroup"' }),
            }),
          }),
        };
      },
    });

    /* ==================================================================== */
    /* added / done — the group notice, in app mode                         */
    /* ==================================================================== */
    /* discoverGroups (mock-bridge.ts:811-856) does not only answer: it makes
       `sol` a `Member · visibility 0` party of Engineering. That is why the
       roster behind `added` is one person longer when the step was reached by
       pressing `Check now` (6 people · 1 group) than on a deep link (5). The
       party is put in for the length of one render and taken back out, so no
       other page inherits it. */
    var SOL_PARTY = {
      store: 'team:eng', username: 'sol', label: 'you', party_kind: 'user',
      generation: 1, locally_manageable: true,
      party_id_hex: '01' + '6666666666666666666666666666666666666666666666666666666666666666',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
      name: 'sol', initials: D.initials('sol'), hue: D.hue('sol'),
    };
    function withDiscovery(s, fn) {
      var eng = D.parties['team:eng'];
      if (s.v.discovered !== 'yes' || eng.some(function (p) { return p.username === 'sol'; })) return fn();
      var flat = D.partyList;
      D.parties['team:eng'] = eng.concat([SOL_PARTY]);
      D.partyList = D.parties['team:eng'].concat(D.parties['team:household']);
      try { return fn(); } finally { D.parties['team:eng'] = eng; D.partyList = flat; }
    }
    function groupPane(s, page, state) {
      var invited = state === 'added';
      var f = invited ? FACTS.invited : FACTS.own;
      var w = D.world(s);
      /* `done` names `checkpoint.group` — the group `Create group` just made,
         which on a deep link is the fixture's Household and after a create is
         whatever was typed. The mock has no vault page for a group it
         invented, so `Open your vault` is drawn the way the app draws it when
         the store cannot be resolved: disabled. */
      var created = (!invited && s.v.group) ? String(s.v.group) : '';
      var groupName = created || f.groupName;
      var kindWord = (created && s.v.choice === 'adhoc') ? 'ad-hoc' : 'named';
      var storeId = created ? null : f.store;
      var found = (storeId && w.storeById) ? w.storeById[storeId] : null;
      /* addedStores also demands `store.active && storeReadable(world, id)`
         (first-run-screen.tsx:851-866), so a lapsed lease — which hides the
         group's stores — leaves `addedStore` undefined and `Open your vault`
         disabled, with nothing listed under the notice. */
      var store = (found && found.active !== false && found.readable) ? found : null;
      var items = store ? w.items.filter(function (i) { return i.store === storeId; }) : [];
      var openAttrs = store ? 'data-act="go" data-page="' + esc(M.storePages[storeId]) + '"' : '';
      var notice = UI.notice({
        eyebrow: esc(groupName) + ' · ' + esc(f.report.canonicalName),
        title: invited ? 'You’re in ' + esc(groupName) : esc(groupName) + ' exists',
        body: invited
          ? '<p>' + esc(adminOf(f)) + ' added <code>' + esc(f.username) + '</code> as a Member, and ' +
            esc(f.groupName) + ' is listed here now: its ' + items.length +
            ' items are listed, and what your role can read is open.</p>' +
            '<p class="fn">Groups appear after this Mac creates them or checks for groups it has joined.</p>'
          : '<p>You are its Owner and nobody else is in it yet. Add people by username from the group’s settings — they need an account on ' +
            esc(f.report.canonicalName) +
            ' first: send them the address, ask for the username they picked, then add them as Member, Admin or Owner.</p>' +
            '<p class="fn">Select Resume to continue interrupted group creation without repeating completed steps. Pairing is under <b>Settings › Recovery devices</b>, alongside your 17-token backup phrase.</p>',
        actions: UI.btn('Dismiss', { attrs: 'data-act="go" data-page="all"' }) +
          UI.btn('Open your vault', { variant: 'primary', disabled: !store, attrs: store ? openAttrs : '' }),
      });
      var rest;
      if (invited) {
        rest = '<div class="hdr"><span></span><span>Name</span><span>Readable by</span><span>Version</span><span></span></div>' +
          items.slice(0, 4).map(function (item) {
            return '<div class="row">' + UI.kindIcon(D.kindOf(item)) +
              '<span class="name">' + esc(D.nameOf(item.path)) + '<small>' + esc(item.path) + '</small></span>' +
              '<span>' + UI.chip(esc(D.readableBy(item).label)) + '</span>' +
              '<span class="n">v' + esc(item.version) + '</span><span></span></div>';
          }).join('');
      } else {
        rest = '<div class="empty">' + M.icon('key') + '<h2>No items here</h2>' +
          '<p>Anything saved in ' + esc(groupName) +
          ' is read by everyone in it at their role. Nothing is listed until something is written.</p>' +
          UI.btn('New', { attrs: 'data-act="go" data-page="all"' }) + '</div>';
      }
      return {
        appClass: invited ? 'with-details' : undefined,
        side: appSide(s, page, 0),
        main: pane({
          header: {
            title: groupName,
            subtitle: kindWord + ' group on ' + f.report.canonicalName,
          },
          wide: true,
          body: notice + rest,
        }),
        details: invited ? addedDetails(s, storeId) : '',
      };
    }
    /* AddedDetails (first-run-screen.tsx:389-465) — always the staging-token
       row, always locked: nothing has been opened yet. */
    function addedDetails(s, storeId) {
      var w = D.world(s);
      var store = w.storeById ? w.storeById[storeId] : null;
      var item = w.items.filter(function (i) {
        return i.store === storeId && i.path.indexOf('staging-token') >= 0;
      })[0];
      if (!item) return '<aside class="details"><div class="dh"><span class="t"><h2>Details</h2></span></div></aside>';
      var kind = D.kindOf(item);
      var where = store ? store.name : 'this group';
      return '<aside class="details">' +
        '<div class="dh">' + UI.kindIcon(kind) +
        '<span class="t"><h2>' + esc(D.nameOf(item.path)) + '</h2>' +
        '<small>' + esc(kind) + ' in ' + esc(where) + '</small></span></div>' +
        '<div class="scroll">' +
        UI.sectionLabel('Value') +
        UI.inset(UI.insetRow({
          variant: 'preview', label: 'Value', value: '— locked —', valueClass: 'mask',
          action: UI.btn('Show', { size: 'sm', disabled: true }),
        }), { variant: 'preview' }) +
        '<p class="pfn">Your Member role determines whether this exact version can be opened.</p>' +
        UI.sectionLabel('Info') +
        '<div class="meta">' +
        '<b>Path</b><code>' + esc(item.path) + '</code>' +
        '<b>Kind</b><span>' + esc(kind) + '</span>' +
        '<b>Version</b><span>' + esc(item.version) + '</span>' +
        '<b>Size</b><span>' + esc(item.size) + ' bytes</span>' +
        '<b>Read role</b>' + UI.chip(esc(D.formatRole(item.read) || item.read)) +
        '<b>Write role</b>' + UI.chip(esc(D.formatRole(item.write) || item.write)) +
        '</div>' +
        UI.sectionLabel('Sharing') +
        '<p>Everyone in ' + esc(where) +
        ' at the item’s read role or above can read it. These facts were learned from the current roster.</p>' +
        '</div></aside>';
    }
    M.page({
      id: 'fri-added',
      title: 'You’re in',
      path: ['First run', 'Invited to a group', '7 · You’re in'],
      note: 'state=added: the only first-run step with a details panel, and the only one that adds `with-details` to .app.',
      controls: [
        { key: 'discovered', label: 'Roster', note: 'Reached by pressing `Check now`, discovery has already made `sol` a Member of Engineering, so the roster is 6 people; a deep link to `?state=added` has not run it and shows 5.', values: [{ v: '', label: 'Deep link (5)' }, { v: 'yes', label: 'After Check now (6)' }] },
      ],
      status: function (s) { return statusSlot({ left: 5, checklist: 'fri-checklist-invited' }); },
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return withDiscovery(s, function () { return groupPane(s, M.pages['fri-added'], 'added'); });
      },
    });
    M.page({
      id: 'fr-done',
      title: 'You’re in',
      path: ['First run', 'Your own account', '7 · You’re in'],
      note: 'state=done: the group exists and nobody else is in it yet. A deep link seeds the fixture group (Household, whose store the sidebar already lists); arriving from `Create group` names the group that was just made — Platform — which this world holds no store for, so `Open your vault` is disabled, the way the app draws it whenever `addedStore` cannot be resolved (a lapsed lease does the same to Household).',
      controls: [
        { key: 'group', label: 'Group', note: 'checkpoint.group.name: the fixture group on a deep link, or the name typed on `create-group` — `Platform`, the same group 60-groups’ create sheet makes.', values: [{ v: '', label: 'Household (fixture)' }, { v: 'Platform', label: 'Platform (created)' }] },
        { key: 'choice', label: 'Group kind', note: 'checkpoint.group.kind — the page-header subtitle reads `named group on …` or `ad-hoc group on …`.', values: [{ v: 'named', label: 'Named group' }, { v: 'adhoc', label: 'Ad-hoc group' }] },
      ],
      status: function (s) { return statusSlot({ left: 5, checklist: 'fr-checklist-own' }); },
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return groupPane(s, M.pages['fr-done'], 'done');
      },
    });

    /* ==================================================================== */
    /* checklist-invited / checklist-own — Get started                      */
    /* ==================================================================== */
    function checklistPane(s, path, page) {
      var f = FACTS[path];
      var invited = path === 'invited';
      var protectedNow = s.v.progress === '4';
      var left = protectedNow ? 4 : 3;
      var sentence = 'Add ' + f.username + ' on ' + f.report.canonicalName + ' to ' + f.groupName;
      var protectPage = invited ? 'fri-protect' : 'fr-protect';
      var row5Actions = '<span class="checklist-actions">' +
        (invited
          ? UI.btn('Copy the sentence', { size: 'sm', icon: 'copy', attrs: 'data-act="toast" data-text="Sentence copied."' })
          : '') +
        UI.btn(invited ? 'Check now' : 'Create a group…', {
          size: 'sm',
          attrs: 'data-act="go" data-page="' + (invited ? 'fri-waiting' : 'fr-create-group') + '"',
        }) + '</span>';
      var body = '<p class="lead">' +
        (invited ? 'You closed FOKS while waiting.' : 'You stopped part way last time.') +
        ' Completed steps are saved. Continue with the remaining steps below.</p>' +
        UI.inset(
          checkRow('✓', 'Prepare this Mac', 'Agent ready. Encrypted client state initialised.') +
          checkRow('✓', invited ? 'Their server' : 'A server',
            '<code>' + esc(f.report.canonicalName) + '</code> · host ID <code>' +
            esc(f.report.hostId.slice(0, 10)) + '…</code> · checked and pinned on this Mac.') +
          checkRow('✓', 'Your account',
            '<code>' + esc(f.username) + '</code> on ' + esc(f.report.canonicalName) +
            ' · this Mac is <b>' + esc(f.deviceName) + '</b>.') +
          checkRow(protectedNow ? '✓' : '!', 'Save recovery phrase',
            protectedNow
              ? 'Passphrase set · backup phrase written down · YubiKey later, from Settings'
              : esc('Skipped. This Mac holds the only key to ' + f.username +
                  '; a passphrase, YubiKey or 17-token backup phrase gives you a second way in.'),
            UI.btn(protectedNow ? 'Review' : 'Protect now', {
              size: 'sm', variant: protectedNow ? undefined : 'primary',
              attrs: 'data-act="go" data-page="' + protectPage + '"',
            })) +
          checkRow(invited ? '5' : '!',
            invited ? 'Waiting for ' + esc(adminOf(f)) + ' to add you' : 'Create a group',
            invited
              ? esc(f.groupName) + ' isn’t listed yet. Check again after ' + esc(adminShortOf(f)) +
                ' adds you, or send them the sentence above.'
              : PERSONAL_FIXED + ' You become its Owner and add people by username from the group’s settings.',
            row5Actions,
            invited
              ? UI.band({ label: 'Group discovery', text: 'Check now searches for groups using your authenticated account.' })
              : ''),
          { className: 'checklist' }) +
        (protectedNow ? '' :
          '<div class="checklist-notice">' + UI.notice({
            eyebrow: 'Save recovery phrase · skipped',
            title: 'This Mac holds the only key to ' + esc(f.username),
            body: 'Lose it and the account is gone — a passphrase, YubiKey or 17-token backup phrase is a second way in. Nothing else in the list is blocked by this.',
          }) + '</div>');
      return {
        side: appSide(s, page, invited ? 1 : 0),
        main: pane({
          header: { title: 'Get started', subtitle: left + ' of 5 done' },
          wide: true, body: body,
        }),
      };
    }
    var CHECKLIST_CONTROLS = [
      { key: 'progress', label: 'Steps done', note: 'completedFirstRunSteps: initialized + profile + account + (passphrase | backup) + (added | group). Skipped work does not count.', values: [{ v: '3', label: '3 done' }, { v: '4', label: '4 done' }] },
    ];
    M.page({
      id: 'fr-checklist-own',
      title: 'Get started',
      path: ['First run', 'Your own account', '7 · Get started'],
      note: 'state=checklist-own, in app mode: the ordinary sidebar plus a `Get started` status row.',
      status: function (s) {
        return statusSlot({ left: s.v.progress === '4' ? 4 : 3, checklist: 'fr-checklist-own' });
      },
      controls: CHECKLIST_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return checklistPane(s, 'own', M.pages['fr-checklist-own']);
      },
    });
    M.page({
      id: 'fri-checklist-invited',
      title: 'Get started',
      path: ['First run', 'Invited to a group', '7 · Get started'],
      note: 'state=checklist-invited: the Alerts badge is forced to 1 and a side note says the group is not listed yet.',
      status: function (s) {
        return statusSlot({
          left: s.v.progress === '4' ? 4 : 3,
          checklist: 'fri-checklist-invited',
          note: FACTS.invited.groupName + ' isn’t listed yet. A group appears under GROUPS when this Mac asks the server for it; nothing is pushed here.',
        });
      },
      controls: CHECKLIST_CONTROLS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        return checklistPane(s, 'invited', M.pages['fri-checklist-invited']);
      },
    });

    /* ==================================================================== */
    /* the managed-local path (three-step sidebar)                          */
    /* ==================================================================== */
    M.page({
      id: 'frl-local',
      title: 'Set up FOKS',
      path: ['First run', 'Managed local server', '1 · Local server'],
      note: 'state=local. A launcher-prepared profile already runs a private server on this Mac; the mock bridge never reports one, so it sits at `Checking`.',
      controls: [
        { key: 'result', label: 'Managed report', note: 'sharedServerStatus: `Checking` until the report lands, `Ready` once it does, or the error block when the profile is not usable. Unset it and the app-wide `Managed profile` key decides — the mock bridge names none, which is why the capture sits at Checking.', values: [{ v: '', label: 'From `managed`' }, { v: 'ready', label: 'Report ready' }, { v: 'checking', label: 'Still checking' }, { v: 'error', label: 'Not usable' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        /* No report while appInfo names no managed profile — the mock
           bridge's own answer, and what the capture shows. */
        var ready = localReady(s);
        var body = '<h1>Set up FOKS</h1>' +
          '<p class="lead">A private FOKS server is already running on this Mac. Use it to create your Personal vault.</p>' +
          '<div class="local-server-card"><div class="local-server-head">' +
          '<span class="local-server-mark" aria-hidden="true">' + M.icon('server') + '</span>' +
          '<span class="local-server-title"><b>Local server</b><small>On this Mac</small></span>' +
          '<span class="local-ready"><i></i>' + (ready ? 'Ready' : 'Checking') + '</span></div>' +
          '<dl class="local-server-facts"><dt>Address</dt><dd><code>' + esc(LOCAL.probe) + '</code></dd>' +
          '<dt>Trust</dt><dd>App-managed certificate</dd>' +
          '<dt>Storage</dt><dd>Local only</dd></dl></div>' +
          (s.v.result === 'error'
            ? '<div class="crit"><b>Local server unavailable</b>The managed local server is not ready.' +
              '<div class="btns">' + UI.btn('Check again', { attrs: 'data-act="set" data-key="v.result" data-val="ready"' }) + '</div></div>'
            : '') +
          '<div class="local-alternates">' + UI.sectionLabel('Other ways to begin') +
          '<div class="btns">' +
          UI.btn('Connect to another server…', { attrs: 'data-act="go" data-page="fr-address" data-set=\'{"v.path":"own"}\'' }) +
          UI.btn('Recover an existing account…', {
            disabled: !ready,
            attrs: ready ? 'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"local","v.path":"own","v.recovery":"","v.phrase":"","v.refusal":""}\'' : '',
          }) +
          '</div></div>';
        return {
          side: setupSide({ local: true, current: 0, managedNoAccount: true, recoverEnabled: ready }),
          main: pane({
            body: body,
            foot: foot({
              children: UI.btn('Continue', {
                variant: 'primary', disabled: !ready,
                attrs: ready ? 'data-act="go" data-page="frl-account" data-set=\'{"v.made":"","v.disclosure":""}\'' : '',
              }),
            }),
          }),
        };
      },
    });

    M.page({
      id: 'frl-account',
      title: 'Create your account',
      path: ['First run', 'Managed local server', '2 · Create your account'],
      note: 'state=account with managedLocal: a single column, and the optional fields behind `More options`.',
      /* `result` is frl-local's managed-report knob; it still says whether
         the sidebar's `Recover account` is live. */
      keeps: ['result'],
      controls: [
        { key: 'disclosure', label: 'Optional fields', note: 'The <details class="local-more"> holding Email and Invite.', values: [{ v: '', label: 'Closed' }, { v: 'open', label: 'Expanded' }] },
        { key: 'pending', label: 'Pending operation', note: 'A matching account-signup row makes the primary read `Resume account setup`.', values: [{ v: '', label: 'None' }, { v: 'yes', label: 'Resumable' }] },
        { key: 'made', label: 'Account exists', note: 'checkpoint.account — `‹ Back` from Recovery returns here: every field is disabled and the primary reads `Continue`.', values: [{ v: '', label: 'Not yet' }, { v: 'yes', label: 'Already created' }] },
        { key: 'refusal', label: 'Refusal', note: 'The `.crit` between the fields and the recover link: the bridge’s answer when a resumed account signup is no longer pending.', values: [{ v: '', label: 'None' }, { v: 'resume', label: 'Not pending' }] },
        { key: 'username', label: 'Username', freeText: true, note: 'The Username field. `Create my account` is refused while it or This Mac’s name is empty; both go disabled once the account exists.', values: [{ v: '', label: 'rae' }, { v: 'rae.chen', label: 'rae.chen' }, { v: EMPTY, label: '(cleared)' }] },
        { key: 'dev', label: 'This Mac’s name', freeText: true, note: 'The device-name field, the pane’s other required one.', values: [{ v: '', label: 'MacBook Pro' }, { v: 'Studio Mac', label: 'Studio Mac' }, { v: EMPTY, label: '(cleared)' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var open = s.v.disclosure === 'open';
        /* Every field is disabled once the account exists — this pane is what
           `‹ Back` from Recovery returns to (first-run-screen.tsx:2062-2096). */
        var made = s.v.made === 'yes';
        /* Same rule as the wide pane (first-run-screen.tsx:2035-2041), plus
           the account alias — which is the managed profile's, never typed. */
        var uname = vtext(s, 'username', LOCAL.username);
        var dname = vtext(s, 'dev', LOCAL.deviceName);
        var live = made || (uname.trim() && dname.trim());
        var body = '<h1>Create your account</h1>' +
          '<p class="lead">Choose how you appear on this server and name this Mac.</p>' +
          '<div class="local-field-card">' +
          '<label class="local-field-row"><span>Username</span>' + input({ value: uname, disabled: made, bind: made ? null : 'v.username' }) + '</label>' +
          '<label class="local-field-row"><span>This Mac’s name</span>' + input({ value: dname, disabled: made, bind: made ? null : 'v.dev' }) + '</label>' +
          '</div>' +
          '<details class="local-more"' + (open ? ' open=""' : '') + '>' +
          '<summary data-act="set" data-key="v.disclosure" data-val="' + (open ? '' : 'open') + '">More options</summary>' +
          '<label class="local-field-row"><span>Email (optional)</span>' + input({ placeholder: 'you@example.net', value: '', disabled: made }) + '</label>' +
          '<label class="local-field-row"><span>Invite (optional)</span>' + input({ value: '', disabled: made }) + '</label>' +
          '</details>' +
          (s.v.refusal === 'resume' ? '<p class="crit">That account setup is no longer pending.</p>' : '') +
          /* openExisting('account') — existingBack is the account pane, not
             `local` (first-run-screen.tsx:2103), so `‹ Back` over there
             returns to this managed pane. */
          lnk('Recover an existing account…', 'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"local-account","v.path":"own","v.made":"' +
            (made ? 'yes' : '') + '","v.recovery":"","v.phrase":"","v.refusal":""}\'', 'local-recover-link');
        return {
          /* The foot rows are `managedLocal && !checkpoint.account`, so they
             go once the account exists (first-run-screen.tsx:3140-3149). */
          side: setupSide({ local: true, current: 1, managedNoAccount: !made, recoverEnabled: localReady(s) }),
          main: pane({
            body: body,
            foot: foot({
              back: 'data-act="go" data-page="frl-local"',
              children: UI.btn(made ? 'Continue' : (s.v.pending === 'yes' ? 'Resume account setup' : 'Create my account'), {
                variant: 'primary', disabled: !live,
                attrs: live ? 'data-act="go" data-page="frl-protect" data-set=\'{"v.disclosure":"","v.written":"","v.result":"","v.made":"yes"}\'' : '',
              }),
            }),
          }),
        };
      },
    });

    M.page({
      id: 'frl-protect',
      title: 'Keep access to your account',
      path: ['First run', 'Managed local server', '3 · Recovery'],
      note: 'state=protect/phrase with managedLocal: the phrase is revealed inline, not in a sheet.',
      controls: [
        { key: 'disclosure', label: 'Phrase', note: 'state=protect (collapsed) vs state=phrase (the 17 tokens inline).', values: [{ v: '', label: 'Collapsed' }, { v: 'open', label: 'Shown' }] },
        { key: 'written', label: 'Backup phrase', note: 'phraseWritten while the phrase is on screen — `Start using FOKS` stays disabled until the box is ticked. `Already committed` is checkpoint.backupCommitted instead: the collapsed card’s button then reads `Recovery phrase saved` and is spent (first-run-screen.tsx:2455-2463). Collapsing does not commit, so that only comes back with a stored checkpoint.', values: [{ v: '', label: 'Not ticked' }, { v: '1', label: 'Written down' }, { v: 'saved', label: 'Already committed' }] },
        { key: 'result', label: 'Prepared phrase', note: 'prepareOwnerBackup runs in an effect when the phrase is revealed; until it answers the card says so instead of listing tokens.', values: [{ v: '', label: 'Prepared' }, { v: 'preparing', label: 'Still preparing' }] },
        { key: 'refusal', label: 'Refusal', note: 'The `.crit` under the cards: `Start using FOKS` commits the phrase, and the bridge refuses one it did not prepare (mock-bridge.ts:774-779).', values: [{ v: '', label: 'None' }, { v: 'commit', label: 'Phrase no longer matches' }] },
        { key: 'returning', label: 'Returning', note: 'checkpoint.returning — this pane’s `‹ Back` is `go(returning ? \'existing\' : \'account\')` (first-run-screen.tsx:2389), so after a recovery it returns to `existing` rather than to Create your account.', values: [{ v: '', label: 'First time' }, { v: 'yes', label: 'Recovered' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var shown = s.v.disclosure === 'open';
        var ticked = s.v.written === '1';
        var committed = s.v.written === 'saved';
        var prepared = s.v.result !== 'preparing';
        var card = shown
          ? (prepared
              ? '<div class="words">' + D.backupPhraseWords.map(function (w, i) {
                  return '<div class="word"><i>' + (i + 1) + '</i>' + esc(w) + '</div>';
                }).join('') + '</div>'
              : '<p>Preparing the one-time phrase…</p>') +
            '<label class="local-confirm"><input type="checkbox"' + (ticked ? ' checked=""' : '') +
            ' style="" data-act="set" data-key="v.written" data-val="' + (ticked ? '' : '1') + '">' +
            '<span>I have written down all 17 tokens.</span></label>' +
            lnk('Hide recovery phrase', 'data-act="set" data-key="v.disclosure" data-val=""', 'local-quiet-link')
          /* Collapsed the button is the reveal — or, once the phrase has been
             committed, the spent label (first-run-screen.tsx:2455-2463). */
          : UI.btn(committed ? 'Recovery phrase saved' : 'Show recovery phrase', {
              variant: 'primary', disabled: committed,
              attrs: committed ? '' : 'data-act="set" data-key="v.disclosure" data-val="open"',
            });
        var body = '<h1>Keep access to your account</h1>' +
          '<p class="lead">Set up a backup phrase now so you can recover your account if this Mac is lost.</p>' +
          '<div class="local-recovery-card">' +
          '<div class="local-recovery-head"><h2>Backup phrase</h2>' + UI.chip('Recommended') + '</div>' +
          '<p>Write down these 17 tokens and keep them somewhere other than this Mac. Anyone with them can recover your account.</p>' +
          card + '</div>' +
          '<div class="local-other-protection">' +
          '<div class="local-quiet-card"><b>Passphrase</b><span>Add one later from Settings.</span></div>' +
          '<div class="local-quiet-card"><b>Security key</b><span>Enroll a YubiKey later from Settings.</span></div>' +
          '</div>' +
          (s.v.refusal === 'commit' ? '<p class="crit">The prepared backup phrase no longer matches.</p>' : '');
        return {
          side: setupSide({ local: true, current: 2 }),
          main: pane({
            body: body,
            foot: foot({
              back: s.v.returning === 'yes'
                ? 'data-act="go" data-page="frx-existing" data-set=\'{"v.back":"local","v.made":"yes","v.recovery":"","v.phrase":"","v.refusal":""}\''
                : 'data-act="go" data-page="frl-account" data-set=\'{"v.made":"yes"}\'',
              note: lnk('Do this later', 'data-act="go" data-page="frl-local-done"'),
              /* disabled={busy || (state === 'phrase' && (!backupPhrase ||
                 !phraseWritten))} — collapsed it is always live, because
                 going on without a phrase is a first-class answer. */
              children: UI.btn('Start using FOKS', {
                variant: 'primary', disabled: shown && (!ticked || !prepared),
                attrs: (shown && (!ticked || !prepared)) ? '' : 'data-act="go" data-page="frl-local-done"',
              }),
            }),
          }),
        };
      },
    });

    M.page({
      id: 'frl-local-done',
      title: 'Your Personal vault is ready',
      path: ['First run', 'Managed local server', '4 · Vault ready'],
      note: 'state=local-done. The item count is the resolved account store’s, from the world — seven in the fixture, which is the only branch a capture has.',
      controls: [
        { key: 'empty', label: 'Personal vault', note: 'The preview body has three branches (first-run-screen.tsx:2513-2519) and the fixture only ever reaches the last: `Personal vault unavailable` when `addedStore` does not resolve, `No items yet` at zero, else plural(n, "item") — whose singular the fixture never shows either.', values: [{ v: '', label: '7 items (fixture)' }, { v: 'one', label: '1 item' }, { v: 'yes', label: 'No items yet' }, { v: 'gone', label: 'Store unresolved' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var w = D.world(s);
        var store = s.v.empty === 'gone' ? null : (w.storeById ? w.storeById['acct:personal'] : null);
        var count = w.items.filter(function (i) { return i.store === 'acct:personal'; }).length;
        if (s.v.empty === 'yes') count = 0;
        if (s.v.empty === 'one') count = 1;
        var body = '<div class="local-success">' +
          '<span class="local-success-mark" aria-hidden="true">✓</span>' +
          '<h1>Your Personal vault is ready</h1>' +
          '<p class="lead">Your account is connected to the local server on this Mac.</p>' +
          '<div class="local-vault-preview"><div class="local-vault-head">' + M.icon('vault') +
          '<b>Personal</b><code>' + esc(FACTS.own.report.canonicalName) + '</code></div>' +
          '<div class="local-vault-empty">' +
          (!store ? 'Personal vault unavailable' : (count === 0 ? 'No items yet' : esc(D.plural(count, 'item')))) +
          '</div></div></div>';
        return {
          side: setupSide({ local: true, current: 3 }),
          main: pane({
            body: body,
            foot: foot({
              children: UI.btn('Open Personal', {
                variant: 'primary', disabled: !store,
                attrs: store ? 'data-act="go" data-page="store-personal"' : '',
              }),
            }),
          }),
        };
      },
    });

    /* ==================================================================== */
    /* existing — Add this Mac to your account (recovery / pairing)         */
    /* ==================================================================== */
    M.page({
      id: 'frx-existing',
      title: 'Add this Mac to your account',
      path: ['First run', 'Recover or pair', '4 · Add this Mac'],
      note: 'state=existing. Reached from `who` (Add this as a secondary device), from `account` (I already have an account on this server), or from `local` (Recover an existing account…). `Recover` and `Accept pairing` need their secret typed first, exactly as the app has them.',
      controls: [
        { key: 'card', label: 'Cards', note: 'The bridge behind the pane. Both cards are always drawn; only the recovery card’s `Account alias` row is native-only (first-run-screen.tsx:2222-2229), and a selected Go CLI candidate also retitles the pairing card — and adds the copy card, but only when that candidate is copyable, which the `Go CLI candidate` chips decide.', values: [{ v: '', label: 'Web bridge' }, { v: 'pair', label: 'Native bridge' }, { v: 'cli', label: 'Go CLI candidate' }] },
        { key: 'path', label: 'Path', note: '`existing` is not path-forced: reached from the invited `account` screen it names their server and the username you picked there, and the sixth sidebar step still reads `Wait to be added`.', values: [{ v: 'own', label: 'Own' }, { v: 'invited', label: 'Invited' }] },
        { key: 'pending', label: 'Pending operation', note: 'A matching account-recovery row makes the recover button read `Resume recovery`. `Resume acceptance` is always drawn.', values: [{ v: '', label: 'None' }, { v: 'yes', label: 'Resumable' }] },
        { key: 'refusal', label: 'Refusal', note: 'The `.crit` under the cards (first-run-screen.tsx:2360): `Resume acceptance` with nothing pending, or `Resume recovery` with no matching account-recovery row (mock-bridge.ts:790-800).', values: [{ v: '', label: 'None' }, { v: 'resume', label: 'Pairing not pending' }, { v: 'recovery', label: 'Recovery not pending' }] },
        { key: 'back', label: 'Back target', note: 'existingBack: `account` (came from Create an account), `checked` (returning), `local` (the managed `Recover an existing account…`), `local-account` (the same link on the managed Create your account pane, which sets existingBack to `account` — still the managed world, so ‹ Back returns to that pane).', values: [{ v: '', label: 'account' }, { v: 'checked', label: 'checked' }, { v: 'local', label: 'local' }, { v: 'local-account', label: 'local · account' }] },
        { key: 'made', label: 'Account exists', note: 'checkpoint.account. The managed sidebar foot rows are `managedLocal && !account` (first-run-screen.tsx:3140-3149), so they go once the account is made — which is the state the `local-existing` capture froze.', values: [{ v: '', label: 'Not yet' }, { v: 'yes', label: 'Already created' }] },
        { key: 'recovery', label: 'Backup phrase', note: 'The typed recovery phrase — `Recover` is disabled while it is empty (first-run-screen.tsx:2247-2252). Type in the field to fill it.', values: [{ v: '', label: 'Empty' }, { v: 'typed', label: 'Typed' }] },
        { key: 'phrase', label: 'Pairing phrase', note: 'The typed pairing phrase — `Accept pairing` is disabled while it is empty; `Resume acceptance` never needs it.', values: [{ v: '', label: 'Empty' }, { v: 'typed', label: 'Typed' }] },
        CANDIDATE_CONTROL,
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var local = s.v.back === 'local' || s.v.back === 'local-account';
        /* managedLocal forces the own path (first-run-state.ts) — otherwise
           `existing` keeps whichever path the checkpoint holds. */
        var path = (!local && s.v.path === 'invited') ? 'invited' : 'own';
        var f = FACTS[path];
        var canonical = local ? LOCAL.probe : f.report.canonicalName;
        var card = s.v.card || '';
        var native = card === 'pair' || card === 'cli';
        var cli = card === 'cli';
        /* With a Go candidate the app fills `recoveryAlias` from the
           candidate's username, slugged (first-run-screen.tsx:1779-1783), not
           from the fixture's account alias. Which candidate rode over is
           `v.choice`; the walkthrough picks the first, `rae`. */
        var cand = cli ? (candidateOf(s) || GO_CANDIDATES[0]) : null;
        var alias = cand ? goSlug(cand.username) : f.accountAlias;
        var phrase = !!s.v.recovery;
        var pairphrase = !!s.v.phrase;
        var protectPage = local ? 'frl-protect' : (path === 'invited' ? 'fri-protect' : 'fr-protect');
        var accountPage = local ? 'frl-account' : (path === 'invited' ? 'fri-account' : 'fr-account');
        var backPage = s.v.back === 'local' ? 'frl-local'
          : (s.v.back === 'checked' ? (path === 'invited' ? 'fri-checked' : 'fr-checked') : accountPage);
        /* Every one of these ends in `account-complete`, so the account
           exists on the far side. On the managed path `‹ Back` from Recovery
           then reads `checkpoint.returning` and returns here, not to the
           account pane (first-run-screen.tsx:2389). */
        var onward = 'data-act="go" data-page="' + protectPage +
          '" data-set=\'{"v.written":"","v.refusal":"","v.disclosure":"","v.recovery":"","v.phrase":"","v.made":"yes"' +
          (local ? ',"v.returning":"yes"' : '') + '}\'';

        var recoverRows = '';
        if (native) {
          var ra = rr();
          recoverRows += UI.insetRow({ label: 'Account alias', forId: ra, value: input({ id: ra, value: alias }) });
        }
        var ph = rr(), dv = rr();
        recoverRows += UI.insetRow({ label: 'Phrase', forId: ph, value: input({ ariaLabel: 'Backup phrase', placeholder: 'token token token …', id: ph, type: 'password', value: s.v.recovery || '', bind: 'v.recovery' }) });
        recoverRows += UI.insetRow({ label: 'This Mac’s name', forId: dv, value: input({ id: dv, value: f.deviceName }) });

        var card1 = '<div class="pcard"><h3>Recover with your backup phrase</h3>' +
          '<p>Enter all 17 tokens from your backup phrase to add this Mac as an owner device.</p>' +
          UI.inset(recoverRows, { className: 'recovery-fields' }) +
          /* disabled={busy || !recoveryTargetAlias || !recoveryPhrase.trim() ||
             !deviceName.trim()} — the alias and the device name are prefilled,
             so the phrase is the one that holds it back. */
          '<div class="btns">' + UI.btn(s.v.pending === 'yes' ? 'Resume recovery' : 'Recover', {
            variant: 'primary', disabled: !phrase,
            attrs: phrase ? onward : '',
          }) + '</div></div>';

        var pa = rr(), pd = rr(), pp = rr();
        var card2 = '<div class="pcard"><h3>' +
          (cli ? 'Pair from the official FOKS CLI' : 'Pair from a Mac you already use') + '</h3>' +
          (cli
            ? '<p>In Terminal, switch the official FOKS CLI to this account, then run:</p>' +
              UI.copyBox({ display: '<code>foks --simple-ui key assist</code>', attrs: 'data-act="toast" data-text="Command copied."' }) +
              '<p>Confirm the account, paste its key-exchange code below, and leave the command running until this Mac connects.</p>'
            : '<p>On another signed-in Mac, open Settings › Recovery devices and choose Start pairing.</p>') +
          UI.inset(
            UI.insetRow({ label: 'Account alias', forId: pa, value: input({ ariaLabel: 'Pairing account alias', id: pa, value: alias }) }) +
            UI.insetRow({ label: 'This Mac’s name', forId: pd, value: input({ ariaLabel: 'Pairing device name', id: pd, value: f.deviceName }) }) +
            UI.insetRow({ label: 'Pairing phrase', forId: pp, value: input({ ariaLabel: 'Pairing phrase', placeholder: 'short phrase from the other Mac', id: pp, type: 'password', value: s.v.phrase || '', bind: 'v.phrase' }) }),
            { className: 'recovery-fields' }) +
          '<div class="btns">' +
          UI.btn('Accept pairing', {
            disabled: !pairphrase,
            attrs: pairphrase ? onward : '',
          }) +
          UI.btn('Resume acceptance', { attrs: 'data-act="set" data-key="v.refusal" data-val="resume"' }) +
          '</div></div>';

        /* `{goCandidate?.copyable ? … : null}` — the third card is the
           candidate's, not the CLI import's: a passphrase-backed profile can
           be paired but never copied (first-run-screen.tsx:2337). */
        var card3 = '';
        if (cand && cand.copyable) {
          var ca = rr();
          card3 = '<div class="pcard"><h3>Copy this Mac’s CLI device</h3>' +
            '<p>Advanced: both apps will use the same FOKS device. macOS may request Keychain access. Revoking it disables both, and CLI passphrase changes will not alter this desktop copy.</p>' +
            UI.inset(UI.insetRow({ label: 'Account alias', forId: ca, value: input({ ariaLabel: 'Copied account alias', id: ca, value: alias }) }),
              { className: 'recovery-fields' }) +
            UI.btn('Copy existing device', { attrs: onward }) + '</div>';
        }

        var body = '<h1>Add this Mac to your account</h1>' +
          '<p class="lead">Your account already exists on ' + esc(canonical) +
          '. Add this Mac with your backup phrase or approve it from another signed-in Mac.</p>' +
          '<div class="two">' + card1 + card2 + card3 + '</div>' +
          (s.v.refusal === 'resume' ? '<p class="crit">That pairing acceptance is no longer pending.</p>'
            : s.v.refusal === 'recovery' ? '<p class="crit">That recovery is no longer pending.</p>' : '') +
          '<p class="hint">This Mac will become a device for <code>' + esc(f.username) +
          '</code>. You can also use an enrolled YubiKey.</p>';
        return {
          /* Reached from `local` there is no account yet, so the managed foot
             rows are still drawn and `Recover account` is live (the report is
             what let you press it in the first place). Once the account
             exists they go, exactly as they do on the managed account pane. */
          side: local
            ? setupSide({ local: true, current: 1, managedNoAccount: s.v.made !== 'yes', recoverEnabled: true })
            : setupSide({ current: 3, path: path }),
          main: pane({
            wide: true, body: body,
            foot: foot({
              back: 'data-act="go" data-page="' + backPage + '"',
              children: UI.btn('I don’t have an account yet', { attrs: 'data-act="go" data-page="' + accountPage + '"' }),
            }),
          }),
        };
      },
    });

    /* ==================================================================== */
    /* the FOKS Go CLI import (native only — transcribed from source)       */
    /* ==================================================================== */
    M.page({
      id: 'frg-scan',
      title: 'Looking for existing FOKS accounts',
      path: ['First run', 'FOKS CLI import', '2 · Looking for accounts'],
      note: 'Native only: `who` while discoverGoProfiles() is in flight. The mock bridge is not native, so this is transcribed from first-run-screen.tsx:1746-1755.',
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var body = '<h1>Looking for existing FOKS accounts</h1>' +
          '<p class="lead">Checking the official FOKS client’s standard local profile store. Nothing is changed or unlocked.</p>' +
          UI.btn('Checking…', { disabled: true });
        return {
          side: setupSide({ current: 1, path: null }),
          main: pane({ body: body }),
        };
      },
    });

    /* GO_CANDIDATES / goShort / goSlug / candidateOf are at the top of this
       file, beside the other fixtures: `fr-address`, `fr-error` and
       `frx-existing` read the picked candidate too. */
    M.page({
      id: 'frg-chooser',
      title: 'FOKS is already set up on this Mac',
      path: ['First run', 'FOKS CLI import', '2 · Choose an account'],
      note: 'Native only: `who` once discoverGoProfiles() returns pairable or copyable candidates (go-profile-chooser.tsx).',
      controls: [
        { key: 'choice', label: 'Candidate', note: 'The chosen candidateId; `Connect selected account` stays disabled until one is picked.', values: [{ v: '', label: 'Nothing' }, { v: 'rae', label: 'First candidate' }, { v: 'ops', label: 'Second candidate' }, { v: 'draft', label: 'Third (unusable)' }] },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        rid = 0;
        var picked = s.v.choice || null;
        var rows = GO_CANDIDATES.map(function (c) {
          var unavailable = !c.pairable && !c.copyable;
          var status = c.provisional ? 'Incomplete'
            : c.hidden ? 'Hidden'
            : c.copyable ? 'Pair or copy'
            : c.pairable ? 'Pair' : 'Unavailable';
          var on = picked === c.candidateId;
          return UI.insetRow({
            label: esc(c.username || ('Account ' + goShort(c.userId))),
            value: '<span>' + esc(c.serverHint || ('Server ' + goShort(c.hostId))) + ' ' + UI.chip(status) + '</span>' +
              '<small>' + esc(c.role) + ' · ' + esc(c.storageKind) + ' · device ' + esc(goShort(c.deviceId)) + '</small>',
            action: UI.btn(on ? 'Selected' : 'Select', {
              size: 'sm', variant: on ? 'primary' : undefined, disabled: unavailable,
              attrs: unavailable ? '' : 'data-act="set" data-key="v.choice" data-val="' + c.candidateId + '"',
            }),
          });
        }).join('');
        var body = '<h1>FOKS is already set up on this Mac</h1>' +
          '<p class="lead">Choose an account from the official FOKS command-line client. The next step adds this desktop as a separate device or copies the existing device with your approval.</p>' +
          UI.inset(rows, { className: 'settings-inset middle' }) +
          '<div class="actions">' +
          UI.btn('Connect selected account', {
            variant: 'primary', disabled: !picked,
            attrs: picked ? 'data-act="go" data-page="fr-address" data-set=\'{"v.path":"own","v.returning":"yes","v.card":"cli","v.made":"","v.recovery":"","v.phrase":""}\'' : '',
          }) +
          UI.btn('Set up another account', { attrs: 'data-act="go" data-page="fr-who" data-set=\'{"v.choice":""}\'' }) +
          '</div>';
        return {
          side: setupSide({ current: 1, path: null }),
          main: pane({ wide: true, body: body }),
        };
      },
    });
  })();
