  /* --------------------------------------------------------- 60-groups.js
     Group settings (People / Settings), the party panel, the federation
     table, the six-in-one GroupSheet, and Settings › Groups.

     Transcribed from foks-ui/src/screens/groups-screen.tsx (whole file),
     screens/store-access.tsx, the GroupsSection + invite dialog of
     screens/settings-screen.tsx, and model/roles.ts + model/readers.ts.
     Ground truth for structure and classes: /tmp/foks-mock/cap/groups/*.html.

     Pages
       eng-people        Groups › Engineering › People
       eng-settings      Groups › Engineering › Settings
       household-people  Groups › Household › People
       household-settings
       homelab-people    Groups › Homelab › People       (inactive ad-hoc)
       homelab-settings
       platform-people   Groups › Platform › People      (what F1 creates; the
       platform-settings                                  §1.14 fallback until
                                                          `created` is set)
       group-unavailable Groups › Group unavailable      (the §1.14 fallback)
       set-groups        Settings › Groups               (settings layout)
       store-platform    Vault › Platform                (created by F1; see
                                                          the note on it below)

     View keys (all declared per page): tab · panel · sheet · target · menu ·
     role · visibility · remote · gkind · name · username · account · created ·
     applied · adhoc · failure · inspect.

     `applied` is the post-mutation world, held as URL-reproducible tokens
     (`op:store:arg…`, several joined by `,`) rather than as a one-way
     mutation: every render rebuilds M.data.parties / M.data.federation from
     the pristine fixture and replays the tokens, so the sidebar caption, the
     reader counts and the Alerts badge all move with the roster exactly as
     they do in the app, and a capture URL reproduces the state.  */

  (function () {
    var D = M.data;
    var UI = M.ui;
    var esc = M.esc;
    var h = M.h;

    /* ------------------------------------------------------- the fixture
       The pristine copy of everything a mutation touches. syncWorld() puts
       M.data back to this and replays `v.applied` / `v.created` on top, so
       nothing here ever drifts and every state is reachable from its URL
       alone; restoreFixture() (from after()) puts it back again the moment
       this render is on screen, so the residue never reaches a page owned by
       another part — the sidebar, Alerts and the vault pages all re-derive
       from M.data and none of them resets the roster. */
    var TEAMS = ['team:eng', 'team:household', 'team:homelab'];
    var BASE = {
      parties: {},
      federation: D.federation.slice(),
      stores: D.stores.slice(),
      partyList: D.partyList.slice(),
      /* `active` is read off the shared store object, which 40-items writes in
         place from its own sync — so the fixture's value is kept, not the
         object's. */
      homelabActive: (D.stores.filter(function (st) { return st.id === 'team:homelab'; })[0] || {}).active,
    };
    TEAMS.forEach(function (ref) { BASE.parties[ref] = D.partiesOf(ref).slice(); });

    /* team:eng → 'eng', for compact `applied` tokens. */
    var SHORT = { 'team:eng': 'eng', 'team:household': 'household', 'team:homelab': 'homelab', 'team:platform': 'platform' };
    var LONG = { eng: 'team:eng', household: 'team:household', homelab: 'team:homelab', platform: 'team:platform' };

    /* The group F1 creates, exactly as mock-bridge.ts:556 `createGroup`
       answers it: `team:<teamAlias>`, `name || teamAlias` (so an AD-HOC group
       — which sends no name — is known by its alias, lower-case), the
       account's server and account, and a `03` / `14` prefixed id. The whole
       identity rides in `v.created` as `alias|name|kind|accountStore`, so
       `?page=store-platform&created=…` reproduces either branch of F1. */
    function createdOf(token) {
      var bits = String(token).split('|');
      var alias = bits[0] || 'platform';
      var kind = bits[2] === 'adhoc' ? 'adhoc' : 'named';
      return {
        alias: alias, kind: kind,
        name: kind === 'named' ? (bits[1] || 'Platform') : alias,
        account: bits[3] || 'acct:work',
      };
    }
    function createdToken(o) {
      return [o.alias, o.kind === 'named' ? o.name : '', o.kind, o.account].join('|');
    }
    function createdStore(o) {
      var account = BASE.stores.filter(function (st) { return st.id === o.account; })[0] ||
        BASE.stores.filter(function (st) { return st.kind === 'account'; })[0];
      return {
        id: 'team:' + o.alias, kind: 'team', name: o.name, alias: o.alias,
        server: account.server, account: account.account, active: true,
        team_kind: o.kind,
        team_id_hex: (o.kind === 'named' ? '03' : '14') + new Array(65).join('1'),
      };
    }
    function createdParty(store) {
      var who = D.accounts.filter(function (a) {
        return a.server === store.server && a.alias === store.account;
      })[0];
      var username = who ? who.username : store.account;
      return {
        store: store.id, username: username, label: 'you', party_kind: 'user',
        generation: 1, locally_manageable: true,
        party_id_hex: '01' + new Array(65).join('2'),
        source_role: { role: 'Owner' }, destination_role: { role: 'Owner' },
      };
    }

    /* ------------------------------------------------------------ state */
    function v(s, key, dflt) {
      var val = s.v[key];
      return (val === undefined || val === null || val === '') ? dflt : val;
    }
    /* A text field the user can legitimately empty: the core's bound inputs
       keep '' as '' (README, "Bound inputs"), so typing the field empty is
       read back as ''. Only a DECK CHIP cannot write '' — putKey folds it to
       null, i.e. back to the default — so the "(cleared)" chips carry EMPTY
       and this reads that token as the empty string too. */
    var EMPTY = '(empty)';
    function vtext(s, key, dflt) {
      var val = s.v[key];
      if (val === EMPTY) return '';
      return val == null ? dflt : val;
    }
    function ops(s) {
      return String(v(s, 'applied', '')).split(',').filter(Boolean);
    }
    function pushOp(token) {
      var list = ops(M.s);
      if (list.indexOf(token) < 0) list.push(token);
      M.set('v.applied', list.join(','), { silent: true });
    }

    /* One party row as the mock bridge answers it — the derived avatar facts
       (name/initials/hue) are carried like every other fixture party. */
    function userParty(ref, username, role, visibility, idHex, generation) {
      var dest = role === 'Member' ? { role: 'Member', visibility: visibility } : { role: role };
      return {
        store: ref, username: username, party_kind: 'user',
        generation: generation == null ? 1 : generation, locally_manageable: true,
        party_id_hex: idHex, source_role: dest, destination_role: dest,
        name: username, initials: D.initials(username), hue: D.hue(username),
      };
    }

    var OPS = { add: 1, demote: 1, remove: 1, admit: 1, restore: 1 };
    function applyOp(token) {
      var bits = token.split(':');
      var op = bits[0];
      var ref = LONG[bits[1]] || bits[1];
      /* `applied` is a key 70-settings and 71-servers also write (as 'yes' /
         '1'), and a flow can carry theirs onto one of our pages — anything
         that is not one of our tokens is simply not a group mutation. */
      if (!OPS[op] || !D.parties[ref]) return;
      var list = D.parties[ref];
      if (op === 'add') {
        list.push(userParty(ref, bits[2], bits[3], +bits[4] || 0,
          '01' + '3333333333333333333333333333333333333333333333333333333333333339'.slice(0, 64)));
      } else if (op === 'demote') {
        list.forEach(function (p, i) {
          if (p.username !== bits[2]) return;
          var next = Object.assign({}, p, { generation: p.generation + 1 });
          next.destination_role = bits[3] === 'Member'
            ? { role: 'Member', visibility: +bits[4] || 0 } : { role: bits[3] };
          list[i] = next;
        });
      } else if (op === 'remove') {
        D.parties[ref] = list.filter(function (p) { return p.username !== bits[2]; });
      } else if (op === 'admit') {
        var remote = D.stores.filter(function (st) { return st.alias === bits[2] && st.kind === 'team'; })[0];
        if (!remote) return;
        var host = D.servers.filter(function (srv) { return srv.id === remote.server; })[0];
        var vis = +bits[3] || 0;
        var count = D.federation.filter(function (f) { return f.store === ref; }).length + 1;
        D.parties[ref] = list.concat([{
          store: ref, username: null, party_kind: 'named-team',
          team_name: remote.alias + ' @ ' + (host ? host.name : remote.server),
          generation: 1, locally_manageable: false,
          party_id_hex: remote.team_id_hex,
          scoped_host_id_hex: host ? host.host_id : null,
          source_role: { role: 'Owner' },
          destination_role: { role: 'Member', visibility: vis },
          name: remote.alias + ' @ ' + (host ? host.name : remote.server),
          initials: '', hue: 'var(--c-team)',
        }]);
        D.federation = D.federation.concat([{
          store: ref, remote_profile: remote.server, remote_team_alias: remote.alias,
          remote_host_id_hex: host ? host.host_id : null, remote_team_id_hex: remote.team_id_hex,
          destination: { role: 'Member', visibility: vis },
          operation_id_hex: 'admission-' + count, active: true,
        }]);
      } else if (op === 'restore') {
        D.federation = D.federation.map(function (entry) {
          return (entry.store === ref && entry.operation_id_hex === bits[2])
            ? Object.assign({}, entry, { active: true }) : entry;
        });
      }
    }

    var FAILURES = {
      roster: { source: 'roster', code: 'rate-limited', message: 'Roster request was limited.', retryable: true },
      federation: { source: 'federation', code: 'quota-exceeded', message: 'Federation capacity was reached.', retryable: false },
    };

    /* Put the fixture back, then replay this state's knobs on top. Called at
       the head of every renderer here, before M.data.world(s) is read.
       Nothing in BASE is ever mutated — `adhoc` swaps in a *copy* of the
       Homelab store, so unsetting the knob puts the inactive one back. */
    function syncWorld(s, ref) {
      restoreFixture();
      var resumed = v(s, 'adhoc', 'no') === 'yes';
      /* Always a copy, never the fixture object: 40-items writes
         `homelab.active` in place from its own sync, so reading BASE's object
         back would carry that write into a state that has the knob off. */
      D.stores = BASE.stores.map(function (st) {
        return st.id === 'team:homelab' ? Object.assign({}, st, { active: resumed }) : st;
      });

      if (v(s, 'created', '')) ensureCreated(createdOf(v(s, 'created', '')));

      var failure = v(s, 'failure', '');
      if (ref && failure) {
        if (failure === 'roster' || failure === 'both')
          D.groupDetailFailures.push(Object.assign({ store: ref }, FAILURES.roster));
        if (failure === 'federation' || failure === 'both')
          D.groupDetailFailures.push(Object.assign({ store: ref }, FAILURES.federation));
      }
      ops(s).forEach(applyOp);
      D.partyList = D.parties['team:eng'].concat(D.parties['team:household']);
    }
    /* Put M.data back exactly as it was found, with nothing of ours left in
       it. Called from after(), once this render is on screen: every page here
       re-derives its whole world from `v.applied` / `v.created` / `v.adhoc` /
       `v.failure` at the head of its own render, so the fixture is only ever
       ours for the length of one render — and a page owned by another part
       (the sidebar's caption, Alerts, any vault) can never inherit a roster
       we changed or a group we created. */
    function restoreFixture() {
      TEAMS.forEach(function (team) {
        if (BASE.parties[team]) D.parties[team] = BASE.parties[team].slice();
        else delete D.parties[team];
      });
      D.federation = BASE.federation.slice();
      D.groupDetailFailures = [];
      D.partyList = BASE.partyList.slice();
      D.stores = BASE.stores.map(function (st) {
        return st.id === 'team:homelab'
          ? Object.assign({}, st, { active: BASE.homelabActive }) : st;
      });
    }

    /* The group `v.created` names, built into the catalog for this render.
       It lives exactly as long as the key does: F1 lands on `store-platform`
       with the token set, and every page that shows the new group declares
       `created`, so `M.go` carries it — and a page that does not is a page
       the app would have refreshed the catalog for anyway. */
    function ensureCreated(o) {
      var store = createdStore(o);
      SHORT[store.id] = o.alias; LONG[o.alias] = store.id;
      /* The sidebar row, the hero's Back and every `Open in vault` read
         M.storePages; `store-platform` is the page that draws whatever F1
         made, so point the new ref at it (and at nothing else — a second
         creation in one session replaces the first). */
      M.storePages[store.id] = 'store-platform';
      PLATFORM_PAGES.forEach(function (pg) { pg.gref = store.id; pg.nav = store.id; });
      if (D.stores.some(function (st) { return st.id === store.id; })) return;
      D.stores = D.stores.concat([store]);
      /* Never into BASE: the created group is this render's, and TEAMS only
         records the ref so restoreFixture knows to take it out again. */
      if (TEAMS.indexOf(store.id) < 0) TEAMS.push(store.id);
      D.parties[store.id] = [createdParty(store)];
    }
    var PLATFORM_PAGES = [];
    /* lease.ts:107 — 20-data owns it, and M.data.world reads the same array,
       so the pane, the sidebar caption and the Settings attention row can
       never disagree about a failure. */
    var failureFor = D.groupDetailFailure;

    /* ------------------------------------------------- model (roles.ts) */
    function sortRoster(list) {
      return list.map(function (p, i) { return { p: p, i: i }; })
        .sort(function (a, b) {
          var rank = D.roleRank(b.p.destination_role) - D.roleRank(a.p.destination_role);
          if (rank) return rank;
          if (a.p.label === 'you' && b.p.label !== 'you') return -1;
          if (b.p.label === 'you' && a.p.label !== 'you') return 1;
          return a.i - b.i;
        }).map(function (x) { return x.p; });
    }
    function fmtRole(role) {
      var parsed = D.parseRole(role);
      return parsed ? D.formatRole(parsed) : (typeof role === 'string' ? role : role.role);
    }
    function roleName(role) {
      var parsed = D.parseRole(role);
      if (!parsed) return typeof role === 'string' ? role : role.role;
      return parsed.kind === 'owner' ? 'Owner' : parsed.kind === 'admin' ? 'Admin' : 'Member';
    }
    function shortName(party) {
      return party.party_kind === 'user'
        ? D.partyName(party)
        : (party.team_name ? party.team_name.split(' @')[0] : D.partyName(party));
    }
    function demotionFor(party) {
      var role = D.parseRole(party.destination_role);
      if (!role) return null;
      if (role.kind === 'owner' || role.kind === 'admin') return { role: 'Member', visibility: 0 };
      var vis = D.visibilityOf(role);
      return vis > -32768 ? { role: 'Member', visibility: vis - 1 } : null;
    }
    function oxfordOr(names) {
      if (names.length <= 1) return names[0] || '';
      if (names.length === 2) return names[0] + ' or ' + names[1];
      return names.slice(0, -1).join(', ') + ', or ' + names[names.length - 1];
    }
    function itemsOf(w, ref) {
      return w.items.filter(function (item) { return item.store === ref; });
    }
    function readsOf(w, party) {
      return itemsOf(w, party.store).filter(function (item) {
        var readers = D.readersOf(item);
        return !!readers && readers.some(function (c) { return c.party_id_hex === party.party_id_hex; });
      });
    }
    function canTarget(party) { return D.actionableGroupMember(party); }
    function partySubtitle(party) {
      if (party.party_kind !== 'user') {
        var host = (party.team_name && party.team_name.indexOf(' @ ') >= 0)
          ? party.team_name.split(' @ ')[1] : 'another server';
        return 'group on ' + esc(host) +
          (party.scoped_host_id_hex ? ' · host <code>' + esc(party.scoped_host_id_hex) + '</code>' : '');
      }
      var machine = !!(party.note && party.note.indexOf('service account') >= 0);
      return (machine ? 'machine' : 'person') + ' · generation ' + party.generation;
    }
    function roleChip(role, extra) {
      if (!role) return '<span class="rolecell">' + UI.chip('—') + '</span>';
      var parsed = D.parseRole(role);
      return '<span class="rolecell">' + UI.chip(esc(roleName(role))) +
        (parsed && parsed.kind === 'member' ? '<small>visibility ' + D.visibilityOf(parsed) + '</small>' : '') +
        (extra ? '<small>' + esc(extra) + '</small>' : '') + '</span>';
    }
    function canCreateInStore(w, ref) {
      var store = w.storeById[ref];
      if (!store || !store.readable) return false;
      if (store.kind === 'account') return true;
      return D.partiesOf(ref).filter(function (p) {
        return p.label === 'you' && p.party_kind === 'user' && p.locally_manageable &&
          D.admissionActive(p, ref);
      }).length === 1;
    }
    /* ================================================== rendered pieces */

    function mark(store, size) {
      var background = (store.kind === 'team' && store.active === false)
        ? 'var(--c-none)' : UI.rgb(D.hue(store.name));
      return '<span class="kico ' + size + ' group" style="background: ' + background + ';">' +
        esc(store.name.slice(0, 1)) + '</span>';
    }

    /* groups-screen.tsx:902/1018/1865/1927 — every one of these navigates to
       {kind:'settings', section:'servers', profile: store.server}: the
       server's own detail pane, not the server list. */
    function serverNav(store) {
      return 'data-act="go" data-page="srv-server" data-set=\'{"v.profile":"' + esc(store.server) + '"}\'';
    }

    function ghero(s, store) {
      var unavailable = store.state !== 'normal' && store.state !== 'inactive';
      var inactive = store.active === false;
      var back = 'Back to ' + store.name;
      var open = v(s, 'menu', '') === 'more';
      return '<div class="ghero">' +
        '<button type="button" class="back" title="' + esc(back) + '" aria-label="' + esc(back) + '"' +
        ' data-act="go" data-page="' + esc(M.storePages[store.id] || 'all') + '">' + M.icon('chev') + '</button>' +
        mark(store, 'big') +
        '<div class="t"><h1><span>' + esc(store.name) + '</span>' +
        (inactive ? UI.chip('Inactive', { tone: 'warn' }) : '') +
        '</h1><div class="sub">Group settings</div></div>' +
        '<div class="header-action">' +
        (unavailable ? UI.btn(store.state === 'lease-lapsed' ? 'Open server' : 'Review server',
          { attrs: serverNav(store) }) : '') +
        (inactive ? '' : '<span class="menuwrap">' + UI.btn(M.icon('more'), {
          variant: 'quiet', title: 'More', ariaLabel: 'More',
          attrs: 'aria-haspopup="menu" aria-expanded="' + (open ? 'true' : 'false') + '"' +
            ' data-act="set" data-key="v.menu" data-val="' + (open ? '' : 'more') + '"',
        }) + '</span>') +
        '</div></div>';
    }

    function situationBand(store, tab) {
      if (store.kind === 'team' && store.active === false) {
        return UI.band({
          label: 'Setup incomplete.',
          text: 'The roster and items are unavailable until the group is finished. ' +
            '<button type="button" class="lnk" data-act="call" data-fn="gFinishSetup">Finish setup</button>',
        });
      }
      if (store.kind === 'team' && store.team_kind === 'adhoc' && tab !== 'settings') {
        return UI.band({
          severity: 'info', label: 'Ad-hoc group.',
          text: 'Membership is fixed when it’s created; people can’t be added or removed here. Its items and roles read normally.',
        });
      }
      return '';
    }

    function partyRow(w, store, party, selected, manageable) {
      var active = D.admissionActive(party, party.store);
      var actionable = manageable && canTarget(party);
      var extras = [
        party.party_kind !== 'user' ? fmtRole(party.source_role) + ' at ' + shortName(party) : null,
        active ? null : 'admission inactive',
      ].filter(Boolean).join(' · ');
      var key = shortName(party);
      return '<div class="prow' + (selected ? ' sel' : '') + (active ? '' : ' dim') + '"' +
        ' role="button" tabindex="0" aria-pressed="' + (selected ? 'true' : 'false') + '"' +
        ' data-act="call" data-fn="gTogglePanel" data-arg="' + esc(key) + '">' +
        '<span class="who2">' + UI.avatar(party, { className: 'pav' }) +
        '<span class="t"><b><span>' + esc(key) + '</span>' +
        (party.label ? UI.chip('you', { tone: 'you' }) : '') + '</b>' +
        '<small>' + partySubtitle(party) + '</small></span></span>' +
        roleChip(party.destination_role, extras || undefined) +
        '<span class="acts">' + (actionable
          ? '<button type="button" title="Change role or remove" aria-label="Change role or remove"' +
            ' data-act="call" data-fn="gOpenPanel" data-arg="' + esc(key) + '">' + M.icon('more') + '</button>'
          : '') + '</span></div>';
    }

    function peopleTab(s, w, store, selected, manageable) {
      var parties = sortRoster(D.partiesOf(store.id));
      var inactive = store.kind === 'team' && store.active === false;
      var showAdd = store.kind === 'team' && store.team_kind === 'named';
      var hint = manageable ? undefined : 'Not available for this group';
      var failure = failureFor(store.id, 'roster');
      var inner;
      if (failure) {
        inner = UI.notice({
          title: 'Roster unavailable', body: '<p>' + esc(failure.message) + '</p>',
          actions: failure.retryable
            ? UI.btn('Refresh', { attrs: 'data-act="toast" data-text="Group roster refreshed"' })
            : undefined,
        });
      } else if (inactive) {
        inner = '';
      } else {
        inner = '<div class="rhead"><h2>People &amp; groups</h2>' +
          '<span class="n">' + esc(D.peopleGroups(parties)) + '</span>' +
          (showAdd ? '<div class="right">' +
            UI.btn('Add a group', {
              icon: 'people', disabled: !manageable,
              title: manageable ? 'Give every member of another group a role here' : hint,
              attrs: 'data-act="call" data-fn="gSheet" data-arg="admit"',
            }) +
            UI.btn('Add someone', {
              icon: 'plus', disabled: !manageable,
              title: manageable ? 'Add someone who already has an account on this server' : hint,
              attrs: 'data-act="call" data-fn="gSheet" data-arg="add"',
            }) + '</div>' : '') +
          '</div>';
        inner += parties.length
          ? '<div class="rt"><div class="hdr"><span>Who</span><span>Role</span><span></span></div>' +
            parties.map(function (party) {
              return partyRow(w, store, party,
                !!selected && selected.party_id_hex === party.party_id_hex, manageable);
            }).join('') + '</div>'
          : '<div class="callout">' +
            '<span class="kico" style="background: var(--chip-bg); color: var(--muted);">' + M.icon('people') + '</span>' +
            '<span class="t"><b>No people yet.</b>' +
            (showAdd ? ' Add someone by their username on this server, or admit another group.' : '') +
            '</span>' +
            (manageable ? UI.btn('Add someone', { attrs: 'data-act="call" data-fn="gSheet" data-arg="add"' }) : '') +
            '</div>';
      }
      return '<div class="roster">' + situationBand(store, 'people') + inner + '</div>';
    }

    function federationSection(s, w, store, manageable) {
      var entries = D.federation.filter(function (entry) { return entry.store === store.id; });
      var inactive = store.kind === 'team' && store.active === false;
      var parties = D.partiesOf(store.id);
      var showAdd = store.kind === 'team' && store.team_kind === 'named';
      var failure = failureFor(store.id, 'federation');
      var inner;
      if (failure) {
        inner = UI.notice({
          title: 'Federation unavailable', body: '<p>' + esc(failure.message) + '</p>',
          actions: failure.retryable
            ? UI.btn('Refresh', { attrs: 'data-act="toast" data-text="Group federation refreshed"' })
            : undefined,
        });
      } else if (inactive) {
        inner = '';
      } else {
        inner = '<div class="rhead"><h2>Groups on other servers</h2>' +
          (showAdd ? '<div class="right">' + UI.btn('Add a group', {
            icon: 'plus', disabled: !manageable,
            title: manageable ? undefined : 'Not available for this group',
            attrs: 'data-act="call" data-fn="gSheet" data-arg="admit"',
          }) + '</div>' : '') + '</div>';
        inner += entries.length
          ? '<div class="rt fed"><div class="hdr"><span>Group</span><span>Role here</span>' +
            '<span>Admission</span><span>Operation</span><span></span></div>' +
            entries.map(function (entry) {
              var party = parties.filter(function (c) { return c.party_id_hex === entry.remote_team_id_hex; })[0];
              var remoteName = (w.serverById[entry.remote_profile] || {}).name || entry.remote_profile;
              var readable = party ? readsOf(w, party).length : 0;
              return '<div class="prow"><span class="who2">' +
                (party ? UI.avatar(party, { className: 'pav' })
                  : '<span class="pav team" style="background: var(--c-team);">' + M.icon('people') + '</span>') +
                '<span class="t"><b><span>' + esc(entry.remote_team_alias) + '</span></b>' +
                '<small>on ' + esc(remoteName) + ' · host <code>' + esc(entry.remote_host_id_hex) + '</code></small>' +
                '</span></span>' +
                roleChip(entry.destination, 'for every member') +
                '<span class="reads">' + UI.chip(entry.active ? 'Active' : 'Inactive', { tone: entry.active ? 'ok' : 'warn' }) +
                '<small>' + (entry.active ? esc(D.plural(readable, 'item')) + ' readable' : 'members read nothing here') + '</small>' +
                '</span>' +
                '<span class="ver">' + esc(entry.operation_id_hex) + '</span>' +
                '<span class="cellact">' + (!entry.active && entry.operation_id_hex
                  ? UI.btn('Restore access', {
                      size: 'sm', icon: 'again', disabled: !manageable,
                      attrs: 'data-act="call" data-fn="gRestore" data-arg="' + esc(entry.operation_id_hex) + '"',
                    })
                  : '') + '</span></div>';
            }).join('') + '</div>'
          : '<div class="callout">' +
            '<span class="kico" style="background: var(--chip-bg); color: var(--muted);">' + M.icon('people') + '</span>' +
            '<span class="t"><b>No groups from other servers.</b>' +
            (showAdd ? ' Admitting a group gives every one of its members the same role here.' : '') +
            '</span>' +
            (manageable ? UI.btn('Add a group', { attrs: 'data-act="call" data-fn="gSheet" data-arg="admit"' }) : '') +
            '</div>';
      }
      return '<div class="roster federation-section">' + inner + '</div>';
    }

    function partyPanel(w, store, party, manageable) {
      var items = itemsOf(w, party.store);
      var readable = readsOf(w, party);
      var readableIds = {};
      readable.forEach(function (item) { readableIds[item.path] = true; });
      var machine = party.party_kind === 'user' && !!(party.note && party.note.indexOf('service account') >= 0);
      var here = fmtRole(party.destination_role);
      var active = D.admissionActive(party, party.store);
      var kind = party.party_kind === 'user' ? 'person' : 'group';
      var pinned = party.scoped_host_id_hex
        ? w.servers.filter(function (srv) { return srv.host_id && srv.host_id === party.scoped_host_id_hex; })[0]
        : undefined;
      var body = '<div class="sec">Can read<span class="n">· ' + readable.length + ' of ' + items.length + '</span></div>';
      body += items.length
        ? '<div class="rlist">' + items.map(function (item) {
            var kindName = D.kindOf(item);
            return '<div class="ir' + (readableIds[item.path] ? '' : ' no') + '">' +
              (kindName === 'Folder' ? '' : UI.kindIcon(kindName)) +
              '<span class="ipth">' + esc(item.path) + '</span>' +
              '<span class="pchip">' + esc(fmtRole(item.read)) + '</span></div>';
          }).join('') + '</div>'
        : '<p class="hint">No items in ' + esc(store.name) + ' yet.</p>';
      body += !active
        ? '<p class="hint">This group’s admission is inactive, so its members read nothing here until it is re-run.</p>'
        : (readable.length < items.length ? '<p class="hint">Greyed items need a higher role than ' + esc(here) + '.</p>' : '');
      body += '<div class="sec">Details</div><div class="meta">' +
        '<b>Role here</b><span>' + esc(here) + '</span>' +
        (party.party_kind !== 'user'
          ? '<b>At source</b><span>' + esc(fmtRole(party.source_role)) + ' in ' + esc(shortName(party)) + '</span>' +
            '<b>Host</b><code>' + esc(party.scoped_host_id_hex) + '</code>' +
            (pinned ? '<b>Pin</b><span>matches this Mac’s pin</span>'
              : (party.scoped_host_id_hex ? '<b>Pin</b><span>local pin fact unavailable</span>' : ''))
          : '') +
        '<b>Generation</b><span>' + esc(party.generation) + '</span>' +
        '<b>Party id</b><code>' + esc(party.party_id_hex) + '</code>' +
        '<b>Managed</b><span>' + (party.locally_manageable ? 'from this server' : 'from its own group') + '</span>' +
        '</div>';
      if (machine) {
        body += '<p class="hint">' + esc(party.note.charAt(0).toUpperCase() + party.note.slice(1)) + '.</p>';
      }
      var foot = (manageable && canTarget(party))
        ? '<div class="dfoot">' +
          (demotionFor(party)
            ? UI.btn('Lower role…', { attrs: 'data-act="call" data-fn="gSheetTarget" data-arg="demote|' + esc(shortName(party)) + '"' })
            : '') +
          UI.btn('Remove…', { variant: 'danger', attrs: 'data-act="call" data-fn="gSheetTarget" data-arg="remove|' + esc(shortName(party)) + '"' }) +
          '</div>'
        : '';
      return '<aside class="details"><div class="dh">' +
        UI.avatar(party, { className: 'pav' }) +
        '<span class="t"><h2>' + esc(shortName(party)) +
        (party.label ? UI.chip('you', { tone: 'you' }) : '') + '</h2>' +
        '<small>' + kind + ' · ' + esc(here) + ' here</small></span>' +
        '<button type="button" class="x" title="Close" aria-label="Close" data-act="set" data-key="v.panel" data-val="">' +
        M.icon('x') + '</button></div>' +
        '<div class="scroll">' + body + '</div>' + foot + '</aside>';
    }

    function removableOf(store, manageable) {
      if (!manageable) return [];
      return D.partiesOf(store.id).map(function (party, index) { return { party: party, index: index }; })
        .filter(function (row) { return canTarget(row.party); })
        .sort(function (l, r) {
          var rank = D.roleRank(l.party.destination_role) - D.roleRank(r.party.destination_role);
          if (rank) return rank;
          var lr = D.parseRole(l.party.destination_role);
          var rr = D.parseRole(r.party.destination_role);
          var vis = (lr && lr.kind === 'member' ? D.visibilityOf(lr) : 0) -
            (rr && rr.kind === 'member' ? D.visibilityOf(rr) : 0);
          if (vis) return vis;
          return l.index - r.index;
        }).map(function (row) { return row.party; });
    }

    function settingsTab(s, w, store, manageable) {
      var parties = D.partiesOf(store.id);
      var mine = parties.filter(function (p) { return p.label === 'you'; })[0];
      var owner = parties.filter(function (p) {
        var r = D.parseRole(p.destination_role);
        return r && r.kind === 'owner';
      })[0];
      var account = D.accounts.filter(function (a) {
        return a.alias === store.account && a.server === store.server;
      })[0];
      var seniors = parties.filter(function (p) {
        return p.party_kind === 'user' && p.label !== 'you' && D.roleRank(p.destination_role) >= 2;
      }).map(shortName);
      var removable = removableOf(store, manageable);
      var open = v(s, 'menu', '') === 'rekey';
      var rows = UI.insetRow({ label: 'Name', value: esc(store.name) }) +
        UI.insetRow({
          label: 'Server', value: esc(store.serverName),
          action: UI.btn('Open server', { size: 'sm', attrs: serverNav(store) }),
        }) +
        UI.insetRow({
          label: 'Your account',
          value: esc((account ? account.username : store.account) + ' · ' + (mine ? fmtRole(mine.destination_role) : '—')),
        }) +
        UI.insetRow({ label: 'Owner', value: owner ? esc(shortName(owner)) : '—' }) +
        UI.insetRow({
          label: 'Group ID', value: '<code>' + esc(store.team_id_hex) + '</code>',
          action: UI.btn('Copy', { size: 'sm', icon: 'copy', attrs: 'data-act="toast" data-text="Group ID copied."' }),
        });
      function dangerRow(title, detail, action) {
        return UI.insetRow({
          value: '<span class="t"><b>' + title + '</b><small>' + detail + '</small></span>',
          action: action,
        });
      }
      var danger =
        dangerRow('Remove a group member',
          'Removing anyone rotates the group key and blocks their future reads. Copies already downloaded are not erased.',
          '<span class="menuwrap">' + UI.btn('Remove…' + M.icon('chev', null, { cls: 'chevron' }), {
            variant: 'danger', disabled: !removable.length,
            attrs: 'aria-haspopup="menu" aria-expanded="' + (open ? 'true' : 'false') + '"' +
              ' data-act="set" data-key="v.menu" data-val="' + (open ? '' : 'rekey') + '"',
          }) + '</span>') +
        dangerRow('Leave ' + esc(store.name),
          seniors.length
            ? 'You can’t remove yourself yet. To leave, ask ' + esc(oxfordOr(seniors)) + ' to remove you.'
            : 'You can’t remove yourself yet, and no one else here can remove you.',
          UI.btn('Leave…', { variant: 'danger', disabled: true })) +
        dangerRow('Delete ' + esc(store.name),
          'Removes the group and every item in it for everyone. Not available yet.',
          UI.btn('Delete…', { variant: 'danger', disabled: true })) +
        dangerRow('Reset this Mac’s state for ' + esc(store.serverName),
          'Forgets what this Mac has read and pinned about the server.',
          UI.btn('Servers', { icon: 'out', attrs: serverNav(store) }));
      return '<div class="group-settings">' + situationBand(store, 'settings') +
        UI.sectionLabel('About this group') + UI.inset(rows) +
        (store.team_kind === 'adhoc'
          ? '<p class="fn">An ad-hoc group has no name on the server and a fixed membership.</p>' : '') +
        UI.sectionLabel('Danger') + UI.inset(danger, { className: 'danger settings-inset' }) +
        '</div>';
    }

    /* groups-screen.tsx:1013 — the raw shape, without the derived fields the
       mock's fixture carries for avatars. */
    function inspectResponse(w, store, tab) {
      if (tab === 'settings') {
        return {
          id: store.id, kind: store.kind, name: store.name, alias: store.alias,
          server: store.server, account: store.account, active: store.active,
          team_kind: store.team_kind, team_id_hex: store.team_id_hex,
        };
      }
      return {
        people: D.partiesOf(store.id).map(function (party) {
          var out = {};
          if (party.username) out.username = party.username;
          out.party_kind = party.party_kind;
          out.generation = party.generation;
          out.locally_manageable = party.locally_manageable;
          out.party_id_hex = party.party_id_hex;
          if (party.scoped_host_id_hex) out.scoped_host_id_hex = party.scoped_host_id_hex;
          out.source_role = party.source_role;
          out.destination_role = party.destination_role;
          return out;
        }),
        federation: D.federation.filter(function (entry) { return entry.store === store.id; })
          .map(function (entry) {
            var out = {
              remote_profile: entry.remote_profile,
              remote_team_alias: entry.remote_team_alias,
              remote_host_id_hex: entry.remote_host_id_hex,
              remote_team_id_hex: entry.remote_team_id_hex,
              destination: entry.destination,
            };
            if (entry.operation_id_hex) out.operation_id_hex = entry.operation_id_hex;
            out.active = entry.active;
            return out;
          }),
      };
    }

    function inspectToggle(s, w, store, tab) {
      return UI.toggle({
        id: tab === 'settings' ? '_r_9_' : '_r_0_',
        label: 'Inspect response',
        open: v(s, 'inspect', '') === '1',
        attrs: 'data-act="set" data-key="v.inspect" data-val="' + (v(s, 'inspect', '') === '1' ? '' : '1') + '"',
        body: '<pre>' + esc(JSON.stringify(inspectResponse(w, store, tab), null, 1)) + '</pre>',
      });
    }

    /* store-access.tsx:83 with noHeader — the hero stays, everything under
       it is replaced. */
    function accessTakeover(store) {
      var notice = store.notice;
      if (!notice) return '';
      return UI.body(UI.notice({
        severity: notice.severity === 'warn' ? 'warn' : 'crit',
        title: esc(notice.title),
        body: '<p>' + esc(notice.detail) + '</p>',
        actions: UI.btn(notice.action, {
          variant: notice.actionKind === 'finish-setup' ? 'primary' : undefined,
          attrs: notice.actionKind === 'finish-setup'
            ? 'data-act="call" data-fn="gFinishSetup"'
            : serverNav(store),
        }),
      }));
    }

    /* ===================================================== the sheets
       components/sheet.tsx, as the captures serialise it:
       backdrop[role][aria-modal][aria-labelledby][tabindex=-1] > .sheet[.wide]
       > .hd / .sb / .ft — which is exactly what `M.ui.sheet` emits. The panel
       carries the inert `data-act="stop"` the helper adds, so a click inside
       it never reaches the backdrop's close hook. */
    function sheetFrame(o) {
      return UI.sheet({
        titleId: o.titleId, danger: o.danger, width: o.width,
        glyph: o.glyph, title: o.title, subtitle: o.subtitle,
        body: o.body, footer: o.footer,
        backdropAttrs: 'data-act="call" data-fn="gBackdrop"',
      });
    }

    function targetOf(s, store, sheet) {
      var parties = D.partiesOf(store.id);
      function byName(name) {
        return parties.filter(function (p) { return shortName(p) === name; })[0];
      }
      if (sheet === 'demote-owner') return byName('sam.ortiz') || parties[0];
      if (sheet === 'party-remove') {
        var bot = byName('deploy-bot');
        return bot ? Object.assign({}, bot, { locally_manageable: false }) : null;
      }
      var chosen = v(s, 'target', '');
      if (chosen) return byName(chosen) || null;
      var fallback = sheet === 'demote' ? 'priya.n' : 'dana.okafor';
      return byName(fallback) ||
        parties.filter(function (p) {
          return canTarget(p) && (sheet !== 'demote' || demotionFor(p));
        })[0] || null;
    }

    /* components/field.tsx inside a sheet, as the captures serialise it: the
       controlled input carries React's `style=""`. `data-bind` is the core's
       hook — it keeps a typed-empty field as '', so `Add someone` / `Create
       group` and the `…` slug all follow the field down to nothing. */
    function textField(label, id, bind, value) {
      return '<div class="fr"><label class="k" for="' + id + '">' + esc(label) + '</label>' +
        '<span class="v"><input aria-label="' + esc(label) + '" id="' + id + '" type="text" value="' +
        esc(value) + '" style="" data-bind="' + esc(bind) + '" data-live></span></div>';
    }
    /* activateRow (groups-screen.tsx:287): Enter or Space on the row itself —
       never on a child — toggles the panel. A div[role=button] fires no click
       of its own, so the core's delegated handler needs the relay. */
    document.addEventListener('keydown', function (ev) {
      if (ev.key !== 'Enter' && ev.key !== ' ') return;
      var row = ev.target;
      if (!row || !row.classList || !row.classList.contains('prow')) return;
      if (row.getAttribute('role') !== 'button') return;
      ev.preventDefault();
      row.click();
    });
    /* The core's keydown hook ignores Escape inside an input, and typing
       leaves the caret in the field, so the sheet's own Escape would stop
       working after a keystroke. SheetDialog closes from anywhere inside it,
       so relay it — for a field inside one of OUR sheets only, since every
       other part owns its own Escape rule. */
    document.addEventListener('keydown', function (ev) {
      if (ev.key !== 'Escape' || !MINE[M.s.page]) return;
      var el = ev.target;
      if (!el || !el.closest || !el.closest('#overlays .sheet input')) return;
      if (M.fns.escape) M.fns.escape();
    });
    function stepper(label, dec, inc, decOff, incOff) {
      return '<div class="vis">' +
        UI.btn('−', { size: 'sm', disabled: decOff, attrs: 'data-act="set" data-key="v.visibility" data-val="' + dec + '"' }) +
        '<span>' + esc(label) + '</span>' +
        UI.btn('+', { size: 'sm', disabled: incOff, attrs: 'data-act="set" data-key="v.visibility" data-val="' + inc + '"' }) +
        '</div>';
    }

    function inviteMessageFor(w, store) {
      var ownerStore = w.stores.filter(function (c) {
        return c.kind === 'account' && c.server === store.server && c.account === store.account;
      })[0];
      var ownerAccount = ownerStore
        ? D.accounts.filter(function (a) { return a.store === ownerStore.id; })[0] : undefined;
      var inviter = ownerAccount ? ownerAccount.username.split('.')[0] : 'the group Admin';
      return 'Hi Jules — I\'d like to add you to ' + store.name + ' on FOKS.\n' +
        '1. Install FOKS: https://foks.app/download (placeholder)\n' +
        '2. Add the server: ' + store.serverName + '\n' +
        '3. Create your account there with username firstname.lastname — for you: jules.park\n' +
        '4. Then tell ' + inviter + ' your username — there are no invite links; I add you by username.\n' +
        'Once I have, ' + store.name + ' shows up under Groups in your app.';
    }

    function creationAccounts(w) {
      return w.stores.filter(function (c) { return c.kind === 'account' && canCreateInStore(w, c.id); });
    }
    function remotesFor(w, store) {
      return w.stores.filter(function (c) {
        return c.kind === 'team' && c.active && c.team_kind === 'named' &&
          c.readable && c.server !== store.server;
      });
    }
    function slugify(name) {
      return String(name).trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
    }

    /* What each host can mount. GroupSettingsScreen never names `create`
       (groups-screen.tsx:1673 lists the five it accepts, plus the
       `party-remove` alias); settings-screen.tsx mounts only `create`. The
       two `-owner` / `party-` ids are `?state=` aliases that change nothing
       but the target. */
    var GROUP_SHEET_KIND = {
      invite: 'invite', add: 'add', demote: 'demote', 'demote-owner': 'demote',
      remove: 'remove', 'party-remove': 'remove', admit: 'admit',
    };
    var SETTINGS_SHEET_KIND = { create: 'create' };
    function groupSheet(s, w, store, kinds) {
      var sheet = v(s, 'sheet', '');
      /* Anything else — another part's sheet id riding in on a deep link —
         is not one of ours and draws nothing. */
      var kind = (kinds || GROUP_SHEET_KIND)[sheet];
      if (!kind) return '';
      var target = (kind === 'demote' || kind === 'remove') ? targetOf(s, store, sheet) : null;
      var accounts = creationAccounts(w);
      var accountId = v(s, 'account', '');
      var creationAccount = accounts.filter(function (a) { return a.id === accountId; })[0] ||
        accounts.filter(function (a) { return a.account === 'work'; })[0] || accounts[0];
      var remotes = remotesFor(w, store);
      var remoteId = v(s, 'remote', '');
      var remote = remotes.filter(function (r) { return r.alias === remoteId || r.id === remoteId; })[0] || remotes[0];
      var name = vtext(s, 'name', 'Platform');
      var createKind = v(s, 'gkind', 'named');
      var teamAlias = slugify(name);
      var role = v(s, 'role', 'Member');
      var visibility = s.v.visibility == null || s.v.visibility === '' ? null : +s.v.visibility;
      var server = kind === 'create'
        ? (creationAccount ? (w.serverById[creationAccount.server] || {}).name : undefined)
        : store.serverName;
      var failure = kind === 'admit' ? failureFor(store.id, 'federation')
        : (['add', 'demote', 'remove'].indexOf(kind) >= 0 ? failureFor(store.id, 'roster') : undefined);
      var username = vtext(s, 'username', 'jules.park');

      var title = kind === 'invite' ? 'Invite someone to ' + store.name
        : kind === 'add' ? 'Add someone to ' + store.name
        : kind === 'demote' ? 'Lower ' + (target ? shortName(target) + '’s' : 'their') + ' role'
        : kind === 'remove' ? 'Remove ' + (target ? D.partyName(target) : 'them') + ' from ' + store.name + '?'
        : kind === 'admit' ? 'Add a group to ' + store.name
        : 'Create a group';
      var subtitle = kind === 'create' ? (server || 'Selected server')
        : kind === 'invite' ? 'There are no invite links — this writes the message that gets them an account'
        : kind === 'add' ? 'They must already have an account on ' + server
        : kind === 'demote' ? (target ? fmtRole(target.destination_role) : '') + ' in ' + store.name + ' today'
        : kind === 'remove' ? ''
        : kind === 'admit' ? 'Every member of that group gets the same role here'
        : server;

      var currentRole = target ? D.parseRole(target.destination_role) : null;
      var maxVis = currentRole && currentRole.kind === 'member' ? D.visibilityOf(currentRole) - 1 : 0;
      var demoteRole = v(s, 'role', 'Member');
      var demoteVis = visibility == null ? maxVis : visibility;

      var body = '';
      if (failure) {
        body += UI.notice({
          title: (failure.source === 'roster' ? 'Roster' : 'Federation') + ' unavailable',
          body: '<p>' + esc(failure.message) + ' Close this sheet and refresh before making changes.</p>',
        });
      }

      if (kind === 'invite') {
        var message = inviteMessageFor(w, store);
        body += '<p>They need an account on ' + esc(server) + ' before you can add them. Send this message, then add their username.</p>' +
          UI.inset(UI.insetRow({
            label: 'Message',
            value: '<span class="msg">' + esc(message) + '</span>' +
              UI.btn('Copy', { size: 'sm', attrs: 'data-act="toast" data-text="Message copied."' }),
          })) +
          UI.sectionLabel('Message contents') +
          UI.inset(
            UI.insetRow({
              label: 'Install link',
              value: '<code>https://foks.app/download</code> ' + UI.chip('placeholder', { tone: 'warn' }) +
                '<span class="hint">Temporary download address.</span>',
            }) +
            UI.insetRow({ label: 'Server', value: '<span>' + esc(server) + '<span class="hint">They enter this address during setup.</span></span>' }) +
            UI.insetRow({ label: 'Signup invite', value: '<span>Optional<span class="hint">Required only if the server requires an invite.</span></span>' }) +
            UI.insetRow({
              label: 'When they reply',
              value: '<span>Add their username as a Member, Admin, or Owner.</span>' +
                UI.btn('Add someone', { size: 'sm', attrs: 'data-act="call" data-fn="gSheet" data-arg="add"' }),
            }),
            { className: 'rows' }
          ) +
          '<p class="hint">Copying this message does not change the server.</p>';
      } else if (kind === 'add') {
        var vis = visibility == null ? 0 : visibility;
        body += UI.inset(textField('Username', '_r_2_', 'v.username', username)) +
          UI.sectionLabel('Role in ' + esc(store.name)) +
          UI.inset(UI.radioGroup(['Member', 'Admin', 'Owner'].map(function (next) {
            return UI.radioCard({
              selected: role === next, title: next,
              detail: next === 'Member'
                ? 'Opens items at or above its visibility band. Visibility 0 is the default.'
                : next === 'Admin'
                  ? 'Changes items and adds or removes people. Cannot change other Admins or the Owner.'
                  : 'Everything, including deleting the group.',
              attrs: 'data-act="set" data-key="v.role" data-val="' + next + '"',
            });
          }), { label: 'Role in ' + store.name })) +
          (role === 'Member'
            ? stepper('Visibility ' + vis, vis - 1, vis + 1, vis <= -32768, vis >= 32767) : '') +
          '<p class="fn">They read every item at or below their role as soon as this applies. ' +
          'No invitation is sent; they see ' + esc(store.name) + ' the next time their app checks.</p>';
      } else if (kind === 'demote') {
        var cards = [];
        if (currentRole && currentRole.kind === 'owner') {
          cards.push(UI.radioCard({
            selected: demoteRole === 'Admin', title: 'Admin',
            detail: 'Below Owner. Keeps roster management and loses Owner-only reads.',
            attrs: 'data-act="set" data-key="v.role" data-val="Admin"',
          }));
        }
        cards.push(UI.radioCard({
          selected: demoteRole === 'Member',
          disabled: maxVis < -32768,
          title: currentRole && currentRole.kind === 'member' ? 'Member · visibility ' + maxVis : 'Member',
          detail: currentRole && currentRole.kind === 'member'
            ? 'Same role, lower level; ' + maxVis + ' at most.'
            : 'Loses roster management and opens only what its visibility level admits.',
          attrs: 'data-act="set" data-key="v.role" data-val="Member"',
        }));
        cards.push(UI.radioCard({
          off: true, selected: false,
          title: (target ? esc(fmtRole(target.destination_role)) : 'Current role') + ' ' + UI.chip('current'),
          detail: 'Roles can only be lowered here. To raise a role, remove and re-add the person.',
        }));
        body += '<p>Roles can only be lowered from here. To raise one, remove the person and add them again at the higher role.</p>' +
          UI.inset(UI.radioGroup(cards, { label: 'New role' })) +
          (demoteRole === 'Member'
            ? stepper('Visibility ' + demoteVis, demoteVis - 1, demoteVis + 1,
                demoteVis <= -32768, demoteVis >= maxVis)
            : '');
      } else if (kind === 'remove') {
        body += '<p>Removing blocks future reads and rekeys the group. This user may retain a local copy of their current records.</p>';
        if (target && !canTarget(target)) {
          body += UI.notice({
            title: esc(D.partyName(target)) + ' cannot be removed here',
            body: 'This party does not have a unique username managed by this account. Remove it from the account or server that manages it.',
          });
        }
      } else if (kind === 'admit') {
        var avis = visibility == null ? 0 : visibility;
        body += UI.sectionLabel('Group') +
          UI.inset(remotes.length
            ? UI.radioGroup(remotes.map(function (group) {
                return UI.radioCard({
                  selected: !!remote && remote.id === group.id,
                  title: esc(group.alias), detail: 'on ' + esc(group.serverName),
                  attrs: 'data-act="set" data-key="v.remote" data-val="' + esc(group.alias) + '"',
                });
              }), { label: 'Group' })
            : UI.insetRow({ label: 'Group', value: '<span class="dim">No eligible remote group</span>' })) +
          UI.sectionLabel('Role for its members') +
          UI.inset(
            UI.insetRow({ label: 'Role', value: UI.chip('Member') + '<span class="dim">for every member</span>' }) +
            UI.insetRow({
              label: 'Visibility', value: String(avis),
              action: UI.btn('−', { size: 'sm', disabled: avis <= -32768, attrs: 'data-act="set" data-key="v.visibility" data-val="' + (avis - 1) + '"' }) +
                UI.btn('+', { size: 'sm', disabled: avis >= 32767, attrs: 'data-act="set" data-key="v.visibility" data-val="' + (avis + 1) + '"' }),
            })
          ) +
          '<p class="fn">' + esc(store.name) + ' keeps following that group’s roster. This can’t be taken back from here yet.</p>';
      } else if (kind === 'create') {
        body += UI.inset(textField('Name', '_r_5_', 'v.name', name)) +
          '<p class="fn">Others find it as <code>' + esc(teamAlias || '…') + '</code> on the server.</p>' +
          UI.sectionLabel('Server and account') +
          UI.inset(accounts.length
            ? UI.radioGroup(accounts.map(function (account) {
                var who = D.accounts.filter(function (c) { return c.store === account.id; })[0];
                return UI.radioCard({
                  selected: !!creationAccount && account.id === creationAccount.id,
                  title: esc(account.serverName),
                  detail: 'as ' + esc(who ? who.username : account.account),
                  attrs: 'data-act="set" data-key="v.account" data-val="' + esc(account.id) + '"',
                });
              }), { label: 'Server and account' })
            : UI.insetRow({ label: 'Account', value: '<span class="dim">No account can create a group.</span>' })) +
          UI.sectionLabel('Kind') +
          UI.inset(UI.radioGroup([
            UI.radioCard({
              selected: createKind === 'named', title: 'Named',
              detail: 'Has an alias on the server; people can be added and removed over time.',
              attrs: 'data-act="set" data-key="v.gkind" data-val=""',
            }),
            UI.radioCard({
              selected: createKind === 'adhoc', title: 'Ad-hoc',
              detail: 'Fixed membership, chosen now, no alias. For a one-off share.',
              attrs: 'data-act="set" data-key="v.gkind" data-val="adhoc"',
            }),
          ], { label: 'Kind' }));
      }

      var primary;
      if (kind === 'remove') {
        primary = UI.btn('Remove and rekey', {
          variant: 'primary', className: 'danger',
          disabled: !!failure || !target || !canTarget(target),
          attrs: 'data-act="call" data-fn="gApply" data-arg="remove:' + esc(SHORT[store.id] || store.id) +
            ':' + esc(target ? target.username : '') + '"',
        });
      } else {
        var label = kind === 'invite' ? 'Copy message'
          : kind === 'add' ? 'Add ' + (String(username).trim() || 'someone')
          : kind === 'demote' ? 'Change role'
          : kind === 'admit' ? 'Admit ' + (remote ? remote.alias : 'group')
          : 'Create ' + (String(name).trim() || 'group');
        var arg = kind === 'invite' ? 'invite'
          : kind === 'add' ? 'add:' + (SHORT[store.id] || store.id) + ':' + String(username).trim() + ':' + role + ':' + (visibility == null ? 0 : visibility)
          : kind === 'demote' ? 'demote:' + (SHORT[store.id] || store.id) + ':' + (target ? target.username : '') + ':' + demoteRole + ':' + demoteVis
          : kind === 'admit' ? 'admit:' + (SHORT[store.id] || store.id) + ':' + (remote ? remote.alias : '') + ':' + (visibility == null ? 0 : visibility)
          /* `createGroup` is re-resolved from the account radio at the moment
             of the write, so the token carries it; `|` because an account
             StoreRef has a colon in it. */
          : 'create:' + createdToken({ alias: teamAlias, name: String(name).trim(),
              kind: createKind, account: creationAccount ? creationAccount.id : 'acct:work' });
        primary = UI.btn(esc(label), {
          variant: 'primary',
          disabled: !!failure ||
            (kind === 'add' && !String(username).trim()) ||
            (kind === 'demote' && (!target || !canTarget(target))) ||
            (kind === 'admit' && !remote) ||
            (kind === 'create' && (!teamAlias || !creationAccount)),
          attrs: 'data-act="call" data-fn="gApply" data-arg="' + esc(arg) + '"',
        });
      }

      return sheetFrame({
        titleId: '_r_1_',
        danger: kind === 'remove',
        width: kind === 'invite' ? 'wide' : 'base',
        glyph: kind === 'create'
          ? '<span class="kico md group" style="background: ' + UI.rgb(D.hue(name || 'group')) + ';">' +
            esc((String(name).trim() || 'G').slice(0, 1)) + '</span>'
          : mark(store, 'md'),
        title: esc(title),
        subtitle: esc(subtitle),
        body: body,
        footer: UI.btn('Cancel', { attrs: 'data-act="set" data-key="v.sheet" data-val=""' }) + primary,
      });
    }

    /* settings-screen.tsx:989 — a different dialog from the group `invite`
       sheet: its own glyph, its own message, Done + Copy message. */
    function inviteDialog(s, w) {
      var ref = v(s, 'account', 'acct:work');
      var store = w.storeById[ref];
      var account = store ? D.accounts.filter(function (a) { return a.store === store.id; })[0] : undefined;
      var ok = !!(store && account);
      var message = ok
        ? '1. Install FOKS: https://foks.app/download\n' +
          '2. When it asks for a server address, type ' + store.serverName + '\n' +
          '3. Create your account with username firstname.lastname\n' +
          '4. Then tell ' + account.username + ' your username — there are no invite links, so that is what I add.'
        : '';
      var body = ok
        ? UI.inset(UI.insetRow({ label: 'Message', value: '<span class="msg">' + esc(message) + '</span>' })) +
          UI.sectionLabel('Message contents') +
          UI.inset(
            UI.insetRow({
              label: 'Install link',
              value: '<code>https://foks.app/download</code> ' + UI.chip('placeholder', { tone: 'warn' }) +
                '<small>Temporary download address.</small>',
            }) +
            UI.insetRow({ label: 'Server', value: esc(store.serverName) + '<small>They enter this address during setup.</small>' }) +
            UI.insetRow({ label: 'Signup invite', value: 'Optional<small>Required only if the server requires an invite.</small>' }) +
            UI.insetRow({ label: 'When they reply', value: 'Add their username to a group from that group’s settings.' }),
            { className: 'settings-inset' }
          )
        : '<p class="fn">This account is no longer in the catalog. Close this dialog and choose another account.</p>';
      return sheetFrame({
        titleId: '_r_4_',
        glyph: '<span class="kico md invite">I</span>',
        title: ok ? 'Invite to ' + esc(store.serverName) : 'That account is no longer available',
        subtitle: ok ? 'Help set up a new user' : 'Message unavailable',
        body: body,
        footer: UI.btn('Done', { attrs: 'data-act="set" data-key="v.sheet" data-val=""' }) +
          (ok ? UI.btn('Copy message', { variant: 'primary', attrs: 'data-act="toast" data-text="Message copied."' }) : ''),
      });
    }

    /* -------------------------------------------------------- the menus
       Both are portalled into #overlays in the app (`.menu-portal`, hidden by
       app.css until placeAnchoredMenu has measured the trigger). MenuButton
       defaults to align="end" with gap 4, so `M.placeMenuPortal` is run from
       after() with the trigger below: hard-coding the captures' offsets put
       the hero menu 40px past the frame's right edge, because `.frame` is
       min(1280px,100%) and the app window is the whole viewport. */
    var MENU_TRIGGER = {
      more: '#win .ghero .header-action .menuwrap button',
      rekey: '#win .group-settings .inset.danger .menuwrap button',
    };
    function placeMenu(s) {
      var open = v(s, 'menu', '');
      if (!open || !MENU_TRIGGER[open]) return;
      M.placeMenuPortal({ trigger: MENU_TRIGGER[open], width: false, align: 'end' });
    }
    function heroMenu(store) {
      return UI.menuPortal({
        menuLabel: 'Group actions',
        menuHtml:
          '<button type="button" role="menuitem" tabindex="-1" data-act="call" data-fn="gRefreshGroup">' +
          M.icon('again') + 'Refresh group</button>' +
          '<button type="button" role="menuitem" tabindex="-1" data-act="call" data-fn="gCopyGroupId">' +
          M.icon('copy') + 'Copy group ID</button>' +
          '<button type="button" role="menuitem" tabindex="-1" data-act="go" data-page="' +
          esc(M.storePages[store.id] || 'all') + '" data-set=\'{"v.menu":""}\'>' +
          M.icon('out') + 'Open in vault</button>',
      });
    }
    function rekeyMenu(w, store, manageable) {
      var removable = removableOf(store, manageable);
      var total = itemsOf(w, store.id).length;
      return UI.menuPortal({
        menuLabel: 'Removable members',
        menuHtml: removable.map(function (party) {
          return '<button type="button" role="menuitem" tabindex="-1"' +
            ' data-act="call" data-fn="gSheetTarget" data-arg="remove|' + esc(shortName(party)) + '">' +
            UI.avatar(party, { className: 'pav' }) +
            '<span class="t"><b>' + esc(shortName(party)) + '</b><small>' +
            esc(fmtRole(party.destination_role)) + ' · reads ' + readsOf(w, party).length + ' of ' + total +
            '</small></span></button>';
        }).join(''),
      });
    }

    /* ================================================== page assembly */

    var MINE = {};
    /* Set at the bottom of the file; every page of ours calls it from after()
       to re-claim M.fns.escape from whichever page rendered last. */
    var groupsAfter = function () {};
    /* after() is the core's last step, once the page and its overlays are in
       the DOM — so this is where the fixture goes back, before any other
       part's page can read it. */
    function pageAfter(s) { groupsAfter(); placeMenu(s || M.s); restoreFixture(); }

    function tabOf(s, page) { return v(s, 'tab', page.gtab); }

    function renderGroup(page, s) {
      var takeover = M.globalTakeover(s);
      if (takeover) return takeover;
      syncWorld(s, page.gref);
      var w = D.world(s);
      var store = w.storeById[page.gref];
      if (!store || store.kind !== 'team') return { main: UI.main(unavailableMain()) };

      var tab = tabOf(s, page);
      var unavailable = store.state !== 'normal' && store.state !== 'inactive';
      var inactive = store.active === false;
      var manageable = store.team_kind === 'named' && !inactive && !unavailable;
      var rosterManageable = manageable && !failureFor(store.id, 'roster');
      var federationManageable = manageable && !failureFor(store.id, 'federation');
      var rosterFailure = failureFor(store.id, 'roster');
      var federationFailure = failureFor(store.id, 'federation');

      var main = ghero(s, store);
      if (unavailable) {
        main += accessTakeover(store);
        return { main: UI.main(main) };
      }
      var peopleCount = D.partiesOf(store.id).length +
        D.federation.filter(function (e) { return e.store === store.id; }).length;
      main += UI.tabs({
        label: 'Group sections', value: tab,
        items: [
          Object.assign({ id: 'people', label: 'People' },
            (rosterFailure || federationFailure) ? {} : { count: peopleCount },
            { attrs: 'data-act="go" data-page="' + esc(page.gsiblings.people) + '" data-set=\'{"v.tab":"","v.panel":""}\'' }),
          { id: 'settings', label: 'Settings', attrs: 'data-act="go" data-page="' + esc(page.gsiblings.settings) + '" data-set=\'{"v.tab":"","v.panel":""}\'' },
        ],
      });

      var chosen = v(s, 'panel', '');
      var selected = (!rosterFailure && chosen)
        ? D.partiesOf(store.id).filter(function (p) { return shortName(p) === chosen; })[0] || null
        : null;

      var wrap = tab === 'people'
        ? peopleTab(s, w, store, selected, rosterManageable) + federationSection(s, w, store, federationManageable)
        : settingsTab(s, w, store, rosterManageable);
      if (tab === 'settings' || (!rosterFailure && !federationFailure)) {
        wrap += inspectToggle(s, w, store, tab);
      }
      main += '<div class="body' + (selected ? ' group-panel-open' : '') + '">' +
        '<div class="groups-wrap">' + wrap + '</div></div>';
      if (selected) main += partyPanel(w, store, selected, rosterManageable);
      return { main: UI.main(main) };
    }

    /* `App` returns the lock / boot screens INSTEAD of <VaultShell>, so
       GroupSettingsScreen — and every sheet it owns — is not mounted. The
       core skips overlay() under a takeover, so no guard is needed here. */
    function overlayGroup(page, s) {
      syncWorld(s, page.gref);
      var w = D.world(s);
      var store = w.storeById[page.gref];
      if (!store || store.kind !== 'team') return '';
      var menu = v(s, 'menu', '');
      /* groups-screen.tsx:1917-2014: only the hero's MenuButton sits outside
         the access branch. `<GroupSheet>` and the Settings tab (which owns
         the rekey MenuButton) are both inside the readable else-branch, so an
         unavailable group has neither. */
      var unavailable = store.state !== 'normal' && store.state !== 'inactive';
      var manageable = store.team_kind === 'named' && store.active !== false &&
        !unavailable && !failureFor(store.id, 'roster');
      var out = '';
      if (!unavailable) out += groupSheet(s, w, store);
      if (menu === 'more' && store.active !== false) out += heroMenu(store);
      /* MenuButton ignores a click while `disabled`, and `defaultOpen` is
         `defaultOpen && !disabled`: with nothing removable there is no menu. */
      if (menu === 'rekey' && !unavailable && tabOf(s, page) === 'settings' &&
        removableOf(store, manageable).length) out += rekeyMenu(w, store, manageable);
      return out;
    }

    function unavailableMain() {
      return UI.pageHeader({ title: 'Group unavailable', subtitle: '' }) +
        UI.body(UI.notice({
          title: 'This group is no longer available',
          body: 'Refresh the catalog or choose another group from the sidebar.',
        }));
    }

    /* ------------------------------------------------------- controls
       Chip labels are prose: the group label is the sentence's subject and
       the chip is only its value (`Sheet target` · `priya.n`). Sheet chips
       keep the GroupSheetKind ids because those are what a deep link needs.
       `clickText` searches inside #frame first, so a chip may now read like
       an in-frame control — but the `applied` chips still avoid the sheets'
       own button text so that neither reading is ambiguous. */
    function panelValues(ref) {
      return [{ v: '', label: 'None' }].concat((BASE.parties[ref] || []).map(function (p) {
        return { v: shortName(p), label: shortName(p) };
      }));
    }
    function targetValues(ref) {
      return [{ v: '', label: 'Default' }].concat((BASE.parties[ref] || [])
        .filter(function (p) { return D.actionableGroupMember(p); })
        .map(function (p) { return { v: shortName(p), label: shortName(p) }; }));
    }
    /* Two lists, because the two hosts mount different sheets: the group
       screen owns GroupSheetKind minus `create` (groups-screen.tsx:1673 never
       names it), and the Settings host owns `create` plus its own invite
       dialog (settings-screen.tsx:956-1056). Neither can show the other's. */
    var GROUP_SHEET_VALUES = [
      { v: '', label: 'None' },
      { v: 'add', label: 'add', hint: 'Add someone to <group> — username, role radio, visibility stepper.' },
      { v: 'demote', label: 'demote', hint: 'Lower priya.n’s role (an Admin target: Member + the disabled current card).' },
      { v: 'demote-owner', label: 'demote-owner', hint: 'The same sheet on sam.ortiz (an Owner target: an extra Admin option).' },
      { v: 'remove', label: 'remove', hint: 'Remove dana.okafor from <group>? — an alertdialog.' },
      { v: 'party-remove', label: 'party-remove', hint: 'The refusal: the target is forced locally_manageable:false, so the confirm is disabled behind the Notice.' },
      { v: 'admit', label: 'admit', hint: 'Add a group to <group> — the remote-group radio and the visibility row.' },
      { v: 'invite', label: 'invite', hint: 'Deep link only: GroupSheetKind includes it and a render test pins it, but nothing in the app opens it — map §7.1.' },
    ];
    var SETTINGS_SHEET_VALUES = [
      { v: '', label: 'None' },
      { v: 'create', label: 'create', hint: 'Create a group — named vs ad-hoc, the account radio, the live slug footnote.' },
      { v: 'join-invite', label: 'join-invite', hint: 'The Settings invite dialog (settings-screen.tsx:989) — different copy from the group invite sheet.' },
    ];
    var COMMON_CONTROLS = [
      { key: 'tab', label: 'Tab', note: 'GroupSettingsScreen tab state; null = the page’s own tab.', values: [{ v: '', label: 'Page default' }, { v: 'people', label: 'People' }, { v: 'settings', label: 'Group settings' }] },
      { key: 'sheet', label: 'Sheet', note: 'GroupSheet (groups-screen.tsx:1070). `create` is the Settings host’s, so it is not offered here.', values: GROUP_SHEET_VALUES },
      { key: 'menu', label: 'Menu', note: 'The two portalled menus: the hero “Group actions” menu and the Danger “Removable members” menu.', values: [{ v: '', label: 'None' }, { v: 'more', label: 'Group actions' }, { v: 'rekey', label: 'Removable members' }] },
      { key: 'role', label: 'Role draft', note: 'The add sheet’s role radio, and the demote sheet’s new role.', values: [{ v: '', label: 'Default' }, { v: 'Member', label: 'Member' }, { v: 'Admin', label: 'Admin' }, { v: 'Owner', label: 'Owner' }] },
      { key: 'visibility', label: 'Visibility draft', note: 'The visibility stepper (add / demote / admit). Bounds −32768…32767.', values: [{ v: '', label: 'Default' }, { v: '0', label: '0' }, { v: '-1', label: '-1' }, { v: '-2', label: '-2' }, { v: '1', label: '+1' }] },
      { key: 'remote', label: 'Remote group', note: 'The admit sheet’s candidate: team ∧ active ∧ named ∧ readable ∧ a different server from this group.', values: [{ v: '', label: 'First eligible' }, { v: 'household', label: 'household' }, { v: 'engineering', label: 'engineering' }] },
      { key: 'gkind', label: 'Create kind', note: 'The create sheet’s Named / Ad-hoc radio (ad-hoc sends name: ""; the Name field stays on screen either way).', values: [{ v: '', label: 'Named' }, { v: 'adhoc', label: 'Ad-hoc' }] },
      { key: 'name', label: 'Create name', note: 'The create sheet’s Name field; the slug footnote and the “Create X” label track it live. “(cleared)” is the field typed empty — the slug reads “…” and the primary “Create group”.', values: [{ v: '', label: 'Platform' }, { v: 'Ops', label: 'Ops' }, { v: 'Design squad', label: 'Design squad' }, { v: EMPTY, label: '(cleared)' }] },
      { key: 'username', label: 'Add username', note: 'The add sheet’s Username field; the primary reads “Add <username>” and is disabled when the field is “(cleared)”.', values: [{ v: '', label: 'jules.park' }, { v: 'nils', label: 'nils' }, { v: EMPTY, label: '(cleared)' }] },
      { key: 'account', label: 'Account', note: 'The create sheet’s server/account radio and the Settings invite dialog’s account (shared with 70-settings.js).', values: [{ v: '', label: 'Default' }, { v: 'acct:work', label: 'acct:work' }, { v: 'acct:personal', label: 'acct:personal' }] },
      {
        key: 'applied', label: 'Applied', note: 'The post-mutation world, replayed from tokens so the roster, the sidebar caption and the reader counts all move together. Several can be combined with commas.',
        values: [
          { v: '', label: 'Nothing' },
          { v: 'add:eng:jules.park:Member:0', label: 'Added jules.park' },
          { v: 'demote:eng:priya.n:Member:0', label: 'Lowered priya.n' },
          { v: 'remove:eng:dana.okafor', label: 'Removed dana.okafor' },
          { v: 'admit:eng:household:0', label: 'Admitted household' },
          { v: 'restore:eng:7c14a9f0', label: 'Restored 7c14a9f0' },
        ],
      },
      { key: 'adhoc', label: 'Homelab resumed', note: 'resumeGroupCreation landed: Homelab becomes an active ad-hoc group (info band, 0 people, no add affordances).', values: [{ v: '', label: 'No' }, { v: 'yes', label: 'Yes' }] },
      { key: 'failure', label: 'Detail failure', note: 'world.groupDetailFailures — CODE-DERIVED. No ?state= alias reaches these in the app (map §6, §7.3); the copy comes from groups-screen.tsx and vault-shell.render.test.tsx:3230.', values: [{ v: '', label: 'None' }, { v: 'roster', label: 'Roster (retryable)' }, { v: 'federation', label: 'Federation (final)' }, { v: 'both', label: 'Both' }] },
      { key: 'inspect', label: 'Inspect response', note: 'The disclosure under the tab body.', values: [{ v: '', label: 'Collapsed' }, { v: '1', label: 'Expanded' }] },
      {
        key: 'created', label: 'Created group',
        note: 'What createGroup made: alias|name|kind|accountStore (mock-bridge.ts:556). It puts the new group in the catalog, so every page here — and the sidebar — moves with it. An ad-hoc group sends no name, so it is known by its alias.',
        values: [
          { v: '', label: 'Not created' },
          { v: 'platform|Platform|named|acct:work', label: 'Platform · named · work' },
          { v: 'platform||adhoc|acct:work', label: 'platform · ad-hoc · work' },
          { v: 'ops|Ops|named|acct:personal', label: 'Ops · named · personal' },
        ],
      },
    ];
    function controlsFor(ref) {
      return [
        { key: 'panel', label: 'Party panel', note: 'The selected party — the aside and the .body.group-panel-open class.', values: panelValues(ref) },
        { key: 'target', label: 'Sheet target', note: 'Which party the demote / remove sheet acts on.', values: targetValues(ref) },
      ].concat(COMMON_CONTROLS);
    }

    function groupPages(o) {
      var siblings = { people: o.people, settings: o.settings };
      var made = [];
      [['people', o.people], ['settings', o.settings]].forEach(function (pair) {
        var page = M.page({
          id: pair[1],
          title: o.name + ' · ' + (pair[0] === 'people' ? 'People' : 'Settings'),
          path: ['Groups', o.name, pair[0] === 'people' ? 'People' : 'Settings'],
          nav: o.ref,
          note: o.note,
          controls: controlsFor(o.ref),
          render: function (s) { return renderGroup(this, s); },
          overlay: function (s) { return overlayGroup(this, s); },
          after: pageAfter,
        });
        page.gref = o.ref;
        page.gtab = pair[0];
        page.gsiblings = siblings;
        MINE[pair[1]] = true;
        made.push(page);
      });
      return made;
    }

    groupPages({
      ref: 'team:eng', name: 'Engineering', people: 'eng-people', settings: 'eng-settings',
      note: 'The 6-party roster with the inactive homelab admission; you are Admin, so every affordance is live.',
    });
    groupPages({
      ref: 'team:household', name: 'Household', people: 'household-people', settings: 'household-settings',
      note: 'A two-person group on foks.example.net: you are the Owner, so the Leave copy has no seniors and the federation table is the empty callout.',
    });
    groupPages({
      ref: 'team:homelab', name: 'Homelab', people: 'homelab-people', settings: 'homelab-settings',
      note: 'An inactive ad-hoc group: only the Setup incomplete band, no More menu, a disabled Remove…. Set “Homelab resumed” for the post-resume ad-hoc world.',
    });

    /* The group F1 makes. Before it has been created `world.storeById` has no
       such ref, so both pages answer with the app's own "Group unavailable"
       fallback; `ensureCreated` re-points `gref`/`nav` at whatever alias the
       create sheet actually sent (Ops, an ad-hoc `platform`, …). */
    PLATFORM_PAGES = groupPages({
      ref: 'team:platform', name: 'Platform', people: 'platform-people', settings: 'platform-settings',
      note: 'The group F1 creates, once it exists (?created=platform|Platform|named|acct:work). Without `created` this is the Group-unavailable fallback, which is what the app shows for a ref that is not in the catalog.',
    });

    M.page({
      id: 'group-unavailable',
      title: 'Group unavailable',
      path: ['Groups', 'Group unavailable'],
      /* shell/sidebar.tsx:159 — the row keeps `on` for a `group-settings`
         location as well as a `store` one on the same ref, so the capture's
         highlighted row is Personal, the ref the location names. */
      nav: 'acct:personal',
      note: 'groups-screen.tsx:1815 — location.ref is not a team store (?state=group-settings&store=acct:personal).',
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        syncWorld(s, null);
        return { main: UI.main(unavailableMain()) };
      },
      after: pageAfter,
    });
    MINE['group-unavailable'] = true;

    /* ------------------------------------------------- Settings › Groups
       settings-screen.tsx:95. The page furniture (header, .settings-cols,
       nav.settings-sections, .settings-main) comes from M.settingsFrame in
       70-settings.js so every Settings pane is identical. */
    function groupsSection(s, w) {
      var accounts = w.stores.filter(function (st) { return st.kind === 'account'; });
      var canCreate = accounts.some(function (st) { return canCreateInStore(w, st.id); });
      var groups = w.stores.filter(function (st) { return st.kind === 'team'; });
      /* settings-screen.tsx:110 — not normal, or carrying a detail failure;
         `world.failure` is the same answer, already derived. */
      var attention = groups.filter(function (st) {
        return st.state !== 'normal' || !!st.failure;
      });
      var out = UI.sectionLabel('Create a group') +
        UI.inset(UI.insetRow({
          label: 'New group',
          action: UI.btn('Create group…', {
            size: 'sm', icon: 'plus', variant: 'primary', disabled: !canCreate,
            title: canCreate ? undefined : 'No available account can create a group',
            attrs: 'data-act="call" data-fn="gSheet" data-arg="create"',
          }),
        }), { className: 'settings-inset middle' });
      if (!canCreate) {
        out += '<p class="fn">No available account can create a group right now. Add an account, or restore access to a server.</p>';
      }
      if (attention.length) {
        out += UI.sectionLabel('Needs attention') +
          UI.inset(attention.map(function (store) {
            return UI.insetRow({
              valueClass: 'group-attention',
              value: '<span class="who2">' + UI.stack(D.partiesOf(store.id), { size: 'lg' }) +
                /* settings-screen.tsx:152 — `storeDescription` decides this,
                   which M.data.world has already answered as `description`. */
                '<span class="t"><b>' + esc(store.name) + '</b><small>' +
                esc(store.description) + ' · ' + esc(store.serverName) + '</small></span></span>' +
                UI.chip(store.state === 'inactive' ? 'Inactive' : 'Unavailable', { tone: 'warn' }),
              action: UI.btn('Open', {
                size: 'sm',
                attrs: 'data-act="go" data-page="' + esc(M.storePages[store.id] || 'all') + '"',
              }),
            });
          }).join(''), { className: 'settings-inset middle' });
      }
      out += UI.sectionLabel('Invite someone') +
        UI.inset(accounts.length
          ? accounts.map(function (store) {
              var account = D.accounts.filter(function (a) { return a.store === store.id; })[0];
              return UI.insetRow({
                label: esc(store.serverName),
                action: UI.btn(account ? 'Invite as ' + esc(account.username) + '…' : 'Invite someone…', {
                  size: 'sm', disabled: !account,
                  attrs: 'data-act="call" data-fn="gInvite" data-arg="' + esc(store.id) + '"',
                }),
              });
            }).join('')
          : UI.insetRow({ label: 'None', value: 'No account on this Mac yet.' }),
          { className: 'settings-inset' });
      return out;
    }

    M.page({
      id: 'set-groups',
      title: 'Groups',
      path: ['Settings', 'Groups'],
      nav: 'settings',
      note: 'settings-screen.tsx:95 — Create a group, Needs attention (only when non-empty) and Invite someone, inside the Settings columns.',
      controls: [
        { key: 'sheet', label: 'Sheet', note: 'The two sheets settings-screen.tsx mounts: the create sheet and the invite dialog.', values: SETTINGS_SHEET_VALUES },
        COMMON_CONTROLS.filter(function (c) { return c.key === 'account'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'gkind'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'name'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'applied'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'adhoc'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'failure'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'created'; })[0],
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        syncWorld(s, 'team:eng');
        var w = D.world(s);
        return { main: M.settingsFrame(s, 'groups', groupsSection(s, w)) };
      },
      overlay: function (s) {
        syncWorld(s, 'team:eng');
        var w = D.world(s);
        var sheet = v(s, 'sheet', '');
        if (sheet === 'join-invite') return inviteDialog(s, w);
        /* settings-screen.tsx:956 mounts GroupSheet with sheet="create" and
           `stores[0]` behind it; no other kind is reachable from here. */
        return groupSheet(s, w, w.storeById['team:eng'], SETTINGS_SHEET_KIND);
      },
      after: pageAfter,
    });
    MINE['set-groups'] = true;

    /* ------------------------------------------------- the created vault
       F1 lands here. 40-items.js owns the vault screen; if it publishes a
       renderer (M.renderStore) this page defers to it, otherwise it draws the
       empty vault itself from the create-applied capture. */
    M.page({
      id: 'store-platform',
      title: 'Platform',
      path: ['Vault', 'Platform'],
      nav: 'team:platform',
      note: 'The group F1 creates: one party (you, Owner) and no items. `created` carries its identity — alias|name|kind|account — so the ad-hoc branch (F1b) lands here as the lower-case “platform”.',
      controls: [
        COMMON_CONTROLS.filter(function (c) { return c.key === 'created'; })[0],
        COMMON_CONTROLS.filter(function (c) { return c.key === 'applied'; })[0],
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        /* This page IS the created group, so the key is never unset here, and
           a short form (`?created=platform`, which is all a flow step needs)
           is folded to the canonical alias|name|kind|account token so the URL,
           the deck chip and the render all agree. */
        var canonical = createdToken(createdOf(v(s, 'created', '')));
        if (v(s, 'created', '') !== canonical) {
          M.set('v.created', canonical, { silent: true });
        }
        syncWorld(s, null);
        var ref = 'team:' + createdOf(v(s, 'created', '')).alias;
        if (typeof M.renderStore === 'function') return M.renderStore(s, ref);
        var w = D.world(s);
        var store = w.storeById[ref];
        var toolbar = UI.toolbar(
          '<span class="menuwrap">' + UI.btn('New' + M.icon('chev', null, { cls: 'chevron' }), {
            variant: 'primary', attrs: 'aria-haspopup="menu" aria-expanded="false"',
          }) + '</span>' +
          UI.segmented({
            label: 'Which kinds to list', value: 'All',
            items: [{ id: 'All', label: 'All' }].concat(D.KIND_LIST.map(function (k) {
              return { id: k, label: D.KINDS[k].plural, title: D.KINDS[k].blurb };
            })),
          }) + UI.spacer() +
          '<span class="menuwrap">' + UI.btn('Sort by name' + M.icon('chev', null, { cls: 'chevron' }), {
            attrs: 'aria-haspopup="menu" aria-expanded="false"',
          }) + '</span>' +
          UI.segmented({
            label: 'How to show the items', variant: 'icon', value: 'list',
            items: [{ id: 'list', icon: 'list', title: 'List' }, { id: 'grid', icon: 'grid', title: 'Cards' }],
          }) +
          UI.btn('', { variant: 'quiet', icon: 'info', title: 'Details', ariaLabel: 'Details', on: false }) +
          UI.btn('', {
            variant: 'quiet', icon: 'gear', title: 'Group settings', ariaLabel: 'Group settings',
            /* items-screen's toolbar gear: {group-settings, ref, tab:'people'}
               for THIS vault — the created group's own page, not Engineering's. */
            attrs: 'data-act="go" data-page="platform-people"',
          })
        );
        return {
          main: UI.main(
            UI.pageHeader({
              title: store.name, subtitle: store.heading,
              action: UI.stack(D.partiesOf(ref)),
              search: { value: '', bind: 'v.search' },
            }) + toolbar +
            UI.body(UI.emptyState({
              kind: 'Password', title: 'No items here',
              action: '<span class="menuwrap">' + UI.btn('New' + M.icon('chev', null, { cls: 'chevron' }), {
                variant: 'primary', attrs: 'aria-haspopup="menu" aria-expanded="false"',
              }) + '</span>',
            }))
          ),
        };
      },
      after: pageAfter,
    });
    MINE['store-platform'] = true;

    /* ================================================== the handlers */

    function currentStoreRef() {
      var page = M.pages[M.s.page];
      return (page && page.gref) || 'team:eng';
    }
    function currentStore() {
      syncWorld(M.s, currentStoreRef());
      return D.world(M.s).storeById[currentStoreRef()];
    }

    M.fns.gTogglePanel = function (name) {
      M.set('v.panel', v(M.s, 'panel', '') === name ? '' : name);
    };
    /* The row's `…` always selects; it never toggles the panel off. */
    M.fns.gOpenPanel = function (name) { M.set('v.panel', name); };
    /* A GroupSheet is mounted fresh, so every draft starts at its default. */
    var DRAFTS = { 'v.target': null, 'v.menu': null, 'v.role': null, 'v.visibility': null,
      'v.remote': null, 'v.name': null, 'v.gkind': null, 'v.username': null };
    M.fns.gSheet = function (kind) {
      M.patch(Object.assign({}, DRAFTS, { 'v.sheet': kind }));
      M.render();
    };
    M.fns.gSheetTarget = function (arg) {
      var bits = String(arg).split('|');
      M.patch(Object.assign({}, DRAFTS, { 'v.sheet': bits[0], 'v.target': bits[1] }));
      M.render();
    };
    M.fns.gInvite = function (ref) {
      M.patch({ 'v.sheet': 'join-invite', 'v.account': ref });
      M.render();
    };
    /* The panel carries data-act="stop", so this only ever fires for a click
       on the backdrop itself. */
    M.fns.gBackdrop = function () { M.set('v.sheet', ''); };
    M.fns.gRefreshGroup = function () {
      M.set('v.menu', '');
      M.toast('Group refreshed');
    };
    M.fns.gCopyGroupId = function () {
      M.set('v.menu', '');
      M.toast('Group ID copied.');
    };
    M.fns.gFinishSetup = function () {
      M.set('v.adhoc', 'yes');
      M.toast('Group creation resumed');
    };
    M.fns.gRestore = function (operationId) {
      pushOp('restore:' + (SHORT[currentStoreRef()] || currentStoreRef()) + ':' + operationId);
      M.render();
      M.toast('Group access restored');
    };
    /* GroupSheet.apply (groups-screen.tsx:1191): the write, then
       onApplied(`${title} completed`), then the sheet closes. */
    M.fns.gApply = function (token) {
      var store = currentStore();
      var bits = String(token).split(':');
      var message;
      if (bits[0] === 'invite') {
        M.set('v.sheet', '');
        M.toast('Message copied.');
        return;
      }
      if (String(token).indexOf('create:') === 0) {
        var made = createdOf(String(token).slice(7));
        ensureCreated(made);
        M.patch({ 'v.sheet': null, 'v.created': createdToken(made) });
        /* settings-screen's create host: refresh the world, then
           navigate {kind:'store', ref} into the new vault. */
        M.go('store-platform');
        M.toast('Create a group completed');
        return;
      }
      if (bits[0] === 'add') message = 'Add someone to ' + store.name + ' completed';
      else if (bits[0] === 'demote') message = 'Lower ' + bits[2] + '’s role completed';
      else if (bits[0] === 'remove') message = 'Remove ' + bits[2] + ' from ' + store.name + '? completed';
      else if (bits[0] === 'admit') message = 'Add a group to ' + store.name + ' completed';
      pushOp(token);
      M.patch({ 'v.sheet': null, 'v.target': null, 'v.role': null, 'v.visibility': null, 'v.panel': null });
      M.render();
      M.toast(message);
    };

    /* The shell's rule, unchanged — which IS app-root's rule here. Escape
       closes a sheet (SheetDialog handles it and calls preventDefault) and a
       menu (MenuButton does the same); app-root's own handler then clears the
       search and drops the item selection, neither of which this screen has.
       It does NOT close the party panel: `selected` is GroupSettingsScreen
       state that nothing listens for Escape on (verified against the app —
       the `aside.details` survives an Escape). This only has to be re-claimed
       from after(), because the vault pages install a rule of their own the
       same way and it would otherwise still be installed when we render. */
    var baseEscape = M.fns.escape;
    function groupsEscape() { return baseEscape(); }
    M.fns.escape = groupsEscape;
    groupsAfter = function () { M.fns.escape = groupsEscape; };
  })();
