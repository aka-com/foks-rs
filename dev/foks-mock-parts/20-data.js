  /* ------------------------------------------------------------ 20-data.js
     The world every page draws from.

       M.icons              the 37 icon bodies from foks-ui/src/icons.ts, keyed
                            by name; M.icon(name) wraps them exactly as
                            components/icon.tsx does.
       M.app(...)           the six app-wide state groups, in the brief's order.
       M.data               the fixture (foks-ui/src/fixture.ts) as typed data,
                            plus the model helpers pages need.
       M.data.world(s)      the fixture adjusted for s.acme / s.agent — server
                            states, per-store state/description/notice, the
                            All-items bands, the alerts list and its badge
                            count. Pages call this instead of re-deriving.

     Nothing here renders markup; 30-shell.js owns the component helpers. */

  /* ------------------------------------------------------------------ icons
     Each value is the *inner* SVG body — what sits between the <svg class="ic"
     …> that M.icon writes and </svg>. Element order and attribute order match
     foks-ui/src/icons.ts entry for entry, so an icon lifted out of the mock is
     byte-comparable with one lifted out of the app. 37 names, in declaration
     order: key term file link people person vault search grid list chev info
     download copy path trash eye eyeoff bell plus arrow x gear server check
     pencil back shield alert minus again more out mail flag plug door. */
  M.icons = {
    key: '<circle cx="8" cy="14" r="4"/><path d="M11 11l9-9M17 5l2.5 2.5M14.5 7.5 17 10"/>',
    term: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
    file: '<path d="M6 3h8l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path d="M14 3v4h4"/>',
    link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
    people: '<circle cx="9" cy="8" r="3.2"/><path d="M3 19c0-3.3 2.7-5.5 6-5.5s6 2.2 6 5.5"/><path d="M15.5 5.2a3.2 3.2 0 0 1 0 5.6"/><path d="M17 13.6c2.4.5 4 2.5 4 5.4"/>',
    person: '<circle cx="12" cy="8" r="3.6"/><path d="M5 20c.6-4 3.4-6 7-6s6.4 2 7 6"/>',
    vault: '<rect x="3" y="4" width="18" height="16" rx="2.5"/><circle cx="12" cy="12" r="3.5"/><path d="M12 8.5v1.5M12 14v1.5M8.5 12H10M14 12h1.5"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    grid: '<rect x="4" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.5"/>',
    list: '<path d="M9 6h11M9 12h11M9 18h11"/><circle cx="5" cy="6" r="1" fill="currentColor"/><circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="5" cy="18" r="1" fill="currentColor"/>',
    chev: '<path d="M6 9l6 6 6-6"/>',
    info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>',
    download: '<path d="M12 4v11M7 10l5 5 5-5M4 20h16"/>',
    copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a1 1 0 0 1 1-1h10"/>',
    path: '<circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path d="M8 6h5a4 4 0 0 1 0 8h-2a4 4 0 0 0 0 4h5"/>',
    trash: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
    eye: '<path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6-10-6-10-6z"/><circle cx="12" cy="12" r="3"/>',
    eyeoff: '<path d="M3 3l18 18M10.6 10.6a2 2 0 0 0 2.8 2.8M6.6 6.6C4 8.2 2 12 2 12s3.5 6 10 6c1.7 0 3.2-.4 4.4-1M9.9 6.2C10.5 6.1 11.2 6 12 6c6.5 0 10 6 10 6s-.8 1.4-2.3 2.9"/>',
    bell: '<path d="M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 21h4"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
    x: '<path d="M6 6l12 12M18 6L6 18"/>',
    gear: '<path d="M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.3a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4A2 2 0 0 0 4 9.9l.2.1a2 2 0 0 1 1 1.7v.5a2 2 0 0 1-1 1.8l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.3a2 2 0 0 1 1 1.7v.2a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.3a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.8v-.5a2 2 0 0 1 1-1.7l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.3a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
    server: '<rect x="3" y="4" width="18" height="6" rx="1.5"/><rect x="3" y="14" width="18" height="6" rx="1.5"/><path d="M7 7h.01M7 17h.01"/>',
    check: '<path d="M5 12l5 5 9-10"/>',
    pencil: '<path d="M4 20l4.5-1L19 8.5a2 2 0 0 0-3-3L5.5 16z"/><path d="M14 7l3 3"/>',
    back: '<path d="M15 5l-7 7 7 7"/>',
    shield: '<path d="M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z"/><path d="M9 12l2 2 4-4"/>',
    alert: '<circle cx="12" cy="12" r="8.5"/><path d="M12 8v5M12 16h.01"/>',
    minus: '<path d="M5 12h14"/>',
    again: '<path d="M20 12a8 8 0 1 1-2.6-5.9"/><path d="M20 4v4h-4"/>',
    more: '<circle cx="5" cy="12" r="1.2" fill="currentColor"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="19" cy="12" r="1.2" fill="currentColor"/>',
    out: '<path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
    mail: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 7l9 6 9-6"/>',
    flag: '<path d="M5 21V4h13l-2.5 4L18 12H5"/>',
    plug: '<path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4"/>',
    door: '<path d="M5 21V3h9v18M14 21h5M3 21h2"/><circle cx="11.5" cy="12" r="1" fill="currentColor"/>',
  };
  /* Every icon name, in declaration order (icons.ts FOKS_ICON_NAMES). */
  M.iconNames = Object.keys(M.icons);

  /* ------------------------------------------------- app-wide state groups
     Six keys, in the brief's order. Pages read M.s.agent, M.s.acme, … .  */

  M.app({
    key: 'agent',
    label: 'Agent',
    note: 'AgentStatus.phase (model/types.ts:191) plus the socket loss the shell polls once a second (app-root.tsx:431-456).',
    values: [
      { v: 'ready', label: 'Ready', hint: 'phase "Ready" — the titlebar pill reads "Agent ready" with a green dot.' },
      { v: 'starting', label: 'Starting', hint: 'phase "Bootstrap" — <span class="agent warn">, dot --warning-dot, text "Agent starting".' },
      { v: 'lost', label: 'Lost', hint: 'takeAgentConnectionLoss returned a message: the .stopwrap full-window stop covers everything below the titlebar. Retry sets this back to ready. The pill itself still reads "Agent ready" — the phase is a separate fact.' },
    ],
  });

  M.app({
    key: 'acme',
    label: 'Acme server',
    note: 'The state of foks.acme-corp.com, which carries Work (Acme) and Engineering. Drives storeDescriptionState → the sidebar caption, the page heading, the store takeover, the All-items band and the Alerts list.',
    values: [
      { v: 'ok', label: 'Reachable', hint: 'server.state "ok", lease { fresh, 12 d } — the world FIXTURE = applyLease(RAW, "fresh") starts in.' },
      { v: 'lapsed', label: 'Check-in expired', hint: 'applyLease(world, "lapsed"): server.state "lease-lapsed", lease { lapsed, null }. Work and Engineering stop, the sidebar captions become "Connection error", All items drops to 10 rows behind a band, the Alerts badge goes 2 → 3.' },
      { v: 'checkin-unavailable', label: 'Status unavailable', hint: 'server.state "lease-unavailable": FOKS has no signed check-in. Takeover "Server status unavailable", action Review server.' },
      { v: 'unavailable', label: 'Stores unlistable', hint: 'the catalog could not list Acme’s stores: world.unavailableStores = [acct:work, team:eng]. Store fact, not a server one — takeover "Store connection failed".' },
      { v: 'unprobed', label: 'Never checked', hint: 'server.state "never-probed": takeover "Server not checked yet", action Review server.' },
      { v: 'blocked', label: 'Access blocked', hint: 'server.state "blocked": a protocol safety check refused the server. Takeover "Server access blocked", action Review server.' },
    ],
  });

  M.app({
    key: 'lock',
    label: 'App lock',
    note: 'AppLockState (bridge.ts:342-347). Read before loadWorld; a locked app renders only the overlay. Unreachable in the real mock bridge (it starts unlocked and nothing calls lockApp) — transcribed from app-root.tsx:172-223.',
    values: [
      { v: 'off', label: 'Unlocked', hint: 'locked:false — the ordinary shell.' },
      { v: 'locked', label: 'Locked', hint: '.app-lock dialog "Unlock FOKS"; the mock bridge’s mechanism is "password", so the copy says "your operating-system password".',
        band: 'The app lock cannot be reached in the running app — the mock bridge starts unlocked and nothing calls lockApp — so this screen is transcribed from app-root.tsx:172-223 rather than captured.' },
    ],
  });

  M.app({
    key: 'boot',
    label: 'Boot',
    note: 'The first two things App can render instead of the shell (app-root.tsx:225-254). Neither is reachable with the mock bridge, which never throws at boot.',
    values: [
      { v: 'ok', label: 'Loaded', hint: 'the world loaded — VaultShell.' },
      { v: 'loading', label: 'Loading', hint: '<div class="app-loading">Connecting to the local agent…</div>.',
        band: 'The boot placeholder cannot be reached in the running app — the mock bridge answers loadWorld immediately — so it is transcribed from app-root.tsx:225-254.' },
      { v: 'error', label: 'Load error', hint: '.app-lock alertdialog "Couldn’t load FOKS" (curly apostrophe) with the normalized error message and Retry.',
        band: 'The boot-error screen cannot be reached in the running app — the mock bridge never throws at boot — so it is transcribed from app-root.tsx:225-254.' },
    ],
  });

  M.app({
    key: 'managed',
    label: 'Managed profile',
    note: 'appInfo().managedProfile (bridge.ts:308-315). Its only structural effect is which first-run step the shell enters when there is no account store — "local" for a managed profile on an ok server, otherwise "who".',
    values: [
      { v: 'none', label: 'None', hint: 'the mock bridge supplies none: appInfo is { version 0.3.0, agentSocket /private/foks/agent.sock }.' },
      { v: 'local', label: 'Local', hint: 'a launcher-prepared, already authenticated profile: the shorter (3-step) first run and an extra Settings › About line.' },
    ],
  });

  M.app({
    key: 'refreshing',
    label: 'Refreshing',
    note: 'refreshingWorld (app-root.tsx:306, 610-624) — the titlebar refresh button while a loadWorld is in flight.',
    values: [
      { v: 'no', label: 'Idle', hint: 'aria-label "Refresh", title "Refresh vaults and groups".' },
      { v: 'yes', label: 'In flight', hint: 'disabled; both aria-label and title become "Refreshing vaults and groups".' },
    ],
  });

  /* ==================================================================== data
     foks-ui/src/fixture.ts, transcribed. Ids and names are the fixture's, not
     paraphrases: StoreRefs are acct:personal, acct:work, team:eng,
     team:household, team:homelab. `servers` below is RAW — Acme is pinned
     lapsed in the file and the shell applies applyLease(…, 'fresh') on top;
     M.data.world(s) does that lease application per world, so nothing else
     needs to know. */

  var D = (M.data = {});

  /* ---------------------------------------------------- servers (§3.1) */
  D.servers = [
    { id: 'personal', name: 'foks.example.net', label: 'Personal server', host_id: '9f31c2aa07', chain: 12, epoch: 4821, lease: { state: 'fresh', expires_in: '6 d' }, accounts: ['personal'], state: 'ok' },
    { id: 'acme', name: 'foks.acme-corp.com', label: 'Acme', host_id: 'b04d17e390', chain: 33, epoch: 90417, lease: { state: 'lapsed', expires_in: null }, accounts: ['work'], state: 'lease-lapsed' },
    { id: 'partner', name: 'foks.partner.dev', label: null, host_id: null, chain: null, epoch: null, lease: null, accounts: [], state: 'never-probed' },
  ];
  /* The full 66-hex host ids the mock bridge answers with (mock-bridge.ts:193-197). */
  D.hostIds = {
    personal: '02' + '9f31c2aa'.repeat(8),
    acme: '02' + 'b04d17e3'.repeat(8),
    partner: '02' + '04c8b19e'.repeat(8),
  };

  /* --------------------------------------------------- accounts (§3.2) */
  D.accounts = [
    { store: 'acct:personal', alias: 'personal', username: 'rae', server: 'personal' },
    { store: 'acct:work', alias: 'work', username: 'rae.chen', server: 'acme' },
  ];
  D.accountInventoryComplete = true;

  /* ----------------------------------------------------- stores (§3.3) */
  D.stores = [
    { id: 'acct:personal', kind: 'account', name: 'Personal', server: 'personal', account: 'personal' },
    { id: 'acct:work', kind: 'account', name: 'Work (Acme)', server: 'acme', account: 'work' },
    { id: 'team:eng', kind: 'team', name: 'Engineering', alias: 'engineering', server: 'acme', account: 'work', active: true, team_kind: 'named', team_id_hex: '033f91c2aa07b45e18d0c73a9f2e5b6417ac8d0192f3e4b5c6d7089a1b2c3d4e5f' },
    { id: 'team:household', kind: 'team', name: 'Household', alias: 'household', server: 'personal', account: 'personal', active: true, team_kind: 'named', team_id_hex: '037b83d4f0241a67933c59ed218458ce03a5b98e691e81c544b8cc93c52f4e02ab' },
    { id: 'team:homelab', kind: 'team', name: 'Homelab', alias: 'homelab', server: 'personal', account: 'personal', active: false, team_kind: 'adhoc', team_id_hex: '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6' },
  ];
  /* Stores the catalog knew but could not list this time (world.unavailableStores).
     Empty in the fixture; M.data.world adds Acme's two under acme=unavailable. */
  D.unavailableStores = [];

  /* ------------------------------------------------------ items (§3.5)
     All 15 fixture rows, including the /ssh Folder. A Folder is not an item:
     catalog() drops it and its name becomes the .pchip on the rows below it. */
  D.items = [
    { store: 'acct:personal', path: '/logins/github.com', kind: 'Secret', size: 142, version: 9, read: 'Owner', write: 'Owner', value: 'user: rae\npassword: ••••••••••••\nurl: https://github.com/login' },
    { store: 'acct:personal', path: '/logins/fastmail.com', kind: 'Secret', size: 96, version: 2, read: 'Owner', write: 'Owner', value: 'user: rae@fastmail.com\npassword: ••••••••••' },
    { store: 'acct:personal', path: '/env/prod/DATABASE_URL', kind: 'Secret', size: 96, version: 3, read: 'Owner', write: 'Owner', value: 'postgres://service:••••••••@db.internal/prod' },
    { store: 'acct:personal', path: '/ssh/id_ed25519', kind: 'File', size: 419, version: 1, read: 'Owner', write: 'Owner' },
    { store: 'acct:personal', path: '/ssh', kind: 'Folder', size: 0, version: 1, read: 'Owner', write: 'Owner' },
    { store: 'acct:personal', path: '/latest-key', kind: 'Link', size: 22, version: 2, read: 'Owner', write: 'Owner', target: '/ssh/id_ed25519' },
    { store: 'acct:personal', path: '/documents/passport-scan.pdf', kind: 'File', size: 2841992, version: 1, read: 'Owner', write: 'Owner' },
    { store: 'acct:personal', path: '/agents/anthropic-api-key', kind: 'Secret', size: 108, version: 4, read: 'Owner', write: 'Owner', value: 'sk-ant-••••••••••••••••' },
    { store: 'team:eng', path: '/deploy/production-token', kind: 'Secret', size: 88, version: 12, read: 'Admin', write: 'Owner', value: 'foks_team_token_••••••••' },
    { store: 'team:eng', path: '/deploy/staging-token', kind: 'Secret', size: 88, version: 3, read: 'Member · visibility 0', write: 'Admin', value: 'foks_team_token_••••••••' },
    { store: 'team:eng', path: '/release/bundle.tar', kind: 'File', size: 84399718, version: 5, read: 'Member · visibility 0', write: 'Admin' },
    { store: 'team:eng', path: '/onboarding/README.md', kind: 'File', size: 5120, version: 2, read: 'Member · visibility 0', write: 'Admin' },
    { store: 'team:household', path: '/wifi/guest-password', kind: 'Secret', size: 64, version: 4, read: 'Member · visibility 0', write: 'Admin', value: 'ssid: Chen-Guest\npassword: ••••••••' },
    { store: 'team:household', path: '/documents/emergency.pdf', kind: 'File', size: 2841992, version: 7, read: 'Member · visibility 0', write: 'Owner' },
    { store: 'team:household', path: '/streaming/netflix', kind: 'Secret', size: 70, version: 1, read: 'Member · visibility 0', write: 'Admin', value: 'user: family@example.net\npassword: ••••••••' },
  ];

  /* --------------------------------------------------- plaintext (§3.6)
     Keyed "<store>|<path>" — what an explicit, version-bound Show returns.
     A Link's "plaintext" is its target, so /latest-key reads /ssh/id_ed25519. */
  D.plaintext = {
    'acct:personal|/logins/github.com': 'hx7-Qm2!vTe9-pale-orbit',
    'acct:personal|/logins/fastmail.com': 'kettle-91-Lumen-tide',
    'acct:personal|/env/prod/DATABASE_URL': 'postgres://service:o4Kq9wLm2x@db.internal/prod',
    'acct:personal|/agents/anthropic-api-key': 'sk-ant-api03-Rf2xQ7bTn41La9mZ4',
    'team:eng|/deploy/production-token': 'foks_team_token_7f31ac09',
    'team:eng|/deploy/staging-token': 'foks_team_token_02be44d1',
    'team:household|/wifi/guest-password': 'sunny-kettle-42',
    'team:household|/streaming/netflix': 'popcorn-Sofa-77',
  };

  /* ---------------------------------------------------- parties (§3.7)
     Keyed by StoreRef, in sortRoster order (groups-screen.tsx:167 — rank
     descending, "you" first on a tie). For this fixture that is also the
     fixture's own order, so the sidebar Stack (which uses partiesOf, i.e.
     fixture order) draws the same first two either way.
     Each row carries the derived avatar facts: `name` (partyName), `initials`
     and `hue` (model/format.ts) so no page recomputes them. */
  D.parties = {
    'team:eng': [
      { store: 'team:eng', username: 'sam.ortiz', party_kind: 'user', generation: 4, locally_manageable: true, party_id_hex: '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5', source_role: { role: 'Owner' }, destination_role: { role: 'Owner' }, name: 'sam.ortiz', initials: 'SO', hue: '#a2845e' },
      { store: 'team:eng', username: 'rae.chen', label: 'you', party_kind: 'user', generation: 6, locally_manageable: true, party_id_hex: '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211', source_role: { role: 'Admin' }, destination_role: { role: 'Admin' }, name: 'rae.chen', initials: 'RC', hue: '#34c759' },
      { store: 'team:eng', username: 'priya.n', party_kind: 'user', generation: 5, locally_manageable: true, party_id_hex: '0184f0c2a71b3d95e6082c47fa19b6d3e05c84719fb2a6c308d5e71f94c3b0d726', source_role: { role: 'Admin' }, destination_role: { role: 'Admin' }, name: 'priya.n', initials: 'PN', hue: '#ff9f0a' },
      { store: 'team:eng', username: 'dana.okafor', party_kind: 'user', generation: 5, locally_manageable: true, party_id_hex: '012f8b6d13c07a4e5991b2f8d04a6c3e17bd50927fe83a1c4670d9b2e518f4a06c', source_role: { role: 'Member', visibility: 0 }, destination_role: { role: 'Member', visibility: 0 }, name: 'dana.okafor', initials: 'DO', hue: '#34c759' },
      { store: 'team:eng', username: null, party_kind: 'named-team', team_name: 'homelab @ foks.example.net', generation: 3, locally_manageable: false, party_id_hex: '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6', scoped_host_id_hex: '9f31c2aa07', source_role: { role: 'Owner' }, destination_role: { role: 'Member', visibility: 0 }, note: "a member group; change it from Engineering's Federation page (Groups), not from here", name: 'homelab @ foks.example.net', initials: '', hue: 'var(--c-team)' },
      { store: 'team:eng', username: 'deploy-bot', party_kind: 'user', generation: 6, locally_manageable: true, party_id_hex: '01c93d2f8b17e4a0148ab7f2e60155c9a71c6f93d23140e5b839d1c0f60ba48e57', source_role: { role: 'Member', visibility: 0 }, destination_role: { role: 'Member', visibility: 0 }, note: 'a service account — a user of foks.acme-corp.com like any other, so its role can be changed from here', name: 'deploy-bot', initials: 'DB', hue: '#a2845e' },
    ],
    'team:household': [
      { store: 'team:household', username: 'rae', label: 'you', party_kind: 'user', generation: 2, locally_manageable: true, party_id_hex: '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211', source_role: { role: 'Owner' }, destination_role: { role: 'Owner' }, name: 'rae', initials: 'R', hue: '#5e5ce6' },
      { store: 'team:household', username: 'sam', party_kind: 'user', generation: 2, locally_manageable: true, party_id_hex: '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5', source_role: { role: 'Member', visibility: 0 }, destination_role: { role: 'Member', visibility: 0 }, name: 'sam', initials: 'S', hue: '#ff9f0a' },
    ],
    /* Homelab is inactive and has no roster: the sidebar draws the neutral
       --c-none glyph in place of a stack. */
    'team:homelab': [],
  };
  /* Flat, in the fixture's own order — the shape world.parties has. */
  D.partyList = D.parties['team:eng'].concat(D.parties['team:household']);

  /* ------------------------------------------------- federation (§3.8)
     One admission, and it is inactive — which is why the member group reads
     nothing and staging-token/bundle.tar are readable by 5, not 6. */
  D.federation = [
    { store: 'team:eng', remote_profile: 'personal', remote_team_alias: 'homelab', remote_host_id_hex: '9f31c2aa07', remote_team_id_hex: '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6', destination: { role: 'Member', visibility: 0 }, operation_id_hex: '7c14a9f0', active: false },
  ];
  /* world.groupDetailFailures — empty in the fixture. 60-groups.js rewrites
     this array from its `failure` view key before it calls D.world(s), and
     D.world consults it (lease.ts:119-127 `storeDescription`), so a roster or
     federation failure changes the SIDEBAR caption too, not just the pane. */
  D.groupDetailFailures = [];
  /* lease.ts:107 groupDetailFailure(world, store, source). */
  D.groupDetailFailure = function (ref, source) {
    for (var i = 0; i < D.groupDetailFailures.length; i++) {
      var f = D.groupDetailFailures[i];
      if (f.store === ref && f.source === source) return f;
    }
    return undefined;
  };

  /* -------------------------------------- devices, YubiKeys, cards (§3.9) */
  D.devices = [
    { alias: 'personal', name: 'MacBook Pro', role: 'owner', current: true, id_hex: '02a779c40674942e2d0fc18aa8d59b2ee4fa95ea7a652310e3f13d2d317f170b22' },
    { alias: 'laptop', name: 'Travel Mac', role: 'owner', current: false, id_hex: '02c1e08d5f3a94b7d21e6f0c8a3b5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809' },
  ];
  /* The Macs pane lists an account's devices by the ids the mocked
     `list_devices` answers with, which are the fixture's rewritten 02… → 04…
     (map/settings.md §6); every account other than acct:personal gets one
     synthetic current row. Both facts live here so nothing has to guess them.
     70-settings.js still keeps its own copies (it owns the pane and cannot be
     edited this round) — these are the same values. */
  D.DEVICE_ID_PREFIX = '04';
  D.deviceIdFor = function (idHex) { return D.DEVICE_ID_PREFIX + String(idHex).slice(2); };
  D.syntheticDevice = { name: 'MacBook Pro', role: 'owner', current: true, id: '04' + new Array(65).join('8') };
  /* Backup-phrase enrollments per account: acct:personal has one, every other
     account none — "1 authenticated enrollment on this account" vs "No backup
     phrase for this account." */
  D.backupEnrollments = { 'acct:personal': [{ alias: 'paper-backup' }] };
  /* `state` is what one yubiAccount becomes as an enrollment row: the fixture
     has exactly one and it is complete, which is why Resume enrollment is
     always disabled with "No pending enrollment was reported". */
  D.yubi = {
    accounts: [{ alias: 'primary key', server: 'personal', serial: 20993145, state: 'complete' }],
    cardsConnected: [{ serial: 20993145 }],
  };
  /* The mock bridge's 17-word backup phrase (mock-bridge.ts:166-167). */
  D.backupPhrase = 'orbit velvet lantern cactus mirror harbor pistol thumb copper fossil meadow rotate silent wagon bright ladder ivory';
  D.backupPhraseWords = D.backupPhrase.split(' ');

  /* ---------------------------------------------------- alerts (§3.10)
     notesNow() drops lease-acme unless the world's leaseState is 'lapsed',
     so the badge is 2 fresh and 3 lapsed. Note the curly apostrophes. */
  D.alerts = [
    { id: 'lease-acme', severity: 'crit', title: 'foks.acme-corp.com is locked', detail: 'The server’s check-in expired. Work and Engineering groups are unavailable until the agent renews it.', action: 'Wait for the agent' },
    { id: 'team-homelab', severity: 'warn', title: 'Homelab is inactive', detail: 'Group setup incomplete. Items and members are unavailable until setup is finished.', action: 'Resume creation' },
    { id: 'fed-homelab', severity: 'warn', title: 'Homelab cannot access Engineering', detail: 'Homelab’s membership in Engineering is inactive. Homelab members cannot access Engineering until it is restored.', action: 'Restore access' },
  ];

  /* --------------------------------------------------- appInfo (§2.8) */
  D.appInfo = {
    version: '0.3.0',
    agentSocket: '/private/foks/agent.sock',
    managedProfile: null,
    computerName: null,
  };
  /* AppLockState the mock bridge answers with; mechanism decides the lock copy. */
  D.appLock = { locked: false, available: true, mechanism: 'password' };

  /* ------------------------------------------- sanctioned copy (§3.11) */
  D.COPY = {
    lease_lapsed: 'This server is locked until the agent renews its check-in. Items on this server are hidden and cannot be changed. Other servers are unaffected.',
    remove_item: 'FOKS removes the item only if its version has not changed. After removal, earlier versions cannot be read.',
    personal_store_fixed: 'Your Personal vault has no roster and cannot be shared. To share an item, put it in a group.',
    resumable: 'Select Resume to continue an interrupted operation. FOKS checks completed steps and does not repeat them.',
  };

  /* =============================================== model helpers (ported)
     model/format.ts, model/kinds.ts, model/roles.ts, model/readers.ts,
     model/order.ts, model/lease.ts, screens/scope.ts. Same names, same
     answers — pages must not re-implement any of this. */

  /* --- format.ts --- */
  D.HUES = ['#5e5ce6', '#ff9f0a', '#30b0c7', '#ff2d55', '#34c759', '#af52de', '#0a7cff', '#a2845e'];
  D.fmtSize = function (bytes) {
    if (bytes === 0) return '0 B';
    if (bytes < 1000) return bytes + ' B';
    if (bytes < 1e6) return (bytes / 1000).toFixed(bytes < 10000 ? 1 : 0) + ' KB';
    return (bytes / 1e6).toFixed(1) + ' MB';
  };
  D.initials = function (name) {
    return String(name).replace(/@.*/, '').split(/[.\-_ ]/).filter(Boolean).slice(0, 2)
      .map(function (w) { return w[0].toUpperCase(); }).join('');
  };
  D.hue = function (name) {
    var sum = 0;
    for (var i = 0; i < name.length; i++) sum += name.charCodeAt(i);
    return D.HUES[sum % D.HUES.length];
  };
  D.shortId = function (value, tail) {
    tail = tail == null ? 4 : tail;
    return value.length > 10 + tail ? value.slice(0, 10) + '…' + value.slice(-tail) : value;
  };
  D.plural = function (count, one, many) {
    return count + ' ' + (count === 1 ? one : (many || one + 's'));
  };

  /* --- kinds.ts --- */
  D.KINDS = {
    Password: { label: 'Password', plural: 'Passwords', icon: 'key', blurb: 'Keep passwords, secrets, and tokens here.' },
    Resource: { label: 'Note', plural: 'Notes', icon: 'term', blurb: 'Keep keys, tokens, connection strings, and notes here.' },
    File: { label: 'File', plural: 'Files', icon: 'file', blurb: 'Keep documents and files in encrypted storage.' },
    Link: { label: 'Link', plural: 'Links', icon: 'link', blurb: 'Keep links and shortcuts to other paths in the store.' },
  };
  D.KIND_LIST = ['Password', 'Resource', 'File', 'Link'];
  D.kindLabel = function (kind) { return D.KINDS[kind].label; };
  /* The client-side reading of the node type: a Secret with a `password:`
     line, or one under /logins/, is a Password; every other Secret is a
     Resource (shown as "Note"). File and Link pass through. */
  D.kindOf = function (item) {
    if (item.kind !== 'Secret') return item.kind;
    return /^password:/m.test(item.value || '') || item.path.indexOf('/logins/') === 0 ? 'Password' : 'Resource';
  };
  D.isLogin = function (item) {
    return D.kindOf(item) === 'Password' && item.path.indexOf('/logins/') === 0;
  };
  D.nameOf = function (path) { return path.slice(path.lastIndexOf('/') + 1) || '/'; };
  D.prefixOf = function (path) {
    var cut = path.lastIndexOf('/');
    return cut <= 0 ? '' : path.slice(1, cut);
  };
  D.rtype = function (item) {
    return { Secret: 'small_file', File: 'file', Link: 'symlink', Folder: 'directory' }[item.kind];
  };
  D.rtypeWords = function (item) {
    return { Secret: 'a Secret', File: 'a File node', Link: 'a symlink', Folder: 'a directory' }[item.kind];
  };

  /* --- roles.ts --- */
  var MEMBER_ROLE = /^Member(?:\s*·\s*visibility\s*(-?\d+))?$/;
  D.parseRole = function (wire) {
    var kinds = { Member: 'member', Admin: 'admin', Owner: 'owner' };
    if (wire && typeof wire !== 'string') {
      var k = kinds[wire.role];
      if (!k) return null;
      return k === 'member' ? { kind: k, visibility: wire.visibility == null ? 0 : wire.visibility } : { kind: k };
    }
    if (typeof wire !== 'string') return null;
    var m = MEMBER_ROLE.exec(wire);
    if (m) return { kind: 'member', visibility: m[1] != null ? +m[1] : 0 };
    var kk = kinds[wire];
    return kk ? { kind: kk } : null;
  };
  D.roleRank = function (role) {
    if (role == null) return 0;
    var parsed = typeof role !== 'string' && role.kind ? role : D.parseRole(role);
    if (!parsed) return 0;
    return { owner: 3, admin: 2, member: 1 }[parsed.kind];
  };
  D.visibilityOf = function (role) { return role.visibility == null ? 0 : role.visibility; };
  /* Refuses an unparseable role on either side — the rank comparison must not
     run first, or an unknown read role would be admitted by a Member. */
  D.admits = function (held, need) {
    var h = D.parseRole(held), n = D.parseRole(need);
    if (!h || !n) return false;
    var hr = D.roleRank(h), nr = D.roleRank(n);
    if (hr !== nr) return hr > nr;
    return h.kind === 'member' ? D.visibilityOf(h) >= D.visibilityOf(n) : true;
  };
  D.formatRole = function (role) {
    var r = typeof role === 'string' || !role.kind ? D.parseRole(role) : role;
    if (!r) return '';
    if (r.kind === 'owner') return 'Owner';
    if (r.kind === 'admin') return 'Admin';
    return 'Member · visibility ' + D.visibilityOf(r);
  };
  /* The three role labels and their ranks, for pickers and menus. */
  D.ROLES = [
    { id: 'Owner', label: 'Owner', rank: 3 },
    { id: 'Admin', label: 'Admin', rank: 2 },
    { id: 'Member', label: 'Member', rank: 1 },
  ];

  /* --- readers.ts --- */
  D.storeOf = function (ref, w) { return byId((w || D).stores, ref); };
  D.partiesOf = function (ref) { return D.parties[ref] || []; };
  D.partyName = function (party) {
    return party.username || party.team_name || party.party_id_hex.slice(0, 10) + '…';
  };
  D.peopleLabel = function (count) { return count + ' ' + (count === 1 ? 'person' : 'people'); };
  D.peopleGroups = function (parties) {
    var groups = parties.filter(function (p) { return p.party_kind !== 'user'; }).length;
    var people = parties.length - groups;
    var head = D.peopleLabel(people);
    if (!groups) return head;
    return head + ' · ' + groups + ' ' + (groups === 1 ? 'group' : 'groups');
  };
  /* A party that is a team reads through an admission; an inactive admission
     reads nothing, however good the role looks. */
  D.admissionActive = function (party, ref) {
    if (party.party_kind === 'user') return true;
    var entries = D.federation.filter(function (f) {
      return f.store === ref && f.remote_team_id_hex === party.party_id_hex &&
        (!party.scoped_host_id_hex || f.remote_host_id_hex === party.scoped_host_id_hex);
    });
    return entries.length === 1 && entries[0].active === true;
  };
  /* The roster filtered by the item's read role; null on an account store,
     which has no roster. Goldens: production-token 3, staging-token 5,
     bundle.tar 5, guest-password 2. */
  D.readersOf = function (item) {
    var store = byId(D.stores, item.store);
    if (!store || store.kind !== 'team') return null;
    return D.partiesOf(store.id).filter(function (p) {
      return D.admissionActive(p, store.id) && D.admits(p.destination_role, item.read);
    });
  };
  /* The "Readable by" cell: { label, title? }. An account store answers
     "only you"; the list row shows the bare count with the names as title. */
  D.readableBy = function (item) {
    var readers = D.readersOf(item);
    if (!readers) return { label: 'only you' };
    return { label: D.peopleLabel(readers.length), title: readers.map(D.partyName).join(', ') };
  };
  D.actionableGroupMember = function (party) {
    return !!(party.username && party.party_kind === 'user' && party.locally_manageable &&
      party.label !== 'you' &&
      D.partiesOf(party.store).filter(function (c) { return c.username === party.username; }).length === 1);
  };

  /* --- order.ts --- */
  D.storeNavigationOrder = function (stores) {
    stores = stores || D.stores;
    return stores.filter(function (s) { return s.kind === 'account'; })
      .concat(stores.filter(function (s) { return s.kind === 'team' && s.team_kind === 'named'; }))
      .concat(stores.filter(function (s) { return s.kind === 'team' && s.team_kind === 'adhoc'; }));
  };
  D.storeDisplayOrder = function (stores) {
    stores = (stores || D.stores).slice();
    var order = {}, src = {};
    D.servers.forEach(function (s, i) { order[s.id] = i; });
    stores.forEach(function (s, i) { src[s.id] = i; });
    return stores.sort(function (a, b) {
      return (order[a.server] - order[b.server]) ||
        (a.kind === b.kind ? 0 : a.kind === 'account' ? -1 : 1) ||
        (src[a.id] - src[b.id]);
    });
  };

  /* --- scope.ts --- */
  D.whereOf = function (item, w) {
    var s = byId((w || D).stores, item.store);
    return s ? s.name : item.store;
  };
  /* Every item in one store, folders included (pass {items:…} of a world to
     scope it to a lease world). */
  D.itemsIn = function (ref, items) {
    return (items || D.items).filter(function (i) { return i.store === ref; });
  };
  D.itemAt = function (ref, path) {
    return D.items.filter(function (i) { return i.store === ref && i.path === path; })[0];
  };
  D.itemKey = function (item) { return item.store + '|' + item.path; };
  /* The Show value: a Link reads its target, everything else its plaintext. */
  D.plaintextOf = function (item) {
    if (item.kind === 'Link') return item.target;
    return D.plaintext[D.itemKey(item)];
  };

  function byId(list, id) {
    for (var i = 0; i < list.length; i++) if (list[i].id === id) return list[i];
    return undefined;
  }

  /* ============================================== the world for a state
     M.data.world(s) — the fixture with s.acme applied, exactly as
     applyLease/storeDescriptionState/storeAccessBands/notesNow would leave it.

     Returns (all fields live; nothing is memoised, so a mutation of M.data.items
     shows on the next call):

       leaseState   'fresh' | 'lapsed' | 'unavailable'
       agent        { phase: 'Ready' | 'Bootstrap' }
       servers      the three servers with Acme's lease and state applied
       serverById   { id: server }
       unavailableStores  [StoreRef]
       stores       every store, each a copy carrying:
                      state    storeDescriptionState — 'normal' | 'blocked' |
                               'lease-unavailable' | 'lease-lapsed' |
                               'never-probed' | 'catalog-unavailable' | 'inactive'
                      description  storeDescription  — the sidebar caption,
                               including `Roster unavailable` / `Federation
                               unavailable` when D.groupDetailFailures holds
                               one for this store
                      heading      storeHeadingDescription — the header <small>
                               ('' on any problem, a detail failure included)
                      failure  null | 'roster' | 'federation' | 'both' — which
                               groupDetailFailures apply, for the pane that
                               draws the retry notice
                      readable     storeReadable
                      serverName   the server's name
                      parties      partiesOf(store)
                      notice       null, or { severity, title, detail, action,
                                              actionKind, profile } — the
                                   store-access takeover copy
       storeById    { ref: store }
       navOrder / vaults / groups / shares   storeNavigationOrder, split
       items        catalog(): listable items (no Folders, readable stores only)
       bands        storeAccessBands(): [{ key, text }] — no severity, so the
                    All-items bands draw as `.band` (amber), not `.band.stop`
       alerts       notesNow(): the entries that apply
       alertCount   alerts.length — the sidebar badge (2 fresh, 3 lapsed)

     The acme knob maps onto the model like this (world.md §2.2-2.3):
       ok                  server.state 'ok', lease { fresh, '12 d' }
       lapsed              server.state 'lease-lapsed', lease { lapsed, null }
       checkin-unavailable server.state 'lease-unavailable'
       unprobed            server.state 'never-probed'
       unavailable         server stays 'ok'; Acme's stores land in
                           world.unavailableStores → 'catalog-unavailable'
       blocked             server.state 'blocked' */
  var ACME = {
    ok: { server: 'ok', lease: { state: 'fresh', expires_in: '12 d' } },
    lapsed: { server: 'lease-lapsed', lease: { state: 'lapsed', expires_in: null } },
    'checkin-unavailable': { server: 'lease-unavailable', lease: null },
    unavailable: { server: 'ok', lease: { state: 'fresh', expires_in: '12 d' }, catalog: true },
    unprobed: { server: 'never-probed', lease: null },
    blocked: { server: 'blocked', lease: { state: 'fresh', expires_in: '12 d' } },
  };

  function unavailableSubject(store) {
    return store.kind === 'team'
      ? 'Items and members in ' + store.name + ' are unavailable'
      : 'Items in ' + store.name + ' are unavailable';
  }
  /* screens/store-access.tsx:27-71, verbatim. */
  function accessCopy(state, store, serverName) {
    var subject = unavailableSubject(store);
    switch (state) {
      case 'blocked':
        return { title: 'Server access blocked', detail: 'A protocol safety check blocked ' + serverName + '. ' + subject + '.', action: 'Review server', actionKind: 'review-server' };
      case 'lease-unavailable':
        return { title: 'Server status unavailable', detail: 'FOKS has no signed check-in for ' + serverName + '. ' + subject + '.', action: 'Review server', actionKind: 'review-server' };
      case 'never-probed':
        return { title: 'Server not checked yet', detail: serverName + ' has not been checked. ' + subject + ' until it is checked and its identity pinned.', action: 'Review server', actionKind: 'review-server' };
      case 'catalog-unavailable':
        return { title: 'Store connection failed', detail: store.name + ' remains known on this Mac, but its current server state could not be loaded. ' + subject + '.', action: 'Review server', actionKind: 'review-server' };
      case 'lease-lapsed':
        return { title: 'Check-in expired', detail: subject + ' until the agent renews the check-in for ' + serverName + '.', action: 'Open server', actionKind: 'open-server' };
      case 'inactive':
        return { title: 'Group setup incomplete', detail: 'Items and members are unavailable until setup is finished.', action: 'Finish setup', actionKind: 'finish-setup' };
      default:
        return null;
    }
  }

  D.world = function (s) {
    s = s || M.s;
    var acme = ACME[s.acme] || ACME.ok;
    var unavailable = acme.catalog ? ['acct:work', 'team:eng'] : D.unavailableStores.slice();

    var servers = D.servers.map(function (srv) {
      if (srv.id !== 'acme') return { id: srv.id, name: srv.name, label: srv.label, host_id: srv.host_id, chain: srv.chain, epoch: srv.epoch, lease: srv.lease, accounts: srv.accounts, state: srv.state };
      return { id: srv.id, name: srv.name, label: srv.label, host_id: srv.host_id, chain: srv.chain, epoch: srv.epoch, lease: acme.lease, accounts: srv.accounts, state: acme.server };
    });
    var serverById = {};
    servers.forEach(function (srv) { serverById[srv.id] = srv; });

    var stores = D.stores.map(function (store) {
      var srv = serverById[store.server];
      var state = 'normal';
      if (srv && srv.state === 'blocked') state = 'blocked';
      else if (srv && srv.state === 'lease-unavailable') state = 'lease-unavailable';
      else if (srv && srv.state === 'lease-lapsed') state = 'lease-lapsed';
      else if (srv && srv.state === 'never-probed') state = 'never-probed';
      else if (unavailable.indexOf(store.id) >= 0) state = 'catalog-unavailable';
      else if (store.kind === 'team' && !store.active) state = 'inactive';

      var parties = D.partiesOf(store.id);
      /* lease.ts:119-141. A group whose roster or federation could not be read
         says so in place of its people count, and storeHeadingDescription
         drops every failing reading — the pane under the heading already
         explains it, so the heading does not say it twice. */
      var rosterFailed = !!D.groupDetailFailure(store.id, 'roster');
      var fedFailed = !!D.groupDetailFailure(store.id, 'federation');
      var description;
      if (state === 'inactive') description = 'Setup incomplete';
      else if (state !== 'normal') description = 'Connection error';
      else if (store.kind === 'account') description = srv ? srv.name : '';
      else if (rosterFailed) description = 'Roster unavailable';
      else if (fedFailed) description = 'Federation unavailable';
      else description = D.peopleGroups(parties);

      var out = {
        id: store.id, kind: store.kind, name: store.name, alias: store.alias,
        server: store.server, account: store.account, active: store.active,
        team_kind: store.team_kind, team_id_hex: store.team_id_hex,
        state: state,
        description: description,
        heading: (state === 'normal' && !rosterFailed && !fedFailed) ? description : '',
        failure: rosterFailed ? (fedFailed ? 'both' : 'roster') : (fedFailed ? 'federation' : null),
        readable: unavailable.indexOf(store.id) < 0 && !!srv && srv.state === 'ok' &&
          (store.kind !== 'team' || store.active),
        serverName: srv ? srv.name : store.server,
        parties: parties,
        notice: null,
      };
      if (state !== 'normal') {
        var copy = accessCopy(state, out, out.serverName);
        out.notice = {
          severity: state === 'inactive' ? 'warn' : 'crit',
          title: copy.title, detail: copy.detail,
          action: copy.action, actionKind: copy.actionKind,
          profile: store.server,
        };
      }
      return out;
    });
    var storeById = {};
    stores.forEach(function (store) { storeById[store.id] = store; });

    /* storeAccessBands: one band per state:server bucket. `inactive` never
       produces a band — it stays on the group. */
    var order = [], buckets = {};
    stores.forEach(function (store) {
      if (store.state === 'normal' || store.state === 'inactive') return;
      var key = store.state + ':' + store.server;
      if (!buckets[key]) { buckets[key] = { state: store.state, stores: [] }; order.push(key); }
      buckets[key].stores.push(store);
    });
    var bands = order.map(function (key) {
      var bucket = buckets[key];
      var names = bucket.stores.map(function (st) { return st.name; });
      var joined = names.length < 2 ? (names[0] || '')
        : names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1];
      var verb = bucket.stores.length === 1 ? 'is' : 'are';
      var serverName = bucket.stores[0].serverName;
      var text =
        bucket.state === 'blocked' ? joined + ' ' + verb + ' unavailable because a protocol safety check blocked ' + serverName + '.'
        : bucket.state === 'lease-unavailable' ? joined + ' ' + verb + ' unavailable because FOKS has no signed check-in for ' + serverName + '.'
        : bucket.state === 'lease-lapsed' ? joined + ' ' + verb + ' unavailable because the check-in for ' + serverName + ' expired.'
        : bucket.state === 'catalog-unavailable' ? joined + ' ' + verb + ' unavailable because the current store inventory could not be loaded.'
        : joined + ' ' + verb + ' unavailable because ' + serverName + ' has not been checked.';
      /* StoreAccessBand is { key, text } and nothing more: items-screen.tsx:534
         renders `<Band>{band.text}</Band>` with no severity, so the class is
         plain `band` (amber), NOT `band stop`. */
      return { key: key, text: text };
    });

    var items = D.items.filter(function (item) {
      return item.kind !== 'Folder' && storeById[item.store] && storeById[item.store].readable;
    });

    var leaseState = acme.server === 'lease-lapsed' ? 'lapsed'
      : acme.server === 'lease-unavailable' ? 'unavailable' : 'fresh';
    /* notesNow(world) — the fixture's three notes, minus the lapse entry
       unless the world is lapsed (model/lease.ts:228).

       The other stopped worlds (`checkin-unavailable`, `unprobed`, `blocked`)
       are the deck's own: the app can only be driven between fresh and lapsed,
       and its ?state= presets never touch a server's state. Since this file
       builds those worlds by hand, it also builds the note the bridge would
       have attached to one — bridge.ts:1706-1745 gives every state that hides
       a server's stores a crit note, `never-probed` included. Same id shape
       (`${state}-${id}`), same title, same detail, and first in the list, as
       notificationsOf emits the server notes before the catalog failures.
       72-alerts treats only `catalog-` ids as actionable, so the button is
       inert — which is right: none of these is a thing this window can fix. */
    var STOPPED_NOTE = {
      'lease-unavailable': { detail: 'The agent has no signed check-in for this server. Its stores are hidden until it gets one.', action: 'Inspect' },
      blocked: { detail: 'The server’s history no longer matches what this Mac pinned. Its stores are hidden. Open the server to see the reported error.', action: 'Inspect' },
      'never-probed': { detail: 'This server has not been checked. Its stores are hidden until it is checked and its identity pinned.', action: 'Check' },
    };
    var alerts = D.alerts.filter(function (n) { return n.id !== 'lease-acme' || leaseState === 'lapsed'; });
    var stopped = STOPPED_NOTE[acme.server];
    if (stopped) {
      alerts = [{
        id: acme.server + '-acme', severity: 'crit',
        title: serverById.acme.name + ' is locked',
        detail: stopped.detail, action: stopped.action,
      }].concat(alerts);
    }

    /* The catalog-failed world (`acme=unavailable`): notificationsOf pushes
       one note per catalog failure after the server notes (bridge.ts:1745-
       1757) — `catalog-store-<i>`, `warn` when not fatal, action `Retry` when
       retryable. This is the one alert the pane can act on. The message is the
       store takeover's own sentence; the app's bridge never produces this
       world, so there is no captured string to copy. */
    if (acme.catalog) {
      alerts = alerts.concat(unavailable.map(function (ref, i) {
        return {
          id: 'catalog-store-' + i, severity: 'warn',
          title: 'Could not list one store on acme',
          detail: (storeById[ref] ? storeById[ref].name : ref) + ' remains known on this Mac, but its current store inventory could not be loaded.',
          action: 'Retry',
        };
      }));
    }

    var nav = D.storeNavigationOrder(stores);
    return {
      acme: s.acme || 'ok',
      leaseState: leaseState,
      agent: { phase: s.agent === 'starting' ? 'Bootstrap' : 'Ready' },
      servers: servers,
      serverById: serverById,
      unavailableStores: unavailable,
      stores: stores,
      storeById: storeById,
      navOrder: nav,
      vaults: nav.filter(function (st) { return st.kind === 'account'; }),
      groups: nav.filter(function (st) { return st.kind === 'team' && st.team_kind === 'named'; }),
      shares: nav.filter(function (st) { return st.kind === 'team' && st.team_kind === 'adhoc'; }),
      items: items,
      bands: bands,
      alerts: alerts,
      alertCount: alerts.length,
    };
  };

  /* ----------------------------------------------------- derived facts
     Kept here so a page asserts against them rather than re-deriving. The
     captions are §3.4's table; the counts are the README's goldens and are
     computed from readersOf, not typed. */
  D.captions = {
    fresh: { 'acct:personal': 'foks.example.net', 'acct:work': 'foks.acme-corp.com', 'team:eng': '5 people · 1 group', 'team:household': '2 people', 'team:homelab': 'Setup incomplete' },
    lapsed: { 'acct:personal': 'foks.example.net', 'acct:work': 'Connection error', 'team:eng': 'Connection error', 'team:household': '2 people', 'team:homelab': 'Setup incomplete' },
  };
  D.goldens = {
    /* production-token 3 · staging-token 5 · bundle.tar 5 · guest-password 2 */
    readers: {
      'team:eng|/deploy/production-token': D.readersOf(D.itemAt('team:eng', '/deploy/production-token')).length,
      'team:eng|/deploy/staging-token': D.readersOf(D.itemAt('team:eng', '/deploy/staging-token')).length,
      'team:eng|/release/bundle.tar': D.readersOf(D.itemAt('team:eng', '/release/bundle.tar')).length,
      'team:household|/wifi/guest-password': D.readersOf(D.itemAt('team:household', '/wifi/guest-password')).length,
    },
    engineering: D.peopleGroups(D.partiesOf('team:eng')),   /* "5 people · 1 group" */
    household: D.peopleGroups(D.partiesOf('team:household')), /* "2 people" */
    alertBadge: { fresh: 2, lapsed: 3 },
  };
