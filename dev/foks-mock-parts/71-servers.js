/* --------------------------------------------------------- 71-servers.js
     Settings › Servers — `foks-ui/src/screens/servers-screen.tsx`.

     The section is a list of every server this Mac knows, then one server at a
     time. Nothing here is a page of its own in the app: it is drawn inside the
     Settings two-column frame, so every page below renders
     `M.settingsFrame(s, 'servers', body)` (70-settings.js).

       srv-list     the list, with `Needs attention` hoisting
       srv-server   one server: four labelled blocks + the response inspector
       srv-add      the Add sheet over the list
       srv-forget   the Forget alertdialog over the detail
       srv-reset    the Reset alertdialog over the detail

     `resolveServerUiState` (servers-screen.tsx:110) is transcribed literally
     below and is the only thing that decides what a server looks like; the
     `status` view knob works by handing it a different server/snapshot pair,
     never by short-circuiting it. */
(function () {
  var UI = M.ui,
    esc = M.esc,
    h = M.h,
    D = M.data;

  /* ------------------------------------------------------------- clock
       The mock bridge answers `describe_server_status` with now+6d for
       `personal`, now−3d for `acme` and null for an unpinned host
       (mock-bridge.ts:858). Those are frozen here so the mock's dates are the
       captures' dates whenever it is opened:
         NOW    Sep 3, 2026, 9:37 PM
         FUTURE Sep 9, 2026, 9:37 PM   (long) / Sep 9, 9:37 PM   (short)
         PAST   Aug 31, 2026, 9:37 PM  (long) / Aug 31, 9:37 PM  (short) */
  var NOW = 1788496631;
  var FUTURE = 1789015031;
  var PAST = 1788237432;

  /* servers-screen.tsx:52 — the long form, for bands and the detail. */
  function expires(value) {
    if (value === null) return 'No signed expiry is available';
    var date = new Date(value * 1000);
    return isFinite(date.getTime())
      ? new Intl.DateTimeFormat(undefined, {
          dateStyle: 'medium',
          timeStyle: 'short',
        }).format(date)
      : 'Unix time ' + value + ' seconds';
  }
  /* servers-screen.tsx:63 — the row's compact form; the year is noise. */
  function expiresShort(value) {
    if (value === null) return 'no signed expiry';
    var date = new Date(value * 1000);
    return isFinite(date.getTime())
      ? new Intl.DateTimeFormat(undefined, {
          month: 'short',
          day: 'numeric',
          hour: 'numeric',
          minute: '2-digit',
        }).format(date)
      : 'Unix time ' + value;
  }
  function acceptanceText(value) {
    return value === 'inserted'
      ? 'New pin'
      : value === 'advanced'
        ? 'Pinned history advanced safely'
        : 'Pinned history unchanged';
  }

  /* model/lease.ts:36-53, with the frozen clock. */
  function signedLeaseState(expiresAt) {
    if (expiresAt === null || expiresAt === undefined) return 'unavailable';
    return expiresAt > NOW ? 'fresh' : 'lapsed';
  }
  function serverLeaseState(status) {
    if (!status) return 'unavailable';
    return status.leaseRequired
      ? signedLeaseState(status.leaseExpiresAt)
      : 'fresh';
  }

  /* --------------------------------------------------- the six ui states */
  var STATE_LABEL = {
    checked: 'Checked',
    unprobed: 'Never checked',
    lapsed: 'Check-in expired',
    unavailable: 'Status unknown',
    blocked: 'History changed',
    pending: 'Reading status',
  };
  function isLocked(state) {
    return state === 'lapsed' || state === 'unavailable' || state === 'blocked';
  }
  function markTone(state) {
    return state === 'checked' ? 'ok' : isLocked(state) ? 'bad' : '';
  }
  /* servers-screen.tsx:110, verbatim — the order of these tests is the
       whole behaviour, so it is transcribed rather than rewritten. */
  function resolveServerUiState(
    server,
    snapshot,
    statusFailed,
    fixtureLapsed,
    rollback,
  ) {
    if (rollback || server.state === 'blocked') return 'blocked';
    var lease = serverLeaseState(snapshot);
    if (fixtureLapsed || server.state === 'lease-lapsed' || lease === 'lapsed')
      return 'lapsed';
    if (statusFailed) return 'unavailable';
    if (snapshot) {
      if (snapshot.host && lease === 'fresh') return 'checked';
      if (!snapshot.host) return 'unprobed';
      if (lease === 'unavailable') return 'unavailable';
    }
    if (server.state === 'lease-unavailable') return 'unavailable';
    if (server.state === 'never-probed') return 'unprobed';
    return 'pending';
  }
  function statusChip(state) {
    return UI.chip(esc(STATE_LABEL[state]), {
      tone:
        state === 'checked'
          ? 'ok'
          : state === 'unprobed' || state === 'pending'
            ? 'default'
            : 'bad',
    });
  }
  /* The mark with the dot that carries the state's colour; a locked server
       swaps the server glyph for `alert` and drops the dot. */
  function serverMark(state) {
    var locked = isLocked(state);
    var cls = ['smark', markTone(state)].filter(Boolean).join(' ');
    return (
      '<span class="' +
      cls +
      '">' +
      M.icon(locked ? 'alert' : 'server') +
      (locked ? '' : '<i class="dot"></i>') +
      '</span>'
    );
  }

  /* ------------------------------------------------------- host records
       PINS is the mock bridge's `serverHosts` map: what an explicit check
       pinned this session. CHECKS is the last `check_server` report per
       profile — its presence is what makes the Check-in row read "Checked
       just now." rather than "Identity pinned on this Mac.". */
  var PINS = {};
  var CHECKS = {};

  function fixtureHost(srv) {
    if (!srv.host_id) return null;
    return {
      lookupName: srv.name,
      canonicalName: srv.name,
      hostId: D.hostIds[srv.id] || '02' + new Array(65).join('7'),
      chain: srv.chain,
      epoch: srv.epoch,
    };
  }
  /* mock-bridge.ts:198 — `serverHosts` is seeded from every fixture row that
       carries a pinned host (host_id + chain + epoch), and a check adds one.
       The catalog's `Server.state` has no say in it. */
  function pinnedHost(srv) {
    if (PINS[srv.id]) return PINS[srv.id];
    return srv.host_id && srv.chain !== null && srv.epoch !== null
      ? fixtureHost(srv)
      : null;
  }
  /* The `status` knob's synthetic world instead: "never probed" there means
       nothing is pinned, whatever the fixture row still carries. */
  function synthHost(srv) {
    if (PINS[srv.id]) return PINS[srv.id];
    return srv.state === 'never-probed' ? null : fixtureHost(srv);
  }
  /* mock-bridge.ts:874 — the host a first check would pin. */
  function firstCheckHost(srv) {
    return {
      lookupName: srv.name,
      canonicalName: srv.name,
      hostId: D.hostIds[srv.id] || '02' + new Array(65).join('7'),
      chain: 4,
      epoch: 118204,
    };
  }
  /* describe_server_status, exactly as mock-bridge.ts:857 answers it: the
       expiry is keyed off the PROFILE, not off the catalog's Server.state —
       personal is always now+6d, foks.acme-corp.com always now−3d, anything
       else now+6d once something is pinned and null before that.

       That is why the Servers pane shows foks.acme-corp.com as
       `Check-in expired` in every reachable scene, including the fresh-lease
       world: `resolveServerUiState` tests the signed lease before it looks at
       `server.state`, so the app-wide `acme` key cannot make this pane read
       healthy. Verified against ?state=servers, ?state=servers-list and
       ?state=settings&section=servers&profile=acme with lease=fresh — all
       four show `Needs attention` + `Ready`. */
  function bridgeSnapshot(srv) {
    if (srv.state === 'blocked') return undefined; /* the probe loop skips it */
    var host = pinnedHost(srv);
    return {
      profile: srv.id,
      configuredProbe: srv.name,
      host: host,
      leaseRequired: true,
      leaseExpiresAt:
        srv.id === 'personal'
          ? FUTURE
          : srv.id === 'acme'
            ? PAST
            : host
              ? FUTURE
              : null,
    };
  }
  /* The `status` knob's synthetic answer: the expiry follows the state the
       chip asked for, so every ServerUiState is reachable on every profile. */
  function synthSnapshot(srv) {
    if (srv.state === 'blocked') return undefined;
    var host = synthHost(srv);
    var exp = !host
      ? null
      : srv.state === 'lease-lapsed'
        ? PAST
        : srv.state === 'lease-unavailable'
          ? null
          : FUTURE;
    return {
      profile: srv.id,
      configuredProbe: srv.name,
      host: host,
      leaseRequired: true,
      leaseExpiresAt: exp,
    };
  }

  /* ------------------------------------------------------- the `status` knob
       Every value works by handing resolveServerUiState a different
       server/snapshot pair. `''` and `ok` mean "whatever the world and the
       signed status say" — i.e. exactly what the app renders; every other
       value replaces the passive answer with one that produces that state.
       The app-wide `acme` key is deliberately NOT consulted here: it moves
       the catalog's Server.state, and the signed lease that this pane reads
       first is fixed per profile (see bridgeSnapshot), so acme reads
       `Check-in expired` under every one of its five values — as it does in
       the app. Use `status=` to see foks.acme-corp.com in another state. */
  function view(s, srv) {
    var knob = s.v.status || '';
    var auto = knob === '' || knob === 'ok';
    var server = {
      id: srv.id,
      name: srv.name,
      label: srv.label,
      host_id: srv.host_id,
      chain: srv.chain,
      epoch: srv.epoch,
      accounts: srv.accounts,
      state: srv.state,
    };
    var rollback = knob === 'rollback';
    var statusFailed = knob === 'unavailable';
    var pending = knob === 'pending';
    if (knob === 'checked') server.state = 'ok';
    else if (knob === 'unprobed') server.state = 'never-probed';
    else if (knob === 'lapsed') server.state = 'lease-lapsed';
    else if (knob === 'checkin-unavailable') server.state = 'lease-unavailable';
    else if (knob === 'blocked') server.state = 'blocked';
    else if (pending) server.state = 'ok';

    var snapshot = auto
      ? bridgeSnapshot(server)
      : statusFailed || pending
        ? undefined
        : synthSnapshot(server);
    /* "checked" has to have something pinned even for a server the fixture
         never probed, so it borrows the host a first check would write. */
    if (knob === 'checked' && snapshot && !snapshot.host) {
      snapshot = {
        profile: server.id,
        configuredProbe: server.name,
        host: firstCheckHost(server),
        leaseRequired: true,
        leaseExpiresAt: FUTURE,
      };
    }
    var state = resolveServerUiState(
      server,
      snapshot,
      statusFailed,
      false,
      rollback,
    );
    /* `applied=yes` is the deep-link form of "an explicit check has run": it
         stands in for the report the live Check button would have stored. It
         is computed for this render only — writing it into PINS/CHECKS would
         make the knob one-way, and stepping a flow backwards would find a
         server the fixture says was never probed already pinned. */
    var checked = CHECKS[server.id];
    if (
      applied(s, 'yes') &&
      !checked &&
      state !== 'lapsed' &&
      state !== 'blocked'
    ) {
      var already = fixtureHost(server);
      var pinned =
        (snapshot && snapshot.host) || already || firstCheckHost(server);
      checked = mergeCheck(
        server.id,
        already ? 'unchanged' : 'inserted',
        pinned,
      );
      snapshot = {
        profile: server.id,
        configuredProbe: server.name,
        host: pinned,
        leaseRequired: true,
        leaseExpiresAt: FUTURE,
      };
      state = resolveServerUiState(server, snapshot, false, false, rollback);
    }
    return {
      server: server,
      snapshot: snapshot,
      statusFailed: statusFailed,
      rollback: rollback,
      state: state,
      checked: checked,
      /* servers-screen.tsx:345 — the checked report outranks the passive host. */
      host: checked || (snapshot && snapshot.host) || null,
      expiry:
        snapshot && snapshot.leaseExpiresAt !== undefined
          ? snapshot.leaseExpiresAt
          : null,
    };
  }
  function mergeCheck(id, acceptance, host) {
    return {
      profile: id,
      acceptance: acceptance,
      lookupName: host.lookupName,
      canonicalName: host.canonicalName,
      hostId: host.hostId,
      chain: host.chain,
      epoch: host.epoch,
    };
  }

  /* Row state on the list: the world and the signed status decide, with two
       knobs. `unread` is the list's first paint — `statuses` is still the empty
       Map the screen starts with (servers-screen.tsx:176), so every row
       resolves without a snapshot: foks.example.net and foks.acme-corp.com
       answer `pending` ("Reading status · Waiting for a signed check-in"),
       partner is `never-probed` in the catalog, and so is acme once the `acme`
       key has moved it (only the catalog can speak before the answers come).
       That is the only way the pending ROW is reachable — map §3.5. */
  function rowView(s, srv, unread) {
    var snapshot = unread ? undefined : bridgeSnapshot(srv);
    var state = resolveServerUiState(srv, snapshot, false, false, false);
    /* `applied=yes` is the deep-link form of "the Check button has already
         run", and the app's check stores the fresh describe_server_status it
         triggers in `statuses` — so a never-checked ROW reads `Checked` from
         then on, which is exactly what clicking Check here does through PINS.
         Only before the first answers arrive can no report exist, so `unread`
         wins; a locked server refuses the check and keeps its state. */
    if (
      !unread &&
      applied(s, 'yes') &&
      !CHECKS[srv.id] &&
      state !== 'checked' &&
      state !== 'lapsed' &&
      state !== 'blocked'
    ) {
      snapshot = {
        profile: srv.id,
        configuredProbe: srv.name,
        host:
          (snapshot && snapshot.host) ||
          fixtureHost(srv) ||
          firstCheckHost(srv),
        leaseRequired: true,
        leaseExpiresAt: FUTURE,
      };
      state = resolveServerUiState(srv, snapshot, false, false, false);
    }
    return {
      server: srv,
      state: state,
      expiry:
        snapshot && snapshot.leaseExpiresAt !== undefined
          ? snapshot.leaseExpiresAt
          : null,
    };
  }

  /* `busy` in the screen is the check's own in-flight flag; the mock has no
       in-flight, so the app-wide `refreshing` key stands in for it — that is
       what makes every Check button's disabled state reachable. */
  function busyOf(s) {
    return s.refreshing === 'yes';
  }

  /* ------------------------------------------------- the added server
       `applied=added` is the deep-link form of "Add server has run", so the
       end of the add flow is reachable cold. It is replayed onto a pristine
       copy of the fixture on every render — nothing is pushed into M.data —
       so reloading or stepping back never stacks rows. ADDED names what the
       Add sheet last created; before that it is the sheet's second pair of
       values, which is what the deck's chips and the flow use. */
  var ADDED_DEFAULT = { id: 'lab', name: 'foks.lab.test' };
  var ADDED = { id: ADDED_DEFAULT.id, name: ADDED_DEFAULT.name };
  /* A forgotten server is dropped from THIS SECTION's list only. The fixture
       is shared with every other page, and `list_servers` is the only thing
       Servers reads it through, so splicing M.data.servers would take the
       Personal vault, its caption and 10 of All items' rows with it — a world
       no scene of the app can show. Leaving the section clears it, which is
       also what the app does: the reload the forget triggers puts the server
       back (map §8.2). */
  var FORGOTTEN = {};
  /* `applied` carries tokens, comma-separated the way 60-groups.js does it,
       because the add flow's last step is both: the server exists AND it has
       just been checked (`added,yes`). */
  function applied(s, token) {
    return (s.v.applied || '').split(',').indexOf(token) >= 0;
  }
  function withApplied(s, token) {
    var have = (s.v.applied || '').split(',').filter(Boolean);
    if (have.indexOf(token) < 0) have.push(token);
    return have.join(',') || null;
  }
  function serversOf(s) {
    var list = M.data.world(s).servers.filter(function (srv) {
      return !FORGOTTEN[srv.id];
    });
    if (!applied(s, 'added')) return list;
    for (var i = 0; i < list.length; i++)
      if (list[i].id === ADDED.id) return list;
    return list.concat([
      {
        id: ADDED.id,
        name: ADDED.name,
        label: null,
        host_id: null,
        chain: null,
        epoch: null,
        lease: null,
        accounts: [],
        state: 'never-probed',
      },
    ]);
  }
  function serverFor(s, id) {
    var list = serversOf(s);
    for (var i = 0; i < list.length; i++) if (list[i].id === id) return list[i];
    return undefined;
  }
  function selectedId(s) {
    return s.v.profile || 'personal';
  }

  /* The Settings two-column frame, from 70-settings.js: the `.path` header,
       `nav.settings-sections` with Servers lit, and `.settings-main` around
       the body. Every Settings page in the mock renders through it, so the
       chrome is identical everywhere. */
  function frame(s, body) {
    return M.settingsFrame(s, 'servers', body);
  }

  /* ============================================================ the list */

  /* servers-screen.tsx:492 — the state in bold, then the one fact that
       matters. The separators are `<span class="sep">·</span>`. */
  function statusLine(state, account, groups, expiry) {
    var sep = '<span class="sep">·</span>';
    if (state === 'checked')
      return (
        esc(account ? account.username : 'no account') +
        sep +
        esc(groups.length ? D.plural(groups.length, 'group') : 'no groups') +
        sep +
        'check-in until ' +
        esc(expiresShort(expiry))
      );
    if (state === 'pending')
      return '<b>Reading status</b>' + sep + 'Waiting for a signed check-in';
    if (state === 'unprobed')
      return '<b>Never checked</b>' + sep + 'Check it before use';
    if (state === 'lapsed')
      return (
        '<b>Check-in expired</b>' +
        sep +
        'Locked since ' +
        esc(expiresShort(expiry))
      );
    if (state === 'unavailable')
      return (
        '<b>Check-in status unknown</b>' +
        sep +
        'Locked until the agent gets one'
      );
    return (
      '<b>History changed</b>' +
      sep +
      'Locked. The server no longer matches what this Mac pinned'
    );
  }

  function serverRow(s, row) {
    var w = M.data.world(s);
    var srv = row.server;
    var account = null;
    D.accounts.forEach(function (a) {
      if (!account && (a.server === srv.id || a.server === srv.name))
        account = a;
    });
    var groups = w.stores
      .filter(function (st) {
        return st.kind === 'team' && st.server === srv.id;
      })
      .map(function (st) {
        return st.name;
      });
    return UI.insetRow({
      className: ['srow', isLocked(row.state) ? 'crit' : '']
        .filter(Boolean)
        .join(' '),
      valueClass: 'srv',
      value:
        serverMark(row.state) +
        '<span class="t"><b><span>' +
        esc(srv.name) +
        '</span>' +
        (srv.label ? '<em>' + esc(srv.label) + '</em>' : '') +
        '</b>' +
        '<small>' +
        statusLine(row.state, account, groups, row.expiry) +
        '</small></span>',
      action:
        statusChip(row.state) +
        (row.state === 'unprobed'
          ? UI.btn('Check', {
              size: 'sm',
              variant: 'primary',
              icon: 'again',
              disabled: busyOf(s),
              attrs:
                'data-act="call" data-fn="srvCheck" data-arg="' +
                esc(srv.id) +
                '"',
            })
          : '') +
        UI.btn('Open', {
          size: 'sm',
          attrs:
            'data-act="call" data-fn="srvOpen" data-arg="' + esc(srv.id) + '"',
        }),
    });
  }

  function listBody(s) {
    var unread = s.v.status === 'pending';
    var rows = serversOf(s).map(function (srv) {
      return rowView(s, srv, unread);
    });
    if (s.v.empty === 'yes') rows = [];
    /* `empty=locked` is the other end of the same branch: with nothing
         ready, `Add a server…` moves onto the Needs attention label
         (servers-screen.tsx:632). Unreachable with the fixture — map §8.1. */
    else if (s.v.empty === 'locked')
      rows = rows.filter(function (r) {
        return isLocked(r.state);
      });
    var attention = rows.filter(function (r) {
      return isLocked(r.state);
    });
    var ready = rows.filter(function (r) {
      return !isLocked(r.state);
    });
    function box(entries) {
      return UI.inset(
        entries
          .map(function (r) {
            return serverRow(s, r);
          })
          .join(''),
        { className: 'settings-inset middle' },
      );
    }
    var add =
      '<span class="right">' +
      UI.btn('Add a server…', {
        size: 'sm',
        icon: 'plus',
        attrs: 'data-act="call" data-fn="srvOpenAdd"',
      }) +
      '</span>';
    return h(
      attention.length
        ? UI.sectionLabel('Needs attention', {
            className: 'danger-title',
            action: ready.length ? undefined : add,
          }) + box(attention)
        : '',
      ready.length
        ? UI.sectionLabel(attention.length ? 'Ready' : 'Servers on this Mac', {
            action: add,
          }) + box(ready)
        : '',
      !rows.length
        ? UI.sectionLabel('Servers on this Mac', { action: add }) +
            UI.inset(
              '<div class="sempty">' +
                serverMark('unprobed') +
                '<b>No servers on this Mac yet</b>Add one, then check it to pin its identity here.</div>',
              { className: 'settings-inset' },
            )
        : '',
      '<div class="sfoot">' +
        M.icon('shield') +
        'Before every operation, FOKS checks the server’s history against what this Mac pinned.</div>',
    );
  }

  /* ========================================================== the detail */

  /* servers-screen.tsx:690 — the band that says what stops work and the one
       action that answers it. A checked server has no band: its Check lives
       in the Check-in row so the verb is never offered twice. */
  function statusBand(s, v) {
    var busy = busyOf(s);
    var check = UI.btn('Check now', {
      size: 'sm',
      variant: 'primary',
      icon: 'again',
      disabled: busy,
      attrs:
        'data-act="call" data-fn="srvCheck" data-arg="' +
        esc(v.server.id) +
        '"',
    });
    if (v.state === 'unprobed')
      return UI.band({
        severity: 'info',
        label: 'Not checked yet.',
        action: check,
        text: 'The address is saved. Check the server to verify its certificate and connection.',
      });
    if (v.state === 'lapsed')
      /* No Check: the agent renews the check-in, not this button. */
      return UI.band({
        severity: 'crit',
        label: 'Check-in expired.',
        text:
          'The server connection expired ' +
          esc(expires(v.expiry)) +
          '. This server is locked until reconnected.',
      });
    if (v.state === 'unavailable')
      return UI.band({
        severity: 'crit',
        label: 'Check-in status unknown.',
        action: check,
        text: 'Cannot verify the status of this server. This server is locked until reconnected.',
      });
    if (v.state === 'blocked')
      return UI.band({
        severity: 'crit',
        label: 'History changed.',
        action: UI.btn('Reset…', {
          size: 'sm',
          variant: 'danger',
          attrs: 'data-act="call" data-fn="srvOpenReset"',
        }),
        text: 'The server’s history no longer matches the saved connection. This server is locked.',
      });
    return '';
  }

  /* The Toggle's open state. `1` is the spelling the other parts use for
       this key; `open` is still accepted so older links keep working. */
  function inspectOpen(s) {
    return s.v.inspect === '1' || s.v.inspect === 'open';
  }

  /* servers-screen.tsx:781 — a named group on this server, as a chip. */
  function groupChip(store) {
    return (
      '<button type="button" class="gchip"' +
      (M.pages[M.storePages[store.id]]
        ? ' data-act="go" data-page="' + M.storePages[store.id] + '"'
        : '') +
      '>' +
      '<span class="av team" style="background: ' +
      UI.rgb(D.hue(store.name)) +
      ';">' +
      esc(D.initials(store.name)) +
      '</span>' +
      esc(store.name) +
      '</button>'
    );
  }

  function detailBody(s, v) {
    var w = M.data.world(s);
    var srv = v.server;
    var state = v.state;
    var locked = isLocked(state);
    var blocked = state === 'blocked';
    var busy = busyOf(s);
    var account = null;
    D.accounts.forEach(function (a) {
      if (!account && (a.server === srv.id || a.server === srv.name))
        account = a;
    });
    var groups = w.stores.filter(function (st) {
      return st.kind === 'team' && st.server === srv.id;
    });
    var named = groups.filter(function (st) {
      return st.team_kind === 'named';
    });
    var subtitle = srv.label
      ? srv.label +
        (account && !locked ? ' · signed in as ' + account.username : '')
      : account && !locked
        ? 'signed in as ' + account.username
        : 'No label';
    var hasHost = !!v.host && state !== 'unavailable';

    /* ------------------------------------------------------- Check-in */
    var checkin;
    if (state === 'checked') {
      checkin =
        UI.insetRow({
          label: 'Status',
          value:
            (v.checked
              ? 'Checked just now. History unchanged.'
              : 'Identity pinned on this Mac.') +
            '<small>History is checked before every operation.</small>',
          action: UI.btn('Check', {
            size: 'sm',
            icon: 'again',
            disabled: busy,
            attrs:
              'data-act="call" data-fn="srvCheck" data-arg="' +
              esc(srv.id) +
              '"',
          }),
        }) +
        UI.insetRow({
          label: 'Expires',
          value:
            esc(expires(v.expiry)) + '<small>Renewed by the agent.</small>',
        });
    } else if (state === 'pending') {
      checkin = UI.insetRow({
        label: 'Status',
        value: 'Reading signed status…',
      });
    } else if (state === 'unprobed') {
      checkin = UI.insetRow({
        label: 'Expires',
        value: '<span class="stopped">Not yet. Never checked.</span>',
      });
    } else if (state === 'lapsed') {
      checkin = UI.insetRow({
        label: 'Expired',
        value:
          '<b class="danger-title">' +
          esc(expires(v.expiry)) +
          '</b><small>The agent renews it. Nothing to do here.</small>',
      });
    } else if (state === 'unavailable') {
      checkin = UI.insetRow({
        label: 'Expires',
        value: '<span class="stopped">Unknown</span>',
      });
    } else {
      checkin = UI.insetRow({
        label: 'Expires',
        value: '<span class="stopped">Not read while locked</span>',
      });
    }

    /* -------------------------------------------------- On this server */
    var youValue = locked
      ? '<span class="stopped">Hidden while locked</span>'
      : account
        ? '<b>' +
          esc(account.username) +
          '</b><small>local alias ' +
          esc(account.alias) +
          '</small>'
        : 'No account on this server<small>You can still read from it</small>';
    var groupsValue = locked
      ? '<span class="stopped">' +
        (groups.length
          ? esc(
              groups
                .map(function (st) {
                  return st.name;
                })
                .join(', '),
            ) + ' — locked'
          : 'Hidden while locked') +
        '</span>'
      : named.length
        ? '<span class="gchips">' + named.map(groupChip).join('') + '</span>'
        : 'None yet';

    /* ------------------------------------------------------- Identity */
    var identity;
    if (hasHost) {
      identity =
        UI.inset(
          UI.insetRow({ label: 'Address', value: esc(srv.name) }) +
            UI.insetRow({
              label: 'Host ID',
              value:
                '<span class="hostid" title="' +
                esc(v.host.hostId) +
                '"><code>' +
                esc(D.shortId(v.host.hostId, 8)) +
                '</code></span>' +
                '<small>Hover for the full ID. Copy copies all of it.</small>',
              action: UI.btn('Copy', {
                size: 'sm',
                icon: 'copy',
                disabled: blocked,
                attrs:
                  'data-act="call" data-fn="srvCopyHost" data-arg="' +
                  esc(v.host.hostId) +
                  '"',
              }),
            }) +
            UI.insetRow({
              label: 'Signed history',
              value:
                esc(v.host.chain) + ' entries · version ' + esc(v.host.epoch),
            }),
          { className: 'settings-inset middle' },
        ) +
        UI.toggle({
          label: 'Inspect last check response',
          open: v.rollback || inspectOpen(s),
          disabled: v.rollback,
          attrs: v.rollback
            ? ''
            : 'data-act="set" data-key="v.inspect" data-val="' +
              (inspectOpen(s) ? '' : '1') +
              '"',
          body:
            '<pre>' +
            esc(
              JSON.stringify(
                {
                  profile: (v.snapshot && v.snapshot.profile) || srv.id,
                  configuredProbe:
                    (v.snapshot && v.snapshot.configuredProbe) || srv.name,
                  host: v.host,
                  leaseRequired: v.snapshot ? v.snapshot.leaseRequired : null,
                  leaseExpiresAt: v.expiry,
                },
                null,
                2,
              ),
            ) +
            '</pre>',
        });
    } else {
      identity = UI.inset(
        UI.insetRow({ label: 'Address', value: esc(srv.name) }) +
          UI.insetRow({
            label: 'Host ID',
            value:
              '<span class="stopped">' +
              (state === 'unprobed'
                ? 'Set by the first check'
                : state === 'pending'
                  ? 'Reading signed status…'
                  : 'Hidden while locked') +
              '</span>',
          }),
        { className: 'settings-inset middle' },
      );
    }

    return h(
      '<button type="button" class="crumb" data-act="call" data-fn="srvBack">' +
        M.icon('back', null, { cls: 'ic' }) +
        'Servers</button>',
      '<div class="shead">' +
        serverMark(state) +
        '<span class="t"><b>' +
        esc(srv.name) +
        '</b><small>' +
        esc(subtitle) +
        '</small></span>' +
        statusChip(state) +
        '</div>',
      statusBand(s, v),
      UI.sectionLabel('Check-in'),
      UI.inset(checkin, { className: 'settings-inset middle' }),
      UI.sectionLabel('On this server'),
      UI.inset(
        UI.insetRow({ label: 'You', value: youValue }) +
          UI.insetRow({ label: 'Groups', value: groupsValue }),
        { className: 'settings-inset' },
      ),
      UI.sectionLabel('Identity'),
      identity,
      UI.sectionLabel('On this Mac', { className: 'danger-title' }),
      UI.inset(
        UI.insetRow({
          className: 'dangerrow',
          label: 'Forget this server',
          value: '<small>Removes it from this Mac.</small>',
          action: UI.btn('Forget…', {
            size: 'sm',
            icon: 'trash',
            disabled: blocked,
            attrs: 'data-act="call" data-fn="srvOpenForget"',
          }),
        }) +
          UI.insetRow({
            className: 'dangerrow',
            label: 'Reset local state',
            value:
              '<small>Drops the pinned identity, cache and unfinished operations.</small>',
            action: UI.btn('Reset…', {
              size: 'sm',
              variant: 'danger',
              attrs: 'data-act="call" data-fn="srvOpenReset"',
            }),
          }),
        { className: 'settings-inset middle danger-box' },
      ),
    );
  }

  /* The section renders the list whenever the requested profile does not
       resolve — `selected = serverFor(world, profile)` and the body branches
       on it (servers-screen.tsx:410). */
  function sectionBody(s, wantDetail) {
    if (!wantDetail) return listBody(s);
    var srv = serverFor(s, selectedId(s));
    if (!srv) return listBody(s);
    return detailBody(s, view(s, srv));
  }

  /* ============================================================= sheets */

  function sheetOf(s, page) {
    var v = s.v.sheet;
    if (v != null && v !== '') return v;
    if (v === '') return '';
    return page === 'srv-add'
      ? 'add'
      : page === 'srv-forget'
        ? 'forget'
        : page === 'srv-reset'
          ? 'reset'
          : '';
  }
  var CLOSE = 'data-act="call" data-fn="srvCloseSheet"';
  var SHEET_ID = 'srv-sheet-title';
  /* SheetFrame (servers-screen.tsx:1064) over components/sheet.tsx — now
       just M.ui.sheet, which since the core gained `aria-modal`/`tabindex` on
       the backdrop and the inert `data-act="stop"` on the panel emits the
       same bytes this part used to write out by hand. Only the glyph is
       local: SheetFrame's server mark, with the trailing space React leaves
       in the non-danger class. */
  function sheetFrame(o) {
    return UI.sheet({
      width: 'wide',
      danger: o.danger,
      titleId: SHEET_ID,
      glyph:
        '<span class="server-mark ' +
        (o.danger ? 'danger' : '') +
        '">' +
        M.icon(o.danger ? 'trash' : 'server') +
        '</span>',
      title: esc(o.title),
      subtitle: esc(o.subtitle),
      body: o.body || '',
      footer: o.footer || '',
      closeAttrs: CLOSE,
    });
  }
  /* <Field> writes `aria-label`, `id`, `type`, `value` in that order and no
       placeholder; M.ui.field emits `type` first, so the two Add fields are
       written out here to keep the attribute order the app's. The Confirm
       rows below are bare inputs with neither label nor aria-label. */
  function labelledInput(label, value, bind) {
    var id = 'srv-field-' + bind.slice(2);
    return (
      '<div class="fr"><label class="k" for="' +
      id +
      '">' +
      esc(label) +
      '</label>' +
      '<span class="v"><input aria-label="' +
      esc(label) +
      '" id="' +
      id +
      '" type="text" value="' +
      esc(value) +
      '" data-bind="' +
      esc(bind) +
      '" data-live></span></div>'
    );
  }
  /* The typed confirmation. `1` and `wrong` are the deck's shorthands for
       "the exact profile id" and "anything else"; `1` is the spelling the
       other parts use for the same key. `match` is still accepted so older
       links keep working. */
  function typedText(s, srv) {
    var t = s.v.typed;
    if (t == null || t === '') return '';
    if (t === '1' || t === 'match') return srv.id;
    if (t === 'wrong') return 'nope';
    return t;
  }
  function confirmRow(s, srv) {
    return UI.inset(
      UI.insetRow({
        label: 'Confirm',
        forId: 'srv-confirm',
        value:
          '<input placeholder="type ' +
          esc(srv.id) +
          '" id="srv-confirm" value="' +
          esc(typedText(s, srv)) +
          '" data-bind="v.typed" data-live>',
      }),
    );
  }

  /* `useState('partner')` / `useState('foks.partner.dev')`: only an unset key
       is the default. A CLEARED field is an empty string — the app shows it
       empty and `valid` goes false, which is the only way `Add server` is ever
       seen disabled — so `''` must not fold back to the prefill. (A deck chip
       writing `''` clears the key to null, so the chips still mean "default".) */
  function addProfile(s) {
    return s.v.addid == null ? 'partner' : s.v.addid;
  }
  function addProbe(s) {
    return s.v.addaddr == null ? 'foks.partner.dev' : s.v.addaddr;
  }

  function addSheet(s) {
    var profile = addProfile(s);
    var probe = addProbe(s);
    var clean = probe.replace(/^\s+|\s+$/g, '');
    /* servers-screen.tsx:1113 — the profile pattern, and an address that is
         non-empty, free of NUL/CR/LF and ≤2048 bytes as UTF-8. */
    var valid =
      /^[A-Za-z0-9_-]{1,64}$/.test(profile) &&
      clean.length > 0 &&
      clean.indexOf('\0') < 0 &&
      clean.indexOf('\n') < 0 &&
      clean.indexOf('\r') < 0 &&
      new TextEncoder().encode(clean).length <= 2048;
    return sheetFrame({
      title: 'Add a server',
      subtitle: 'Save its address, then check its identity',
      body:
        '<p>Adding a server saves its profile and address on this Mac. Check it next to verify its identity and save the connection.</p>' +
        UI.inset(
          labelledInput('Profile', profile, 'v.addid') +
            labelledInput('Address', probe, 'v.addaddr'),
        ),
      footer:
        UI.btn('Cancel', { attrs: CLOSE }) +
        UI.btn('Add server', {
          variant: 'primary',
          disabled: !valid,
          attrs: 'data-act="call" data-fn="srvAdd"',
        }),
    });
  }

  function resetSheet(s, srv) {
    var loading = s.v.preview === 'loading';
    /* `available` goes false the moment the one-use token leaves the ref,
         which appends a sentence to the hint and keeps the danger button
         disabled. In the app that survives only a failed resetServer — the
         mock bridge always succeeds — so it is a chip, not a click. */
    var spent = s.v.preview === 'spent';
    var resumables =
      srv.id === 'personal'
        ? [{ kind: 'team-creation', alias: 'homelab' }]
        : [];
    var artifacts = [{ kind: 'catalog-cache', entries: 5, bytes: 8192 }];
    var preview = loading
      ? '<p>Loading reset preview…</p>'
      : h(
          UI.sectionLabel('Also discarded · resumable operations'),
          UI.inset(
            resumables.length
              ? resumables
                  .map(function (r) {
                    return UI.insetRow({
                      label: esc(r.kind),
                      value:
                        esc(r.alias) + (r.target ? ' · ' + esc(r.target) : ''),
                    });
                  })
                  .join('')
              : UI.insetRow({
                  label: 'None',
                  value: 'No resumable operations found.',
                }),
          ),
          UI.sectionLabel('Local artifacts discarded'),
          UI.inset(
            artifacts.length
              ? artifacts
                  .map(function (r) {
                    return UI.insetRow({
                      label: esc(r.kind),
                      value:
                        esc(r.entries) +
                        ' entries · ' +
                        esc(r.bytes.toLocaleString()) +
                        ' bytes',
                    });
                  })
                  .join('')
              : UI.insetRow({
                  label: 'None',
                  value: 'No local cached data to remove.',
                }),
          ),
          '<p class="hint">This preview token expires in 60 seconds and can be used once.' +
            (spent ? ' Preview again by closing and reopening Reset.' : '') +
            '</p>',
        );
    return sheetFrame({
      danger: true,
      title: 'Reset ' + srv.name + '?',
      subtitle: 'Remove local data for this server from this Mac',
      body:
        '<p>This does not delete server data. Check again before using this server.</p>' +
        UI.inset(
          UI.insetRow({
            label: 'Discarded',
            value:
              'Server certificate, connection history, and local cached data.',
          }) +
            UI.insetRow({
              label: 'Lost',
              value:
                'Local writes that were never accepted by the server cannot be recovered.',
            }) +
            UI.insetRow({
              label: 'Kept',
              value:
                'Your keys and account remain on this Mac, but this reset does not sign you in.',
            }) +
            UI.insetRow({
              label: 'Untouched',
              value: 'Every other configured server and its local state.',
            }),
        ) +
        preview +
        confirmRow(s, srv),
      footer:
        UI.btn('Cancel', { attrs: CLOSE }) +
        UI.btn('Reset local state', {
          variant: 'danger',
          disabled: typedText(s, srv) !== srv.id || loading || spent,
          attrs: 'data-act="call" data-fn="srvReset"',
        }),
    });
  }

  function forgetSheet(s, srv) {
    return sheetFrame({
      danger: true,
      title: 'Forget ' + srv.name + '?',
      subtitle: 'Erase this Mac’s keys and state for the whole server',
      body:
        '<p>Your data on the server will not be deleted. Type the server profile name to confirm.</p>' +
        UI.inset(
          UI.insetRow({
            label: 'Erased',
            value:
              'Every account credential this Mac holds for this server, its pinned Host ID, signed-history checkpoint and cached artifacts.',
          }) +
            UI.insetRow({
              label: 'Lost',
              value:
                'An account with no backup phrase and no other paired device cannot be signed in to again.',
            }) +
            UI.insetRow({
              label: 'Untouched',
              value: 'Every other configured server and its local state.',
            }),
        ) +
        confirmRow(s, srv),
      footer:
        UI.btn('Cancel', { attrs: CLOSE }) +
        UI.btn('Forget server', {
          variant: 'danger',
          disabled: typedText(s, srv) !== srv.id,
          attrs: 'data-act="call" data-fn="srvForget"',
        }),
    });
  }

  /* `detail` is the screen's `selected`: ResetSheet and ForgetSheet are only
       rendered with a server open (servers-screen.tsx:386-421), so over the
       list the Add sheet is the only one that can be showing. */
  /* lock=locked / boot=loading / boot=error are returned by `App` INSTEAD of
       <VaultShell>, so ServersSection never mounts and its sheet cannot be on
       screen — the core suppresses `overlay(s)` under a takeover, so there is
       nothing for this to guard. (`agent=lost` is not one of these: the app
       keeps drawing the screen and hangs `.stopwrap` over it, sheet and all —
       see the mock's own agent-lost capture.) */
  function overlayFor(s, page, detail) {
    var sheet = sheetOf(s, page);
    if (sheet === 'add') return addSheet(s);
    if (!detail) return '';
    var srv = serverFor(s, selectedId(s));
    if (!srv) return '';
    if (sheet === 'reset') return resetSheet(s, srv);
    if (sheet === 'forget') return forgetSheet(s, srv);
    return '';
  }

  /* ========================================================== behaviour */

  /* Open does not remount ServersSection — only the body under it swaps —
       so `statuses` and `checked` cross it: `applied` travels whole, `added`
       (the server being opened) and `yes` (the check already run) alike, and
       so does the list's first-paint `pending`, since opening a row before
       the probes answer lands on a pending detail. `inspect` is the Toggle's
       own DOM state and `typed` a sheet field, and both of those really are
       mounted fresh. */
  M.fns.srvOpen = function (id) {
    M.go('srv-server', {
      'v.profile': id,
      'v.status': M.s.v.status === 'pending' ? 'pending' : null,
      'v.inspect': null,
      'v.typed': null,
      'v.applied': M.s.v.applied,
    });
  };
  /* The crumb, the same way back. The one thing that does not survive it is
       a `yes` a live check has since replaced: PINS/CHECKS are per-server, the
       way the app's maps are, while the token names no server — so once a real
       report exists, letting `yes` stand would report every other server as
       checked too. */
  M.fns.srvBack = function () {
    var live = Object.keys(CHECKS).length > 0;
    M.go('srv-list', {
      'v.applied': live
        ? applied(M.s, 'added')
          ? 'added'
          : null
        : M.s.v.applied,
    });
  };
  M.fns.srvOpenAdd = function () {
    M.go('srv-add', { 'v.addid': null, 'v.addaddr': null });
  };
  /* The sheet is drawn OVER the detail, which keeps rendering behind it: the
       screen does not remount, so `applied` (the check report) and `inspect`
       (the Toggle's own DOM state) travel with it — srv-forget and srv-reset
       declare both. Dropping `added` here used to lose the server the sheet is
       about, and the sheet with it. */
  M.fns.srvOpenForget = function () {
    M.go('srv-forget', { 'v.typed': null });
  };
  M.fns.srvOpenReset = function () {
    M.patch({ 'v.typed': null, 'v.preview': 'loading' });
    M.go('srv-reset');
    /* An effect fetches the preview the moment the sheet opens. */
    setTimeout(function () {
      if (M.s.page === 'srv-reset' && M.s.v.preview === 'loading')
        M.set('v.preview', null);
    }, 550);
  };
  /* Cancel, the backdrop and Escape. A click inside the panel never gets
       here: `.sheet` carries the inert data-act="stop". */
  M.fns.srvCloseSheet = function () {
    var page = M.s.page;
    M.patch({ 'v.sheet': null, 'v.typed': null, 'v.preview': null });
    if (page === 'srv-add') M.go('srv-list');
    else if (page === 'srv-forget' || page === 'srv-reset') M.go('srv-server');
    else M.render();
  };

  /* servers-screen.tsx:294 — check(server), with the same refusal. */
  M.fns.srvCheck = function (id) {
    var srv = serverFor(M.s, id);
    if (!srv || busyOf(M.s)) return;
    var v =
      M.s.page === 'srv-server' ||
      M.s.page === 'srv-forget' ||
      M.s.page === 'srv-reset'
        ? view(M.s, srv)
        : { state: rowView(M.s, srv).state, rollback: false };
    if (v.state === 'lapsed' || v.state === 'blocked' || v.rollback) return;
    var existing = pinnedHost(srv);
    var host = existing || firstCheckHost(srv);
    PINS[srv.id] = host;
    CHECKS[srv.id] = mergeCheck(
      srv.id,
      existing ? 'unchanged' : 'inserted',
      host,
    );
    M.toast(
      'Checked ' +
        host.canonicalName +
        ' — ' +
        acceptanceText(CHECKS[srv.id].acceptance),
    );
    M.toast(
      'Checked ' + host.canonicalName + '; refreshed signed server status',
    );
    /* The check clears any forced state — the server really is checked now.
         On a detail page `applied=yes` keeps that in the URL; on the list the
         row reads its state from the pin, and `added` is never disturbed. */
    M.patch({
      'v.status': null,
      'v.applied':
        M.s.page === 'srv-list' || M.s.page === 'srv-add'
          ? M.s.v.applied
          : withApplied(M.s, 'yes'),
    });
    M.render();
  };
  M.fns.srvCopyHost = function () {
    M.toast('Copied the full ID');
  };

  M.fns.srvAdd = function () {
    var profile = addProfile(M.s);
    var probe = addProbe(M.s).replace(/^\s+|\s+$/g, '');
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(profile) || !probe) return;
    var clash = serversOf(M.s).some(function (srv) {
      return srv.id === profile;
    });
    /* mock-bridge.ts:900 — `already-exists`, which app-root.tsx:416 toasts
         with the danger tone. The sheet stays open with the fields as typed. */
    if (clash) {
      M.toast('That server profile already exists.', { tone: 'warning' });
      return;
    }
    ADDED.id = profile;
    ADDED.name = probe;
    M.patch({
      'v.addid': null,
      'v.addaddr': null,
      'v.status': null,
      'v.inspect': null,
    });
    M.toast('Server added — check it before trusting anything on it');
    M.go('srv-server', { 'v.profile': profile, 'v.applied': 'added' });
  };
  M.fns.srvForget = function () {
    var srv = serverFor(M.s, selectedId(M.s));
    if (!srv || typedText(M.s, srv) !== srv.id) return;
    var name = srv.name;
    FORGOTTEN[srv.id] = true;
    delete PINS[srv.id];
    delete CHECKS[srv.id];
    M.patch({
      'v.typed': null,
      'v.profile': null,
      'v.status': null,
      /* Forgetting one server does not un-add another. */
      'v.applied':
        srv.id !== ADDED.id && applied(M.s, 'added') ? 'added' : null,
    });
    M.go('srv-list');
    /* The intended end state: the list without the forgotten server, and
         one neutral toast. The real mock bridge forgets it and then fails the
         reload it triggers (`list_servers omitted a server used by the
         catalog.`), so ?state=servers-server + servers-forget-done.json ends
         on an unchanged list and a red warning pill instead — map §8.2. */
    M.toast('Forgot ' + name + ' on this Mac');
  };
  /* resetServer() spends the token and answers { applied: true }; the mock
       bridge does not drop the pinned host, so the detail is unchanged. */
  M.fns.srvReset = function () {
    var srv = serverFor(M.s, selectedId(M.s));
    if (
      !srv ||
      typedText(M.s, srv) !== srv.id ||
      M.s.v.preview === 'loading' ||
      M.s.v.preview === 'spent'
    )
      return;
    M.patch({ 'v.typed': null, 'v.preview': null });
    M.go('srv-server');
    M.toast('Reset ' + srv.name + ' — check it again before using it');
  };

  /* ⌘R / Ctrl-R checks the open server (servers-screen.tsx:357). ⌘N and the
       old arrow walk are gone. */
  document.addEventListener('keydown', function (ev) {
    if (ev.defaultPrevented) return;
    if (ev.target && /INPUT|TEXTAREA/.test(ev.target.tagName)) return;
    if (!(ev.metaKey || ev.ctrlKey) || ev.key.toLowerCase() !== 'r') return;
    if (
      M.s.page !== 'srv-server' &&
      M.s.page !== 'srv-forget' &&
      M.s.page !== 'srv-reset'
    )
      return;
    if (!serverFor(M.s, selectedId(M.s))) return;
    ev.preventDefault();
    M.fns.srvCheck(selectedId(M.s));
  });

  /* Escape: close the sheet, else fall back to the app-root rule. */
  function escapeRule() {
    var page = M.s.page;
    if (
      page === 'srv-add' ||
      page === 'srv-forget' ||
      page === 'srv-reset' ||
      M.s.v.sheet
    ) {
      return M.fns.srvCloseSheet();
    }
    if (M.s.v.menu) return M.set('v.menu', null);
  }
  function after() {
    M.fns.escape = escapeRule;
  }

  /* Leaving the section remounts ServersSection in the app: `enteredScene`
       is re-read, the status/checked maps are empty again and `location.
       profile` is dropped by the encoder (location.ts:314). So everything
       this part owns goes with it — the pinned hosts do not, because those
       live in the bridge, not the screen. */
  var OWN_KEYS = [
    'profile',
    'status',
    'inspect',
    'applied',
    'typed',
    'preview',
    'addid',
    'addaddr',
    'empty',
  ];
  function leave(s, next) {
    if (/^srv-/.test(next)) return;
    OWN_KEYS.forEach(function (k) {
      s.v[k] = null;
    });
    CHECKS = {};
    /* The forget is the screen's, not the bridge's: coming back re-reads
         list_servers and the server is there again (map §8.2). The Add sheet's
         last profile goes back to the pair the deck chips and the flow name. */
    FORGOTTEN = {};
    ADDED.id = ADDED_DEFAULT.id;
    ADDED.name = ADDED_DEFAULT.name;
  }

  /* ============================================================== pages */

  /* The frozen clock, said once and appended to every group whose chips move
       a date. The app derives the expiry from `now` (mock-bridge.ts:866 —
       now+6d / now−3d); the mock pins it to the hour the captures were shot so
       a deep link renders the same dates as `cap/settings/servers-*`, and so
       the inspector's raw `leaseExpiresAt` and the prose above it always name
       the same instant. Only the wall-clock dates differ from the app, never
       which side of `now` they fall on. */
  var FROZEN =
    ' Dates are frozen at the captures’ clock (now = Sep 3, 2026, 9:37 PM), not derived from the real one, so the mock keeps naming Sep 9 and Aug 31 the way the captures do.';

  var PROFILE = {
    key: 'profile',
    label: 'Server',
    note: 'location.profile — which server the section is open on. A profile that does not resolve falls back to the list, exactly as the screen does.',
    values: [
      { v: '', label: 'personal · foks.example.net', hint: 'checked' },
      {
        v: 'acme',
        label: 'acme · foks.acme-corp.com',
        hint: 'the signed check-in expired 3 days ago, in every scene the app can reach',
      },
      {
        v: 'partner',
        label: 'partner · foks.partner.dev',
        hint: 'never checked',
      },
      {
        v: 'lab',
        label: 'lab · foks.lab.test',
        hint: 'only with applied=added: the server the Add sheet creates',
      },
    ],
  };
  var STATUS = {
    key: 'status',
    label: 'Server state',
    note:
      'resolveServerUiState (servers-screen.tsx:110). "Auto" and "ok" render what the app renders: the catalog Server.state plus the signed describe_server_status, whose expiry the mock bridge fixes per profile. That is why the app-wide acme key cannot move this pane — use these chips to see foks.acme-corp.com in another state.' +
      FROZEN,
    values: [
      {
        v: '',
        label: 'Auto',
        hint: 'the world and the signed status, exactly as the app reads them',
      },
      { v: 'ok', label: 'ok', hint: 'same as Auto — the world decides' },
      {
        v: 'checked',
        label: 'checked',
        hint: 'Checked; no band, Check lives in the Check-in row',
      },
      {
        v: 'unprobed',
        label: 'unprobed',
        hint: '.band.info "Not checked yet." with Check now; Host ID "Set by the first check"',
      },
      {
        v: 'lapsed',
        label: 'lapsed',
        hint: '.band.stop "Check-in expired." with NO action; You/Groups hidden',
      },
      {
        v: 'checkin-unavailable',
        label: 'checkin-unavailable',
        hint: 'server.state lease-unavailable → "Check-in status unknown." with Check now. On foks.partner.dev it falls through to unprobed: with nothing pinned, resolveServerUiState answers on the snapshot first.',
      },
      {
        v: 'unavailable',
        label: 'unavailable',
        hint: 'the status probe threw — same render as checkin-unavailable',
      },
      {
        v: 'rollback',
        label: 'rollback',
        hint: 'the servers-rollback scene: blocked, Copy disabled, inspector forced open and disabled',
      },
      {
        v: 'blocked',
        label: 'blocked',
        hint: 'server.state blocked — no snapshot, so Identity is hidden and Forget is disabled',
      },
      {
        v: 'pending',
        label: 'pending',
        hint: 'no snapshot yet: "Reading status · Waiting for a signed check-in"',
      },
    ],
  };
  var INSPECT = {
    key: 'inspect',
    label: 'Inspector',
    note: 'the Toggle "Inspect last check response"; its <pre> stays in the DOM with `hidden` when closed. Its raw leaseExpiresAt is the same epoch the Check-in row formats, so the two always agree.',
    values: [
      { v: '', label: 'Collapsed' },
      { v: '1', label: 'Expanded' },
    ],
  };
  var SHEET = {
    key: 'sheet',
    label: 'Sheet',
    note: 'the Sheet union (servers-screen.tsx:47). Empty means "the page decides": srv-add opens add, srv-forget forget, srv-reset reset.',
    values: [
      { v: '', label: 'Page default' },
      { v: 'add', label: 'Add' },
      { v: 'forget', label: 'Forget' },
      { v: 'reset', label: 'Reset' },
    ],
  };
  var TYPED = {
    key: 'typed',
    label: 'Confirmation',
    note: 'the typed confirmation, which must equal the local profile id (not the address). The input writes the raw text; these chips are shorthands.',
    values: [
      { v: '', label: 'Empty' },
      { v: '1', label: 'Matches the id' },
      { v: 'wrong', label: 'Mismatch' },
    ],
  };
  var EMPTY = {
    key: 'empty',
    label: 'Empty list',
    note: 'unreachable with the fixture: the .sempty "No servers on this Mac yet" box (§8.1). With no ready server the "Add a server…" button moves onto the Needs attention label.',
    values: [
      { v: '', label: 'Three servers' },
      { v: 'yes', label: 'No servers' },
      {
        v: 'locked',
        label: 'Nothing ready',
        hint: 'only the locked server, so "Add a server…" hangs off Needs attention instead',
      },
    ],
  };
  /* The list's own slice of `status`: the screen starts with an empty
       `statuses` Map and fills it from describe_server_status, so the first
       paint resolves every row without a snapshot. It is the only place the
       pending ROW ("Reading status · Waiting for a signed check-in") shows.
       Same key as the detail's, so the deck shows one group at a time. */
  var LIST_STATUS = {
    key: 'status',
    label: 'Server state',
    note:
      'the list resolves each row from the signed describe_server_status answers it has. Before they arrive there are none, so only the catalog Server.state can say anything: foks.partner.dev is never-probed, a server the acme key has put in lease-lapsed or blocked keeps that, and everything else reads "Reading status".' +
      FROZEN,
    values: [
      {
        v: '',
        label: 'Probed',
        hint: 'the signed status has answered for every server',
      },
      {
        v: 'pending',
        label: 'Reading status',
        hint: 'the first paint: no snapshot yet, so the row line is "Reading status · Waiting for a signed check-in" and a row the catalog already calls locked has no expiry to name',
      },
    ],
  };
  var APPLIED = {
    key: 'applied',
    label: 'Applied',
    note: 'the mutation has already run. "Checked" seeds the check_server report an explicit check would have stored, so the Check-in row reads "Checked just now."; "Added" materialises the server the Add sheet creates, so the end of the add flow is reachable cold. The tokens combine with commas.',
    values: [
      { v: '', label: 'No' },
      {
        v: 'yes',
        label: 'Checked',
        hint: 'the detail’s Check-in row reads "Checked just now. History unchanged."; on the list every server that would accept a check reads Checked instead of Never checked, which is what clicking Check in a row does. The deep link is silent, exactly as ?state=servers-check is',
      },
      {
        v: 'added',
        label: 'Added',
        hint: 'foks.lab.test joins the list; with profile=lab, its never-checked detail',
      },
      {
        v: 'added,yes',
        label: 'Added + checked',
        hint: 'the end of the add flow: the new server exists and has just been checked',
      },
    ],
  };

  M.page({
    id: 'srv-list',
    keeps: [
      'account',
    ] /* the Settings account ref rides through the Servers section (settings-screen.tsx:642) */,
    title: 'Servers & devices',
    path: ['Settings', 'Servers & devices'],
    nav: 'settings',
    note: 'ServerList (servers-screen.tsx:597) — a locked server is hoisted into "Needs attention"; "Add a server…" hangs off the Ready label.',
    controls: [EMPTY, LIST_STATUS, APPLIED, SHEET],
    render: function (s) {
      var t = M.globalTakeover(s);
      if (t) return t;
      return { main: frame(s, listBody(s)) };
    },
    overlay: function (s) {
      return overlayFor(s, 'srv-list', false);
    },
    after: after,
    leave: leave,
  });

  M.page({
    id: 'srv-server',
    keeps: [
      'account',
    ] /* the Settings account ref rides through the Servers section (settings-screen.tsx:642) */,
    title: 'Server',
    path: ['Settings', 'Servers & devices', 'Server'],
    nav: 'settings',
    note: 'ServerBody (servers-screen.tsx:769): crumb, .shead, the band, then Check-in / On this server / Identity / On this Mac.',
    controls: [PROFILE, STATUS, INSPECT, APPLIED, SHEET],
    render: function (s) {
      var t = M.globalTakeover(s);
      if (t) return t;
      return { main: frame(s, sectionBody(s, true)) };
    },
    overlay: function (s) {
      return overlayFor(s, 'srv-server', true);
    },
    after: after,
    leave: leave,
  });

  M.page({
    id: 'srv-add',
    keeps: [
      'account',
    ] /* the Settings account ref rides through the Servers section (settings-screen.tsx:642) */,
    title: 'Add server',
    path: ['Settings', 'Servers & devices', 'Add server sheet'],
    nav: 'settings',
    note: 'AddServerSheet over the list, prefilled partner / foks.partner.dev — which the fixture already has, so Add server is refused with a red "That server profile already exists." pill. Change the fields to lab / foks.lab.test and it lands on the new profile’s never-checked detail (applied=added).',
    controls: [
      {
        key: 'addid',
        label: 'Profile field',
        freeText: true,
        note: 'the local profile name, free text. Only an unset key is the prefill: clear the field and it stays empty, which is how the app’s "Add server" is seen disabled (the id must match ^[A-Za-z0-9_-]{1,64}$).',
        values: [
          {
            v: '',
            label: 'partner',
            hint: 'the prefill — which the fixture already has, so Add server is refused',
          },
          { v: 'lab', label: 'lab' },
        ],
      },
      {
        key: 'addaddr',
        label: 'Address field',
        freeText: true,
        note: 'the probe address, free text. Non-empty, no NUL/CR/LF and ≤2048 bytes as UTF-8, or "Add server" is disabled.',
        values: [
          { v: '', label: 'foks.partner.dev', hint: 'the prefill' },
          { v: 'foks.lab.test', label: 'foks.lab.test' },
        ],
      },
      /* The list keeps rendering behind the sheet, so it keeps its knobs. */
      EMPTY,
      LIST_STATUS,
      APPLIED,
      SHEET,
    ],
    render: function (s) {
      var t = M.globalTakeover(s);
      if (t) return t;
      return { main: frame(s, listBody(s)) };
    },
    overlay: function (s) {
      return overlayFor(s, 'srv-add', false);
    },
    after: after,
    leave: leave,
  });

  M.page({
    id: 'srv-forget',
    keeps: [
      'account',
    ] /* the Settings account ref rides through the Servers section (settings-screen.tsx:642) */,
    title: 'Forget server',
    path: ['Settings', 'Servers & devices', 'Forget server sheet'],
    nav: 'settings',
    note: 'ForgetSheet (alertdialog) over the detail; the confirmation is the local profile id, not the address.',
    /* The detail keeps rendering behind the sheet — the screen does not
         remount — so it keeps every knob it reads, `applied` and `inspect`
         included. */
    controls: [PROFILE, TYPED, STATUS, APPLIED, INSPECT, SHEET],
    render: function (s) {
      var t = M.globalTakeover(s);
      if (t) return t;
      return { main: frame(s, sectionBody(s, true)) };
    },
    overlay: function (s) {
      return overlayFor(s, 'srv-forget', true);
    },
    after: after,
    leave: leave,
  });

  M.page({
    id: 'srv-reset',
    keeps: [
      'account',
    ] /* the Settings account ref rides through the Servers section (settings-screen.tsx:642) */,
    title: 'Reset local state',
    path: ['Settings', 'Servers & devices', 'Reset local state sheet'],
    nav: 'settings',
    note: 'ResetSheet (alertdialog): the preview arrives after "Loading reset preview…" and carries a 60-second, one-use token.',
    controls: [
      PROFILE,
      TYPED,
      {
        key: 'preview',
        label: 'Reset preview',
        note: 'describe_reset is in flight while the sheet is open, and its one-use token is spent by the danger button.',
        values: [
          { v: '', label: 'Preview ready' },
          { v: 'loading', label: 'Preview loading' },
          {
            v: 'spent',
            label: 'Token spent',
            hint: 'the hint gains " Preview again by closing and reopening Reset." and the danger button stays disabled — in the app, only after a resetServer that failed',
          },
        ],
      },
      /* As srv-forget: the detail behind the sheet keeps its knobs. */
      STATUS,
      APPLIED,
      INSPECT,
      SHEET,
    ],
    render: function (s) {
      var t = M.globalTakeover(s);
      if (t) return t;
      return { main: frame(s, sectionBody(s, true)) };
    },
    overlay: function (s) {
      return overlayFor(s, 'srv-reset', true);
    },
    after: after,
    leave: leave,
  });
})();
