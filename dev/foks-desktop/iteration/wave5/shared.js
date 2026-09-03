/* FOKS Desktop — wave 5 shared fixture, helpers and chrome builders.
   Loaded by every wave 5 screen after system.css. Nothing here renders a
   fact the agent cannot return; see ../BRIEF.md §2 and ../WAVE4-BRIEF.md. */

/* ------------------------------------------------------------ fixture */
const FX = {
  agent: {
    phase: 'Ready',
    socket: '~/Library/Application Support/foks-rs/foks-rs.sock',
  },
  servers: [
    {
      id: 'personal',
      name: 'foks.example.net',
      label: 'Personal server',
      host_id: '9f31c2aa07',
      host_id_full:
        '9f31c2aa07e4b1d86c3f52a09e7d41c8b6f0a2d3e95c17b48f6a0d2e3c5b719a4f',
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
      host_id_full:
        'b04d17e390a1f85ddc6ce95b7748dfac2217ed401da781764da8eb55d098f72402',
      chain: 33,
      epoch: 90417,
      lease: { state: 'fresh', expires_in: '2 d' },
      accounts: ['work'],
      state: 'ok',
    },
    {
      id: 'partner',
      name: 'foks.partner.dev',
      label: null,
      host_id: null,
      host_id_full: null,
      chain: null,
      epoch: null,
      lease: null,
      accounts: [],
      state: 'never-probed',
    },
  ],
  accounts: [
    {
      alias: 'personal',
      username: 'rae',
      server: 'personal',
      device: 'MacBook Pro',
    },
    {
      alias: 'work',
      username: 'rae.chen',
      server: 'acme',
      device: 'MacBook Pro',
    },
  ],
  stores: [
    {
      id: 'acct:personal',
      kind: 'account',
      name: 'Personal',
      mark: 'P',
      color: '#0a7cff',
      server: 'personal',
      account: 'personal',
    },
    {
      id: 'acct:work',
      kind: 'account',
      name: 'Work (Acme)',
      mark: 'W',
      color: '#6a5acd',
      server: 'acme',
      account: 'work',
    },
    {
      id: 'team:eng',
      kind: 'team',
      name: 'Engineering',
      mark: 'E',
      color: '#1f7a3a',
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
      mark: 'H',
      color: '#d0741a',
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
      mark: 'L',
      color: '#8a8a93',
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
      read: { role: 'Owner' },
      write: { role: 'Owner' },
      value:
        'user: rae\npassword: correct-horse-battery-staple\nurl: https://github.com/login',
    },
    {
      store: 'acct:personal',
      path: '/logins/fastmail.com',
      kind: 'Secret',
      size: 96,
      version: 2,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
      value: 'user: rae@fastmail.com\npassword: tr0ub4dor&3',
    },
    {
      store: 'acct:personal',
      path: '/env/prod/DATABASE_URL',
      kind: 'Secret',
      size: 96,
      version: 3,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
      value: 'postgres://service:s3cr3t@db.internal/prod',
    },
    {
      store: 'acct:personal',
      path: '/ssh',
      kind: 'Folder',
      size: 0,
      version: 1,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
    },
    {
      store: 'acct:personal',
      path: '/ssh/id_ed25519',
      kind: 'File',
      size: 419,
      version: 1,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
    },
    {
      store: 'acct:personal',
      path: '/latest-key',
      kind: 'Link',
      size: 22,
      version: 2,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
      target: '/ssh/id_ed25519',
    },
    {
      store: 'acct:personal',
      path: '/documents/passport-scan.pdf',
      kind: 'File',
      size: 2841992,
      version: 1,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
    },
    {
      store: 'acct:personal',
      path: '/agents/anthropic-api-key',
      kind: 'Secret',
      size: 108,
      version: 4,
      read: { role: 'Owner' },
      write: { role: 'Owner' },
      value: 'sk-ant-api03-…redacted…',
    },
    {
      store: 'team:eng',
      path: '/deploy/production-token',
      kind: 'Secret',
      size: 88,
      version: 12,
      read: { role: 'Admin' },
      write: { role: 'Owner' },
      value: 'foks_team_token_9f31c2aa07',
    },
    {
      store: 'team:eng',
      path: '/deploy/staging-token',
      kind: 'Secret',
      size: 88,
      version: 3,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Admin' },
      value: 'foks_team_token_b04d17e390',
    },
    {
      store: 'team:eng',
      path: '/release/bundle.tar',
      kind: 'File',
      size: 84399718,
      version: 5,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Admin' },
    },
    {
      store: 'team:eng',
      path: '/onboarding/README.md',
      kind: 'File',
      size: 5120,
      version: 2,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Admin' },
    },
    {
      store: 'team:household',
      path: '/wifi/guest-password',
      kind: 'Secret',
      size: 64,
      version: 4,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Admin' },
      value: 'ssid: Chen-Guest\npassword: welcome-home-2026',
    },
    {
      store: 'team:household',
      path: '/documents/emergency.pdf',
      kind: 'File',
      size: 2841992,
      version: 7,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Owner' },
    },
    {
      store: 'team:household',
      path: '/streaming/netflix',
      kind: 'Secret',
      size: 70,
      version: 1,
      read: { role: 'Member', visibility: 0 },
      write: { role: 'Admin' },
      value: 'user: family@example.net\npassword: popcorn-night',
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
      you: true,
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
      username: 'priya.n',
      party_kind: 'user',
      generation: 6,
      locally_manageable: true,
      party_id_hex:
        '01d2e7a9b3c5f60148ab7f2e60155c9a71c6f93d23140e5b839d1c0f60ba48e5b5c6',
      source_role: { role: 'Admin' },
      destination_role: { role: 'Admin' },
    },
    {
      store: 'team:eng',
      username: 'dana.okafor',
      party_kind: 'user',
      generation: 7,
      locally_manageable: true,
      party_id_hex:
        '012f8b6d13c07a4e5991b2f8d04a6c3e17bd50927fe83a1c4670d9b2e518f4a06c',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
    {
      store: 'team:eng',
      username: null,
      team_name: 'homelab @ foks.example.net',
      party_kind: 'named-team',
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
      machine: true,
      party_kind: 'user',
      generation: 6,
      locally_manageable: true,
      party_id_hex:
        '01c93d2f8b17e4a0148ab7f2e60155c9a71c6f93d23140e5b839d1c0f60ba48e57',
      source_role: { role: 'Member', visibility: 0 },
      destination_role: { role: 'Member', visibility: 0 },
    },
    {
      store: 'team:household',
      username: 'rae',
      you: true,
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
  backup: { enrolled_from_this_mac: true, alias: 'paper-backup' },
  yubi_accounts: [
    { alias: 'primary key', server: 'personal', serial: 20993145 },
  ],
  cards_connected: [{ serial: 20993145 }],
  notifications: [
    {
      id: 'lease-acme',
      severity: 'crit',
      kind: 'lease',
      server: 'acme',
      title: 'Nothing on foks.acme-corp.com can be read or changed',
      detail:
        "This Mac's server check-in (its compatibility lease) for that server has lapsed, so the client refuses every ordinary operation there — reads stop with writes. Items in Work (Acme) are not listed, and Engineering cannot be synced. Your other servers are unaffected.",
      mechanism:
        "The agent on this Mac refreshes the signed lease from the server's lease URL on its own; nothing on this screen can do it sooner.",
      action: null,
      action_label: 'Waiting for the server check-in',
      source: 'ListProfiles · lease state: lapsed',
    },
    {
      id: 'team-homelab',
      severity: 'warn',
      kind: 'team-inactive',
      title: 'Setting up Homelab stopped part way',
      detail:
        'Homelab is listed on foks.example.net but reports itself inactive. Its items are not listed and nobody can be added until it is active.',
      mechanism:
        "A team summary carries one state — active or not — and gives no reason. This Mac's journal still holds the creation, so resuming verifies what was already written rather than repeating it.",
      action: 'Resume creation',
      source:
        "TeamSummary.active = false · this Mac's journal: CreateTeam pending",
    },
    {
      id: 'fed-homelab',
      severity: 'warn',
      kind: 'federation-inactive',
      title: "Homelab's members can't read Engineering",
      detail:
        "Engineering admitted Homelab as a member team, and that admission reports inactive, so Homelab's members read nothing in Engineering through it.",
      mechanism:
        "The admission carries no reason and no last-refresh time — active is the whole of its state. Re-running the admission is what changes it; it needs both servers' check-ins fresh.",
      action: 'Re-run the admission',
      source: 'FederatedMembershipSummary.active = false',
    },
  ],
  copy: {
    lease_lapsed:
      'Nothing on this server can be read or changed from this Mac until a fresh server check-in (compatibility lease) arrives. Reads stop with writes, so items here are not listed rather than shown stale. Every other server is unaffected.',
    rollback_title:
      "This server's history no longer matches what this Mac pinned",
    rollback:
      'Nothing on this server can be read or changed from this Mac. The history check runs before any operation is sent, so it refuses everything on the server — the scope is the whole server, never one store. The signed history went backwards or forked against the checkpoint pinned here, which happens when a server is restored from a backup or shows different devices different histories.',
    rollback_reset:
      "Resetting discards this Mac's pinned checkpoint and cached state for this server, anything written here and not yet accepted by the server, and every half-finished operation you could still have resumed on this server. A later check fetches fresh state. Your other servers are untouched. Do this when you know why the history moved — a restore you performed, say — and not to make a warning go away.",
    remove_party:
      "Removing rekeys the team in one signed operation and ends the party's live authorization and access to future key generations. Keys or ciphertext retained from before removal can still open data from those generations, so rotate every secret it could read.",
    remove_item:
      'Removing unlinks the entry under a version guard, so the change is refused rather than applied if someone else moved it first. Versions before it are not readable afterwards.',
    personal_store:
      "An account's store belongs to that account alone. There is no roster on it and no one to add — sharing anything means putting it in a team, whose members are people and teams with roles.",
    no_join:
      'FOKS has no join button and no invite links. An Admin or Owner of the team adds your username on the server where the team lives; then the team appears here.',
    probe_as_of:
      "These facts are as of the last check, not live. Checking is a network round trip that re-reads the host's signed chain and pins it again; nothing here refreshes on its own.",
    visibility:
      '0 is the default. Lower bands see less: a Member at band N reads items whose read role is Member at N or below, so a read role above 0 admits only Members raised to that band, plus Admins and Owners. Bands run from −16384 to 16383.',
    yubi_facts: [
      'Preparing a card writes both keys in one step that cannot be split. If it stops part way, the slots are no longer empty and the retry is refused; getting the card back means resetting its PIV applet, which erases everything on it.',
      'A card can only be prepared while it still holds its factory management key.',
      'The unlock code (PUK) is yours to choose and yours to keep: nothing generates one for you and nothing will show it to you later.',
    ],
    deferred_team_writes:
      'Deferred — plan §5 group B. Adding, changing or removing items in a team store, and choosing who can read them, come in a later release. Today: items in your own account store (Owner / Owner, readable by you alone). No command writes a team item today either.',
    proposed_discovery:
      'Proposed — needs protocol work. Today the agent lists only teams this Mac created or admitted. A team someone else added you to needs an operation that walks your membership chain on the server and stores the team locally.',
    proposed_acceptance:
      'Proposed — needs protocol work. ProbeOutcome.acceptance (Inserted | Advanced | Unchanged) exists below the agent and is dropped before ProbeReport; surfacing it is what would make this row a fact.',
    gated_rollback:
      'Gated — plan §5 group E. Until typed errors land, the client cannot distinguish a history-check failure from a capability refusal; this banner is drawn as it will read then.',
  },
};

/* ------------------------------------------------------------ lease world */
/* The fixture's Acme server is healthy by default. A screen that draws the
   lapsed world (Home, Attention, Servers' list and lapsed states) calls
   setLease("lapsed") in its state setup and says so in its review note. */
function setLease(state) {
  const acme = FX.servers.find((s) => s.id === 'acme');
  if (state === 'lapsed') {
    acme.lease = { state: 'lapsed', expires_in: null };
    acme.state = 'lease-lapsed';
  } else {
    acme.lease = { state: 'fresh', expires_in: '2 d' };
    acme.state = 'ok';
  }
}
const VIS_MIN = -16384,
  VIS_MAX = 16383;
const clampVis = (n) => Math.max(VIS_MIN, Math.min(VIS_MAX, n | 0));
/* Needs-you, as the rail badge counts it: Homelab's creation, dana.okafor's addition and Protect your
   account (3), plus foks.acme-corp.com's lapsed check-in only while it is lapsed. Computed from the lease
   world every time the rail is drawn — never hard-coded — so a screen that calls setLease gets the badge
   its own world implies (3 fresh / 4 lapsed). */
const serverLapsed = (id) => {
  const s = FX.servers.find((x) => x.id === id);
  return !!(s && s.lease && s.lease.state === 'lapsed');
};
const needsYou = () => 3 + (serverLapsed('acme') ? 1 : 0);

/* ------------------------------------------------------------ helpers */
const $ = (id) => document.getElementById(id);
const esc = (s) =>
  String(s ?? '').replace(
    /[&<>"']/g,
    (c) =>
      ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[
        c
      ],
  );
const bytes = (n) =>
  n < 1024
    ? n + ' B'
    : n < 1048576
      ? (n / 1024).toFixed(1) + ' KB'
      : (n / 1048576).toFixed(1) + ' MB';
const short = (hex) => (hex ? hex.slice(0, 10) + '…' + hex.slice(-4) : '—');
const leaf = (path) => path.slice(path.lastIndexOf('/') + 1) || '/';
const storeById = (id) => FX.stores.find((s) => s.id === id);
const serverById = (id) => FX.servers.find((s) => s.id === id);
const partiesOf = (storeId) => FX.parties.filter((p) => p.store === storeId);
const roleRank = (r) => (r.role === 'Owner' ? 3 : r.role === 'Admin' ? 2 : 1);
/* Member(-0x4000) < Member(0) < Admin < Owner — higher is more. */
function admits(party, readRole) {
  const have = party.destination_role,
    need = readRole;
  if (roleRank(have) !== roleRank(need)) return roleRank(have) > roleRank(need);
  if (have.role !== 'Member') return true;
  return (have.visibility ?? 0) >= (need.visibility ?? 0);
}
const readersOf = (item) =>
  item.store.startsWith('acct:')
    ? []
    : partiesOf(item.store).filter((p) => admits(p, item.read));
const roleText = (r) =>
  r.role === 'Member' ? `Member · ${r.visibility ?? 0}` : r.role;
const roleChip = (r, extra = '') =>
  `<span class="chip ${r.role.toLowerCase()} ${extra}">${roleText(r)}</span>`;
const partyName = (p) => p.username || p.team_name || short(p.party_id_hex);
const initials = (name) =>
  name
    .split(/[.\s@]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((s) => s[0].toUpperCase())
    .join('');
const hue = (s) => {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${h} 45% 52%)`;
};
const avatar = (p, cls = '') =>
  p.party_kind === 'named-team'
    ? `<span class="avatar team ${cls}" style="background:#5b6b8f">⌂</span>`
    : `<span class="avatar ${p.machine ? 'machine' : ''} ${cls}" style="${p.machine ? '' : 'background:' + hue(partyName(p))}">${p.machine ? '⚙' : initials(partyName(p))}</span>`;
const storeMark = (st) =>
  `<span class="store-mark" style="background:${st.color}">${st.mark}</span>`;
const readableChip = (item) => {
  if (item.store.startsWith('acct:'))
    return `<span class="chip">only you</span>`;
  const n = readersOf(item).length,
    all = partiesOf(item.store).length;
  return `<span class="chip click" data-readable="${esc(item.store)}|${esc(item.path)}">Readable by ${n} of ${all} ▾</span>`;
};
const readerList = (item) => {
  if (item.store.startsWith('acct:'))
    return `<div class="note">${FX.copy.personal_store}</div>`;
  const ps = partiesOf(item.store),
    ok = ps.filter((p) => admits(p, item.read)),
    no = ps.filter((p) => !admits(p, item.read));
  return `<div class="inset">${ok.map((p) => `<div class="irow">${avatar(p)}<div class="v">${esc(partyName(p))}${p.you ? ' <span class="chip you">you</span>' : ''}</div>${roleChip(p.destination_role)}</div>`).join('')}
    ${no.length ? `<div class="foot">${no.length} cannot read it at this role: ${no.map((p) => esc(partyName(p))).join(', ')}. There are no per-item grants — the read role and each party's role decide.</div>` : `<div class="foot">Everyone in the team. There are no per-item grants — the read role and each party's role decide.</div>`}</div>`;
};
const fieldsOf = (item) => {
  if (!item.value || !/^\w[\w-]*:\s/.test(item.value)) return null;
  return item.value.split('\n').map((line) => {
    const i = line.indexOf(':');
    return { k: line.slice(0, i).trim(), v: line.slice(i + 1).trim() };
  });
};

/* ------------------------------------------------------------ icons */
const ICON = {
  home: '<svg viewBox="0 0 24 24"><path d="M3 11.5 12 4l9 7.5"/><path d="M5 10v10h14V10"/></svg>',
  key: '<svg viewBox="0 0 24 24"><circle cx="8" cy="14" r="4"/><path d="M11 11 20 2M16 6l2 2M13 9l2 2"/></svg>',
  people:
    '<svg viewBox="0 0 24 24"><circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><circle cx="17" cy="9" r="2.5"/><path d="M15.5 14.5a5 5 0 0 1 6 5"/></svg>',
  bell: '<svg viewBox="0 0 24 24"><path d="M6 16V11a6 6 0 0 1 12 0v5l2 2H4z"/><path d="M10 20a2 2 0 0 0 4 0"/></svg>',
  server:
    '<svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="6" rx="1.5"/><rect x="3" y="14" width="18" height="6" rx="1.5"/><path d="M7 7h.01M7 17h.01"/></svg>',
  gear: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7 7 0 0 0-2-1.2L14.2 3h-4.4l-.4 2.6a7 7 0 0 0-2 1.2l-2.3-.9-2 3.4 2 1.5A7 7 0 0 0 5 12a7 7 0 0 0 .1 1.2l-2 1.5 2 3.4 2.3-.9a7 7 0 0 0 2 1.2l.4 2.6h4.4l.4-2.6a7 7 0 0 0 2-1.2l2.3.9 2-3.4-2-1.5A7 7 0 0 0 19 12z"/></svg>',
  lock: '<svg viewBox="0 0 24 24"><rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/></svg>',
  doc: '<svg viewBox="0 0 24 24"><path d="M7 3h7l5 5v13H7z"/><path d="M14 3v5h5"/></svg>',
  folder: '<svg viewBox="0 0 24 24"><path d="M3 6h6l2 2h10v11H3z"/></svg>',
  link: '<svg viewBox="0 0 24 24"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1.5 1.5"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1.5-1.5"/></svg>',
  user: '<svg viewBox="0 0 24 24"><circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/></svg>',
  laptop:
    '<svg viewBox="0 0 24 24"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>',
  shield:
    '<svg viewBox="0 0 24 24"><path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="m9 12 2 2 4-4"/></svg>',
  search:
    '<svg viewBox="0 0 24 24"><circle cx="11" cy="11" r="6"/><path d="m20 20-4-4"/></svg>',
  copy: '<svg viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5h10"/></svg>',
  plus: '<svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>',
  check: '<svg viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></svg>',
  warn: '<svg viewBox="0 0 24 24"><path d="M12 3 2 21h20z"/><path d="M12 10v5M12 18h.01"/></svg>',
  down: '<svg viewBox="0 0 24 24"><path d="M12 4v12M6 11l6 6 6-6M4 20h16"/></svg>',
  eye: '<svg viewBox="0 0 24 24"><path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/></svg>',
  refresh:
    '<svg viewBox="0 0 24 24"><path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/></svg>',
  terminal:
    '<svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/></svg>',
  chip: '<svg viewBox="0 0 24 24"><rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"/></svg>',
  list: '<svg viewBox="0 0 24 24"><path d="M4 7h16M4 12h16M4 17h16"/></svg>',
  columns:
    '<svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 4v16"/></svg>',
  flag: '<svg viewBox="0 0 24 24"><path d="M5 21V4h11l-2 4 2 4H5"/></svg>',
  chevron: '<svg viewBox="0 0 24 24"><path d="m9 6 6 6-6 6"/></svg>',
  wifi: '<svg viewBox="0 0 24 24"><path d="M2 9a15 15 0 0 1 20 0M5.5 12.5a10 10 0 0 1 13 0M9 16a5 5 0 0 1 6 0"/><circle cx="12" cy="19" r="1"/></svg>',
};
const kindIcon = (kind) =>
  ({ Secret: ICON.key, File: ICON.doc, Folder: ICON.folder, Link: ICON.link })[
    kind
  ] || ICON.doc;

/* ------------------------------------------------------------ chrome */
const RAIL = [
  ['home', 'Home', ICON.home, '01-start.html'],
  ['items', 'Items', ICON.key, '02-items.html'],
  ['teams', 'Teams', ICON.people, '03-teams.html'],
  ['attention', 'Attention', ICON.bell, '06-attention.html'],
];
const RAIL_FOOT = [
  ['servers', 'Servers', ICON.server, '04-servers.html'],
  ['settings', 'Settings', ICON.gear, '05-settings.html'],
];
function rail(active, opts = {}) {
  /* The Attention badge follows the lease world (needsYou); a file passes badges.attention only when it counts its own cards (06). */
  const badges = Object.assign(
    { attention: needsYou(), home: 2, homeClass: 'amber' },
    opts.badges || {},
  );
  const nav = ([id, label, icon, href]) =>
    `<a class="nav ${active === id ? 'on' : ''}" href="${href}">${icon}<span>${label}</span>${badges[id] ? `<span class="badge ${badges[id + 'Class'] || ''}">${badges[id]}</span>` : ''}</a>`;
  const who =
    opts.who === false
      ? ''
      : `<div class="who"><b>rae</b> on foks.example.net<br><b>rae.chen</b> on foks.acme-corp.com</div>`;
  return `<nav class="rail">${RAIL.map(nav).join('')}<div class="foot">${who}${RAIL_FOOT.map(nav).join('')}</div></nav>`;
}
function titlebar(crumbs, agent = 'Agent ready') {
  return `<div class="titlebar"><div class="lights"><i></i><i></i><i></i></div><div class="crumb"><b>FOKS</b>${crumbs.map((c) => `<span>›</span>${esc(c)}`).join('')}</div><div class="agent"><i></i>${esc(agent)}</div></div>`;
}
function pageHead(title, sub = '', actions = '') {
  return `<div class="head"><h1>${title}</h1>${sub ? `<span class="sub">${sub}</span>` : ''}<div class="actions">${actions}</div></div>`;
}
function banner(kind, title, detail, action = '', tag = '') {
  return `<div class="banner ${kind}"><div class="txt"><b>${title} ${tag}</b>${detail}</div>${action ? `<div class="act">${action}</div>` : ''}</div>`;
}
const band = (kind, text) =>
  `<div class="band ${kind}"><span class="lbl">${kind}</span><span>${text}</span></div>`;
const inspect = (label, obj) =>
  `<details class="inspect"><summary>${label}</summary><pre>${esc(JSON.stringify(obj, null, 2))}</pre></details>`;
function openSheet(html, cls = '') {
  let m = $('modal');
  if (!m) {
    m = document.createElement('div');
    m.id = 'modal';
    m.className = 'modal';
    document.querySelector('.window').appendChild(m);
  }
  m.innerHTML = `<div class="sheet ${cls}" role="dialog">${html}</div>`;
  m.hidden = false;
}
function closeSheet() {
  const m = $('modal');
  if (m) m.hidden = true;
}
function toast(html, ms = 0) {
  let t = $('toast');
  if (!t) {
    t = document.createElement('div');
    t.id = 'toast';
    t.className = 'toast';
    document.querySelector('.window').appendChild(t);
  }
  t.innerHTML = html;
  t.hidden = false;
  if (ms) setTimeout(() => (t.hidden = true), ms);
}
/* The one team card. Home's Your teams tile and the Teams list both draw this, so the same team never
   appears two ways. Variants: active (roster stack, "N people and teams", Open ›), inactive (Resume
   creation · journaled), lapsed (the server cannot be read, so no roster is drawn). opts.resume lets a
   file route Resume creation its own way; opts.href overrides the Open › target. */
const teamMark = (st, cls = '') =>
  `<span class="avatar lg team ${cls}" style="background:${st.active === false ? '#c9c9d1' : st.color}">${st.mark}</span>`;
const peopleText = (ps) => {
  const n = ps.length,
    teams = ps.some((p) => p.party_kind === 'named-team');
  return teams
    ? `${n} people and teams`
    : `${n} ${n === 1 ? 'person' : 'people'}`;
};
function teamCard(st, opts = {}) {
  const sv = serverById(st.server),
    acct = FX.accounts.find((a) => a.alias === st.account),
    ps = partiesOf(st.id),
    mine = ps.find((p) => p.you);
  const off = st.active === false,
    lap = serverLapsed(st.server);
  const status = lap
    ? `<span class="chip bad">Server unreadable</span>`
    : off
      ? `<span class="chip warn">Inactive</span>`
      : `<span class="chip">${st.team_kind === 'named' ? 'Named team' : 'Ad-hoc team'}</span>`;
  const resume = opts.resume || `data-act="resume" data-arg="${esc(st.id)}"`;
  const body = off
    ? `<p class="small muted">Creation did not finish. Resuming verifies what was already written rather than repeating it.</p><div class="row"><button class="btn small" ${resume}>${ICON.refresh}Resume creation</button><span class="tiny faint">journaled · CreateTeam pending</span></div>`
    : lap
      ? `<p class="small muted">The roster cannot be listed until this Mac's server check-in for ${esc(sv.name)} is fresh again.</p>`
      : `<div class="row"><span class="stack">${ps.map((p) => avatar(p)).join('')}</span><span class="small muted">${peopleText(ps)}</span><a class="open" href="${esc(opts.href || '03-teams.html?state=people')}">Open ›</a></div>`;
  return `<div class="tile teamcard" data-team="${esc(st.id)}">
<div class="thead">${teamMark(st)}<div class="grow"><h2>${esc(st.name)}</h2><div class="small muted">${esc(sv.name)}</div></div>${status}</div>
<div class="tbody"><div class="row small muted"><span>via your account <b class="bold" style="color:var(--ink-2)">${esc(acct.username)}</b></span>${mine && !off ? `<span>·</span><span>your role</span>${roleChip(mine.destination_role)}` : ''}</div>${body}</div></div>`;
}
function lock(title, detail, step) {
  return `<div class="lock"><div class="card" style="width:400px;text-align:center;padding:26px"><div class="avatar lg" style="margin:0 auto 12px;background:var(--accent)">F</div><h2>${title}</h2><p>${detail}</p>${step ? `<div class="codebox" style="margin-top:12px;justify-content:center">${step}</div>` : ''}</div></div>`;
}

/* The one Reset sheet, used by Servers and Attention alike. `resumables` are
   the journaled operations on that server that the reset also discards. */
function resetSheetHtml(server, resumables = []) {
  return `<h2>Reset ${esc(server)}'s local state on this Mac</h2>
    <p class="lead">${FX.copy.rollback_reset}</p>
    <div class="inset">
      <div class="irow"><span class="k">Discarded</span><span class="v">The pinned checkpoint and cached state for ${esc(server)}</span></div>
      <div class="irow"><span class="k">Lost</span><span class="v">Anything written here and not yet accepted by the server</span></div>
      <div class="irow"><span class="k">Also discarded</span><span class="v">${resumables.length ? resumables.map(esc).join('<br>') : 'No half-finished operations on this server'}<span class="sub">Every operation you could still have resumed on this server</span></span></div>
      <div class="irow"><span class="k">Kept</span><span class="v">Your keys and account on this Mac; a later check fetches fresh state</span></div>
      <div class="irow"><span class="k">Untouched</span><span class="v">Every other server</span></div>
    </div>
    <div class="field"><label>Type ${esc(server)} to confirm</label><div class="input mono"><input id="resetConfirm" placeholder="${esc(server)}" autocomplete="off"></div></div>
    <div class="foot"><span class="left">${FX.copy.gated_rollback}</span><button class="btn" id="resetCancel">Cancel</button><button class="btn danger" id="resetGo" disabled>Reset local state</button></div>`;
}
function openResetSheet(server, resumables, onConfirm) {
  openSheet(resetSheetHtml(server, resumables));
  const input = $('resetConfirm'),
    go = $('resetGo');
  input.oninput = () => {
    go.disabled = input.value.trim() !== server;
  };
  $('resetCancel').onclick = closeSheet;
  go.onclick = () => {
    closeSheet();
    onConfirm && onConfirm();
  };
  input.focus();
}

/* ------------------------------------------------------------ review strip + state */
function review(states, current, note, extra = '') {
  const strip =
    document.querySelector('.review') ||
    document.body.insertBefore(
      Object.assign(document.createElement('div'), { className: 'review' }),
      document.body.firstChild,
    );
  strip.innerHTML = `<b>Review</b>${states.map((s) => `<button class="chip ${s === current ? 'on' : ''}" data-state="${s}">${s}</button>`).join('')}${extra}<span class="note">${note || ''} — everything outside the window is review scaffolding, not the app. Deep-link with ?state=.</span>`;
  strip
    .querySelectorAll('[data-state]')
    .forEach((b) => (b.onclick = () => go(b.dataset.state)));
}
function getState(def) {
  const s = new URLSearchParams(location.search).get('state');
  return s || def;
}
function setUrl(s) {
  const q = new URLSearchParams(location.search);
  q.set('state', s);
  history.replaceState(null, '', '?' + q);
}
