  /* ----------------------------------------------------------- 40-items.js
     The vault surface: All items and the five store pages.

       pages       all · store-personal · store-work · store-eng ·
                   store-household · store-homelab
       screens     screens/items-screen.tsx (header, toolbar, list, cards,
                   empties, All-items bands), screens/scope.ts (what is
                   listed, in what order), screens/store-access.tsx (the
                   takeover), screens/details-panel.tsx (the aside),
                   screens/write-workflows.tsx (New / exists / conflict /
                   discard / remove), screens/edit-value.ts.

     View keys, all read as s.v.<key> (null = the first value):

       view     list | grid                 state.view
       kind     All | Password | Resource | File | Link
       sort     name | kind | group | version
       search   the query (paths and store names only, never contents)
       sel      "<storeRef>|<path>" — M.data.itemKey
       details  shown | hidden              null follows the selection
       reveal   1                           the one-shot exact-version read
       sheet    new-password | new-resource | new-file | new-link | edit |
                replace | remove | exists | conflict | discard
       menu     new | new-empty | sort | save-in
       dest     the StoreRef the New sheet saves in
       readRole / writeRole  Owner | Admin | Member   (+ readVis / writeVis)
       advanced 1     the New sheet's Advanced disclosure
       inspect  1     the details panel's "Inspect response" disclosure
       site/user/pw/web/rname/val/target/dpath/draft   free-text draft fields
       ekind / epath   which create the `exists` refusal is about
       applied  the write ops, replayed onto the fixture on every render
       adhoc    yes    60-groups' key: Homelab's creation was resumed

     Every write goes through `applied`, so a create, an edit and a remove are
     each a URL: the fixture is restored and the ops replayed at the head of
     every render, and clicking Create / Save version n+1 / Remove in the
     frame pushes the same op onto the list. */

  (function () {
    var D = M.data;
    var UI = M.ui;
    var esc = M.esc;
    var h = M.h;

    var ITEM_PAGES = {
      all: null,
      'store-personal': 'acct:personal',
      'store-work': 'acct:work',
      'store-eng': 'team:eng',
      'store-household': 'team:household',
      'store-homelab': 'team:homelab',
    };
    var GROUP_PAGES = {
      'team:eng': 'eng-people',
      'team:household': 'household-people',
      'team:homelab': 'homelab-people',
    };
    /* write-workflows.tsx:81-86 */
    var DRAFT_PATH = {
      Password: '/logins/github.com',
      Resource: '/agents/anthropic-api-key',
      File: '/documents/emergency.pdf',
      Link: '/latest-key',
    };
    var SHEET_KIND = {
      'new-password': 'Password',
      'new-resource': 'Resource',
      'new-file': 'File',
      'new-link': 'Link',
    };
    /* shell/toolbar.tsx:23-28 */
    var SORT_LABELS = { name: 'Sort by name', kind: 'Sort by kind', group: 'Grouped', version: 'Sort by version' };
    var SORT_KEYS = ['name', 'kind', 'group', 'version'];
    /* details-panel.tsx:58-63 */
    var FIELD_LABELS = { user: 'User name', password: 'Password', url: 'Website', ssid: 'Network' };

    /* --------------------------------------------- applied (the mutations)
       The same contract 60-groups uses: `v.applied` is a comma-separated list
       of write ops, the fixture is put back at the head of every render and
       the list is replayed on top. So a create, a save and a remove are all
       in the URL and a flow step can land on the result cold, while a click
       in the frame is just one more op pushed onto the list.

         new~<Kind>~<store>~<path>~<read>~<write>   a create
         save~<store>~<path>~<version>              an edit / a replace
         rm~<store>~<path>                          a remove

       Roles are wire words: `Owner`, `Admin`, `Member.0`; stores are the
       short aliases below, so a token carries no `:` and 60-groups' own
       `applied` parser (which splits on `:`) cannot mistake it for a roster
       op. What was typed into the sheet or the editor stays out of the URL —
       the token carries the row, TYPED carries the value Show would return. */
    var SHORT = { 'acct:personal': 'personal', 'acct:work': 'work', 'team:eng': 'eng', 'team:household': 'household', 'team:homelab': 'homelab' };
    var LONG = { personal: 'acct:personal', work: 'acct:work', eng: 'team:eng', household: 'team:household', homelab: 'team:homelab' };
    var BASE_ITEMS = D.items.map(function (row) { return Object.assign({}, row); });
    var BASE_PLAINTEXT = Object.assign({}, D.plaintext);
    var TYPED = {};
    /* Readable aliases so `?applied=created-password` is a state a flow can
       point at. Each expands to one op above. */
    var NAMED = {
      'created-password': 'new~Password~personal~/logins/gitlab.com~Owner~Owner',
      'created-note': 'new~Resource~personal~/agents/deploy-token~Owner~Owner',
      'created-file': 'new~File~household~/documents/insurance.pdf~Member.0~Admin',
      'created-link': 'new~Link~personal~/current-key~Owner~Owner',
      'saved-github': 'save~personal~/logins/github.com~10',
      'removed-github': 'rm~personal~/logins/github.com',
    };
    /* The value a named create would have carried, so Show still answers. */
    var NAMED_VALUE = {
      'acct:personal|/logins/gitlab.com': 'user: rae\npassword: swift-Anchor-42-loom\nurl: https://gitlab.com/login',
      'acct:personal|/agents/deploy-token': 'dpl-9f2c1a7b64e0a3d5',
      'acct:personal|/current-key': '/agents/anthropic-api-key',
    };
    function parseWire(word) {
      if (!word || word === 'Owner') return { role: 'Owner' };
      if (word === 'Admin') return { role: 'Admin' };
      var bits = word.split('.');
      return { role: 'Member', visibility: Number(bits[1]) || 0 };
    }
    function wireWord(wire) {
      var role = wire && wire.role;
      return role === 'Member' ? 'Member.' + (wire.visibility || 0) : (role || 'Owner');
    }
    function ops(s) {
      return String((s.v && s.v.applied) || '').split(',').filter(Boolean)
        .map(function (token) { return NAMED[token] || token; });
    }
    function pushOp(token) {
      var list = String(M.s.v.applied || '').split(',').filter(Boolean);
      if (list.indexOf(token) < 0) list.push(token);
      M.s.v.applied = list.join(',');
    }
    /* A write never moves `item.value`, only the plaintext behind it.
       `list_catalog` carries no value at all (mock-bridge.ts:337-345 builds the
       CatalogDto from store/path/kind/size/version/read/write), and the
       non-native merge in bridge.ts `loadWorld` re-attaches the value from the
       PRISTINE fixture (`base = bridge.fixtureWorld`), not from the mutated
       bridge state. So after an edit the details panel still previews the
       original masked login skeleton at the new version and size, and a freshly
       created item has no `value` at all — one masked `Password` row, not the
       fields that were typed into the sheet. `D.plaintext` is the bridge's
       `contents` map: that is what Show returns, and that does move. */
    function applyItemOp(token) {
      var bits = token.split('~');
      var op = bits[0];
      if (op === 'new') {
        var kind = bits[1], store = LONG[bits[2]] || bits[2], path = bits[3];
        if (D.itemAt(store, path)) return;
        var key = store + '|' + path;
        var value = TYPED[key] != null ? TYPED[key] : NAMED_VALUE[key];
        var row = {
          store: store, path: path, kind: 'Secret', size: 0, version: 1,
          read: parseWire(bits[4]), write: parseWire(bits[5]),
        };
        if (kind === 'File') { row.kind = 'File'; row.size = (value || '').length; }
        else if (kind === 'Link') { row.kind = 'Link'; row.target = value || ''; row.size = row.target.length; }
        else { row.size = (value || '').length; D.plaintext[key] = value || ''; }
        D.items.push(row);
        return;
      }
      var ref = LONG[bits[1]] || bits[1];
      var hit = D.itemAt(ref, bits[2]);
      if (op === 'rm') {
        D.items = D.items.filter(function (r) { return !(r.store === ref && r.path === bits[2]); });
        delete D.plaintext[ref + '|' + bits[2]];
        return;
      }
      if (op === 'save' && hit) {
        hit.version = Number(bits[3]) || hit.version + 1;
        var skey = ref + '|' + bits[2];
        var typed = TYPED[skey];
        /* A cold `applied=saved-github` has nothing in TYPED — nobody typed
           anything into this session's editor — but the app still WROTE
           something: the editor opens on `editableValue(item, plaintext)` and
           Save sends exactly that back, so an untouched save replaces the
           stored value with the merged skeleton and the size follows it (142 B
           → 73 B on github.com). Derive it rather than leaving the old size,
           which is the size of a write that never happened. A File replace
           streams a new file and the fixture's size is all the mock knows, so
           that one stays put — which is also what the app shows. */
        if (typed == null && hit.kind !== 'File' && D.kindOf(hit) !== 'Link') {
          typed = editableValue(hit, D.plaintext[skey] || '');
        }
        if (typed != null && hit.kind !== 'File') {
          hit.size = typed.length;
          D.plaintext[skey] = typed;
        }
      }
    }
    function syncItems(s) {
      D.items = BASE_ITEMS.map(function (row) { return Object.assign({}, row); });
      D.plaintext = Object.assign({}, BASE_PLAINTEXT);
      ops(s).forEach(applyItemOp);
    }

    /* ------------------------------------------------------ view readers */
    function vv(s, key, dflt) {
      var v = s.v[key];
      return v == null || v === '' ? dflt : v;
    }
    function viewOf(s) { return vv(s, 'view', 'list'); }
    function kindFilter(s) { return vv(s, 'kind', 'All'); }
    function sortOf(s) { return vv(s, 'sort', 'name'); }
    function queryOf(s) { return s.v.search || ''; }

    /* ------------------------------------------ model helpers not in 20-data
       model/lease.ts:164-224, ported against the world M.data.world returns
       (its stores already carry `readable` and `parties`). */
    function ownParty(store) {
      return store.parties.filter(function (p) {
        return p.label === 'you' && p.party_kind === 'user' && p.locally_manageable &&
          D.admissionActive(p, store.id);
      });
    }
    function canCreateInStore(w, ref) {
      var store = w.storeById[ref];
      if (!store || !store.readable) return false;
      if (store.kind === 'account') return true;
      return ownParty(store).length === 1;
    }
    function canChangeItem(w, item) {
      var store = w.storeById[item.store];
      if (!store || !canCreateInStore(w, store.id)) return false;
      if (store.kind === 'account') return true;
      var own = ownParty(store);
      return own.length === 1 && D.admits(own[0].destination_role, item.write);
    }
    function defaultCreateStore(w) {
      var order = w.navOrder;
      for (var i = 0; i < order.length; i++) if (canCreateInStore(w, order[i].id)) return order[i].id;
      return order[0] ? order[0].id : '';
    }
    /* items-screen.tsx: `createStore` is the store being looked at when it can
       be written to, and `defaultCreateStore(world)` otherwise — so a New sheet
       opened on Engineering saves into Engineering, and one opened on All items
       (or on a store nothing can be written to) falls back to the first store in
       navigation order that takes a write. `v.dest` is the choice already made
       in the Save-in chooser; this is what it starts from. */
    function createStoreFor(s, w) {
      var page = M.pages[s.page];
      var ref = page && page.ref;
      return ref && canCreateInStore(w, ref) ? ref : defaultCreateStore(w);
    }
    /* write-workflows.tsx:183-196 */
    function writeBlockReason(w, store) {
      if (canCreateInStore(w, store.id)) return null;
      var srv = w.serverById[store.server];
      if (srv && srv.state === 'lease-lapsed') return 'server check-in lapsed — nothing can be written here';
      if (srv && srv.state === 'blocked') return 'server access is blocked — nothing can be written here';
      if (srv && srv.state === 'lease-unavailable') return 'check-in status unknown — nothing can be written here';
      if (store.kind === 'team' && !store.active) return 'reports inactive — resume its creation first';
      if (store.kind === 'team') return 'no authenticated local group identity — writing is unavailable';
      return 'nothing can be written here';
    }
    /* screens/edit-value.ts */
    function editableValue(item, learned) {
      if (D.kindOf(item) !== 'Password' || !item.value) return learned;
      var lines = learned.split('\n');
      for (var i = 0; i < lines.length; i++) if (lines[i].indexOf('password: ') === 0) return learned;
      if (!/password: .*/.test(item.value)) return learned;
      return item.value.replace(/password: .*/, function () { return 'password: ' + learned; });
    }

    /* ------------------------------------------------- scope.ts (verbatim) */
    var SORTS = {
      name: function () { return function (a, b) { return D.nameOf(a.path).localeCompare(D.nameOf(b.path)); }; },
      kind: function () {
        return function (a, b) {
          return D.KIND_LIST.indexOf(D.kindOf(a)) - D.KIND_LIST.indexOf(D.kindOf(b)) ||
            D.nameOf(a.path).localeCompare(D.nameOf(b.path));
        };
      },
      group: function (w) {
        return function (a, b) {
          return ((w.storeById[a.store] || {}).name || '').localeCompare((w.storeById[b.store] || {}).name || '') ||
            D.nameOf(a.path).localeCompare(D.nameOf(b.path));
        };
      },
      version: function () {
        return function (a, b) { return b.version - a.version || D.nameOf(a.path).localeCompare(D.nameOf(b.path)); };
      },
    };
    function scopedItems(s, w, ref) {
      var items = w.items;
      if (ref) items = items.filter(function (i) { return i.store === ref; });
      var kind = kindFilter(s);
      if (kind !== 'All') items = items.filter(function (i) { return D.kindOf(i) === kind; });
      var q = queryOf(s);
      if (q) {
        var needle = q.toLowerCase();
        items = items.filter(function (i) {
          return i.path.toLowerCase().indexOf(needle) >= 0 ||
            (((w.storeById[i.store] || {}).name) || '').toLowerCase().indexOf(needle) >= 0;
        });
      }
      return items.slice().sort(SORTS[sortOf(s)](w));
    }

    /* ------------------------------------------------------ the selection */
    function selectedItem(s, w) {
      if (!s.v.sel) return null;
      var cut = s.v.sel.indexOf('|');
      if (cut < 0) return null;
      var store = s.v.sel.slice(0, cut), path = s.v.sel.slice(cut + 1);
      var hit = w.items.filter(function (i) { return i.store === store && i.path === path; });
      return hit[0] || null;
    }
    /* app-root.tsx:508-512 — the toggle writes `details`, a selection forces
       it open, deselecting leaves it open, and an unreadable store closes it. */
    function detailsShown(s, w, store) {
      if (store && store.state !== 'normal') return false;
      var d = s.v.details;
      var open = d === 'shown' ? true : d === 'hidden' ? false : !!s.v.sel;
      if (!open) return false;
      if (s.v.sel) {
        var ref = s.v.sel.slice(0, s.v.sel.indexOf('|'));
        var selStore = w.storeById[ref];
        if (selStore && !selStore.readable) return false;
      }
      return true;
    }

    /* =================================================== toolbar and header */
    function pathChip(path) {
      var prefix = D.prefixOf(path);
      return prefix ? '<span class="pchip">' + esc(prefix) + '</span>' : '';
    }
    /* components/menus.tsx wraps every MenuButton's list in a `Popover`, so the
       trigger stays in `.menuwrap` and the list is portalled:
       `#overlays > .menu-portal > .menu[role=menu]`, placed from the trigger's
       rect (`align` is placement, never a class — the app emits no `.menu.right`).
       That is the recipe 30-shell.js documents and 60-groups.js follows: the
       trigger comes from `main`, the portal from `overlay(s)`, `M.placeMenuPortal`
       from `after()`. `M.ui.menuPortal` now writes `class`, `aria-label`, `role`
       in the app's own order (cap/shell/new-menu-open.html), so it is used
       directly rather than copied here. */
    function menuTrigger(o) {
      return '<span class="menuwrap">' + UI.btn(
        o.label + M.icon('chev', null, { cls: 'chevron' }),
        {
          variant: o.variant,
          attrs: 'aria-haspopup="menu" aria-expanded="' + (o.open ? 'true' : 'false') + '" ' +
            (o.open ? 'data-act="set" data-key="v.menu" data-val=""'
              : 'data-act="set" data-key="v.menu" data-val="' + o.menuKey + '"'),
        }) + '</span>';
    }
    function menuPortal(label, inner) {
      return UI.menuPortal({ menuLabel: label, menuHtml: inner });
    }
    function newMenuHtml() {
      return D.KIND_LIST.map(function (k) {
        return '<button type="button" role="menuitem" class="kind-menu-item" tabindex="-1" ' +
          'data-act="call" data-fn="itemsNew" data-arg="' + k + '">' +
          UI.kindIcon(k) + esc(D.kindLabel(k)) + '</button>';
      }).join('');
    }
    function sortMenuHtml(s) {
      var sort = sortOf(s);
      return SORT_KEYS.map(function (k) {
        var on = sort === k;
        return '<button type="button" role="menuitem" class="' + (on ? 'on' : '') + '" ' +
          'aria-checked="' + (on ? 'true' : 'false') + '" tabindex="-1" ' +
          'data-act="call" data-fn="itemsSort" data-arg="' + k + '">' +
          (on ? M.icon('check') : '<span class="ic"></span>') + esc(SORT_LABELS[k]) + '</button>';
      }).join('');
    }
    /* shell/toolbar.tsx:44-77 — one primary button over the kind menu. */
    function newItemButton(s, menuKey) {
      return menuTrigger({ label: 'New', variant: 'primary', menuKey: menuKey, open: s.v.menu === menuKey });
    }
    function sortMenu(s) {
      return menuTrigger({ label: esc(SORT_LABELS[sortOf(s)]), menuKey: 'sort', open: s.v.menu === 'sort' });
    }
    /* placeAnchoredMenu's anchors, one per open menu. `width:false` keeps a
       `.menu` at its own width; the sort menu is `align="end"`. */
    var MENU_TRIGGER = {
      new: { trigger: '#win .toolbar .menuwrap .btn.primary' },
      'new-empty': { trigger: '#win .empty .menuwrap .btn.primary' },
      sort: { trigger: '#win .toolbar .menuwrap .btn:not(.primary)', align: 'end' },
    };
    function placeMenus(s) {
      var open = s.v.menu;
      if (open === 'save-in') return M.placeMenuPortal();
      var anchor = MENU_TRIGGER[open];
      if (anchor) M.placeMenuPortal({ trigger: anchor.trigger, width: false, align: anchor.align });
    }
    function toolbar(s, w, store, details) {
      var kind = kindFilter(s);
      var view = viewOf(s);
      var kinds = [{ id: 'All', label: 'All', attrs: 'data-act="set" data-key="v.kind" data-val=""' }]
        .concat(D.KIND_LIST.map(function (k) {
          return { id: k, label: D.KINDS[k].plural, title: D.KINDS[k].blurb,
            attrs: 'data-act="set" data-key="v.kind" data-val="' + k + '"' };
        }));
      return UI.toolbar(h(
        newItemButton(s, 'new'),
        UI.segmented({ label: 'Which kinds to list', value: kind, items: kinds }),
        UI.spacer(),
        sortMenu(s),
        UI.segmented({
          label: 'How to show the items', variant: 'icon', value: view,
          items: [
            { id: 'list', icon: 'list', title: 'List', attrs: 'data-act="set" data-key="v.view" data-val=""' },
            { id: 'grid', icon: 'grid', title: 'Cards', attrs: 'data-act="set" data-key="v.view" data-val="grid"' },
          ],
        }),
        UI.btn('', {
          variant: 'quiet', icon: 'info', on: details, title: 'Details', ariaLabel: 'Details',
          attrs: 'data-act="call" data-fn="itemsDetails"',
        }),
        store && store.kind === 'team' ? UI.btn('', {
          variant: 'quiet', icon: 'gear', title: 'Group settings', ariaLabel: 'Group settings',
          attrs: 'data-act="go" data-page="' + esc(GROUP_PAGES[store.id] || 'set-groups') + '"',
        }) : ''
      ));
    }
    /* shell/page-header.tsx:23-45 */
    function pageHeader(s, store) {
      if (!store) {
        return UI.pageHeader({ title: 'All items', subtitle: '', search: { value: s.v.search, bind: 'v.search' } });
      }
      return UI.pageHeader({
        title: store.name, subtitle: store.heading,
        tail: store.kind === 'team' ? UI.stack(store.parties) : undefined,
        search: { value: s.v.search, bind: 'v.search' },
      });
    }

    /* ============================================================ the list */
    function listBody(s, w, items) {
      var searching = !!queryOf(s);
      var sel = s.v.sel;
      return '<div class="list-window">' + UI.listHeader({ sort: sortOf(s), key: 'v.sort' }) +
        '<div class="virtual-rows">' + items.map(function (item) {
          var key = D.itemKey(item);
          return UI.listRow({
            item: item, searching: searching, selected: sel === key,
            storeName: (w.storeById[item.store] || {}).name,
            server: (w.serverById[(w.storeById[item.store] || {}).server] || {}).name,
            attrs: 'data-act="call" data-fn="itemsSelect" data-arg="' + esc(key) + '"',
          });
        }).join('') + '</div></div>';
    }

    /* ============================================================ the cards */
    /* items-screen.tsx:87-147 — only tiles carry these; rows never do. */
    function itemActions(w, item) {
      var kind = D.kindOf(item);
      var key = esc(D.itemKey(item));
      var store = w.storeById[item.store];
      var removeDisabled = !(store && store.readable) || !canChangeItem(w, item);
      function act(label, icon, fn, danger, disabled) {
        return '<button type="button"' + (danger ? ' class="danger"' : '') +
          ' title="' + esc(label) + '" aria-label="' + esc(label) + '"' + (disabled ? ' disabled' : '') +
          ' data-act="call" data-fn="' + fn + '" data-arg="' + key + '">' + M.icon(icon) + '</button>';
      }
      var head = kind === 'Password' || kind === 'Resource'
        ? act('Show', 'eye', 'itemsReveal') +
          act(kind === 'Password' ? 'Copy password' : 'Copy value', 'copy', 'itemsCopyValue')
        : kind === 'File' ? act('Download', 'download', 'itemsDownload')
          : act('Open target', 'arrow', 'itemsOpenTarget');
      return '<span class="acts">' + head + act('Copy path', 'path', 'itemsCopyPath') +
        act(removeDisabled ? 'Your current access does not allow removing this item'
          : 'Remove version ' + item.version + ' exactly', 'trash', 'itemsRemove', true, removeDisabled) +
        '</span>';
    }
    function tile(s, w, item) {
      var key = D.itemKey(item);
      var selected = s.v.sel === key;
      var store = w.storeById[item.store];
      var kind = D.kindOf(item);
      var parties = store && store.kind === 'team' ? store.parties : [];
      var readers = D.readableBy(item);
      var sub = kind === 'Link' ? pathChip(item.path) + ' target read when opened'
        : kind === 'File' ? pathChip(item.path) + ' ' + esc(D.fmtSize(item.size))
          : D.prefixOf(item.path) ? pathChip(item.path) : '<span class="dim">at the root</span>';
      return '<div class="' + (selected ? 'tile sel' : 'tile') + '" role="button" tabindex="0" aria-pressed="' +
        (selected ? 'true' : 'false') + '" data-act="call" data-fn="itemsSelect" data-arg="' + esc(key) + '">' +
        '<div class="glyph">' + UI.kindGlyph(item, 'big') +
        (parties.length ? UI.stack(parties, {
          size: 'xs',
          title: 'In ' + (store.name || '') + ' · readable by ' + readers.label + ' of ' + parties.length,
        }) : '') +
        '<span class="qa">' + itemActions(w, item) + '</span></div>' +
        '<div class="cap"><div class="nm">' + esc(D.nameOf(item.path)) + '</div>' +
        '<div class="sub">' + sub + '</div></div></div>';
    }
    function tileSection(w, store) {
      if (store.kind === 'team') {
        return '<div class="gsec">' + UI.stack(store.parties) + '<span>' + esc(store.name) + '</span>' +
          '<span class="n">· ' + esc(D.peopleGroups(store.parties)) + '</span></div>';
      }
      return '<div class="gsec">' + esc(store.name) +
        '<span class="n">· ' + esc((w.serverById[store.server] || {}).name || '') + '</span></div>';
    }
    function gridBody(s, w, items, ref) {
      var cards = items.slice(0, 200);
      if (!ref) {
        return D.storeDisplayOrder(w.stores).map(function (store) {
          var section = cards.filter(function (i) { return i.store === store.id; });
          if (!section.length) return '';
          return tileSection(w, store) + '<div class="tiles">' +
            section.map(function (i) { return tile(s, w, i); }).join('') + '</div>';
        }).join('');
      }
      var meta = kindFilter(s) === 'All' ? null : D.KINDS[kindFilter(s)];
      return '<div class="gsec first">' + esc(meta ? meta.plural : 'Everything') +
        '<span class="n">· ' + items.length + '</span></div>' +
        '<div class="tiles">' + cards.map(function (i) { return tile(s, w, i); }).join('') + '</div>';
    }

    /* ======================================================= the empty body */
    function emptyBody(s, store) {
      var q = queryOf(s);
      if (q) return UI.emptySearch({ store: store ? store.name : '', query: q });
      var kind = kindFilter(s);
      var meta = kind === 'All' ? null : D.KINDS[kind];
      return '<div class="empty"><div class="big">' + M.icon(meta ? meta.icon : 'key') + '</div>' +
        '<h2>No ' + (meta ? esc(meta.plural.toLowerCase()) : 'items') + ' here</h2>' +
        '<p>' + esc((meta || D.KINDS.Password).blurb) + '</p>' +
        newItemButton(s, 'new-empty') + '</div>';
    }

    /* ================================================ store-access takeover */
    function takeover(s, w, store) {
      var notice = store.notice;
      var action = notice.actionKind === 'finish-setup'
        ? UI.btn('Finish setup', { variant: 'primary', attrs: 'data-act="call" data-fn="itemsFinishSetup" data-arg="' + esc(store.id) + '"' })
        : UI.btn(notice.actionKind === 'open-server' ? 'Open server' : 'Review server', {
            attrs: 'data-act="go" data-page="srv-server" data-set=\'{"v.profile":"' + esc(notice.profile) + '"}\'',
          });
      var gear = store.kind === 'team'
        ? UI.btn('', {
            variant: 'quiet', icon: 'gear', title: 'Group settings', ariaLabel: 'Group settings',
            attrs: 'data-act="go" data-page="' + esc(GROUP_PAGES[store.id] || 'set-groups') + '"',
          })
        : '';
      var head = UI.pageHeader({
        title: store.name, subtitle: store.heading,
        lead: store.kind === 'team' ? UI.stack(store.parties) : undefined,
        action: (notice.severity === 'warn' && notice.actionKind === 'finish-setup' ? '' : action) + gear,
      });
      return head + UI.body(UI.notice({
        severity: notice.severity, title: esc(notice.title),
        body: '<p>' + esc(notice.detail) + '</p>', actions: action,
      }));
    }

    /* ========================================================= details panel
       screens/details-panel.tsx. `sheet=edit` / `sheet=replace` are the two
       editing modes; `reveal=1` is the exact-version read. */
    function parseFields(value) {
      return value.split('\n').map(function (line) {
        var cut = line.indexOf(': ');
        return cut > 0 ? [line.slice(0, cut), line.slice(cut + 2)] : [null, line];
      });
    }
    /* details-panel.tsx:138-165, verbatim. */
    function passwordFields(masked, shown) {
      if (shown !== null && shown !== undefined) {
        var learned = parseFields(shown);
        var structured = learned.some(function (p) { return p[0] === 'password'; });
        if (structured) {
          return learned.map(function (p) { return { field: p[0], value: p[1], secret: p[0] === 'password' }; });
        }
        if (!masked) return [{ field: null, value: shown, secret: true }];
        return parseFields(masked).map(function (p) {
          return { field: p[0], value: p[0] === 'password' ? shown : p[1], secret: p[0] === 'password' };
        });
      }
      if (!masked) return [{ field: 'password', value: '••••••••••', secret: true }];
      return parseFields(masked).map(function (p) {
        return { field: p[0], value: p[0] === 'password' ? '••••••••••' : p[1], secret: p[0] === 'password' };
      });
    }
    function roleText(wire) {
      var role = D.parseRole(wire);
      if (role) return D.formatRole(role);
      return typeof wire === 'string' ? wire : wire.role;
    }
    function wireRole(wire) {
      var role = D.parseRole(wire);
      if (!role) return { role: typeof wire === 'string' ? wire : wire.role };
      if (role.kind === 'member') return { role: 'Member', visibility: role.visibility == null ? 0 : role.visibility };
      return { role: role.kind === 'admin' ? 'Admin' : 'Owner' };
    }
    function revealActions(revealed) {
      return revealed
        ? '<button type="button" data-act="call" data-fn="itemsHide">' + M.icon('eyeoff') + 'Hide</button>' +
          '<button type="button" data-act="call" data-fn="itemsCopyValue">' + M.icon('copy') + 'Copy</button>'
        : '<button type="button" data-act="call" data-fn="itemsShow">' + M.icon('eye') + 'Show</button>' +
          '<button type="button" data-act="call" data-fn="itemsCopyValue">' + M.icon('copy') + 'Copy</button>';
    }
    function detailsEmpty() {
      return '<aside class="details" aria-label="Details">' +
        '<div class="dh"><span class="t"><h2>Details</h2></span>' +
        '<button type="button" class="x" aria-label="Close" data-act="call" data-fn="itemsCloseDetails">' +
        M.icon('x') + '</button></div>' +
        '<div class="scroll"><p class="hint">Select an item to view its details.</p></div></aside>';
    }
    function detailsPanel(s, w) {
      var item = selectedItem(s, w);
      if (!item) return detailsEmpty();
      var store = w.storeById[item.store];
      var serverName = (w.serverById[store ? store.server : ''] || {}).name || '';
      var kind = D.kindOf(item);
      var editing = s.v.sheet === 'edit' || s.v.sheet === 'replace';
      var fileMode = kind === 'File' || s.v.sheet === 'replace';
      var displayKind = fileMode ? 'File' : kind;
      var team = store && store.kind === 'team';
      var change = canChangeItem(w, item);
      var readers = D.readersOf(item);
      var parties = store ? store.parties : [];
      var revealed = s.v.reveal === '1';
      var shownValue = revealed ? (D.plaintextOf(item) || null) : null;

      var preview;
      if (editing && fileMode) {
        preview = UI.inset(UI.insetRow({
          variant: 'preview', label: 'Replace',
          value: '<span class="dim">Drop a file here, or use the native picker when saving</span>',
        }), { variant: 'preview' });
      } else if (editing) {
        var draft = s.v.draft != null ? s.v.draft : editableValue(item, D.plaintextOf(item) || '');
        preview = UI.inset('<textarea aria-label="Contents" data-bind="v.draft">' + esc(draft) + '</textarea>',
          { variant: 'preview' });
      } else if (kind === 'Password' && !fileMode) {
        preview = UI.inset(passwordFields(item.value, shownValue).map(function (f, i) {
          if (!f.secret) {
            return UI.insetRow({ variant: 'preview', label: esc(FIELD_LABELS[f.field || ''] || f.field), value: esc(f.value) });
          }
          return UI.insetRow({
            variant: 'preview', className: shownValue === null ? undefined : 'rev',
            label: esc(FIELD_LABELS[f.field || ''] || f.field || 'Value'),
            valueClass: shownValue === null ? 'mask' : 'mono',
            value: esc(f.value), action: revealActions(shownValue !== null),
          });
        }).join(''), { variant: 'preview' });
      } else if (kind === 'Resource' && !fileMode) {
        preview = UI.inset(UI.insetRow({
          variant: 'preview', className: 'rev', label: 'Value',
          valueClass: shownValue === null ? 'mask' : 'mono',
          value: esc(shownValue === null ? '••••••••••••••••••••' : shownValue),
          action: revealActions(shownValue !== null),
        }), { variant: 'preview' });
      } else if (fileMode) {
        preview = UI.inset('<div class="pad"><div class="fileglyph"><span class="g">' + M.icon('file') + '</span>' +
          '<span><b>' + esc(D.nameOf(item.path)) + '</b><span>' + esc(D.fmtSize(item.size)) +
          ' · version ' + item.version + '</span></span></div>' +
          '<div class="row2">' + UI.btn('Download', {
            variant: 'primary', icon: 'download', attrs: 'data-act="call" data-fn="itemsPanelDownload"',
          }) + '</div></div>', { variant: 'preview' });
      } else {
        preview = UI.inset('<div class="pad"><div class="fileglyph"><span class="g Link">' + M.icon('link') + '</span>' +
          '<span><b>' + esc(D.nameOf(item.path)) + '</b><span>' +
          (shownValue === null ? 'target masked until read' : 'points to <code>' + esc(shownValue) + '</code>') +
          '</span></span></div>' +
          /* details-panel.tsx:720-747 — the panel's Open target only selects
             what the catalog holds; unlike the card's quick action it raises
             no "Nothing is currently at …" toast when the path is now free. */
          '<div class="row2">' + UI.btn(shownValue === null ? 'Read target' : 'Open target', {
            variant: 'primary', icon: 'arrow',
            attrs: 'data-act="call" data-fn="' + (shownValue === null ? 'itemsShow' : 'itemsPanelOpenTarget') + '"' +
              (shownValue === null ? '' : ' data-arg="' + esc(D.itemKey(item)) + '"'),
          }) + '</div></div>', { variant: 'preview' });
      }

      var footnote = kind === 'Link'
        ? (shownValue === null
          ? 'Read target loads the link from version ' + item.version + '. FOKS hides it again when the window loses focus.'
          : 'Version ' + item.version + ' links to ' + shownValue + ' on ' + serverName + '. Opening it reads the current item at that path.')
        : '';

      var meta = '<div class="meta"><b>Path</b><code>' + esc(item.path) + '</code>' +
        '<b>Kind</b><span>' + esc(displayKind) + '</span>' +
        '<b>Version</b><span>' + item.version + '</span>' +
        '<b>Size</b><span>' + esc(D.fmtSize(item.size)) + '</span>' +
        '<b>Read role</b><span>' + UI.chip(esc(roleText(item.read))) + '</span>' +
        '<b>Write role</b><span>' + UI.chip(esc(roleText(item.write))) + '</span></div>';

      var who = '<div class="who">' + UI.sectionLabel('Sharing') +
        (team && readers
          ? '<p>' + esc(D.peopleLabel(readers.length)) + ' can read this — everyone in ' + esc(store.name) +
            ' at <b>' + esc(roleText(item.read)) + '</b> or above.</p>' +
            parties.map(function (party) {
              var canRead = readers.indexOf(party) >= 0;
              return '<div class="party">' + UI.avatar(party, { className: 'pav' }) +
                '<span class="t">' + esc(D.partyName(party)) +
                (party.label ? ' ' + UI.chip('you', { tone: 'you' }) : '') +
                '<small>' + esc(party.note || (party.party_kind !== 'user' ? 'a member group' : serverName)) +
                ' · ' + esc(D.shortId(party.party_id_hex)) + ' · gen ' + party.generation +
                (canRead ? '' : ' · cannot read this') + '</small></span>' +
                UI.chip(esc(roleText(party.destination_role))) + '</div>';
            }).join('')
          : '<p>Only you</p>') + '</div>';

      var inspect = UI.toggle({
        label: 'Inspect response', open: s.v.inspect === '1',
        attrs: 'data-act="set" data-key="v.inspect" data-val="' + (s.v.inspect === '1' ? '' : '1') + '"',
        body: '<pre>' + esc(JSON.stringify({
          path: item.path, node_type: D.rtype(item), version: item.version, size: item.size,
          read_role: wireRole(item.read), write_role: wireRole(item.write),
        }, null, 1)) + '</pre>',
      });

      var foot = editing
        ? UI.btn('Cancel', { attrs: 'data-act="call" data-fn="itemsCancelEdit"' }) +
          UI.btn('Save version ' + (item.version + 1), { variant: 'primary', attrs: 'data-act="call" data-fn="itemsSave"' })
        : UI.btn('Edit', {
            icon: 'pencil', disabled: !change || kind === 'Link',
            title: !change ? 'Your current role does not admit the ' + roleText(item.write) + ' write role'
              : kind === 'Link' ? 'A link cannot be edited atomically; remove it and create the new target at the same path'
                : 'Edit — Save uses ExactVersion(' + item.version + ')',
            attrs: 'data-act="call" data-fn="itemsEdit"',
          }) +
          UI.btn('Copy path', { attrs: 'data-act="call" data-fn="itemsCopyPath"' }) +
          UI.btn('Remove', {
            variant: 'danger', icon: 'trash', disabled: !change,
            title: change ? 'Removes version ' + item.version + ' exactly'
              : 'Your current role does not admit the ' + roleText(item.write) + ' write role',
            attrs: 'data-act="call" data-fn="itemsRemove"',
          });

      return '<aside class="details" aria-label="Details for ' + esc(D.nameOf(item.path)) + '">' +
        '<div class="dh">' + UI.kindIcon(displayKind) +
        '<span class="t"><h2>' + esc(D.nameOf(item.path)) + '</h2>' +
        '<small>' + esc(displayKind) + ' in ' + esc(store ? store.name : '') + '</small></span>' +
        '<button type="button" class="x" title="Close" aria-label="Close" ' +
        'data-act="call" data-fn="itemsCloseDetails">' + M.icon('x') + '</button></div>' +
        '<div class="scroll">' +
        (editing || displayKind !== 'Resource'
          ? UI.sectionLabel(editing ? 'Edit' : (displayKind === 'Password' ? 'Login' : displayKind))
          : '') +
        preview +
        (editing
          ? '<div class="pfn">Save succeeds only if this item is still version ' + item.version +
            '. If it changed, refresh it and try again.</div>'
          : (footnote ? '<div class="pfn">' + esc(footnote) + '</div>' : '')) +
        UI.sectionLabel('Info') + meta + who + inspect +
        '</div><div class="dfoot">' + foot + '</div></aside>';
    }

    /* ============================================================== sheets
       The write overlays. `M.ui.sheet` labels its panel by id, which is what
       `SheetDialog` does; the three write-workflow dialogs label the backdrop
       with `aria-label` and leave the <h2> bare, so those are built here to
       keep the markup byte-comparable. */
    function sheetPanel(o) {
      /* data-act="stop" shields the panel from the backdrop's close hook. */
      return '<div class="sheet' + (o.width === 'mid' ? ' mid' : '') + '" data-act="stop"><div class="hd">' + (o.glyph || '') +
        '<span class="t"><h2' + (o.titleId ? ' id="' + o.titleId + '"' : '') + '>' + o.title + '</h2>' +
        (o.subtitle === undefined ? '' : '<small>' + o.subtitle + '</small>') + '</span></div>' +
        '<div class="sb">' + (o.body || '') + '</div>' +
        (o.footer === undefined ? '' : '<div class="ft">' + o.footer + '</div>') + '</div>';
    }
    /* write-workflows.tsx — Dialog/DismissibleDialog: aria-label on the backdrop. */
    function labelledSheet(o) {
      return '<div aria-label="' + esc(o.ariaLabel) + '" class="backdrop" role="' + (o.danger ? 'alertdialog' : 'dialog') +
        '" aria-modal="true" tabindex="-1"' + (o.dismiss ? ' ' + o.dismiss : '') + '>' + sheetPanel(o) + '</div>';
    }

    /* The Save-in chooser is `M.ui.cardSelect` / `M.ui.cardSelectMenu`, the
       shared pair that mirrors components/card-select.tsx: the trigger sits in
       the sheet body, the open list is portalled beside it (the sheet body
       scrolls and would clip it) and `M.placeMenuPortal()` — whose defaults are
       a CardSelect's: the trigger's width, gap 4, align start — runs from
       after(). The helpers take their content slots raw, so the fixture's own
       names are escaped here. */
    function saveInOptions(s, w) {
      var storeId = s.v.dest || createStoreFor(s, w);
      return {
        label: 'Save in', value: (w.storeById[storeId] || {}).id || createStoreFor(s, w),
        open: s.v.menu === 'save-in',
        triggerAttrs: 'data-act="set" data-key="v.menu" data-val="save-in"',
        closeAttrs: 'data-act="set" data-key="v.menu" data-val=""',
        optionAttrs: function (id) { return 'data-act="call" data-fn="itemsSaveIn" data-arg="' + esc(id) + '"'; },
        options: w.navOrder.map(function (candidate) {
          return {
            id: candidate.id, title: esc(candidate.name), detail: esc(candidate.description),
            off: !canCreateInStore(w, candidate.id),
          };
        }),
      };
    }

    function draftOf(s, kind) {
      var site = s.v.site != null ? s.v.site : (kind === 'Password' ? 'github.com' : '');
      var rname = s.v.rname || '';
      var derived = kind === 'Password' ? '/logins/' + site
        : kind === 'Resource' ? (rname ? '/agents/' + rname.toLowerCase().replace(/[^a-z0-9._-]+/g, '-') : DRAFT_PATH.Resource)
          : DRAFT_PATH[kind];
      return {
        kind: kind, site: site, username: s.v.user || '', password: s.v.pw || '',
        website: s.v.web || '', value: s.v.val || '', resourceName: rname,
        target: s.v.target || '', path: s.v.dpath != null ? s.v.dpath : derived,
      };
    }
    function roleWireOf(s, side) {
      var base = vv(s, side + 'Role', side === 'read' ? 'Member' : 'Admin');
      if (base === 'Owner' || base === 'Admin') return { role: base };
      return { role: 'Member', visibility: Number(vv(s, side + 'Vis', '0')) || 0 };
    }
    function roleLabel(wire) {
      return wire.role === 'Member' ? 'Member · visibility ' + (wire.visibility || 0) : wire.role;
    }
    /* write-workflows.tsx:198-360 */
    function accessBlock(s, w, store) {
      if (store.kind !== 'team') return '';
      var readWire = roleWireOf(s, 'read'), writeWire = roleWireOf(s, 'write');
      var candidate = { store: store.id, path: '/preview', kind: 'Secret', size: 0, version: 0, read: readWire, write: writeWire };
      var roster = store.parties;
      var admitted = D.readersOf(candidate) || [];
      var excluded = roster.filter(function (p) { return admitted.indexOf(p) < 0; });
      var changers = D.readersOf({ store: store.id, path: '/preview', kind: 'Secret', size: 0, version: 0, read: writeWire, write: writeWire }) || [];
      function cards(side, wire, forWrite) {
        return ['Owner', 'Admin', 'Member'].map(function (label) {
          var detail = label === 'Owner' ? 'Only owners.'
            : label === 'Admin' ? 'Admins and owners.' : 'Members at this visibility level and above.';
          if (forWrite) detail = detail.replace('.', ' can change or remove it.');
          return UI.radioCard({
            title: label, detail: detail, selected: wire.role === label,
            attrs: 'data-act="set" data-key="v.' + side + 'Role" data-val="' + label + '"',
          });
        });
      }
      /* The number input is labelled "Read visibility" / "Write visibility"
         while its row reads "Member visibility", so it is built here rather
         than through M.ui.field (which labels the input from the row). */
      function visRow(side, wire) {
        if (wire.role !== 'Member') return '';
        var id = 'items-' + side + '-vis';
        return UI.insetRow({
          label: 'Member visibility', forId: id,
          value: '<input min="-32768" max="32767" aria-label="' + (side === 'read' ? 'Read' : 'Write') +
            ' visibility" id="' + id + '" type="number" value="' + (wire.visibility || 0) +
            '" data-bind="v.' + side + 'Vis" data-live>',
        });
      }
      return UI.sectionLabel('Who can read', {
        action: '<span class="pv">would be readable by <b>' + admitted.length + ' of ' + roster.length + '</b></span>',
      }) +
        UI.inset(UI.radioGroup(cards('read', readWire, false), { label: 'Who can read' }) + visRow('read', readWire)) +
        '<p class="hint">At <b>' + esc(roleLabel(readWire)) + '</b> this item would be readable by <b>' +
        admitted.length + ' of ' + roster.length + '</b> in ' + esc(store.name) + ': ' +
        esc(admitted.map(D.partyName).join(', ') || 'nobody') + '.' +
        (excluded.length ? ' ' + esc(excluded.map(D.partyName).join(', ')) +
          ' is not counted when its role or group membership provides no access here.' : '') + '</p>' +
        UI.sectionLabel('Who can change <span class="pv">changeable by <b>' + changers.length + ' of ' + roster.length + '</b></span>') +
        UI.inset(UI.radioGroup(cards('write', writeWire, true), { label: 'Who can change' }) + visRow('write', writeWire)) +
        '<p class="hint">The read and write roles are independent and carried by this group item, so someone ' +
        'allowed to change it might not be allowed to read it. They are checked against the current ' +
        'authenticated roster, so the preview is computed rather than typed.</p>';
    }
    function newSheet(s, w) {
      var kind = SHEET_KIND[s.v.sheet];
      var storeId = s.v.dest || createStoreFor(s, w);
      var store = w.storeById[storeId] || w.storeById[createStoreFor(s, w)];
      var draft = draftOf(s, kind);
      var canWrite = !!(store && canCreateInStore(w, store.id));
      var blocked = store ? writeBlockReason(w, store) : null;

      var chooser = w.stores.length
        ? UI.cardSelect(saveInOptions(s, w))
        : UI.insetRow({ label: 'Vault', value: '<span class="dim">No vault is available to save into.</span>' });

      var fields = '';
      if (kind === 'Password') {
        fields = UI.field({ label: 'Site', value: draft.site, placeholder: 'e.g. github.com', bind: 'v.site' }) +
          UI.field({ label: 'User name', value: draft.username, placeholder: 'username', bind: 'v.user' }) +
          UI.field({ label: 'Password', value: draft.password, type: 'password', placeholder: '', bind: 'v.pw' }) +
          UI.field({ label: 'Website', value: draft.website, placeholder: 'https://github.com/login', bind: 'v.web' });
      } else if (kind === 'Resource') {
        fields = UI.field({ label: 'Name', value: draft.resourceName, placeholder: 'e.g. ANTHROPIC_API_KEY', bind: 'v.rname' }) +
          UI.field({ label: 'Value', value: draft.value, placeholder: 'sk-ant-…', mono: true, bind: 'v.val' });
      } else if (kind === 'Link') {
        fields = UI.field({ label: 'Points to', value: draft.target, placeholder: '/ssh/id_ed25519', mono: true, bind: 'v.target' });
      } else {
        fields = UI.insetRow({
          label: 'File', value: '<span class="dim">Drop a file here, or use the native picker</span>',
        });
      }

      var body = UI.sectionLabel('Save in') + UI.inset(chooser) +
        (store && blocked ? UI.band({ text: esc(store.name + ': ' + blocked) }) : '') +
        (store ? accessBlock(s, w, store) : '') +
        UI.sectionLabel(esc(D.kindLabel(kind))) + UI.inset(fields) +
        UI.toggle({
          label: 'Advanced', className: 'sheet-advanced', open: s.v.advanced === '1',
          attrs: 'data-act="set" data-key="v.advanced" data-val="' + (s.v.advanced === '1' ? '' : '1') + '"',
          body: UI.inset(UI.field({ label: 'Path', value: draft.path, placeholder: DRAFT_PATH[kind], bind: 'v.dpath' })) +
            '<p class="hint">Where this lands in the vault. Every kind fills it in from what you typed above — ' +
            'the site, the name, the dropped file — and typing here stops that only until the field it follows ' +
            'changes again. Any folder in the path that does not exist yet is created with this item.</p>',
        });

      return labelledSheet({
        ariaLabel: 'New ' + D.kindLabel(kind).toLowerCase(), width: 'mid',
        dismiss: 'data-act="call" data-fn="itemsCloseSheet"',
        glyph: UI.kindIcon(kind),
        title: 'New ' + esc(D.kindLabel(kind).toLowerCase()),
        subtitle: kind === 'Password' ? 'A login with a masked password.'
          : kind === 'Resource' ? 'A value such as an API key or recovery code.'
            : kind === 'File' ? 'A file streamed from its path by the local agent.'
              : 'A path pointing to another path in the same store.',
        body: body,
        footer: UI.btn('Cancel', { attrs: 'data-act="call" data-fn="itemsCloseSheet"' }) +
          UI.btn(kind === 'File' && !s.v.src ? 'Choose file and create' : 'Create in this vault',
            { variant: 'primary', disabled: !canWrite, attrs: 'data-act="call" data-fn="itemsCreate"' }),
      });
    }
    function existsSheet(s, w) {
      var kind = s.v.ekind || 'Password';
      var storeId = s.v.dest || 'acct:personal';
      var path = s.v.epath || DRAFT_PATH[kind];
      var clash = w.items.filter(function (i) { return i.store === storeId && i.path === path; })[0];
      var store = w.storeById[storeId];
      return labelledSheet({
        ariaLabel: 'Creation refused because the path exists',
        dismiss: 'data-act="call" data-fn="itemsCloseSheet"',
        glyph: clash ? UI.kindIcon(D.kindOf(clash)) : '',
        title: 'Something is already at ' + esc(path),
        subtitle: 'New ' + esc(D.kindLabel(kind).toLowerCase()) + ' · not created',
        body: '<p>Nothing was created and nothing was overwritten. Creating carries “must not exist”' +
          (clash ? ', and ' + esc(D.nameOf(path)) + ' was listed at version ' + clash.version +
            ' in ' + esc(store ? store.name : '') : '') + '.</p>' +
          '<p class="fn">Refresh and open what is there to review its exact version, or save this one at ' +
          'another path. There is no “create anyway”.</p>',
        footer: UI.btn('Change the path', { attrs: 'data-act="call" data-fn="itemsChangePath"' }) +
          UI.btn(clash ? 'Open version ' + clash.version : 'Open existing item',
            { variant: 'primary', attrs: 'data-act="call" data-fn="itemsOpenExisting"' }),
      });
    }
    function conflictItem(s, w) {
      return selectedItem(s, w) || w.items.filter(function (i) { return D.isLogin(i); })[0] || w.items[0];
    }
    function conflictSheet(s, w) {
      var item = conflictItem(s, w);
      if (!item) return '';
      return labelledSheet({
        ariaLabel: 'Edit conflict',
        glyph: UI.kindIcon(D.kindOf(item)),
        title: 'Someone else changed this first',
        subtitle: esc(D.nameOf(item.path)) + ' · save refused',
        body: '<p>You edited version ' + item.version + ', but that exact version is no longer current, ' +
          'so nothing was saved and nothing was overwritten.</p>' +
          '<p class="fn">Refresh to see the current version beside your retained draft, review it, and save ' +
          'again under the refreshed version. There is no “save anyway” and Retry never replays this write.</p>',
        footer: UI.btn('Discard my edit', { attrs: 'data-act="set" data-key="v.sheet" data-val="discard"' }) +
          UI.btn('Refresh and review', { variant: 'primary', attrs: 'data-act="call" data-fn="itemsRefreshConflict"' }),
      });
    }
    function discardSheet(s, w) {
      var item = conflictItem(s, w);
      if (!item) return '';
      return labelledSheet({
        ariaLabel: 'Discard your edit', danger: true,
        glyph: '<span class="kico md danger">' + M.icon('trash') + '</span>',
        title: 'Discard your edit?',
        subtitle: esc(D.nameOf(item.path)) + ' · not saved anywhere',
        body: '<p>Your draft has not been saved. Discarding it here is the only copy gone — the item itself ' +
          'is untouched at its current version.</p>',
        footer: UI.btn('Keep editing', { attrs: 'data-act="set" data-key="v.sheet" data-val="conflict"' }) +
          UI.btn('Discard my edit', { variant: 'primary', className: 'danger', attrs: 'data-act="call" data-fn="itemsDiscard"' }),
      });
    }
    /* SheetDialog: labelled by the <h2>'s id, not by aria-label. */
    function removeSheet(s, w) {
      var item = selectedItem(s, w);
      if (!item) return '';
      return '<div class="backdrop" role="alertdialog" aria-modal="true" aria-labelledby="items-remove-title" ' +
        'tabindex="-1" data-act="call" data-fn="itemsCloseSheet">' +
        sheetPanel({
          titleId: 'items-remove-title',
          glyph: '<span class="kico md danger">' + M.icon('trash') + '</span>',
          title: 'Remove ' + esc(D.nameOf(item.path)) + '?',
          subtitle: esc(item.path),
          body: '<p>This will remove the item. This can’t be undone.</p>',
          footer: UI.btn('Cancel', { attrs: 'data-act="call" data-fn="itemsCloseSheet"' }) +
            UI.btn('Remove', { variant: 'primary', className: 'danger', attrs: 'data-act="call" data-fn="itemsConfirmRemove"' }),
        }) + '</div>';
    }

    /* ======================================================== the renderer
       `sync` is this part's syncWorld: the fixture is put back and this
       state's knobs are replayed before M.data.world(s) is read. */
    function sync(s) {
      syncItems(s);
      /* `adhoc` is the groups part's key for a resumed ad-hoc group
         (resumeGroupCreation landed). Only 60-groups' own syncWorld puts the
         store back, so honour the knob here too: `store-homelab` must show
         the active empty vault at `adhoc=yes` and the Setup-incomplete
         takeover without it, whichever page was visited before. */
      var homelab = D.stores.filter(function (st) { return st.id === 'team:homelab'; })[0];
      if (homelab) homelab.active = (s.v.adhoc === 'yes');
      /* app-root.tsx:431-456 — a connection loss sets the workflow to
         `agent-lost`, so it TAKES the one workflow slot: whatever sheet was up
         is gone, not layered under the stop. The same message bumps
         `concealSignal`, which drops any revealed value and cancels an open
         editor. `agent` is app-wide here while `sheet` and `reveal` are view
         keys, so the deck can ask for both; fold them the way the app does. */
      if (s.agent === 'lost') { s.v.sheet = null; s.v.reveal = null; }
    }
    function renderPage(ref) {
      return function (s) {
        sync(s);
        var t = M.globalTakeover(s);
        if (t) return t;
        var w = D.world(s);
        /* write-workflows.tsx: the conflict (and the discard question behind
           it) is raised by an edit, so the item it is about is the selected
           one. A cold `sheet=conflict` selects it the way the app's demo
           selection does — only cold, though: `details` is untouched then, and
           an Escape over the sheet (which deselects) has already pinned it. */
        if ((s.v.sheet === 'conflict' || s.v.sheet === 'discard') && !s.v.sel && s.v.details == null) {
          var subject = conflictItem(s, w);
          if (subject) { s.v.sel = D.itemKey(subject); s.v.details = 'shown'; }
        }
        var store = ref ? w.storeById[ref] : null;
        if (store && store.state !== 'normal') {
          return { main: UI.main(takeover(s, w, store)) };
        }
        var items = scopedItems(s, w, ref);
        var open = detailsShown(s, w, store);
        var body = '';
        if (!ref) body += w.bands.map(function (band) { return UI.band({ text: esc(band.text) }); }).join('');
        if (viewOf(s) === 'grid' && items.length > 200) {
          body += UI.band({ text: 'Showing the first 200 cards. Narrow the list with search or a kind filter to see the rest.' });
        }
        body += !items.length ? emptyBody(s, store)
          : viewOf(s) === 'list' ? listBody(s, w, items) : gridBody(s, w, items, ref);
        /* The conflict and its discard question are the kit's plain `Dialog`,
           which does not portal: the app draws them as a sibling of `.app`
           inside `.window` (cap/shell/conflict.html), where the mock's
           `windowTail` goes. New / exists / remove are DismissibleDialog and
           SheetDialog, which do portal, so they stay in overlay(s). An agent
           loss replaces the whole workflow in the app, so the stop wins. */
        var tail;
        if ((s.v.sheet === 'conflict' || s.v.sheet === 'discard') && s.agent !== 'lost') {
          tail = s.v.sheet === 'conflict' ? conflictSheet(s, w) : discardSheet(s, w);
        }
        return {
          appClass: open ? 'with-details' : '',
          main: UI.main(pageHeader(s, store) + toolbar(s, w, store, open) + UI.body(body)),
          details: open ? detailsPanel(s, w) : '',
          windowTail: tail,
        };
      };
    }
    function renderOverlay(s) {
      var w = D.world(s);
      var sheet = s.v.sheet;
      /* A store-access takeover draws no toolbar, so there is no trigger for a
         New or Sort menu to hang off — the app has nothing open there. */
      var page = M.pages[s.page];
      var ref = page && page.ref;
      var covered = !!(ref && w.storeById[ref] && w.storeById[ref].state !== 'normal');
      var menu = SHEET_KIND[sheet]
        ? (s.v.menu === 'save-in' && w.stores.length ? UI.cardSelectMenu(saveInOptions(s, w)) : '')
        : covered ? ''
          : s.v.menu === 'new' || s.v.menu === 'new-empty' ? menuPortal('What to create', newMenuHtml())
            : s.v.menu === 'sort' ? menuPortal('Sort the list by', sortMenuHtml(s)) : '';
      if (SHEET_KIND[sheet]) return newSheet(s, w) + menu;
      if (sheet === 'exists') return existsSheet(s, w);
      if (sheet === 'remove') return removeSheet(s, w);
      return menu;
    }

    /* ============================================================ handlers */
    function currentItem() {
      return selectedItem(M.s, D.world(M.s));
    }
    function itemFor(arg) {
      if (!arg) return currentItem();
      var cut = arg.indexOf('|');
      return D.itemAt(arg.slice(0, cut), arg.slice(cut + 1));
    }
    function clearDraft() {
      ['site', 'user', 'pw', 'web', 'val', 'rname', 'target', 'dpath', 'src', 'advanced', 'draft']
        .forEach(function (k) { M.s.v[k] = null; });
    }

    M.fns.itemsSelect = function (arg) {
      M.s.v.reveal = null;
      M.s.v.sheet = null;
      M.s.v.details = 'shown';
      M.set('v.sel', arg);
    };
    M.fns.itemsDetails = function () {
      var w = D.world(M.s);
      var page = M.pages[M.s.page];
      M.set('v.details', detailsShown(M.s, w, page && page.ref ? w.storeById[page.ref] : null) ? 'hidden' : 'shown');
    };
    M.fns.itemsCloseDetails = function () { M.set('v.details', 'hidden'); };
    M.fns.itemsSort = function (arg) {
      M.s.v.menu = null;
      M.set('v.sort', arg === 'name' ? null : arg);
    };
    M.fns.itemsShow = function () { M.set('v.reveal', '1'); };
    M.fns.itemsHide = function () { M.set('v.reveal', null); };

    M.fns.itemsCopyValue = function (arg) {
      var item = itemFor(arg);
      if (!item) return;
      M.toast((D.kindOf(item) === 'Password' ? 'Password' : 'Value') + ' copied');
    };
    M.fns.itemsCopyPath = function (arg) {
      var item = itemFor(arg);
      if (!item) return;
      M.toast('Path copied: ' + item.path);
    };
    M.fns.itemsDownload = function (arg) {
      var item = itemFor(arg);
      if (!item) return;
      M.s.v.sel = D.itemKey(item);
      M.s.v.details = 'shown';
      M.toast('Downloaded ' + D.nameOf(item.path) + ' at version ' + item.version);
      M.render();
    };
    M.fns.itemsPanelDownload = function () {
      var item = currentItem();
      if (item) M.toast('Downloaded version ' + item.version);
    };
    M.fns.itemsReveal = function (arg) {
      M.s.v.sel = arg;
      M.s.v.details = 'shown';
      M.set('v.reveal', '1');
    };
    function openTargetOf(arg) {
      var item = itemFor(arg);
      if (!item) return null;
      var target = D.plaintextOf(item);
      var w = D.world(M.s);
      /* Resolved against the catalog, not the raw item list — the raw list
         still holds folders (`/ssh`), and selecting one reaches KindIcon with
         a kind it does not draw. */
      var hit = w.items.filter(function (i) { return i.store === item.store && i.path === target; })[0];
      return { target: target, hit: hit || null };
    }
    function selectTarget(found) {
      M.s.v.reveal = null;
      M.s.v.details = 'shown';
      M.set('v.sel', D.itemKey(found.hit));
    }
    /* The card's quick action (items-screen.tsx:447-470): reads the link at its
       exact version, then selects — or says nothing is there. */
    M.fns.itemsOpenTarget = function (arg) {
      var found = openTargetOf(arg);
      if (!found) return;
      if (!found.hit) return M.toast('Nothing is currently at ' + found.target);
      selectTarget(found);
    };
    /* The details panel's button (details-panel.tsx:720-747) selects the target
       or does nothing at all — it raises no toast. */
    M.fns.itemsPanelOpenTarget = function (arg) {
      var found = openTargetOf(arg);
      if (found && found.hit) selectTarget(found);
    };

    M.fns.itemsNew = function (kind) {
      var w = D.world(M.s);
      var page = M.pages[M.s.page];
      var ref = page && page.ref;
      clearDraft();
      M.s.v.menu = null;
      M.s.v.dest = createStoreFor(M.s, w);
      M.set('v.sheet', 'new-' + kind.toLowerCase());
    };
    M.fns.itemsSaveIn = function (arg) {
      M.s.v.menu = null;
      M.set('v.dest', arg);
    };
    /* Cancel / the backdrop / Escape are all `setWorkflow(null)`, and the draft
       lives in the workflow component's own state: closing the sheet is what
       throws it away, so reopening New starts from the kind's defaults again. */
    M.fns.itemsCloseSheet = function () {
      clearDraft();
      M.set('v.sheet', null);
    };
    M.fns.itemsCreate = function () {
      var s = M.s, w = D.world(s);
      var kind = SHEET_KIND[s.v.sheet];
      var storeId = s.v.dest || createStoreFor(s, w);
      var store = w.storeById[storeId];
      if (!store || !kind) return;
      var draft = draftOf(s, kind);
      var path = draft.path;
      if (!path || path.charAt(0) !== '/') return;
      if (D.itemAt(storeId, path)) {
        s.v.ekind = kind;
        s.v.epath = path;
        return M.set('v.sheet', 'exists');
      }
      var roles = store.kind === 'account'
        ? { read: { role: 'Owner' }, write: { role: 'Owner' } }
        : { read: roleWireOf(s, 'read'), write: roleWireOf(s, 'write') };
      TYPED[storeId + '|' + path] = kind === 'Password'
        ? 'user: ' + draft.username + '\npassword: ' + draft.password + '\nurl: ' + draft.website
        : kind === 'Resource' ? draft.value
          : kind === 'Link' ? draft.target : '';
      pushOp('new~' + kind + '~' + (SHORT[storeId] || storeId) + '~' + path + '~' +
        wireWord(roles.read) + '~' + wireWord(roles.write));
      clearDraft();
      s.v.sheet = null;
      M.toast(D.kindLabel(kind) + ' created in ' + store.name);
      M.render();
    };
    M.fns.itemsChangePath = function () {
      M.set('v.sheet', 'new-' + String(M.s.v.ekind || 'Password').toLowerCase());
    };
    M.fns.itemsOpenExisting = function () {
      var s = M.s;
      var storeId = s.v.dest || 'acct:personal';
      var path = s.v.epath || DRAFT_PATH[s.v.ekind || 'Password'];
      var item = D.itemAt(storeId, path);
      /* write-workflows.tsx: Open version N refreshes the catalog first, so a
         path that has since been freed refuses rather than selecting nothing —
         the draft stays and the sheet stays up. */
      if (!item) {
        return M.toast('That path is free now. Your draft is still here; change the path or try creating it again.',
          { tone: 'warning' });
      }
      s.v.sheet = null;
      s.v.details = 'shown';
      s.v.sel = D.itemKey(item);
      M.render();
    };

    M.fns.itemsEdit = function () {
      var item = currentItem();
      if (!item) return;
      M.s.v.reveal = null;
      M.s.v.draft = null;
      M.set('v.sheet', D.kindOf(item) === 'File' ? 'replace' : 'edit');
    };
    M.fns.itemsCancelEdit = function () {
      M.s.v.draft = null;
      M.set('v.sheet', null);
    };
    M.fns.itemsSave = function () {
      var s = M.s;
      var item = currentItem();
      if (!item) return;
      var row = D.itemAt(item.store, item.path);
      var next = row.version + 1;
      if (!(D.kindOf(row) === 'File' || s.v.sheet === 'replace')) {
        TYPED[D.itemKey(row)] = s.v.draft != null ? s.v.draft : editableValue(row, D.plaintextOf(row) || '');
      }
      pushOp('save~' + (SHORT[row.store] || row.store) + '~' + row.path + '~' + next);
      s.v.draft = null;
      s.v.sheet = null;
      M.toast('Saved version ' + next);
      M.render();
    };
    M.fns.itemsRefreshConflict = function () {
      var item = conflictItem(M.s, D.world(M.s));
      M.s.v.sheet = 'edit';
      if (item) {
        M.s.v.sel = D.itemKey(item);
        M.s.v.details = 'shown';
      }
      M.toast('Refreshed the catalog — review your retained draft');
      M.render();
    };
    M.fns.itemsDiscard = function () {
      M.s.v.draft = null;
      M.s.v.reveal = null;
      M.set('v.sheet', null);
    };

    M.fns.itemsRemove = function (arg) {
      var item = itemFor(arg);
      if (!item) return;
      M.s.v.sel = D.itemKey(item);
      M.s.v.details = 'shown';
      M.set('v.sheet', 'remove');
    };
    M.fns.itemsConfirmRemove = function () {
      var item = currentItem();
      if (!item) return;
      var version = item.version;
      pushOp('rm~' + (SHORT[item.store] || item.store) + '~' + item.path);
      M.s.v.sheet = null;
      M.s.v.sel = null;
      M.s.v.reveal = null;
      M.s.v.details = 'shown';
      M.toast('Removed version ' + version);
      M.render();
    };
    /* store-access.tsx: Finish setup runs resumeGroupCreation and refreshes.
       That is the groups part's write (`v.adhoc`), so hand it over — the
       Homelab page then renders the resumed, active ad-hoc vault. */
    M.fns.itemsFinishSetup = function () {
      if (M.fns.gFinishSetup) return M.fns.gFinishSetup();
      M.set('v.adhoc', 'yes');
      M.toast('Group creation resumed');
    };

    /* app-root.tsx:482-497 plus the dialogs that handle Escape first. The
       conflict sheet asks before it discards; the discard question backs out
       to the conflict sheet. Deselecting leaves the panel open. */
    var baseEscape = M.fns.escape;
    function itemsEscape() {
      if (!(M.s.page in ITEM_PAGES)) return baseEscape();
      var v = M.s.v;
      /* ConflictSheet's own onKeyDown flips the discard question but never
         calls preventDefault, so app-root's window rule runs on the SAME
         press: Escape over the conflict both turns the question over and
         clears the query, or drops the selection behind it. The app really
         does leave an empty details panel behind the sheet (steps
         conflict-escape). Every other dialog here is dismissed by the kit,
         which does stop the event. */
      if (v.sheet === 'conflict' || v.sheet === 'discard') {
        v.sheet = v.sheet === 'conflict' ? 'discard' : 'conflict';
        if (v.search) v.search = null;
        else if (v.sel) { v.details = 'shown'; v.sel = null; }
        return M.render();
      }
      /* Innermost first, which is the order the kit's overlays listen in: the
         Save-in listbox inside the New sheet takes Escape and the sheet stays
         up (`?state=new-resource` → open the chooser → Escape). */
      if (v.menu) return M.set('v.menu', null);
      /* New / exists / remove are the kit's dialogs: they handle Escape
         themselves and stop it, so it dismisses them and nothing else, and the
         draft goes with the workflow. `edit` and `replace` are NOT dialogs —
         they are the details panel's own two modes — so Escape there is only
         app-root's window rule: the query first, the selection second. The
         editor dies with the selection (details-panel drops it on change),
         which is why Escape over an open editor leaves an empty panel rather
         than the item back in view mode. */
      if (v.sheet && v.sheet !== 'edit' && v.sheet !== 'replace') {
        clearDraft();
        return M.set('v.sheet', null);
      }
      if (v.search) return M.set('v.search', null);
      if (v.sel) {
        v.details = 'shown';
        v.sheet = null;
        v.draft = null;
        v.reveal = null;
        return M.set('v.sel', null);
      }
    }
    M.fns.escape = itemsEscape;

    /* components/search-field.tsx:33-43 — ⌘K / Ctrl+K focuses the field
       anywhere in the window; it does not open a palette. */
    document.addEventListener('keydown', function (ev) {
      if (ev.key !== 'k' && ev.key !== 'K') return;
      if (!(ev.metaKey || ev.ctrlKey)) return;
      if (!(M.s.page in ITEM_PAGES)) return;
      var input = document.querySelector('#win label.search input');
      if (!input) return;
      ev.preventDefault();
      input.focus();
    });

    /* details-panel.tsx:328-342 — "JS strings cannot be zeroized", so the panel
       drops the plaintext when the window loses focus: the read goes, and so
       does an open editor (which holds the same secret in its textarea). That
       is the promise the Link footnote makes out loud — "FOKS hides it again
       when the window loses focus" — so the mock keeps it. */
    window.addEventListener('blur', function () {
      if (!(M.s.page in ITEM_PAGES)) return;
      var v = M.s.v;
      if (!v.reveal && v.sheet !== 'edit' && v.sheet !== 'replace') return;
      v.reveal = null;
      v.draft = null;
      if (v.sheet === 'edit' || v.sheet === 'replace') v.sheet = null;
      M.render();
    });

    /* The core's Escape hook ignores keystrokes inside a field, but the app's
       does not: the dialog listens on itself, so Escape in a sheet's input
       still closes the sheet (`?state=new` → type in Site → Escape), and
       search-field.tsx:45-50 clears the query and stops propagation, so the
       selection survives that first press. Both live here. */
    document.addEventListener('keydown', function (ev) {
      if (ev.key !== 'Escape') return;
      var el = ev.target;
      if (!el || !/INPUT|TEXTAREA|SELECT/.test(el.tagName)) return;
      if (!(M.s.page in ITEM_PAGES)) return;
      if (!el.closest || !el.closest('#win, #overlays')) return;
      if (el.closest('label.search')) {
        if (M.s.v.search) M.set('v.search', null);
        return;
      }
      itemsEscape();
    });

    /* items-screen.tsx:165-169 and 220-224 — a row and a tile are
       `role="button" tabindex="0"`, so Enter and Space select the item they
       are on (both preventDefault, so Space does not scroll the body). The
       core only listens for clicks, so the keyboard half lives here. */
    document.addEventListener('keydown', function (ev) {
      if (ev.key !== 'Enter' && ev.key !== ' ') return;
      if (!(M.s.page in ITEM_PAGES)) return;
      var el = ev.target;
      if (!el || !el.closest) return;
      var hit = el.closest('#win .body .row, #win .body .tile');
      if (!hit) return;
      ev.preventDefault();
      hit.click();
    });

    /* ui/kit/overlay-primitives.tsx dismisses a Popover on an outside
       pointer-down, so the New, Sort and Save-in lists close as soon as the
       pointer lands anywhere but their own trigger or list. Without this the
       mock's menus stay up for the rest of the session.

       It runs on `click`, not `pointerdown`: this listener is registered after
       the core's, so the core has already handled the same click — closing on
       pointerdown would re-render and pull the row out from under it, and the
       app selects the row it was clicked on. A click in the control deck is
       how the deck OPENS a menu, so the deck is left alone. */
    document.addEventListener('click', function (ev) {
      if (!M.s.v.menu) return;
      if (!(M.s.page in ITEM_PAGES)) return;
      var el = ev.target;
      if (!el || !el.closest) return;
      if (el.closest('.menuwrap, .menu-portal, .card-select, .mock-controls')) return;
      M.set('v.menu', null);
    });

    /* ============================================================== pages */
    /* Chip labels are prose: the group label is the subject and the chip is
       its value (`Kind filter` · `Files`). They no longer have to differ from
       the frame's own words — capture.mjs's clickText looks inside `#frame`
       first — so a chip says what the app says. */
    var CONTROLS = [
      { key: 'view', label: 'View', note: 'state.view — the list/cards segmented control.',
        values: [{ v: '', label: 'List' }, { v: 'grid', label: 'Cards' }] },
      { key: 'kind', label: 'Kind filter', note: 'state.kind — the segmented control; the empty state follows it.',
        values: [{ v: '', label: 'All' }, { v: 'Password', label: 'Passwords' },
          { v: 'Resource', label: 'Notes' }, { v: 'File', label: 'Files' }, { v: 'Link', label: 'Links' }] },
      { key: 'sort', label: 'Sort', note: 'scope.ts SORTS. Grouped = store name then item name; version is descending.',
        values: [{ v: '', label: 'Name' }, { v: 'kind', label: 'Kind' },
          { v: 'group', label: 'Grouped' }, { v: 'version', label: 'Version' }] },
      { key: 'search', label: 'Search', note: 'Paths and store names only — never contents. Rows grow their full path.',
        values: [{ v: '', label: 'None' }, { v: 'wifi', label: 'wifi' }, { v: 'logins', label: 'logins' },
          { v: 'Household', label: 'Household' }, { v: 'zzzz', label: 'zzzz (no match)' }] },
      { key: 'sel', label: 'Selection', note: 'M.data.itemKey — "<storeRef>|<path>". A selection forces the panel open.',
        values: [{ v: '', label: 'None' },
          { v: 'acct:personal|/logins/github.com', label: 'github.com' },
          { v: 'acct:personal|/agents/anthropic-api-key', label: 'anthropic-api-key' },
          { v: 'acct:personal|/latest-key', label: 'latest-key' },
          /* What Read target resolves to, and the File the Replace row is
             about — both are flow steps, so both light a chip. */
          { v: 'acct:personal|/ssh/id_ed25519', label: 'id_ed25519' },
          { v: 'acct:personal|/documents/passport-scan.pdf', label: 'passport-scan.pdf' },
          { v: 'team:household|/documents/emergency.pdf', label: 'emergency.pdf' },
          { v: 'team:household|/wifi/guest-password', label: 'guest-password' },
          { v: 'team:eng|/deploy/production-token', label: 'production-token' }] },
      { key: 'details', label: 'Details panel', note: 'The toolbar toggle. Null follows the selection; deselecting leaves it open.',
        values: [{ v: '', label: 'Follows the selection' }, { v: 'shown', label: 'Open' }, { v: 'hidden', label: 'Closed' }] },
      { key: 'reveal', label: 'Reveal', note: 'The one exact-version read: Show, or Read target on a Link.',
        values: [{ v: '', label: 'Masked' }, { v: '1', label: 'Revealed' }] },
      { key: 'sheet', label: 'Sheet', note: 'The write overlays; edit/replace are the details panel’s two editing modes.',
        values: [{ v: '', label: 'None' }, { v: 'new-password', label: 'New password' },
          { v: 'new-resource', label: 'New note' }, { v: 'new-file', label: 'New file' },
          { v: 'new-link', label: 'New link' }, { v: 'edit', label: 'Edit' },
          { v: 'replace', label: 'Replace' }, { v: 'remove', label: 'Remove' },
          { v: 'exists', label: 'Already exists' }, { v: 'conflict', label: 'Edit conflict' },
          { v: 'discard', label: 'Discard the edit' }] },
      { key: 'menu', label: 'Menu', note: 'MenuButton open-ness is view state here; save-in is the sheet’s CardSelect.',
        values: [{ v: '', label: 'None' }, { v: 'new', label: 'New' },
          { v: 'new-empty', label: 'New (from the empty state)' }, { v: 'sort', label: 'Sort' },
          { v: 'save-in', label: 'Save in' }] },
      { key: 'dest', label: 'Create destination', note: 'Which store the New sheet writes to — the store being looked at, or defaultCreateStore, when unset.',
        values: [{ v: '', label: 'Default' }, { v: 'acct:personal', label: 'Personal' },
          { v: 'acct:work', label: 'Work (Acme)' }, { v: 'team:eng', label: 'Engineering' },
          { v: 'team:household', label: 'Household' }, { v: 'team:homelab', label: 'Homelab' }] },
      { key: 'readRole', label: 'Create read role', note: 'The group create’s read role; readVis carries the Member band.',
        values: [{ v: '', label: 'Member' }, { v: 'Admin', label: 'Admin' }, { v: 'Owner', label: 'Owner' }] },
      { key: 'writeRole', label: 'Create write role', note: 'The group create’s write role; writeVis carries the Member band.',
        values: [{ v: '', label: 'Admin' }, { v: 'Owner', label: 'Owner' }, { v: 'Member', label: 'Member' }] },
      { key: 'advanced', label: 'Advanced disclosure', note: 'The New sheet’s Path disclosure.',
        values: [{ v: '', label: 'Closed' }, { v: '1', label: 'Open' }] },
      /* The three drafts the sheets and the editor read. They are free text,
         so the chips are samples — but the page reads them, so it declares
         them: an undeclared key is dropped by M.go and drawn dimmed. */
      /* Null reads as Password (`s.v.ekind || 'Password'`), so the default chip
         carries that word and clears the key — the README's rule, and what
         lights on `create-item#9`, which sets `sheet=exists` and no `ekind`. */
      { key: 'ekind', label: 'Refused create kind', note: 'Which create the `exists` refusal is about — the sheet’s “New {kind} · not created”.',
        values: [{ v: '', label: 'Password' }, { v: 'Resource', label: 'Note' },
          { v: 'File', label: 'File' }, { v: 'Link', label: 'Link' }] },
      { key: 'epath', label: 'Refused create path', note: 'The path the `exists` refusal is about; the clashing item’s version comes from the catalog.',
        values: [{ v: '', label: 'From the kind' }, { v: '/logins/github.com', label: '/logins/github.com' },
          { v: '/agents/anthropic-api-key', label: '/agents/anthropic-api-key' },
          { v: '/nothing/here', label: '/nothing/here (not in the catalog)' }] },
      { key: 'draft', label: 'Retained draft', note: 'The editor’s textarea — what “Refresh and review” brings back beside the refreshed version.',
        values: [{ v: '', label: 'From the item' }, { v: 'flint-Harbor-19-quay', label: 'flint-Harbor-19-quay' }] },
      { key: 'inspect', label: 'Response disclosure', note: 'The details panel’s “Inspect response” disclosure.',
        values: [{ v: '', label: 'Closed' }, { v: '1', label: 'Open' }] },
      { key: 'applied', label: 'Writes applied', note: 'The mutations, replayed onto the fixture each render — clicking Create / Save / Remove in the frame pushes the same ops.',
        values: [{ v: '', label: 'Nothing written' }, { v: 'created-password', label: 'Created gitlab.com' },
          { v: 'created-note', label: 'Created deploy-token' }, { v: 'created-file', label: 'Created insurance.pdf' },
          { v: 'created-link', label: 'Created current-key' }, { v: 'saved-github', label: 'Saved github.com v10' },
          { v: 'removed-github', label: 'Removed github.com' }] },
      /* resumeGroupCreation is the groups part's write, under the groups
         part's key and label so the deck dedupes them. Every vault page
         declares it: it is a world change, so it survives navigation (and the
         sidebar's Homelab caption follows it), not just the Homelab page. */
      { key: 'adhoc', label: 'Homelab resumed', note: 'resumeGroupCreation landed: Homelab becomes an active ad-hoc vault instead of the Setup-incomplete takeover.',
        values: [{ v: '', label: 'Not resumed' }, { v: 'yes', label: 'Resumed' }] },
    ];

    Object.keys(ITEM_PAGES).forEach(function (id) {
      var ref = ITEM_PAGES[id];
      var name = ref ? (D.storeOf(ref) || {}).name : 'All items';
      M.page({
        id: id,
        title: name,
        path: ['Vault', name],
        nav: ref || 'all',
        ref: ref,
        note: ref
          ? 'items-screen.tsx scoped to ' + ref + '; a store-access problem replaces the whole page.'
          : 'items-screen.tsx at {kind:"all"} — every readable store, plus the storeAccessBands.',
        controls: CONTROLS,
        render: renderPage(ref),
        overlay: renderOverlay,
        /* location.ts:120-158 — a different location drops the selection, but
           `details` is its own flag and survives. The core clears `sel` for
           us; pin the flag first so the panel stays open and empty, exactly
           as it does after Escape. */
        leave: function (s) {
          var w = D.world(s);
          var store = ref ? w.storeById[ref] : null;
          s.v.details = detailsShown(s, w, store) ? 'shown' : 'hidden';
        },
        after: function (s) {
          /* Re-claimed on every render of a vault page, so a later part that
             installs its own rule does not keep it. */
          M.fns.escape = itemsEscape;
          placeMenus(s);
        },
      });
    });
  }());
