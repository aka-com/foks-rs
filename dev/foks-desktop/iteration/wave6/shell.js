/* ============================================================================
   FOKS desktop — wave 6 shell script
   The fixture (BRIEF §4, verbatim, with the WAVE4-BRIEF §1 corrections), the
   kind rule, the role arithmetic, and the builders both HTML files share so
   the sidebar and the item-page header cannot drift apart.
   Loaded as a plain script; everything below is a window global.
   ============================================================================ */

/* ===================== fixture (BRIEF §4, verbatim) ===================== */
const FX = {
  agent: { phase: 'Ready' },
  servers: [
    {
      id: 'personal',
      name: 'foks.example.net',
      label: 'Personal server',
      host_id: '9f31c2aa07',
      chain: 12,
      epoch: 4821,
      lease: { state: 'fresh', expires_in: '6 d' },
      accounts: ['personal'],
      state: 'ok',
    },
    {
      id: 'acme',
      name: 'foks.acme-corp.com',
      label: 'Acme',
      host_id: 'b04d17e390',
      chain: 33,
      epoch: 90417,
      lease: { state: 'lapsed', expires_in: null },
      accounts: ['work'],
      state: 'lease-lapsed',
    },
    {
      id: 'partner',
      name: 'foks.partner.dev',
      label: null,
      host_id: null,
      chain: null,
      epoch: null,
      lease: null,
      accounts: [],
      state: 'never-probed',
    },
  ],
  accounts: [
    { alias: 'personal', username: 'rae', server: 'personal' },
    { alias: 'work', username: 'rae.chen', server: 'acme' },
  ],
  stores: [
    {
      id: 'acct:personal',
      kind: 'account',
      name: 'Personal',
      server: 'personal',
      account: 'personal',
    },
    {
      id: 'acct:work',
      kind: 'account',
      name: 'Work (Acme)',
      server: 'acme',
      account: 'work',
    },
    {
      id: 'team:eng',
      kind: 'team',
      name: 'Engineering',
      alias: 'engineering',
      server: 'acme',
      account: 'work',
      active: true,
      team_kind: 'named',
      team_id_hex:
        '033f91c2aa07b45e18d0c73a9f2e5b6417ac8d0192f3e4b5c6d7089a1b2c3d4e5f',
    },
    {
      id: 'team:household',
      kind: 'team',
      name: 'Household',
      alias: 'household',
      server: 'personal',
      account: 'personal',
      active: true,
      team_kind: 'named',
      team_id_hex:
        '037b83d4f0241a67933c59ed218458ce03a5b98e691e81c544b8cc93c52f4e02ab',
    },
    {
      id: 'team:homelab',
      kind: 'team',
      name: 'Homelab',
      alias: 'homelab',
      server: 'personal',
      account: 'personal',
      active: false,
      team_kind: 'adhoc',
      team_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
    },
  ],
  items: [
    {
      store: 'acct:personal',
      path: '/logins/github.com',
      kind: 'Secret',
      size: 142,
      version: 9,
      read: 'Owner',
      write: 'Owner',
      value: 'user: rae\npassword: ••••••••••••\nurl: https://github.com/login',
    },
    {
      store: 'acct:personal',
      path: '/logins/fastmail.com',
      kind: 'Secret',
      size: 96,
      version: 2,
      read: 'Owner',
      write: 'Owner',
      value: 'user: rae@fastmail.com\npassword: ••••••••••',
    },
    {
      store: 'acct:personal',
      path: '/env/prod/DATABASE_URL',
      kind: 'Secret',
      size: 96,
      version: 3,
      read: 'Owner',
      write: 'Owner',
      value: 'postgres://service:••••••••@db.internal/prod',
    },
    {
      store: 'acct:personal',
      path: '/ssh/id_ed25519',
      kind: 'File',
      size: 419,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/ssh',
      kind: 'Folder',
      size: 0,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/latest-key',
      kind: 'Link',
      size: 22,
      version: 2,
      read: 'Owner',
      write: 'Owner',
      target: '/ssh/id_ed25519',
    },
    {
      store: 'acct:personal',
      path: '/documents/passport-scan.pdf',
      kind: 'File',
      size: 2841992,
      version: 1,
      read: 'Owner',
      write: 'Owner',
    },
    {
      store: 'acct:personal',
      path: '/agents/anthropic-api-key',
      kind: 'Secret',
      size: 108,
      version: 4,
      read: 'Owner',
      write: 'Owner',
      value: 'sk-ant-••••••••••••••••',
    },
    {
      store: 'team:eng',
      path: '/deploy/production-token',
      kind: 'Secret',
      size: 88,
      version: 12,
      read: 'Admin',
      write: 'Owner',
      value: 'foks_team_token_••••••••',
    },
    {
      store: 'team:eng',
      path: '/deploy/staging-token',
      kind: 'Secret',
      size: 88,
      version: 3,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'foks_team_token_••••••••',
    },
    {
      store: 'team:eng',
      path: '/release/bundle.tar',
      kind: 'File',
      size: 84399718,
      version: 5,
      read: 'Member · visibility 0',
      write: 'Admin',
    },
    {
      store: 'team:eng',
      path: '/onboarding/README.md',
      kind: 'File',
      size: 5120,
      version: 2,
      read: 'Member · visibility 0',
      write: 'Admin',
    },
    {
      store: 'team:household',
      path: '/wifi/guest-password',
      kind: 'Secret',
      size: 64,
      version: 4,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'ssid: Chen-Guest\npassword: ••••••••',
    },
    {
      store: 'team:household',
      path: '/documents/emergency.pdf',
      kind: 'File',
      size: 2841992,
      version: 7,
      read: 'Member · visibility 0',
      write: 'Owner',
    },
    {
      store: 'team:household',
      path: '/streaming/netflix',
      kind: 'Secret',
      size: 70,
      version: 1,
      read: 'Member · visibility 0',
      write: 'Admin',
      value: 'user: family@example.net\npassword: ••••••••',
    },
  ],
  parties: [
    {
      store: 'team:eng',
      username: 'sam.ortiz',
      party_kind: 'user',
      generation: 4,
      locally_manageable: true,
      party_id_hex:
        '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Owner' },
    },
    {
      store: 'team:eng',
      username: 'rae.chen',
      label: 'you',
      party_kind: 'user',
      generation: 6,
      locally_manageable: true,
      party_id_hex:
        '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211',
      source_role: { role: 'Admin' },
      destination_role: { role: 'Admin' },
    },
    {
      store: 'team:eng',
      username: 'dana.okafor',
      party_kind: 'user',
      generation: 5,
      locally_manageable: true,
      party_id_hex:
        '012f8b6d13c07a4e5991b2f8d04a6c3e17bd50927fe83a1c4670d9b2e518f4a06c',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
    {
      store: 'team:eng',
      username: null,
      party_kind: 'named-team',
      team_name: 'homelab @ foks.example.net',
      generation: 3,
      locally_manageable: false,
      party_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
      scoped_host_id_hex: '9f31c2aa07',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Member', visibility: 0 },
    },
    {
      store: 'team:eng',
      username: 'deploy-bot',
      party_kind: 'user',
      generation: 6,
      locally_manageable: false,
      party_id_hex:
        '01c93d2f8b17e4a0148ab7f2e60155c9a71c6f93d23140e5b839d1c0f60ba48e57',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
    {
      store: 'team:household',
      username: 'rae',
      label: 'you',
      party_kind: 'user',
      generation: 2,
      locally_manageable: true,
      party_id_hex:
        '016ab09cd4e11f207877c31b900b44aa193d90c1e45f22c8d13e91c7a29c04e211',
      source_role: { role: 'Owner' },
      destination_role: { role: 'Owner' },
    },
    {
      store: 'team:household',
      username: 'sam',
      party_kind: 'user',
      generation: 2,
      locally_manageable: true,
      party_id_hex:
        '01a4c107f822ee91b05d3c44a7e0186b297f10cd439b62ae0831d5f7c46e29b0a5',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
  ],
  federation: [
    {
      store: 'team:eng',
      remote_profile: 'foks.example.net',
      remote_team_alias: 'homelab',
      remote_host_id_hex: '9f31c2aa07',
      remote_team_id_hex:
        '038b17e4a055c93d2fe60148ab7f2c9d3140e5b8a71c6f93d20ba48e5739d1c0f6',
      destination: { role: 'Member', visibility: 0 },
      operation_id_hex: '7c14a9f0',
      active: false,
    },
  ],
  devices: [
    {
      alias: 'personal',
      name: 'MacBook Pro',
      role: 'owner',
      current: true,
      id_hex:
        '02a779c40674942e2d0fc18aa8d59b2ee4fa95ea7a652310e3f13d2d317f170b22',
    },
    {
      alias: 'laptop',
      name: 'Travel Mac',
      role: 'owner',
      current: false,
      id_hex:
        '02c1e08d5f3a94b7d21e6f0c8a3b5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809',
    },
  ],
  yubi_accounts: [
    { alias: 'primary key', server: 'personal', serial: 20993145 },
  ],
  cards_connected: [{ serial: 20993145 }],
  notifications: [
    {
      id: 'lease-acme',
      severity: 'crit',
      title: 'Nothing on foks.acme-corp.com can be read or changed',
      detail:
        'The compatibility lease for foks.acme-corp.com has expired. Work and Engineering groups are unavailable until the agent renews it.',
      action: 'Wait for lease refresh',
    },
    {
      id: 'team-homelab',
      severity: 'warn',
      title: 'Homelab is inactive',
      detail:
        'Group setup incomplete. Items and members are unavailable until setup is finished.',
      action: 'Resume creation',
    },
    {
      id: 'fed-homelab',
      severity: 'warn',
      title: 'Homelab cannot access Engineering',
      detail:
        'Homelab’s membership in Engineering is inactive. Homelab members cannot access Engineering until it is restored.',
      action: 'Restore access',
    },
  ],
};

/* Fixture configuration for multi-admin team permissions, service accounts, and lease states. */
FX.parties.splice(2, 0, {
  store: 'team:eng',
  username: 'priya.n',
  party_kind: 'user',
  generation: 5,
  locally_manageable: true,
  party_id_hex:
    '0184f0c2a71b3d95e6082c47fa19b6d3e05c84719fb2a6c308d5e71f94c3b0d726',
  source_role: { role: 'Admin' },
  destination_role: { role: 'Admin' },
});
((p) => {
  p.locally_manageable = true;
  p.note = 'Service account';
})(FX.parties.find((p) => p.username === 'deploy-bot'));
((p) => {
  p.note = 'Member group managed in Group Settings';
})(FX.parties.find((p) => p.party_kind === 'named-team'));

/* Lease world. "fresh" by default so Engineering and Work list at all. */
FX.leaseState = 'fresh';
function setLease(state) {
  FX.leaseState = state;
  const acme = server('acme');
  acme.lease =
    state === 'lapsed'
      ? { state: 'lapsed', expires_in: null }
      : { state: 'fresh', expires_in: '12 d' };
  acme.state = state === 'lapsed' ? 'lease-lapsed' : 'ok';
}

/* Extension (BRIEF §4 allows extending): illustrative plaintext standing in
   for what ReadKv returns after Show. The fixture itself stays masked. */
FX.plaintext = {
  'acct:personal|/logins/github.com': 'hx7-Qm2!vTe9-pale-orbit',
  'acct:personal|/logins/fastmail.com': 'kettle-91-Lumen-tide',
  'acct:personal|/env/prod/DATABASE_URL':
    'postgres://service:o4Kq9wLm2x@db.internal/prod',
  'acct:personal|/agents/anthropic-api-key': 'sk-ant-api03-Rf2xQ7bTn41La9mZ4',
  'team:eng|/deploy/production-token': 'foks_team_token_7f31ac09',
  'team:eng|/deploy/staging-token': 'foks_team_token_02be44d1',
  'team:household|/wifi/guest-password': 'sunny-kettle-42',
  'team:household|/streaming/netflix': 'popcorn-Sofa-77',
};

/* Sanctioned wordings, verbatim from ../mocks/base.html DATA.copy */
const COPY = {
  lease_lapsed:
    'Access to this server is blocked until the agent renews its compatibility lease. Items on this server are hidden and cannot be changed. Other servers are unaffected.',
  remove_item:
    'Permanently deletes this item if no newer version exists. Previous versions will no longer be accessible.',
  personal_store_fixed:
    'Your Personal vault has no roster and cannot be shared. To share an item, put it in a group.',
  resumable:
    'Select Resume to continue an interrupted operation. FOKS checks completed steps and does not repeat them.',
};

/* ===================== icons ===================== */
const I = {
  key: '<circle cx="8" cy="14" r="4"/><path d="M11 11l9-9M17 5l2.5 2.5M14.5 7.5 17 10"/>',
  term: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
  file: '<path d="M6 3h8l4 4v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/><path d="M14 3v4h4"/>',
  link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
  people:
    '<circle cx="9" cy="8" r="3.2"/><path d="M3 19c0-3.3 2.7-5.5 6-5.5s6 2.2 6 5.5"/><path d="M15.5 5.2a3.2 3.2 0 0 1 0 5.6"/><path d="M17 13.6c2.4.5 4 2.5 4 5.4"/>',
  person:
    '<circle cx="12" cy="8" r="3.6"/><path d="M5 20c.6-4 3.4-6 7-6s6.4 2 7 6"/>',
  vault:
    '<rect x="3" y="4" width="18" height="16" rx="2.5"/><circle cx="12" cy="12" r="3.5"/><path d="M12 8.5v1.5M12 14v1.5M8.5 12H10M14 12h1.5"/>',
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
  eyeoff:
    '<path d="M3 3l18 18M10.6 10.6a2 2 0 0 0 2.8 2.8M6.6 6.6C4 8.2 2 12 2 12s3.5 6 10 6c1.7 0 3.2-.4 4.4-1M9.9 6.2C10.5 6.1 11.2 6 12 6c6.5 0 10 6 10 6s-.8 1.4-2.3 2.9"/>',
  bell: '<path d="M6 16v-5a6 6 0 0 1 12 0v5l2 2H4zM10 21h4"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  gear: '<path d="M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.3a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4A2 2 0 0 0 4 9.9l.2.1a2 2 0 0 1 1 1.7v.5a2 2 0 0 1-1 1.8l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.3a2 2 0 0 1 1 1.7v.2a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.3a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.8v-.5a2 2 0 0 1 1-1.7l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.3a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  server:
    '<rect x="3" y="4" width="18" height="6" rx="1.5"/><rect x="3" y="14" width="18" height="6" rx="1.5"/><path d="M7 7h.01M7 17h.01"/>',
  check: '<path d="M5 12l5 5 9-10"/>',
  pencil:
    '<path d="M4 20l4.5-1L19 8.5a2 2 0 0 0-3-3L5.5 16z"/><path d="M14 7l3 3"/>',
  alert: '<circle cx="12" cy="12" r="8.5"/><path d="M12 8v5M12 16h.01"/>',
  minus: '<path d="M5 12h14"/>',
  again: '<path d="M20 12a8 8 0 1 1-2.6-5.9"/><path d="M20 4v4h-4"/>',
  flag: '<path d="M5 21V4h13l-2.5 4L18 12H5"/>',
  plug: '<path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0zM12 17v4"/>',
};
const ic = (n, cls = '') =>
  `<svg class="ic ${cls}" viewBox="0 0 24 24" aria-hidden="true">${I[n]}</svg>`;

/* ===================== small helpers ===================== */
const esc = (s) =>
  String(s ?? '').replace(
    /[&<>"]/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c],
  );
const store = (id) => FX.stores.find((s) => s.id === id);
const server = (id) => FX.servers.find((s) => s.id === id);
const serverOf = (st) => server(store(st).server);
const nameOf = (p) => p.slice(p.lastIndexOf('/') + 1) || '/';
const prefixOf = (p) => p.slice(1, p.lastIndexOf('/'));
const fmtSize = (b) =>
  b === 0
    ? '0 B'
    : b < 1000
      ? b + ' B'
      : b < 1e6
        ? (b / 1000).toFixed(b < 10000 ? 1 : 0) + ' KB'
        : (b / 1e6).toFixed(1) + ' MB';
const fmtRole = (r) =>
  r.role === 'Member' ? `Member · visibility ${r.visibility}` : r.role;
const partiesOf = (id) => FX.parties.filter((p) => p.store === id);
const key = (it) => it.store + '|' + it.path;
const byKey = (k) => FX.items.find((i) => key(i) === k);
const people = (n) => `${n} ${n === 1 ? 'person' : 'people'}`;
const pname = (p) =>
  p.username || p.team_name || p.party_id_hex.slice(0, 10) + '…';
const initials = (s) =>
  s
    .replace(/@.*/, '')
    .split(/[.\-_ ]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0].toUpperCase())
    .join('');
const HUES = [
  '#5e5ce6',
  '#ff9f0a',
  '#30b0c7',
  '#ff2d55',
  '#34c759',
  '#af52de',
  '#0a7cff',
  '#a2845e',
];
const hue = (s) =>
  HUES[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % HUES.length];

/* avatars / stacks */
const avatar = (p, cls = 'av') =>
  p.party_kind === 'named-team'
    ? `<span class="${cls} team" style="background:var(--c-team)">${ic('people')}</span>`
    : `<span class="${cls}" style="background:${hue(pname(p))}">${esc(initials(pname(p)))}</span>`;
const stack = (sid, cls = '') => {
  const ps = partiesOf(sid).slice(0, 2);
  return `<span class="stack ${cls}">${ps.length ? ps.map((p) => avatar(p)).join('') : `<span class="av" style="background:var(--c-none)">${ic('people')}</span>`}</span>`;
};
const stackOf = (parties, cls = '') =>
  `<span class="stack ${cls}">${parties
    .slice(0, 2)
    .map((p) => avatar(p))
    .join('')}</span>`;

/* ===================== the kind rule (client-side, no protocol) =====================
   Password = a Secret with a `password:` line or under /logins
   Resource = any other Secret · File = a File node · Link = a symlink
   Folders are not items: the path prefix is a chip on the row. */
const KINDS = {
  Password: {
    plural: 'Passwords',
    icon: 'key',
    blurb: 'Keep passwords, secrets, and tokens here.',
  },
  Resource: {
    plural: 'Notes',
    icon: 'term',
    blurb: 'Keep keys, tokens, connection strings, and notes here.',
  },
  File: {
    plural: 'Files',
    icon: 'file',
    blurb: 'Keep documents and files in encrypted storage.',
  },
  Link: {
    plural: 'Links',
    icon: 'link',
    blurb: 'Keep links and shortcuts to other paths in the store.',
  },
};
const KIND_LIST = Object.keys(KINDS);
const kindOf = (it) =>
  it.kind === 'Secret'
    ? /^password:/m.test(it.value || '') || it.path.startsWith('/logins/')
      ? 'Password'
      : 'Resource'
    : it.kind;
/* the node type the kind is a reading of */
const rtype = (it) =>
  ({
    Secret: 'small_file',
    File: 'file',
    Link: 'symlink',
    Folder: 'directory',
  })[it.kind];
const rtypeWords = (it) =>
  ({
    Secret: 'a Secret',
    File: 'a File node',
    Link: 'a symlink',
    Folder: 'a directory',
  })[it.kind];
const isLogin = (it) =>
  kindOf(it) === 'Password' && it.path.startsWith('/logins/');
/* tinted square (rows, details header, sheets) */
const kic = (k, extra = '') =>
  `<span class="kic ${k} ${extra}">${ic(KINDS[k].icon)}</span>`;
/* solid square or site initial (tiles) */
function kico(it, size = '') {
  if (isLogin(it)) {
    const d = nameOf(it.path);
    return `<span class="kico ${size}" style="background:${hue(d)}">${esc(d[0].toUpperCase())}</span>`;
  }
  const k = kindOf(it);
  return `<span class="kico ${k} ${size}">${ic(KINDS[k].icon)}</span>`;
}
const kglyph = (k, size = '') =>
  `<span class="kico ${k} ${size}">${ic(KINDS[k].icon)}</span>`;

/* ===================== roles, readers ===================== */
/* Member(-0x4000) < Member(0) < Admin < Owner. 0 is the default; lower bands see less. */
const parseRole = (r) => {
  if (typeof r !== 'string')
    return { role: r.role, visibility: r.visibility ?? 0 };
  const m = /^Member(?:\s*·\s*visibility\s*(-?\d+))?$/.exec(r);
  return m
    ? { role: 'Member', visibility: m[1] != null ? +m[1] : 0 }
    : { role: r, visibility: 0 };
};
const roleRank = (r) =>
  ({ Owner: 3, Admin: 2, Member: 1 })[parseRole(r).role] || 0;
/* does `held` (a party's destination role) admit reading something whose read role is `need`? */
function admits(held, need) {
  const h = parseRole(held),
    n = parseRole(need),
    hr = roleRank(h),
    nr = roleRank(n);
  if (hr !== nr) return hr > nr;
  return h.role === 'Member' ? h.visibility >= n.visibility : true;
}
/* the roster filtered by the item's read role; null on an account store (no roster) */
/* an admitted team whose admission reports inactive reads nothing here */
function admissionActive(p, storeId) {
  if (p.party_kind !== 'named-team') return true;
  const f = (FX.federation || []).find(
    (f) => f.store === storeId && f.remote_team_id_hex === p.party_id_hex,
  );
  return f ? !!f.active : true;
}
function readersOf(it) {
  const st = store(it.store);
  if (st.kind !== 'team') return null;
  return partiesOf(st.id).filter(
    (p) => admissionActive(p, st.id) && admits(p.destination_role, it.read),
  );
}
// Format member totals distinguishing individual users from federated groups.
function peopleGroups(parties) {
  const groups = parties.filter((p) => p.party_kind === 'named-team').length,
    ppl = parties.length - groups;
  const a = `${ppl} ${ppl === 1 ? 'person' : 'people'}`;
  return groups ? `${a} · ${groups} ${groups === 1 ? 'group' : 'groups'}` : a;
}
const readChip = (it) => {
  const rs = readersOf(it);
  return rs
    ? `<span class="chip" title="${esc(rs.map(pname).join(', '))}">${people(rs.length)}</span>`
    : `<span class="chip">only you</span>`;
};

/* ===================== lease / listability ===================== */
const leaseLapsed = (sid) => serverOf(sid).state === 'lease-lapsed';
const storeReadable = (sid) => {
  const st = store(sid);
  return !leaseLapsed(sid) && (st.kind !== 'team' || st.active);
};
const catalog = () =>
  FX.items.filter((i) => i.kind !== 'Folder' && storeReadable(i.store));
const notesNow = () =>
  FX.notifications.filter(
    (n) => n.id !== 'lease-acme' || FX.leaseState === 'lapsed',
  );
const whereOf = (it) => {
  const st = store(it.store);
  return `${st.name} · ${serverOf(it.store).name}`;
};
const storeDescriptionState = (st) => {
  const state = serverOf(st.id).state;
  if (state === 'blocked') return 'blocked';
  if (state === 'lease-unavailable') return 'lease-unavailable';
  if (state === 'lease-lapsed') return 'lease-lapsed';
  if (st.kind === 'team' && !st.active) return 'inactive';
  return 'normal';
};
const storeDescription = (st) => {
  if (storeDescriptionState(st) !== 'normal') return 'Unavailable';
  return st.kind === 'account'
    ? serverOf(st.id).name
    : peopleGroups(partiesOf(st.id));
};

/* ===================== sidebar (04's, with Set up again) ===================== */
/* active: 'all' | a store id | 'alerts' | 'start' | 'team' | 'servers' | 'settings'
   opts: { stores, statusExtra, note, setup:{steps,current,caption}, footDisabled,
           alerts (count), setupAgain (href or false) } */
function sidebar(active, opts = {}) {
  const o = Object.assign(
    { stores: FX.stores, setupAgain: '02-first-run.html?state=who' },
    opts,
  );
  const navRow = (id, glyph, name, cap, dot, off, tail = '') =>
    `<button class="nav ${active === id ? 'on' : ''} ${off ? 'off' : ''}" data-act="nav" data-id="${esc(id)}">${glyph}<span class="t">${esc(name)}${cap ? `<small>${esc(cap)}</small>` : ''}</span>${dot ? '<span class="dot" title="Has alerts"></span>' : ''}${tail}</button>`;

  if (o.setup) {
    /* first-run mode of the same sidebar */
    const rows = o.setup.steps
      .map(
        (s, i) =>
          `<button class="nav ${s.state || 'todo'}" data-act="go" data-id="${i}"><span class="sdot"></span><span class="t">${esc(s.label)}</span></button>`,
      )
      .join('');
    return (
      rows +
      `<div class="foot">
         <button class="nav" disabled title="After setup">${ic('server')}<span class="t">Servers &amp; devices</span></button>
         <button class="nav" disabled title="After setup">${ic('gear')}<span class="t">Settings</span></button>
       </div>`
    );
  }

  const storeRow = (st) => {
    const connectionError = storeDescriptionState(st) !== 'normal';
    const glyph = st.kind === 'account' ? ic('vault') : stack(st.id);
    return navRow(
      st.id,
      glyph,
      st.name,
      storeDescription(st),
      connectionError,
      connectionError,
    );
  };
  const badge = o.alerts != null ? o.alerts : notesNow().length;
  const foot = `<div class="foot">
      <a class="nav ${active === 'team' ? 'on' : ''}" href="03-groups.html">${ic('plus')}<span class="t">Join or create a group</span></a>
      <a class="nav ${active === 'servers' ? 'on' : ''}" href="04-servers.html">${ic('server')}<span class="t">Servers &amp; devices</span></a>
      <a class="nav ${active === 'alerts' ? 'on' : ''}" href="01-vault.html?state=alerts">${ic('bell')}<span class="t">Alerts</span>${badge ? `<span class="badge">${badge}</span>` : ''}</a>
      <a class="nav ${active === 'settings' ? 'on' : ''}" href="05-settings.html">${ic('gear')}<span class="t">Settings</span></a>
      ${o.setupAgain ? `<a class="nav" href="${o.setupAgain}" title="Revisit initial setup steps">${ic('again')}<span class="t">Set up again</span></a>` : ''}
    </div>`;
  return (
    navRow('all', ic('grid'), 'All items', '', false, false) +
    `<h6>Vaults</h6>` +
    o.stores
      .filter((s) => s.kind === 'account')
      .map(storeRow)
      .join('') +
    `<h6>Groups</h6>` +
    o.stores
      .filter((s) => s.kind === 'team')
      .map(storeRow)
      .join('') +
    (o.statusExtra || '') +
    (o.note ? `<p class="fn">${o.note}</p>` : '') +
    foot
  );
}

/* ===================== item page header (04's) ===================== */
/* location: 'all' | a store id | { title, subtitle }
   opts: { q, search (default true), manage (default true for a live group),
           toolbar (html string, or null for no toolbar bar at all) } */
function headerParts(location) {
  if (location && typeof location === 'object')
    return {
      title: location.title,
      sub: location.subtitle || '',
      lead: '',
      tail: '',
    };
  if (location === 'all') {
    return { title: 'All items', sub: '', lead: '', tail: '' };
  }
  const st = store(location);
  if (st.kind === 'team') {
    return {
      title: st.name,
      sub: storeDescription(st),
      lead: stack(st.id, 'lg'),
      tail:
        storeDescriptionState(st) === 'normal'
          ? `<button class="btn cap" data-act="sheet" data-id="manage" data-store="${st.id}" title="People and roles in ${esc(st.name)}">Manage</button>`
          : '',
    };
  }
  return { title: st.name, sub: storeDescription(st), lead: '', tail: '' };
}
function pageHeader(location, opts = {}) {
  const h = headerParts(location);
  const search =
    opts.search === false
      ? ''
      : `<label class="search">${ic('search')}<input id="q" placeholder="${esc(h.title === 'All items' ? 'Search all items' : ('Search ' + h.title).length > 18 ? 'Search' : 'Search ' + h.title)}" value="${esc(opts.q || '')}" autocomplete="off"><kbd>⌘K</kbd></label>`;
  const tail = opts.manage === false ? '' : h.tail;
  const path = `<div class="path"><div class="loc${h.lead || tail ? '' : ' text-only'}">${h.lead}<h1>${esc(h.title)}</h1><small>${esc(h.sub)}</small>${tail}</div>${search}</div>`;
  if (opts.toolbar === null) return path;
  return path + `<div class="toolbar">${opts.toolbar || ''}</div>`;
}

/* ===================== sheets, flash, review strip, deep links ===================== */
function openSheet(html) {
  const o = document.getElementById('overlay');
  if (o) o.innerHTML = `<div class="backdrop">${html}</div>`;
}
function closeSheet() {
  const o = document.getElementById('overlay');
  if (o) o.innerHTML = '';
}
function flash(msg, host) {
  const w = host || document.getElementById('win');
  if (!w) return;
  const old = w.querySelector('.flash');
  if (old) old.remove();
  const el = document.createElement('div');
  el.className = 'flash';
  el.textContent = msg;
  w.appendChild(el);
  clearTimeout(flash.t);
  flash.t = setTimeout(() => el.remove(), 1900);
}
function review(states, current, extra = '') {
  const r = document.getElementById('review');
  if (!r) return;
  r.innerHTML =
    `<b>Prototype View Controls</b>` +
    states
      .map(
        (k) =>
          `<button class="${current === k ? 'on' : ''}" data-act="state" data-id="${k}">${k}</button>`,
      )
      .join('') +
    extra;
}
const getState = (def, param = 'state') =>
  new URLSearchParams(location.search).get(param) || def;
function setUrl(state, params = {}) {
  const u = new URL(location.href);
  u.searchParams.set('state', state);
  Object.entries(params).forEach(([k, v]) =>
    v == null ? u.searchParams.delete(k) : u.searchParams.set(k, v),
  );
  try {
    history.replaceState(null, '', u);
  } catch (_) {}
}

/* The lease world starts fresh (see corrections above); states that need the
   lapsed world call setLease('lapsed') before rendering. */
setLease('fresh');
