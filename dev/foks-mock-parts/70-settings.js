  /* -------------------------------------------------------- 70-settings.js
     Settings: Recovery devices ("This Mac & other devices"), Security keys,
     Accounts, About, the stale-link takeover, and every sheet those panes own.
     screens/settings-screen.tsx is the source; /tmp/foks-mock/cap/settings/*
     the ground truth for structure and classes.

     ==================================================================
     M.settingsFrame(s, sectionId, bodyHtml, opts)   ← shared, call it
     ==================================================================

       Returns the WHOLE `<main class="main">…</main>` for a Settings pane:
       the `.path` page header, `.body > .settings-cols`, the six-row
       `nav.settings-sections`, and `.settings-main` wrapping `bodyHtml`.
       Every Settings page in the mock (Servers, Groups included) must render
       through it so the chrome is identical everywhere.

       s          the state object the page renderer was handed.
       sectionId  which nav row is `.nav.on` — one of
                    'macs' | 'keys' | 'account' | 'servers' | 'groups' | 'about'
                  (the nav rows navigate to set-devices / set-keys /
                   set-account / srv-list / set-groups / set-about through
                   M.fns.settingsNav, which drops the open sheet and the
                   values typed into it the way the screen does, while
                   `v.account` — the selected StoreRef — survives, exactly as
                   settings-screen.tsx:642 does; a ref the catalog no longer
                   lists is dropped instead, so the next pane falls back to
                   the first account).
       bodyHtml   RAW html for `div.settings-main`. Nothing is wrapped or
                  escaped; emit `.sec` / `.inset.settings-inset` blocks.
       opts       optional:
                    switcher : true            prepend the account switcher
                                               (`.settings-label > .seg.txt`,
                                               aria-label "This account"),
                                               reading/writing `v.account`.
                    switcherKey : 'v.account'  override that view key.
                    title / subtitle           override the page header
                                               (defaults "Settings" /
                                               "Accounts, servers, keys,
                                               groups and this Mac").

       Example (the Servers pane):
         return { main: M.settingsFrame(s, 'servers', listHtml) };

     Also exported for reuse: M.settingsSwitcher(s, key) — just the
     `.settings-label` block; M.settingsAccounts(w) — the account stores in
     catalog order; M.settingsSelected(s, w) — the selected account store (or
     undefined when `v.account` names no store, which is set-unavailable);
     M.settingsStopped(w, store) — `accessStopped` for a store.

     View keys this file owns, and their defaults:
       account   acct:personal | acct:work            which account is selected
                 (a ref that resolves to no store draws UnavailableAccount)
       sheet     '' | pair-offer | pair-accept | phrase-prepare | phrase |
                 recover | passphrase | enrol | provision | yubi-<action> |
                 revoke | remove-device | go-profile
       seg       the open sheet's segmented mode (pairing direction /
                 passphrase action)
       started   '1' once a pairing offer has revealed its phrase
       pending   '1' when the agent holds a pairing operation, so Resume
                 continues it instead of raising "no longer pending"
       written   '1' once "I have written these down" is ticked
       advanced  '1' — <details class="adv"> in Enroll, the About inspector
       applied   '1' — the mutation this pane's sheet performs has been run
       typed     '1' — the typed confirmation matches, so the danger button
                 goes live (typing the exact string does the same)
       devlist   fixture | plus08 | keycurrent | none   (device-row variants
                 the fixture cannot reach; map §8.1)
       yubistate fixture | pending | none               (ditto for enrollments)
       accounts  '' | none  — '' is the fixture's two accounts, `none` a Mac
                 with no account at all, which is the only way to reach either
                 "No available account on this Mac" notice (map §8.1)
       loading   '' | 1 — the reads a pane opens on and leaves before any
                 capture lands (map §3.5): "Loading devices…", "Loading keys…",
                 "Reading check-in…", "Reading app info…"
       device    the StoreRef-free device id the Your Macs Remove… button
                 names; read, never a chip, so it is in `keeps`
     Free-text keys the sheets bind (not deck chips): alias, dev, phrase,
     tokens, username, pin, puk, other, confirm, pw, pw2, serial, slot1,
     slot2, tries1, tries2, invite.                                        */

  (function () {
    var esc = M.esc, h = M.h, UI = M.ui, D = M.data;

    /* ------------------------------------------------------------ chrome */
    var SECTIONS = [
      { id: 'macs', label: 'Recovery devices', icon: 'file', page: 'set-devices' },
      { id: 'keys', label: 'Security keys', icon: 'key', page: 'set-keys' },
      { id: 'account', label: 'Accounts', icon: 'vault', page: 'set-account' },
      { id: 'servers', label: 'Servers', icon: 'server', page: 'srv-list' },
      { id: 'groups', label: 'Groups', icon: 'people', page: 'set-groups' },
      { id: 'about', label: 'About', icon: 'info', page: 'set-about' },
    ];

    /* The account stores, in catalog order — accountStores(world). `v.accounts`
       is the mock's own knob for the one thing the fixture cannot reach: a Mac
       with no account at all, which is what both "No available account on this
       Mac" notices are for (map §8.1). */
    M.settingsAccounts = function (w) {
      if (M.s.v.accounts === 'none') return [];
      return (w || D.world(M.s)).stores.filter(function (st) { return st.kind === 'account'; });
    };
    /* The selected account. `v.account` names a StoreRef; a ref that does not
       resolve is the stale-link case and returns undefined. */
    M.settingsSelected = function (s, w) {
      w = w || D.world(s);
      var accounts = M.settingsAccounts(w);
      var want = s.v.account;
      if (!want) return accounts[0];
      for (var i = 0; i < accounts.length; i++) if (accounts[i].id === want) return accounts[i];
      return undefined;
    };
    /* accessStopped (settings-screen.tsx:416) as the fixture reaches it: the
       server's own state decides, so Work is normal while Acme is ok and
       stopped the moment its check-in lapses or its catalog goes. */
    M.settingsStopped = function (w, store) {
      if (!store) return false;
      if (w.unavailableStores.indexOf(store.id) >= 0) return true;
      var srv = w.serverById[store.server];
      var state = srv ? srv.state : 'never-probed';
      return state === 'blocked' || state === 'lease-lapsed' ||
        state === 'never-probed' || state === 'lease-unavailable';
    };

    M.settingsSwitcher = function (s, key) {
      key = key || 'v.account';
      var w = D.world(s);
      var accounts = M.settingsAccounts(w);
      /* MacsSection returns its Notice before the switcher (settings-screen.tsx:1139),
         so with no account there is no `.settings-label` at all — not an empty one. */
      if (!accounts.length) return '';
      var sel = M.settingsSelected(s, w);
      return '<div class="settings-label">' + UI.segmented({
        label: 'This account',
        value: sel ? sel.id : null,
        key: key,
        items: accounts.map(function (store) {
          var ambiguous = accounts.some(function (other) {
            return other.id !== store.id && other.account === store.account;
          });
          var server = w.serverById[store.server] ? w.serverById[store.server].name : store.server;
          return {
            id: store.id,
            label: ambiguous ? store.account + ' · ' + server : store.account,
            title: store.account + ' on ' + server,
          };
        }),
      }) + '</div>';
    };

    M.settingsFrame = function (s, sectionId, bodyHtml, opts) {
      opts = opts || {};
      var nav = SECTIONS.map(function (item) {
        return '<button type="button" class="' + (item.id === sectionId ? 'nav on' : 'nav') + '"' +
          ' data-act="call" data-fn="settingsNav" data-arg="' + item.page + '">' +
          M.icon(item.icon) + '<span class="t">' + esc(item.label) + '</span></button>';
      }).join('');
      var head = UI.pageHeader({
        title: opts.title || 'Settings',
        subtitle: opts.subtitle === undefined ? 'Accounts, servers, keys, groups and this Mac' : opts.subtitle,
      });
      var main = (opts.switcher ? M.settingsSwitcher(s, opts.switcherKey) : '') + (bodyHtml || '');
      return UI.main(head + UI.body(
        '<div class="settings-cols"><nav class="settings-sections" aria-label="Settings sections">' +
        nav + '</nav><div class="settings-main">' + main + '</div></div>'));
    };

    /* ========================================================= sheet frame
       SheetFrame (settings-screen.tsx:1763) is SheetDialog width="wide" with
       the gear/trash server-mark, so this is M.ui.sheet with those two
       defaults filled in. `dismissible:false` drops the backdrop's close hook
       exactly as SheetDialog drops onClose. */
    function sheet(o) {
      return UI.sheet({
        width: o.width || 'wide',
        danger: o.danger,
        glyph: o.glyph === undefined ? glyph(o.danger) : o.glyph,
        title: o.title,
        subtitle: o.subtitle,
        body: o.body,
        footer: o.footer,
        backdropAttrs: o.dismissible === false ? undefined
          : 'data-act="call" data-fn="settingsClose"',
      });
    }
    /* SheetFrame's glyph — note the trailing space React leaves in the class. */
    function glyph(danger) {
      return '<span class="server-mark ' + (danger ? 'danger' : '') + '">' +
        M.icon(danger ? 'trash' : 'gear') + '</span>';
    }
    function closeBtn(label) {
      return UI.btn(esc(label), { attrs: 'data-act="call" data-fn="settingsClose"' });
    }
    /* A bare <input> row: the app writes these without an aria-label. */
    function inputRow(o) {
      var id = 'sf-' + o.key.replace(/\W/g, '');
      return UI.insetRow({
        label: esc(o.label), forId: id,
        value: '<input' + (o.placeholder ? ' placeholder="' + esc(o.placeholder) + '"' : '') +
          (o.min === undefined ? '' : ' min="' + esc(o.min) + '"') +
          (o.max === undefined ? '' : ' max="' + esc(o.max) + '"') +
          ' id="' + id + '"' + (o.type ? ' type="' + o.type + '"' : '') +
          ' value="' + esc(o.value == null ? '' : o.value) + '"' +
          ' data-bind="v.' + o.key + '" data-live>' +
          (o.hint === undefined ? '' : '<small>' + o.hint + '</small>'),
      });
    }
    function field(o) {
      return UI.field({
        id: 'sf-' + o.key.replace(/\W/g, ''), label: o.label, type: o.type,
        mono: o.mono, min: o.min, max: o.max, hint: o.hint,
        value: o.value == null ? '' : o.value, bind: 'v.' + o.key,
      });
    }
    function val(s, key, fallback) {
      return s.v[key] == null ? fallback : s.v[key];
    }
    /* map §3.5 — the reads every pane starts with. The screen has no
       deep link for them (they are gone by the time a capture lands), so
       `v.loading` holds them still: `macsLoading` / `keysLoading` /
       `!loadedProfiles.has(profile)` / `appInfo === null` are one fact here
       because one mount starts all four. */
    function isLoading(s) {
      return val(s, 'loading', '') === '1';
    }

    /* ============================================================== data
       The per-account facts the bridge answers with (map §6). Devices are
       the fixture's with their ids rewritten 02… → 04…; every other account
       gets one synthetic current row. Backup enrollments live here because
       20-data.js carries the phrase but not the enrollment list. */
    /* A YubiKey device id: the ids `list_devices` answers with start 08 for a
       card-held device and 04 for a software one (map §4.2). Only 20-data's
       04… ids are fixture facts; the 08 row is a devlist variant. */
    var KEY_ID = '08' + new Array(65).join('3');
    function backupsOf(ref) {
      if (!D.backupEnrollments[ref]) D.backupEnrollments[ref] = [];
      return D.backupEnrollments[ref];
    }
    function devicesOf(s, ref) {
      var rows;
      if (ref === 'acct:personal') {
        rows = D.devices.map(function (d) {
          return { name: d.name, role: d.role, current: d.current, id: D.deviceIdFor(d.id_hex) };
        });
      } else {
        rows = [{
          name: D.syntheticDevice.name, role: D.syntheticDevice.role,
          current: D.syntheticDevice.current, id: D.syntheticDevice.id,
        }];
      }
      var mode = val(s, 'devlist', 'fixture');
      if (mode === 'none') return [];
      if (mode === 'plus08') {
        rows = rows.concat([{ name: null, role: 'owner', current: false, id: KEY_ID }]);
      }
      if (mode === 'keycurrent') {
        rows = rows.map(function (d, i) {
          return i === 0 ? { name: 'YubiKey 20993145', role: d.role, current: true, id: KEY_ID } : d;
        });
      }
      if (val(s, 'applied', '') === '1') {
        rows = rows.filter(function (d) { return d.name !== 'Travel Mac'; });
      }
      return rows;
    }
    /* One enrollment per yubi account. `list_yubi_accounts` (mock-bridge.ts:1055)
       answers with EVERY enrollment whatever account is selected — the alias is
       all the agent reports, which is exactly why the sheets say "Which
       connected serial belongs to this alias is not reported." So Security keys
       shows `primary key` under `work` as well as under `personal`; do not
       filter these by the selected store's server. */
    function enrollmentsOf(s) {
      var mode = val(s, 'yubistate', 'fixture');
      if (mode === 'none') return [];
      var rows = D.yubi.accounts.map(function (a) {
        return { alias: a.alias, state: a.state || 'complete' };
      });
      if (mode === 'pending') rows = rows.concat([{ alias: 'half-enrolled', state: 'pending' }]);
      return rows;
    }
    function cardsOf() { return D.yubi.cardsConnected; }
    /* The passive signed status the mock bridge answers with: personal is
       fresh, Acme's signed expiry is three days in the past whatever the
       lease world says, and a server with no pinned host has no snapshot. */
    function snapshotOf(w, profile) {
      var srv = w.serverById[profile];
      var state = srv ? srv.state : 'never-probed';
      if (state === 'never-probed' || state === 'lease-unavailable' || state === 'blocked') {
        return { host: false, leaseRequired: true, lease: 'unavailable' };
      }
      return { host: true, leaseRequired: true, lease: profile === 'acme' ? 'lapsed' : 'fresh' };
    }

    /* ================================================== 1. Recovery devices
       MacsSection (settings-screen.tsx:1114). */
    /* MacsSection's own empty state — unreachable with the fixture, which
       always lists two accounts (map §8.1). */
    function noAccountNotice() {
      return UI.notice({
        title: 'No available account on this Mac',
        body: '<p>Add and check a server, then create or recover an account.</p>',
      });
    }
    function macsBody(s) {
      var w = D.world(s);
      var store = M.settingsSelected(s, w);
      if (!store) return noAccountNotice();
      var stopped = M.settingsStopped(w, store);
      /* macsLoading (settings-screen.tsx:~660): selected, not unavailable, not
         stopped, and either the signed status or the device list is still in
         flight. Nothing is disabled by it — only the values are not there yet
         (map §3.5). */
      var loading = !stopped && isLoading(s);
      var account = D.accounts.filter(function (a) { return a.store === store.id; })[0];
      var devices = stopped || loading ? [] : devicesOf(s, store.id);
      var backups = stopped || loading ? [] : backupsOf(store.id);

      var deviceRows = loading
        ? UI.insetRow({ value: 'Loading devices…' })
        : devices.length
        ? devices.map(function (device) {
            var action = device.current
              ? UI.chip(device.id.indexOf('08') === 0 ? 'current security key' : 'this Mac', { tone: 'you' })
              : device.id.indexOf('04') === 0
                ? UI.btn('Remove…', {
                    size: 'sm', variant: 'danger', disabled: stopped,
                    attrs: 'data-act="call" data-fn="settingsRemoveOpen" data-arg="' + esc(device.id) + '"',
                  })
                : UI.chip('managed under Security keys');
            return UI.insetRow({
              label: esc(device.name || (device.id.indexOf('08') === 0 ? 'YubiKey' : 'Device')),
              value: '<b>' + esc(device.role) + '</b><small>' + esc(device.id) + '</small>',
              action: action,
            });
          }).join('')
        : UI.insetRow({
            label: 'None',
            value: stopped ? 'Not listed while access is stopped'
              : 'No devices were reported for this account.',
          });

      return h(
        UI.sectionLabel('This account'),
        UI.inset(
          UI.insetRow({ label: 'Signed in as', value: '<b>' + esc(account ? account.username : 'Identity unavailable') + '</b>' }) +
          UI.insetRow({ label: 'Local alias', value: esc(store.account) }),
          { className: 'settings-inset' }),
        stopped ? UI.band({
          severity: 'crit', label: 'Account access is stopped',
          text: 'A usable signed server check-in is unavailable. No account, recovery, pairing or security-key read or write is offered here.',
        }) : '',
        UI.sectionLabel('Your Macs'),
        UI.inset(deviceRows, { className: 'settings-inset' }),
        UI.sectionLabel('Pairing'),
        UI.inset(
          UI.insetRow({
            label: 'From this Mac',
            value: 'Get a pairing phrase to connect another device to your account.',
            action: UI.btn('Start pairing…', {
              variant: 'primary', disabled: stopped,
              attrs: 'data-act="go" data-page="set-devices" data-set=\'{"v.sheet":"pair-offer","v.seg":"","v.started":""}\'',
            }),
          }) +
          UI.insetRow({
            label: 'On this Mac',
            value: 'Type a pairing phrase from another FOKS device you use.',
            action: UI.btn('Accept or resume…', {
              disabled: stopped,
              attrs: 'data-act="go" data-page="set-devices" data-set=\'{"v.sheet":"pair-accept","v.seg":"","v.started":""}\'',
            }),
          }),
          { className: 'settings-inset' }),
        UI.sectionLabel('Recovery'),
        UI.inset(
          UI.insetRow({
            label: 'Backup phrase',
            value: loading ? 'Loading…'
              : backups.length
              ? esc(D.plural(backups.length, 'authenticated enrollment') + ' on this account')
              : 'No backup phrase for this account.',
            action: UI.btn(backups.length ? 'Enroll another…' : 'Enroll…', {
              disabled: stopped,
              attrs: 'data-act="go" data-page="set-devices" data-set=\'{"v.sheet":"phrase-prepare","v.written":""}\'',
            }),
          }) +
          UI.insetRow({
            label: 'Recover here',
            value: 'Use a backup phrase to recover another account.',
            action: UI.btn('Recover…', {
              disabled: stopped,
              attrs: 'data-act="go" data-page="set-devices" data-set=\'{"v.sheet":"recover"}\'',
            }),
          }),
          { className: 'settings-inset middle' }));
    }

    /* ==================================================== 2. Security keys
       KeysSection (settings-screen.tsx:1309). */
    var YUBI_ACTIONS = [
      ['sync', 'Sync', 'Update the key account; optionally include federation.'],
      ['pin-status', 'PIN status', 'Read remaining PIN attempts from the connected card.'],
      ['change-pin', 'Change PIN', 'Enter the current PIN and a new PIN.'],
      ['set-passphrase', 'Set passphrase', 'Set a new account passphrase.'],
      ['change-passphrase', 'Change passphrase', 'Enter the card PIN and a confirmed new passphrase.'],
      ['verify-passphrase', 'Verify passphrase', 'Test the passphrase with the server login challenge.'],
      ['unblock', 'Unblock PIN', 'Use the PUK to set a new PIN.'],
      ['change-puk', 'Change PUK', 'Set a new unlock code.'],
      ['recover-management', 'Recover management key', 'Use a software account on this Mac without a card PIN.'],
      ['recover-subkey', 'Recover subkey', 'Re-derive the FOKS subkey with the card PIN.'],
      ['resume-enrollment', 'Resume enrollment', 'Continue an interrupted enrollment.'],
      ['resume-rotation', 'Resume management rotation', 'Continue an interrupted rotation.'],
      ['rotate', 'Rotate management key', 'Replace the card management key.'],
    ];

    function keysBody(s) {
      var w = D.world(s);
      var store = M.settingsSelected(s, w);
      var stopped = M.settingsStopped(w, store);
      /* keysLoading = Boolean(selected && !unavailable && !stopped && …)
         (settings-screen.tsx:665), so a Mac with no account is not "loading"
         — and the effect that fills these lists empties them outright when
         there is no profile to read (settings-screen.tsx:519-524). Hence the
         `!store` arm: no account means no enrollment and no card, not the
         fixture's. */
      var loading = !!store && !stopped && isLoading(s);
      var yubi = !store || stopped || loading ? [] : enrollmentsOf(s);
      var cards = !store || stopped || loading ? [] : cardsOf();
      var hasComplete = yubi.some(function (e) { return e.state === 'complete'; });
      var hasPending = yubi.some(function (e) { return e.state === 'pending'; });

      var enrolled = loading
        ? UI.insetRow({ value: 'Loading keys…' })
        : yubi.length
        ? yubi.map(function (item) {
            return UI.insetRow({
              label: esc(item.alias),
              value: 'Which connected serial belongs to this alias is not reported.',
              action: UI.chip(esc(item.state)),
            });
          }).join('')
        : UI.insetRow({ label: 'None', value: 'No YubiKey enrollment was reported.' });
      var connected = loading
        ? UI.insetRow({ value: 'Loading keys…' })
        : cards.length
        ? cards.map(function (card) {
            return UI.insetRow({
              label: 'YubiKey ' + esc(card.serial),
              value: UI.chip('Connected', { tone: 'ok' }),
            });
          }).join('')
        : UI.insetRow({ label: 'None', value: 'No card is connected right now.' });

      var rows = YUBI_ACTIONS.map(function (entry) {
        var id = entry[0], label = entry[1], copy = entry[2];
        var available = !stopped && !loading &&
          (id === 'resume-enrollment' ? hasPending : hasComplete);
        var title = available ? undefined
          : stopped ? 'Server access is stopped'
            : loading ? 'Reading this account…'
              : id === 'resume-enrollment' ? 'No pending enrollment was reported'
                : 'No complete enrollment was reported';
        return UI.insetRow({
          label: esc(label), value: esc(copy),
          action: UI.btn(esc(label.indexOf(' ') >= 0 ? 'Open…' : label), {
            size: 'sm', disabled: !available, title: title,
            attrs: 'data-act="go" data-page="set-keys" data-set=\'{"v.sheet":"yubi-' + id + '"}\'',
          }),
        });
      }).join('');

      return h(
        UI.sectionLabel('Enrolled keys'),
        UI.inset(enrolled, { className: 'settings-inset' }),
        UI.sectionLabel('Connected now'),
        UI.inset(connected, { className: 'settings-inset' }),
        stopped ? UI.band({
          severity: 'crit', label: 'Security-key access is stopped',
          text: 'A usable signed server check-in is unavailable. Controls stay inert until it is available.',
        }) : '',
        UI.sectionLabel('Add'),
        UI.inset(
          UI.insetRow({
            label: 'New account', value: 'Keys live on the card from the start.',
            action: UI.btn('Create on YubiKey…', {
              variant: 'primary', disabled: stopped,
              attrs: 'data-act="go" data-page="set-keys" data-set=\'{"v.sheet":"enrol"}\'',
            }),
          }) +
          UI.insetRow({
            label: 'Existing account',
            value: 'Make a currently connected card an owner device for a software account on this Mac.',
            action: UI.btn('Provision…', {
              disabled: stopped,
              attrs: 'data-act="go" data-page="set-keys" data-set=\'{"v.sheet":"provision"}\'',
            }),
          }),
          { className: 'settings-inset' }),
        UI.sectionLabel('Everyday and recovery'),
        UI.inset(rows, { className: 'settings-inset' }),
        UI.sectionLabel('Danger', { className: 'danger-title' }),
        UI.inset(UI.insetRow({
          label: 'Revoke YubiKey', value: 'Rotates account keys controlled by this YubiKey.',
          action: UI.btn('Revoke…', {
            variant: 'danger', disabled: stopped || !hasComplete,
            attrs: 'data-act="go" data-page="set-keys" data-set=\'{"v.sheet":"revoke"}\'',
          }),
        }), { className: 'danger-box settings-inset' }));
    }

    /* ========================================================= 3. Accounts
       AccountSection (settings-screen.tsx:1497). */
    /* `pending` is !loadedProfiles.has(store.server): the signed status has not
       come back yet, so nothing about this account is known. It short-circuits
       `stopped`'s status half, wins the chip label, and makes the passphrase
       buttons inert without the "stopped" wording (settings-screen.tsx:1551). */
    function accountStatus(w, store, pending) {
      var srv = w.serverById[store.server];
      var state = srv ? srv.state : 'never-probed';
      var snap = snapshotOf(w, store.server);
      var inventoryUnavailable = w.unavailableStores.indexOf(store.id) >= 0;
      /* While the profile is pending there is no snapshot at all, and
         `serverLeaseState(undefined)` is `unavailable`, not `lapsed`
         (model/lease.ts:45) — so only the server's own state can call an
         account lapsed until the signed status lands. */
      var lapsed = state === 'lease-lapsed' || (!pending && snap.host && snap.lease === 'lapsed');
      var stopped = inventoryUnavailable || lapsed || state === 'blocked' ||
        state === 'never-probed' || (!pending && (!snap.host || snap.lease !== 'fresh'));
      return {
        server: srv, stopped: stopped, pending: !!pending, inert: stopped || !!pending,
        label: pending ? 'Reading check-in…'
          : inventoryUnavailable ? 'connection error'
            : lapsed ? 'server check-in lapsed'
              : stopped ? 'check-in status unknown'
                : snap.leaseRequired === false ? 'check-in not required'
                  : 'signed check-in available',
      };
    }
    function accountBody(s) {
      var w = D.world(s);
      var stores = M.settingsAccounts(w);
      if (!stores.length) {
        return UI.notice({
          title: 'No available account on this Mac',
          body: '<p>Add and check a server, then create or recover an account, or connect one from the official CLI.</p>',
          actions: UI.btn('Connect from FOKS CLI…', {
            variant: 'primary',
            attrs: 'data-act="set" data-key="v.sheet" data-val="go-profile"',
          }),
        });
      }
      return h(
        UI.sectionLabel('Accounts on this Mac', {
          action: UI.btn('Connect from FOKS CLI…', {
            size: 'sm', attrs: 'data-act="set" data-key="v.sheet" data-val="go-profile"',
          }),
        }),
        stores.map(function (store) {
          var st = accountStatus(w, store, isLoading(s));
          var srv = st.server;
          /* accountDeviceNames (settings-screen.tsx:548): the CURRENT device of
             each account, named `Security key` when its id starts 08 — not
             `YubiKey`, which is the Recovery-devices fallback. */
          var current = st.stopped || st.pending ? null
            : devicesOf(s, store.id).filter(function (d) { return d.current; })[0];
          function pass(mode, label) {
            return UI.btn(esc(label), {
              size: 'sm', disabled: st.inert,
              attrs: 'data-act="call" data-fn="settingsPassphraseOpen" data-arg="' + mode + '|' + esc(store.id) + '"',
            });
          }
          return '<div>' + UI.sectionLabel(
            esc(srv ? srv.name : store.server) + (srv && srv.label ? ' · ' + esc(srv.label) : '')) +
            UI.inset(
              UI.insetRow({
                label: 'Username',
                value: '<b>' + esc(D.accounts.filter(function (a) { return a.store === store.id; }).map(function (a) { return a.username; })[0] || 'Identity unavailable') + '</b> ' +
                  UI.chip(esc(st.label), { tone: st.stopped ? 'warn' : 'default' }),
              }) +
              UI.insetRow({
                label: 'Account alias',
                value: esc(store.account) + '<small>The local name; the server never sees it.</small>',
              }) +
              UI.insetRow({
                label: 'Device',
                value: (st.pending ? 'Loading…'
                  : st.stopped ? 'Not listed while access is stopped'
                    : esc(current
                      ? (current.name || (current.id.indexOf('08') === 0 ? 'Security key' : 'Device'))
                      : 'No current device was reported')) +
                  '<small>This account’s current authenticated device. All devices are under Recovery devices.</small>',
              }) +
              UI.insetRow({
                label: 'Passphrase',
                value: '<span class="a">' + pass('set', 'Set…') + pass('change', 'Change…') + pass('verify', 'Verify') + '</span>' +
                  '<small>Whether one is set is not reported. ' +
                  (st.stopped
                    ? 'Nothing can be set, changed or verified until server access is available.'
                    : st.pending
                      ? 'Waiting for a signed check-in before passphrase actions are offered.'
                      : 'Verify runs the server public login challenge.') + '</small>',
              }),
              { className: 'settings-inset' }) + '</div>';
        }).join(''));
    }

    /* ============================================================ 4. About
       AboutSection → AgentSection (settings-screen.tsx:1727 / 1645). */
    function aboutBody(s) {
      var w = D.world(s);
      var ready = w.agent.phase === 'Ready';
      /* `appInfo === null` until app_info answers: the Socket value reads
         "Reading app info…" and loses its Copy button, and Version is the bare
         ellipsis (map §3.5). */
      var info = isLoading(s) ? null : D.appInfo;
      return h(
        UI.sectionLabel('Agent'),
        UI.inset(
          UI.insetRow({
            label: 'Status',
            value: '<span class="' + (ready ? 'agent' : 'agent warn') + '"><i></i>' + esc(w.agent.phase) + '</span>' +
              '<small>FOKS is unavailable until the agent is ready.</small>',
          }) +
          UI.insetRow({
            label: 'Socket', valueClass: 'mono',
            value: esc(info ? info.agentSocket : 'Reading app info…') +
              '<small>Local connection used by this desktop app.</small>',
            action: info ? UI.btn('Copy', {
              size: 'sm', attrs: 'data-act="toast" data-text="Socket path copied"',
            }) : undefined,
          }) +
          UI.insetRow({
            label: 'Connection',
            value: ready ? 'Connected.'
              : 'Reconnect to the local agent. Interrupted changes will not be repeated.',
            action: ready ? undefined : UI.btn('Retry connection', {
              variant: 'primary', attrs: 'data-act="call" data-fn="settingsConnectionRetry"',
            }),
          }),
          { className: 'settings-inset' }),
        UI.toggle({
          label: 'Inspect AgentStatus',
          open: s.v.advanced === '1',
          body: '<pre>' + esc(JSON.stringify({ phase: w.agent.phase }, null, 2)) + '</pre>',
          attrs: 'data-act="set" data-key="v.advanced" data-val="' + (s.v.advanced === '1' ? '' : '1') + '"',
        }),
        UI.sectionLabel('About'),
        UI.inset(UI.insetRow({
          label: 'Version',
          value: 'FOKS Desktop ' + esc(info ? info.version : '…') +
            '<small>Reported by this installed application.</small>',
        }), { className: 'settings-inset' }));
    }

    /* =============================================== 5. Unavailable account
       UnavailableAccount (settings-screen.tsx:1070). It replaces the pane in
       Recovery devices and Security keys, and sits ABOVE it in Accounts,
       Servers, Groups and About (settings-screen.tsx:704-712). Its account
       buttons run `go(section, store.id)` — the section does not change, so
       the target page is whichever Settings page is showing. */
    function unavailableBody(s) {
      var w = D.world(s);
      var stores = M.settingsAccounts(w);
      var here = MINE[s.page] ? (s.page === 'set-unavailable' ? 'set-devices' : s.page) : 'set-devices';
      return UI.notice({
        severity: 'crit',
        title: 'This account is no longer available in the current catalog',
        body: '<p>The address names an account this Mac does not list. No other account has been selected in its place.</p>' +
          (stores.length ? '' : '<p>This Mac has no available account at all. Add and check a server, then create or recover an account.</p>'),
        actions: UI.btn('Refresh the catalog', {
          variant: 'primary', attrs: 'data-act="toast" data-text="Refreshed the catalog"',
        }) + stores.map(function (store) {
          var srv = w.serverById[store.server];
          return UI.btn(esc(store.account) + ' · ' + esc(srv ? srv.name : store.server), {
            attrs: 'data-act="go" data-page="' + here + '" data-set=\'{"v.account":"' + store.id + '"}\'',
          });
        }).join(''),
      });
    }

    /* ============================================================= sheets */
    function subjectOf(s) {
      var w = D.world(s);
      var store = M.settingsSelected(s, w) || M.settingsAccounts(w)[0];
      var srv = store ? w.serverById[store.server] : null;
      var account = store ? D.accounts.filter(function (a) { return a.store === store.id; })[0] : null;
      return { w: w, store: store, server: srv, account: account };
    }

    function phrasePrepareSheet(s) {
      var x = subjectOf(s);
      var alias = val(s, 'alias', 'paper-backup');
      return sheet({
        title: 'Enroll a backup phrase',
        subtitle: esc(x.account ? x.account.username : '') + ' on ' + esc(x.server ? x.server.name : ''),
        body: UI.inset(field({ key: 'alias', label: 'Backup alias', value: alias })),
        footer: closeBtn('Cancel') + UI.btn('Prepare phrase', {
          variant: 'primary', disabled: !String(alias).trim(),
          attrs: 'data-act="go" data-page="set-devices" data-set=\'{"v.sheet":"phrase","v.written":""}\'',
        }),
      });
    }
    function phraseSheet(s) {
      var x = subjectOf(s);
      var written = s.v.written === '1';
      return sheet({
        dismissible: false,
        title: 'Write these 17 tokens down',
        subtitle: esc(x.account ? x.account.username : '') + ' on ' + esc(x.server ? x.server.name : ''),
        body: '<p>This phrase is shown once and cannot be copied. Write it down now. The agent stores only the public key.</p>' +
          '<div class="words">' + D.backupPhraseWords.map(function (word, i) {
            return '<span class="word"><i>' + (i + 1) + '</i>' + esc(word) + '</span>';
          }).join('') + '</div>' +
          '<label class="checkline" data-act="set" data-key="v.written" data-val="' + (written ? '' : '1') + '">' +
          '<input type="checkbox"' + (written ? ' checked' : '') + '>I have written these down</label>',
        footer: UI.btn('Done', {
          variant: 'primary', disabled: !written,
          attrs: 'data-act="call" data-fn="settingsPhraseDone"',
        }),
      });
    }
    function pairSheet(s) {
      var x = subjectOf(s);
      var mode = val(s, 'seg', s.v.sheet === 'pair-accept' ? 'accept' : 'offer');
      var started = s.v.started === '1';
      var alias = val(s, 'alias', x.store ? x.store.account : '');
      var device = val(s, 'dev', 'This Mac');
      var phrase = val(s, 'phrase', '');
      /* Changing direction is not just a mode flip: PairSheet's onChange clears
         the typed phrase on the way to `offer` and the revealed offer on the way
         to `accept` (settings-screen.tsx:2040), so neither half carries the
         other's secret. */
      var body = UI.segmented({
        label: 'Pairing direction', value: mode,
        items: [
          { id: 'offer', label: 'From this Mac', attrs: 'data-act="call" data-fn="settingsPairMode" data-arg="offer"' },
          { id: 'accept', label: 'On this Mac', attrs: 'data-act="call" data-fn="settingsPairMode" data-arg="accept"' },
        ],
      });
      if (mode === 'offer') {
        body += '<p>Select Start, enter the pairing phrase on the other Mac, then select Finish here. Resume opens the pending offer.</p>' +
          (started ? UI.inset(UI.insetRow({ label: 'Pairing phrase', valueClass: 'mono', value: 'cobalt window' })) : '');
      } else {
        body += '<p>Enter the pairing phrase from the other Mac. Resume continues a pending acceptance.</p>' +
          UI.inset(
            field({ key: 'alias', label: 'Account alias', value: alias }) +
            field({ key: 'dev', label: 'Device name', value: device }) +
            field({ key: 'phrase', label: 'Pairing phrase', type: 'password', value: phrase }));
      }
      var footer = closeBtn('Close') + UI.spacer();
      if (mode === 'offer') {
        footer += UI.btn('Resume offer', { attrs: 'data-act="call" data-fn="settingsPairResume" data-arg="offer"' }) +
          UI.btn('Finish', {
            disabled: !started,
            attrs: 'data-act="call" data-fn="settingsPairDone" data-arg="Pairing finished; refreshed authenticated devices"',
          }) +
          UI.btn('Start', {
            variant: 'primary', attrs: 'data-act="set" data-key="v.started" data-val="1"',
          });
      } else {
        footer += UI.btn('Resume acceptance', {
          disabled: !alias,
          attrs: 'data-act="call" data-fn="settingsPairResume" data-arg="accept"',
        }) + UI.btn('Accept', {
          variant: 'primary', disabled: !(alias && device && phrase),
          attrs: 'data-act="call" data-fn="settingsPairDone" data-arg="Pairing accepted; refreshed authenticated devices"',
        });
      }
      return sheet({
        title: 'Set up another Mac', subtitle: 'Start or resume pairing a device',
        body: body, footer: footer,
      });
    }
    function recoverSheet(s) {
      var x = subjectOf(s);
      var alias = val(s, 'alias', x.store ? x.store.account : '');
      var device = val(s, 'dev', 'This Mac');
      var tokens = val(s, 'tokens', '');
      return sheet({
        title: 'Recover on this Mac', subtitle: 'Use the 17-token backup phrase',
        body: '<p>Recovery adds this Mac as a new owner device. If interrupted, resume it from Alerts with the same phrase.</p>' +
          UI.inset(
            field({ key: 'alias', label: 'Local alias', value: alias }) +
            field({ key: 'dev', label: 'Device name', value: device }) +
            UI.insetRow({
              label: '17 tokens', forId: 'sf-tokens',
              value: '<textarea id="sf-tokens" data-bind="v.tokens" data-live>' + esc(tokens) + '</textarea>',
            })),
        footer: closeBtn('Cancel') + UI.btn('Recover', {
          variant: 'primary', disabled: !(alias && device && tokens),
          attrs: 'data-act="call" data-fn="settingsRecover"',
        }),
      });
    }
    function removeDeviceSheet(s) {
      var x = subjectOf(s);
      var rows = devicesOf(s, x.store ? x.store.id : '');
      var id = s.v.device || '';
      /* The row's Remove… names the device; the deck's Sheet chip does not,
         so fall back to the one row that offers removal (RemoveDeviceSheet is
         only ever mounted with a non-current 04… device). */
      var device = rows.filter(function (d) { return d.id === id; })[0] ||
        (id ? null : rows.filter(function (d) { return !d.current && d.id.indexOf('04') === 0; })[0]);
      if (!device) return '';
      var expected = device.name || device.id;
      var typed = s.v.typed === '1' ? expected : val(s, 'confirm', '');
      return sheet({
        danger: true,
        title: 'Remove ' + esc(device.name || 'device') + '?',
        subtitle: esc(x.store.account) + ' · ' + esc(device.id),
        body: '<p>This device loses future access to the account. Copies of values it already read cannot be recalled; rotate those secrets if the device is not under your control.</p>' +
          '<p class="fn">This action is available only for a non-current software device. YubiKeys are revoked under Security keys.</p>' +
          UI.inset(inputRow({ key: 'confirm', label: 'Confirm', placeholder: 'type ' + expected, value: typed })),
        footer: closeBtn('Cancel') + UI.btn('Remove device', {
          variant: 'danger', disabled: typed !== expected,
          attrs: 'data-act="call" data-fn="settingsRemoveDevice" data-arg="' + esc(device.id) + '"',
        }),
      });
    }
    function enrolSheet(s) {
      var x = subjectOf(s);
      var card = cardsOf()[0];
      var alias = val(s, 'alias', 'work-key');
      var username = val(s, 'username', '');
      var device = val(s, 'dev', card ? 'YubiKey ' + card.serial : 'YubiKey');
      var pin = val(s, 'pin', ''), puk = val(s, 'puk', '');
      var slot1 = val(s, 'slot1', '0x82'), slot2 = val(s, 'slot2', '0x83');
      var tries1 = val(s, 'tries1', '3'), tries2 = val(s, 'tries2', '3');
      var slotOk = /^0x[0-9a-fA-F]{2}$/.test(slot1) && /^0x[0-9a-fA-F]{2}$/.test(slot2) && slot1 !== slot2;
      var triesOk = +tries1 > 0 && +tries1 <= 255 && +tries2 > 0 && +tries2 <= 255;
      return sheet({
        title: 'Create a YubiKey account',
        subtitle: 'A new account on ' + esc(x.store ? x.store.server : '') + '; keys live on the card from the start',
        body: '<p>Before you continue:</p>' +
          '<ol class="sheet-steps">' +
          '<li><b>FOKS writes both keys to the card in one step.</b> If that step fails, resetting the PIV applet erases the card.</li>' +
          '<li><b>The card must use its factory management key.</b> A managed card cannot be used.</li>' +
          '<li><b>Save the unlock code you choose.</b> FOKS cannot recover it later.</li>' +
          '</ol>' +
          (card
            ? '<p><b>YubiKey ' + esc(card.serial) + '</b> is connected. FOKS cannot identify an existing alias from the card serial.</p>'
            : UI.band({ label: 'Connect a YubiKey.', text: 'No card is connected.' })) +
          UI.inset(
            field({ key: 'alias', label: 'Alias', value: alias }) +
            field({ key: 'username', label: 'Username', value: username }) +
            field({ key: 'dev', label: 'Device name', value: device }) +
            field({ key: 'pin', label: 'Card PIN', type: 'password', value: pin }) +
            field({ key: 'puk', label: 'Unlock code', type: 'password', value: puk }) +
            inputRow({ key: 'invite', label: 'Invite', type: 'password', value: val(s, 'invite', ''), hint: 'Optional; cleared on submission.' })) +
          '<details class="adv"' + (s.v.advanced === '1' ? ' open' : '') + '>' +
          '<summary data-act="set" data-key="v.advanced" data-val="' + (s.v.advanced === '1' ? '' : '1') + '">Advanced</summary>' +
          UI.inset(
            field({ key: 'slot1', label: 'Signing slot', mono: true, value: slot1 }) +
            field({ key: 'slot2', label: 'Second slot', mono: true, value: slot2 }) +
            inputRow({ key: 'tries1', label: 'PIN tries', type: 'number', min: 1, max: 255, value: tries1 }) +
            inputRow({ key: 'tries2', label: 'PUK tries', type: 'number', min: 1, max: 255, value: tries2 })) +
          '</details>' +
          '<p class="hint">PIN, unlock code and invite go only to the local agent and are cleared when submitted.</p>',
        footer: closeBtn('Cancel') + UI.btn('Prepare card and create account', {
          variant: 'primary',
          disabled: !(card && String(alias).trim() && String(username).trim() && String(device).trim() && pin && puk && slotOk && triesOk),
          attrs: 'data-act="call" data-fn="settingsEnrol"',
        }),
      });
    }
    function provisionSheet(s) {
      var x = subjectOf(s);
      var cards = cardsOf();
      var alias = val(s, 'alias', 'new-key');
      var device = val(s, 'dev', cards[0] ? 'YubiKey ' + cards[0].serial : 'YubiKey');
      var pin = val(s, 'pin', ''), puk = val(s, 'puk', '');
      return sheet({
        title: 'Provision a YubiKey device',
        subtitle: 'A fresh local alias on ' + esc(x.store ? x.store.account : ''),
        body: '<p>Enter a new alias and select a connected card.</p>' +
          (cards.length ? '' : UI.band({ label: 'Connect a YubiKey.', text: 'No card is connected.' })) +
          UI.inset(
            field({ key: 'alias', label: 'Target alias', value: alias }) +
            field({ key: 'dev', label: 'Device name', value: device }) +
            (cards.length ? UI.insetRow({
              label: 'Connected card', forId: 'sf-serial',
              value: '<select id="sf-serial" data-bind="v.serial">' + cards.map(function (card) {
                return '<option value="' + esc(card.serial) + '">YubiKey ' + esc(card.serial) + '</option>';
              }).join('') + '</select>',
            }) : '') +
            field({ key: 'pin', label: 'Card PIN', type: 'password', value: pin }) +
            field({ key: 'puk', label: 'Unlock code', type: 'password', value: puk })),
        footer: closeBtn('Cancel') + UI.btn('Provision card', {
          variant: 'primary', disabled: !(cards.length && String(alias).trim() && String(device).trim() && pin && puk),
          attrs: 'data-act="call" data-fn="settingsProvision"',
        }),
      });
    }
    function yubiSheet(s, action) {
      var list = enrollmentsOf(s);
      var alias = (action === 'resume-enrollment'
        ? list.filter(function (e) { return e.state === 'pending'; })[0]
        : list.filter(function (e) { return e.state === 'complete'; })[0]);
      alias = alias ? alias.alias : '';
      var needsPin = ['pin-status', 'recover-management'].indexOf(action) < 0;
      var needsOther = ['change-pin', 'set-passphrase', 'change-passphrase',
        'verify-passphrase', 'unblock', 'change-puk'].indexOf(action) >= 0;
      var needsConfirm = action === 'set-passphrase' || action === 'change-passphrase';
      var firstLabel = (action === 'unblock' || action === 'change-puk') ? 'Current PUK' : 'Card PIN';
      var otherLabel = (action === 'change-pin' || action === 'unblock') ? 'New PIN'
        : action === 'change-puk' ? 'New PUK' : 'Passphrase';
      var pin = val(s, 'pin', ''), other = val(s, 'other', ''), confirm = val(s, 'confirm', '');
      var valid = !!(alias &&
        (!needsPin || action === 'resume-rotation' || pin) &&
        (!needsOther || other) &&
        (!needsConfirm || other === confirm));
      return sheet({
        title: esc(action.split('-').join(' ')),
        subtitle: esc(alias || 'Choose an enrolled key alias'),
        body: '<p>The values below go only to the local agent and are cleared when submitted.</p>' +
          UI.inset(
            UI.insetRow({ label: 'Key alias', value: '<b>' + esc(alias || 'No key enrolled') + '</b>' }) +
            (needsPin ? inputRow({ key: 'pin', label: firstLabel, type: 'password', value: pin }) : '') +
            (needsOther ? inputRow({ key: 'other', label: otherLabel, type: 'password', value: other }) : '') +
            (needsConfirm ? field({ key: 'confirm', label: 'Confirm', type: 'password', value: confirm }) : '')),
        footer: closeBtn('Cancel') + UI.btn('Continue', {
          variant: 'primary', disabled: !valid,
          attrs: 'data-act="call" data-fn="settingsYubiRun"',
        }),
      });
    }
    function revokeSheet(s) {
      var complete = enrollmentsOf(s).filter(function (e) { return e.state === 'complete'; })[0];
      if (!complete) return '';
      var alias = complete.alias;
      var typed = s.v.typed === '1' ? alias : val(s, 'confirm', '');
      return sheet({
        danger: true,
        title: 'Revoke ' + esc(alias) + '?', subtitle: 'Rotates affected account keys',
        body: '<p>The card will no longer open the account. Copies it already read cannot be recalled. The alias is used because the agent does not report which connected serial belongs to it.</p>' +
          UI.inset(inputRow({ key: 'confirm', label: 'Confirm', placeholder: 'type ' + alias, value: typed })),
        footer: closeBtn('Cancel') + UI.btn('Revoke ' + esc(alias), {
          variant: 'danger', disabled: typed !== alias,
          attrs: 'data-act="call" data-fn="settingsRevoke" data-arg="' + esc(alias) + '"',
        }),
      });
    }
    function passphraseSheet(s) {
      var mode = val(s, 'seg', 'set');
      var pw = val(s, 'pw', ''), pw2 = val(s, 'pw2', '');
      return sheet({
        title: 'Account passphrase', subtitle: 'Whether one is currently set is not reported',
        body: UI.segmented({
          label: 'Passphrase action', value: mode, key: 'v.seg',
          items: [{ id: 'set', label: 'Set' }, { id: 'change', label: 'Change' }, { id: 'verify', label: 'Verify' }],
        }) + UI.inset(
          field({ key: 'pw', label: 'Passphrase', type: 'password', value: pw }) +
          (mode === 'verify' ? '' : field({ key: 'pw2', label: 'Confirm', type: 'password', value: pw2 }))),
        footer: closeBtn('Cancel') + UI.btn(
          mode === 'verify' ? 'Verify' : mode === 'set' ? 'Set passphrase' : 'Change passphrase', {
            variant: 'primary',
            disabled: !pw || (mode !== 'verify' && pw !== pw2),
            attrs: 'data-act="call" data-fn="settingsPassphraseRun" data-arg="' + mode + '"',
          }),
      });
    }
    /* go-profile-connect.tsx:36 — a plain dialog at the base width, with no
       glyph. The mock bridge's discoverGoProfiles() always answers
       { installed:false, candidates:[] }, so this is the only body it has. */
    function goProfileSheet() {
      return sheet({
        width: 'base', glyph: '',
        title: 'Connect from FOKS CLI',
        subtitle: 'Use an account already configured by the official client',
        body: UI.inset('<p>The official FOKS client was not found in its standard location.</p>' +
          UI.btn('Scan again', { attrs: 'data-act="hint" data-text="discoverGoProfiles() — the mock always answers { installed:false, candidates:[] }."' })),
        footer: closeBtn('Cancel'),
      });
    }

    /* Two things take every Settings sheet down without anyone pressing Cancel:
       an agent-connection loss, which remounts the whole screen through
       `key={settings:${concealSignal}}` (app-root.tsx:566, map §1.1), and the
       selected account going stopped, which runs `setSheet(null)` outright
       (settings-screen.tsx:623). Both are app-wide chips here, so the sheet chip
       can be lit while neither the app nor this mock draws a sheet. */
    function sheetConcealed(s) {
      if (s.agent === 'lost') return true;
      var w = D.world(s);
      return M.settingsStopped(w, M.settingsSelected(s, w));
    }
    function overlayFor(s) {
      var open = s.v.sheet;
      if (!open || sheetConcealed(s)) return '';
      if (open.indexOf('yubi-') === 0) return yubiSheet(s, open.slice(5));
      switch (open) {
        case 'phrase-prepare': return phrasePrepareSheet(s);
        case 'phrase': return phraseSheet(s);
        case 'pair-offer': case 'pair-accept': return pairSheet(s);
        case 'recover': return recoverSheet(s);
        case 'remove-device': return removeDeviceSheet(s);
        case 'enrol': return enrolSheet(s);
        case 'provision': return provisionSheet(s);
        case 'revoke': return revokeSheet(s);
        case 'passphrase': return passphraseSheet(s);
        case 'go-profile': return goProfileSheet(s);
        default: return '';
      }
    }

    /* ============================================================ handlers */
    var SHEET_KEYS = ['sheet', 'seg', 'started', 'written', 'typed', 'device',
      'alias', 'dev', 'phrase', 'tokens', 'username', 'pin', 'puk', 'other',
      'confirm', 'pw', 'pw2', 'invite'];
    function clearSheet() {
      SHEET_KEYS.forEach(function (k) { M.set('v.' + k, null, { silent: true }); });
    }
    M.fns.settingsClose = function () { clearSheet(); M.render(); };

    M.fns.settingsRemoveOpen = function (id) {
      M.set('v.device', id, { silent: true });
      M.set('v.confirm', null, { silent: true });
      M.set('v.typed', null, { silent: true });
      M.set('v.sheet', 'remove-device');
    };
    M.fns.settingsRemoveDevice = function (id) {
      var device = devicesOf(M.s, 'acct:personal').filter(function (d) { return d.id === id; })[0];
      for (var i = 0; i < D.devices.length; i++) {
        if ('04' + D.devices[i].id_hex.slice(2) === id) { D.devices.splice(i, 1); break; }
      }
      clearSheet();
      M.set('v.applied', '1', { silent: true });
      M.render();
      M.toast('Removed ' + ((device && device.name) || 'device') + ' from this account');
    };
    M.fns.settingsPhraseDone = function () {
      var store = M.settingsSelected(M.s, D.world(M.s));
      if (store) backupsOf(store.id).push({ alias: M.s.v.alias || 'paper-backup' });
      clearSheet();
      M.render();
      M.toast('Backup phrase enrolled; the secret was cleared from this window');
    };
    /* SegmentedControl label="Pairing direction" (settings-screen.tsx:2040). */
    M.fns.settingsPairMode = function (mode) {
      if (mode === 'offer') M.set('v.phrase', null, { silent: true });
      else M.set('v.started', null, { silent: true });
      M.set('v.seg', mode);
    };
    /* resumeDevicePairingOffer / resumeDevicePairingAcceptance. The agent holds
       an offer only after `start_device_pairing` put one there (mock-bridge.ts:937
       fills `pairingOffers`), so Resume re-reveals the SAME phrase once Start has
       run in this sheet — `v.started` is that offer. With nothing pending it
       answers `pending-operation-not-found`, whose message arrives as a red toast
       over the still-open sheet (map §5.2). */
    M.fns.settingsPairResume = function (which) {
      var held = M.s.v.pending === '1' || (which === 'offer' && M.s.v.started === '1');
      if (!held) {
        return M.toast(which === 'offer'
          ? 'That pairing offer is no longer pending.'
          : 'That pairing acceptance is no longer pending.', { tone: 'warning' });
      }
      if (which === 'offer') return M.set('v.started', '1');
      M.fns.settingsPairDone('Pairing acceptance resumed; refreshed authenticated devices');
    };
    M.fns.settingsPairDone = function (message) {
      clearSheet();
      M.render();
      M.toast(message);
    };
    M.fns.settingsRecover = function () {
      clearSheet();
      M.render();
      M.toast('Recovery submitted; refresh and review the authenticated device list');
    };
    M.fns.settingsEnrol = function () {
      var alias = M.s.v.alias || 'work-key';
      var store = M.settingsSelected(M.s, D.world(M.s));
      D.yubi.accounts.push({
        alias: alias, server: store ? store.server : 'personal',
        serial: cardsOf()[0] ? cardsOf()[0].serial : 0, state: 'complete',
      });
      clearSheet();
      M.render();
      M.toast('YubiKey account created; refreshed authenticated key lists');
    };
    M.fns.settingsProvision = function () {
      var alias = M.s.v.alias || 'new-key';
      var store = M.settingsSelected(M.s, D.world(M.s));
      D.yubi.accounts.push({
        alias: alias, server: store ? store.server : 'personal',
        serial: cardsOf()[0] ? cardsOf()[0].serial : 0, state: 'complete',
      });
      clearSheet();
      M.render();
      M.toast('YubiKey device provisioned; refreshed authenticated lists');
    };
    M.fns.settingsYubiRun = function () {
      clearSheet();
      M.render();
      M.toast('Security-key action completed; refreshed authenticated lists');
    };
    M.fns.settingsRevoke = function (alias) {
      D.yubi.accounts = D.yubi.accounts.filter(function (a) { return a.alias !== alias; });
      clearSheet();
      M.render();
      M.toast('YubiKey revoked and affected account keys rotated');
    };
    M.fns.settingsPassphraseOpen = function (arg) {
      var parts = String(arg).split('|');
      M.set('v.seg', parts[0], { silent: true });
      M.set('v.pw', null, { silent: true });
      M.set('v.pw2', null, { silent: true });
      M.set('v.sheet', 'passphrase');
    };
    /* mock-bridge: generation 1 for set, 2 for change and verify. */
    M.fns.settingsPassphraseRun = function () {
      var mode = M.s.v.seg || 'set';
      clearSheet();
      M.render();
      M.toast('Passphrase ' + mode + ' succeeded · generation ' + (mode === 'set' ? 1 : 2));
    };
    M.fns.settingsConnectionRetry = function () {
      M.set('agent', 'ready');
      M.toast('Connection retry: Ready');
    };

    /* A section button: the sheet and everything typed into it goes, the
       account stays — unless it no longer resolves, in which case the address
       loses it (settings-screen.tsx:642) and the rewrite below picks the
       first account. */
    M.fns.settingsNav = function (page) {
      clearSheet();
      if (M.s.v.account && !M.settingsSelected(M.s)) M.set('v.account', null, { silent: true });
      /* `go(next, selected?.id)` always puts the StoreRef back in the address,
         section by section — the real app's URL reads
         `?state=settings&section=servers&store=acct:work` — so the ref is
         handed to the destination as a patch rather than left to survive
         M.go's key drop: Servers and Groups do not declare it, and without
         this a Recovery devices → Servers → Recovery devices round-trip would
         come back on the first account instead of the one that was chosen. */
      var account = M.s.v.account;
      M.go(page, account ? { 'v.account': account } : null);
    };
    /* settings-screen.tsx:337 — an address with no store= is rewritten to
       stores[0].id. Run after the render, so the deck and the URL agree with
       what the pane actually selected. set-unavailable never does this: its
       address names a StoreRef on purpose. */
    function selectDefaultAccount(s) {
      M.fns.escape = ownEscape;
      var accounts = M.settingsAccounts();
      /* With no account to select there is no StoreRef to write: the address
         keeps none and the pane draws its "No available account" notice. */
      if (!accounts.length) {
        if (s.v.account != null) M.set('v.account', null);
        return;
      }
      if (s.v.account == null) M.set('v.account', accounts[0].id);
    }
    /* The pane a Settings section shows when the address names a store the
       catalog does not list: Recovery devices and Security keys are replaced
       by the stale-link notice, every other section renders under it
       (settings-screen.tsx:704). */
    function sectionBody(s, build, replaced) {
      var w = D.world(s);
      if (s.v.account && !M.settingsSelected(s, w)) {
        return {
          body: unavailableBody(s) + (replaced === false ? build(s) : ''),
          switcher: false,
        };
      }
      return { body: build(s), switcher: true };
    }

    var shellEscape = M.fns.escape;
    function ownEscape() {
      if (MINE[M.s.page]) {
        /* PhraseSheet is dismissible={false} once the phrase is showing, so
           neither Escape nor the backdrop closes it — only Done. */
        if (M.s.v.sheet === 'phrase') return;
        if (M.s.v.sheet) return M.fns.settingsClose();
        return;
      }
      shellEscape();
    }
    var MINE = { 'set-devices': 1, 'set-keys': 1, 'set-account': 1, 'set-about': 1, 'set-unavailable': 1 };

    /* ============================================================== pages */
    var SHEET_VALUES_MACS = [
      { v: '', label: 'Closed' },
      { v: 'pair-offer', label: 'Pair · offer' },
      { v: 'pair-accept', label: 'Pair · accept' },
      { v: 'phrase-prepare', label: 'Phrase · name it' },
      { v: 'phrase', label: 'Phrase · 17 tokens' },
      { v: 'recover', label: 'Recover sheet' },
      { v: 'remove-device', label: 'Remove a Mac' },
    ];
    /* The chip labels carry the server because the bare aliases are the account
       switcher's own button text, and a deck label that repeats in-frame button
       text is the first thing `clickText` finds (README, "The deck"). */
    var ACCOUNT_VALUES = [
      { v: 'acct:personal', label: 'personal · foks.example.net', hint: 'rae on foks.example.net — never stopped.' },
      { v: 'acct:work', label: 'work · foks.acme-corp.com', hint: 'rae.chen on foks.acme-corp.com — stopped whenever Acme is not ok.' },
    ];
    var ACCOUNT_CONTROL = {
      key: 'account', label: 'Account',
      note: 'The selected StoreRef (settings-screen.tsx:337). It survives every move between sections (settings-screen.tsx:642), so every Settings page declares it. A ref that resolves to no store is set-unavailable.',
      values: ACCOUNT_VALUES,
    };
    /* Accounts and About render the stale-link notice ABOVE their own pane
       rather than instead of it (settings-screen.tsx:704), so the unresolvable
       ref is one of this key's values there. */
    var ACCOUNT_CONTROL_STALE = {
      key: 'account', label: 'Account',
      note: 'The selected StoreRef. It rides along from Recovery devices; a ref this catalog does not list puts UnavailableAccount above this pane instead of replacing it.',
      values: ACCOUNT_VALUES.concat([
        { v: 'acct:nope', label: 'acct:nope (stale)', hint: 'no such store: the crit notice sits above the pane, which still renders.' },
      ]),
    };
    var ACCOUNTS_CONTROL = {
      key: 'accounts', label: 'Accounts on this Mac',
      note: 'The fixture always lists two accounts, so both “No available account on this Mac” notices are otherwise unreachable (map §8.1).',
      values: [
        { v: '', label: 'Two accounts', hint: 'the fixture: acct:personal and acct:work.' },
        { v: 'none', label: 'None at all', hint: 'no account store: MacsSection and AccountSection each draw their Notice, and UnavailableAccount grows its second paragraph. Settings-only — the sidebar still lists the fixture’s vaults.' },
      ],
    };
    var LOADING_CONTROL = {
      key: 'loading', label: 'Reads in flight',
      note: 'The states every pane opens on and leaves before a capture lands (map §3.5): macsLoading, keysLoading, an unloaded signed status and a null appInfo.',
      values: [
        { v: '', label: 'Loaded' },
        { v: '1', label: 'Reading…', hint: '“Loading devices…”, “Loading keys…”, “Reading check-in…”, “Reading app info…”. Nothing new is disabled by it except the 13 key rows, whose title becomes “Reading this account…”.' },
      ],
    };
    var DEVLIST_CONTROL = {
      key: 'devlist', label: 'Device rows',
      note: 'Device-row variants the fixture cannot reach (map §8.1).',
      values: [
        { v: 'fixture', label: 'Fixture', hint: 'MacBook Pro (current) + Travel Mac (04… → Remove…).' },
        { v: 'plus08', label: '+ 08… key', hint: 'an unnamed 08… device: label "YubiKey", chip "managed under Security keys".' },
        { v: 'keycurrent', label: 'Current is a key', hint: 'the current device id starts 08 → chip "current security key".' },
        { v: 'none', label: 'No devices', hint: 'InsetRow "None" / "No devices were reported for this account."' },
      ],
    };
    var YUBISTATE_CONTROL = {
      key: 'yubistate', label: 'Enrollments',
      note: 'The fixture has one complete enrollment and no pending one, so Resume enrollment is never enabled (map §8.1).',
      values: [
        { v: 'fixture', label: 'Fixture', hint: 'one complete enrollment, "primary key".' },
        { v: 'pending', label: '+ pending', hint: 'adds a pending enrollment, so Resume enrollment goes live.' },
        { v: 'none', label: 'No enrollment', hint: 'no enrollment: all 13 rows and Revoke disabled, "No complete enrollment was reported".' },
      ],
    };
    /* The agent holds at most one pairing operation per account. The mock
       bridge has none, so Resume raises `pending-operation-not-found`, which
       the screen toasts as a red pill and leaves the sheet open; with one
       pending, Resume re-reveals the offer / finishes the acceptance. */
    var PENDING_CONTROL = {
      key: 'pending', label: 'Pending pairing',
      note: 'Whether the agent still holds this account’s pairing operation, which is what Resume continues. Pressing Start puts an offer there itself, so Resume offer works after it whatever this says.',
      values: [
        { v: '', label: 'Nothing pending', hint: 'Resume offer (before Start) and Resume acceptance raise “no longer pending”, in a red toast.' },
        { v: '1', label: 'One pending', hint: 'Resume offer re-reveals the phrase; Resume acceptance finishes it.' },
      ],
    };
    var TYPED_CONTROL = {
      key: 'typed', label: 'Confirmation typed',
      note: 'The typed confirmation matches the expected string, so the danger button goes live. Typing it by hand does the same.',
      values: [{ v: '', label: 'Empty' }, { v: '1', label: 'Matches' }],
    };
    /* What the agent answers with is a fact about this Mac, not about the pane
       looking at it, so these three ride along when the section nav moves — the
       way the account does. A page that reads one declares it; the others keep
       it (README, `keeps`), so Recovery devices → Security keys → Recovery
       devices does not quietly hand the fixture back. */
    var WORLD_KEEPS = ['accounts', 'devlist', 'yubistate', 'loading'];
    var APPLIED_CONTROL = {
      key: 'applied', label: 'Mutation applied',
      note: 'The pane after its sheet ran — Travel Mac gone from Your Macs.',
      values: [{ v: '', label: 'Before' }, { v: '1', label: 'After' }],
    };

    M.page({
      id: 'set-devices',
      title: 'Devices',
      path: ['Settings', 'Devices'],
      nav: 'settings',
      note: 'MacsSection: the account switcher, This account, Your Macs, Pairing, Recovery. Sheets bind v.alias / v.dev / v.phrase / v.tokens.',
      controls: [
        ACCOUNT_CONTROL,
        { key: 'sheet', label: 'Sheet', values: SHEET_VALUES_MACS },
        { key: 'seg', label: 'Pairing direction', values: [{ v: '', label: 'The sheet decides' }, { v: 'offer', label: 'From this Mac' }, { v: 'accept', label: 'On this Mac' }] },
        { key: 'started', label: 'Offer started', note: 'startDevicePairing returned; the phrase row shows and Finish goes live.', values: [{ v: '', label: 'No' }, { v: '1', label: 'Phrase shown' }] },
        PENDING_CONTROL,
        { key: 'written', label: 'Written down', values: [{ v: '', label: 'Unticked' }, { v: '1', label: 'Ticked' }] },
        TYPED_CONTROL,
        APPLIED_CONTROL,
        DEVLIST_CONTROL,
        ACCOUNTS_CONTROL,
        LOADING_CONTROL,
      ],
      /* `device` is the row the Remove… button names. It is not a chip — only
         one row in the fixture ever offers removal — but the page reads it, so
         it must survive arriving here from a flow step. */
      keeps: ['device'].concat(WORLD_KEEPS),
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        var pane = sectionBody(s, macsBody);
        return { main: M.settingsFrame(s, 'macs', pane.body, { switcher: pane.switcher }) };
      },
      overlay: overlayFor,
      after: selectDefaultAccount,
    });

    M.page({
      id: 'set-keys',
      title: 'Security keys',
      path: ['Settings', 'Security keys'],
      nav: 'settings',
      note: 'KeysSection: Enrolled keys, Connected now, Add, the 13 Everyday and recovery rows, Danger. No account switcher — the pane still follows v.account.',
      controls: [
        ACCOUNT_CONTROL,
        {
          key: 'sheet', label: 'Sheet',
          values: [
            { v: '', label: 'Closed' },
            { v: 'enrol', label: 'Create on YubiKey' },
            { v: 'provision', label: 'Provision sheet' },
            { v: 'yubi-sync', label: 'Yubi · sync' },
            { v: 'yubi-change-pin', label: 'Yubi · change pin' },
            { v: 'yubi-set-passphrase', label: 'Yubi · set passphrase' },
            { v: 'yubi-unblock', label: 'Yubi · unblock' },
            { v: 'yubi-pin-status', label: 'Yubi · pin status' },
            { v: 'revoke', label: 'Revoke sheet' },
          ],
        },
        { key: 'advanced', label: 'Advanced open', note: '<details class="adv"> in the Enroll sheet.', values: [{ v: '', label: 'Closed' }, { v: '1', label: 'Expanded' }] },
        TYPED_CONTROL,
        YUBISTATE_CONTROL,
        ACCOUNTS_CONTROL,
        LOADING_CONTROL,
      ],
      keeps: WORLD_KEEPS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        return { main: M.settingsFrame(s, 'keys', sectionBody(s, keysBody).body) };
      },
      overlay: overlayFor,
      after: selectDefaultAccount,
    });

    M.page({
      id: 'set-account',
      title: 'Account',
      path: ['Settings', 'Account'],
      nav: 'settings',
      note: 'AccountSection: one block per account. Acme’s signed check-in is three days in the past in the mock bridge, so Work reads "server check-in lapsed" whatever the lease world is.',
      controls: [
        ACCOUNT_CONTROL_STALE,
        {
          key: 'sheet', label: 'Sheet',
          values: [
            { v: '', label: 'Closed' },
            { v: 'passphrase', label: 'Passphrase' },
            { v: 'go-profile', label: 'Connect from FOKS CLI' },
          ],
        },
        { key: 'seg', label: 'Passphrase action', values: [{ v: '', label: 'The button decides' }, { v: 'set', label: 'Set it' }, { v: 'change', label: 'Change it' }, { v: 'verify', label: 'Verify it' }] },
        ACCOUNTS_CONTROL,
        DEVLIST_CONTROL,
        LOADING_CONTROL,
      ],
      keeps: WORLD_KEEPS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        var pane = sectionBody(s, accountBody, false);
        return { main: M.settingsFrame(s, 'account', pane.body) };
      },
      overlay: overlayFor,
      after: selectDefaultAccount,
    });

    M.page({
      id: 'set-about',
      title: 'About',
      path: ['Settings', 'About'],
      nav: 'settings',
      note: '?state=settings-agent is a pure alias of this pane (map §8.3). Retry connection appears only while the agent phase is not Ready — set Agent to Starting.',
      controls: [
        ACCOUNT_CONTROL_STALE,
        { key: 'advanced', label: 'Inspect AgentStatus', values: [{ v: '', label: 'Closed' }, { v: '1', label: 'Expanded' }] },
        LOADING_CONTROL,
      ],
      keeps: WORLD_KEEPS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        return { main: M.settingsFrame(s, 'about', sectionBody(s, aboutBody, false).body) };
      },
      /* No overlay: About owns no sheet. */
      after: selectDefaultAccount,
    });

    M.page({
      id: 'set-unavailable',
      title: 'Unavailable link',
      path: ['Settings', 'Unavailable link'],
      nav: 'settings',
      note: 'The address named a StoreRef this catalog does not list (?account=acct:nope). In Recovery devices and Security keys the notice replaces the pane; the section nav stays.',
      controls: [
        {
          key: 'account', label: 'Account',
          note: 'This page keeps the unresolvable ref the address named; naming a real one gives the ordinary Recovery devices pane back.',
          values: [
            { v: 'acct:nope', label: 'acct:nope', hint: 'no such store in the catalog — UnavailableAccount.' },
            { v: 'acct:personal', label: 'personal · foks.example.net', hint: 'resolves, so the pane is the ordinary MacsSection.' },
            { v: 'acct:work', label: 'work · foks.acme-corp.com', hint: 'ditto, for the Acme account.' },
          ],
        },
        ACCOUNTS_CONTROL,
      ],
      keeps: WORLD_KEEPS,
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        return { main: M.settingsFrame(s, 'macs', sectionBody(s, macsBody).body) };
      },
      /* Unlike every other Settings page this one does NOT rewrite the
         address to the first account: the whole point is the ref that names
         no store, so an address without one is given the stale ref instead. */
      after: function (s) {
        M.fns.escape = ownEscape;
        if (s.v.account == null) M.set('v.account', 'acct:nope');
      },
    });

    /* =============================================================== alerts
       72-alerts.js is a separate part; this file leaves Alerts alone. */
  })();
