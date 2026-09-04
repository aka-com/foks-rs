/* ----------------------------------------------------------- 30-shell.js
     The window chrome every page shares, and the component helpers that emit
     the app's own DOM (world.md §5).

       M.renderTitlebar(s, page)   .titlebar — lights, FOKS, spacer, agent pill,
                                   the global refresh button.
       M.renderSidebar(s, page)    nav.side — All items / Vaults / Groups /
                                   Shares / footer. `page.nav` says which row
                                   is on: a StoreRef, or 'all' | 'alerts' |
                                   'settings' | 'setup'.
       M.globalTakeover(s)         the boot loading/error screens and the
                                   agent-lost .stopwrap, as a render() result —
                                   or null. EVERY page renderer starts with:

                                       render: function (s) {
                                         var t = M.globalTakeover(s);
                                         if (t) return t;
                                         …
                                       }

       M.windowTail(s)             the agent-loss `.stopwrap`, appended by the
                                   core after `.app` inside `.window` — the
                                   app draws it over, not instead of, the page.
       M.renderGlobalOverlay(s)    a hook for a frame-wide overlay; empty now
                                   that the app lock is a takeover.
       M.ui.*                      the component helpers, below.
       M.fns.refresh               the titlebar refresh handler.
       M.fns.escape                app-root's Escape rule (see the bottom).

     ------------------------------------------------------------------ M.ui

     Conventions, once, for all of them:
       * Content slots (`label`, `body`, `title`, row values, …) are RAW HTML,
         so an icon or a <b> can be composed in. Escape anything you do not
         control with M.esc.
       * Attribute options (`title`, `ariaLabel`, `value`, `placeholder`, …)
         are escaped for you.
       * Every helper accepts `attrs`: a raw attribute string appended to the
         element it owns — that is where a data-act hook goes, e.g.
         `attrs:'data-act="go" data-page="store-eng"'`.
       * Anything undefined is omitted, exactly as the React component omits it.

     | helper | signature | example |
     | --- | --- | --- |
     | `btn` | `btn(label, o)` — `o: {variant:'plain'\|'primary'\|'danger'\|'quiet', size:'md'\|'sm', icon, on, className, title, ariaLabel, disabled, attrs}` | `M.ui.btn('Open server', {attrs:'data-act="go" data-page="srv-list"'})` |
     | `chip` | `chip(text, o{tone:'default'\|'you'\|'warn'\|'ok'\|'bad', title, className, attrs})` | `M.ui.chip('5', {title:'sam.ortiz, …'})` |
     | `tag` | `tag(text, o{tone:'default'\|'warn', title})` | `M.ui.tag('logins')` |
     | `badge` | `badge(count, label)` — **nothing at zero** | `M.ui.badge(2, 'Open alerts')` |
     | `avatar` | `avatar(party, o{className})` — `className` defaults to `av`, the panel roster passes `pav` | `M.ui.avatar(p)` |
     | `stack` | `stack(parties, o{size:'md'\|'lg'\|'xs', title})` — first two only | `M.ui.stack(store.parties)` |
     | `kindIcon` | `kindIcon(kind, className)` | `M.ui.kindIcon('Password','md')` |
     | `kindGlyph` | `kindGlyph(item, size)` — a `/logins/` password becomes the site initial | `M.ui.kindGlyph(item,'big')` |
     | `inset` | `inset(rowsHtml, o{variant:'field'\|'preview', off, className})` | `M.ui.inset(rows)` |
     | `insetRow` | `insetRow(o{label, value, valueClass, action, className, variant, forId, attrs})` | `M.ui.insetRow({label:'Password', value:'…', valueClass:'mono'})` |
     | `field` | `field(o{label, value, type, placeholder, mono, hint, action, bind, id, attrs})` — an InsetRow with a controlled input | `M.ui.field({label:'Name', value:'', bind:'v.name'})` |
     | `sectionLabel` | `sectionLabel(text, o{as:'panel'\|'side', action, className})` | `M.ui.sectionLabel('Vaults',{as:'side'})` |
     | `notice` | `notice(o{severity:'warn'\|'info'\|'crit', eyebrow, title, body, footnote, actions})` | `M.ui.notice({severity:'crit', title:'Check-in expired', body:'<p>…</p>'})` |
     | `band` | `band(o{severity, label, title, action, text})` | `M.ui.band({severity:'crit', text:'…expired.'})` |
     | `segmented` | `segmented(o{items:[{id,label,icon,title}], value, label, variant:'text'\|'icon', key})` — `key` makes each button `data-act="set" data-key=…` | `M.ui.segmented({label:'Which kinds to list', value:'All', key:'v.kind', items:[…]})` |
     | `menuButton` | `menuButton(o{label, menuLabel, align, variant, size, icon, trailingIcon, className, disabled, title, ariaLabel, open, items, menuHtml, openAttrs, closeAttrs})` | see below |
     | `splitButton` | `splitButton(o{label, icon, menuLabel, open, items, menuHtml, mainAttrs, toggleAttrs})` | |
     | `searchField` | `searchField(o{value, placeholder, bind})` | `M.ui.searchField({value:s.v.search, placeholder:M.ui.searchPlaceholder('Household'), bind:'v.search'})` |
     | `sheet` | `sheet(o{glyph, title, subtitle, width:'base'\|'mid'\|'wide', body, footer, danger, closeAttrs, backdropAttrs})` — backdrop + `.sheet[.mid\|.wide]` with `.hd/.sb/.ft` | `M.ui.sheet({title:'New password', body:rows, footer:M.ui.btn('Create',{variant:'primary'})})` |
     | `tabs` | `tabs(o{items:[{id,label,count}], value, label, key\|pages})` | `M.ui.tabs({label:'Group sections', value:'people', items:[…]})` |
     | `toggle` | `toggle(o{label, body, open, id, attrs})` — the lucide chevron disclosure | `M.ui.toggle({label:'Advanced', body:'…', open:s.v.adv==='1'})` |
     | `radioCard` | `radioCard(o{title, detail, selected, off, disabled, tail, className, attrs})`, and `radioGroup(cards, o{label, className})` | |
     | `cardSelect` | `cardSelect(o{label, value, options:[{id,title,detail,off}], open, triggerAttrs, optionAttrs(id)})` | |
     | `copyBox` | `copyBox(o{text, display, label, extra, attrs})` | `M.ui.copyBox({text:'Add sam on acme to Household'})` |
     | `pageHeader` | `pageHeader(o{title, subtitle, lead, tail, action, search})` — the `.path` block; `search` is the `searchField` options | `M.ui.pageHeader({title:'All items', subtitle:'', search:{value:q, bind:'v.search'}})` |
     | `searchPlaceholder` | `searchPlaceholder(title)` — `All items` → `Search all items`; else `Search <title>` unless that is over 18 chars | |
     | `toolbar` | `toolbar(inner)`, `M.ui.spacer()` — `.toolbar` and its `<span class="spacer">` | |
     | `listRow` | `listRow(o{item, storeName, server, selected, attrs, searching})` — the `.row` of items-screen; `storeName` defaults to `M.data.whereOf(item)` | |
     | `listHeader` | `listHeader(o{sort, key})` — the `.hdr` above it | |
     | `emptyState` | `emptyState(o{kind, title, body, action})` / `emptySearch(o{store, query})` | |
     | `body` | `body(inner, o{className})` — `.body` | |

     `menuButton` example (an open menu is view state, not a component state):

       M.ui.menuButton({
         label:'New', menuLabel:'What to create', align:'start', variant:'primary',
         open: s.v.menu === 'new',
         openAttrs:'data-act="set" data-key="v.menu" data-val="new"',
         closeAttrs:'data-act="set" data-key="v.menu" data-val=""',
         menuHtml: M.data.KIND_LIST.map(function (k) {
           return '<button type="button" role="menuitem" class="kind-menu-item" ' +
                  'data-act="set" data-key="v.sheet" data-val="new-' + k.toLowerCase() + '">' +
                  M.ui.kindIcon(k) + M.esc(M.data.kindLabel(k)) + '</button>';
         }).join(''),
       })
  */

var UI = (M.ui = {});
var esc = M.esc;
var uid = 0;
function nextId(prefix) {
  uid += 1;
  return (prefix || 'm') + uid;
}
/* ` name="value"` with the value escaped; nothing at all when undefined. */
function attr(name, value) {
  return value === undefined || value === null || value === false
    ? ''
    : ' ' + name + '="' + esc(value) + '"';
}
function raw(extra) {
  return extra ? ' ' + extra : '';
}
function classes() {
  var out = [];
  for (var i = 0; i < arguments.length; i++)
    if (arguments[i]) out.push(arguments[i]);
  return out.join(' ');
}

/* ------------------------------------------------------------- Button
     components/button.tsx spreads `...rest` FIRST and then writes `type`,
     `className` and `aria-pressed` itself, so the serialized order is

       …rest…  type  class  aria-pressed

     and `rest` is whatever the call site passed, in its own order. The app's
     call sites overwhelmingly read `disabled`, `title`, `aria-label`, so that
     is the order here; `attrs` (the mock's data-act hook, plus aria-haspopup /
     aria-expanded, which the app also passes through rest) goes last inside
     rest. `ariaFirst:true` swaps title and aria-label for the one call site
     that writes them the other way round (the titlebar refresh). */
var VARIANT_CLASS = {
  primary: 'primary',
  plain: '',
  danger: 'danger',
  quiet: 'icon',
};
UI.btn = function (label, o) {
  o = o || {};
  var cls = [
    'btn',
    VARIANT_CLASS[o.variant || 'plain'],
    o.size === 'sm' ? 'cap' : '',
    o.on ? 'on' : '',
    o.className || '',
  ]
    .filter(Boolean)
    .join(' ');
  /* `rest` is serialized in the caller's own order, so `disabled` sits where
       that call site puts it: after the labels for the titlebar refresh
       (app-root.tsx:611-623, the `ariaFirst` call site), before them elsewhere. */
  var dis = o.disabled ? ' disabled=""' : '';
  var labels = o.ariaFirst
    ? attr('aria-label', o.ariaLabel) + attr('title', o.title) + dis
    : dis + attr('title', o.title) + attr('aria-label', o.ariaLabel);
  return (
    '<button' +
    labels +
    raw(o.attrs) +
    ' type="' +
    (o.type || 'button') +
    '" class="' +
    cls +
    '"' +
    (o.on === undefined
      ? ''
      : ' aria-pressed="' + (o.on ? 'true' : 'false') + '"') +
    '>' +
    (o.icon ? M.icon(o.icon) : '') +
    (label == null ? '' : label) +
    '</button>'
  );
};

/* -------------------------------------------------- Chip / Tag / Badge */
UI.chip = function (text, o) {
  o = o || {};
  var cls = [
    'chip',
    o.tone && o.tone !== 'default' ? o.tone : '',
    o.className || '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    '<span class="' +
    cls +
    '"' +
    attr('title', o.title) +
    raw(o.attrs) +
    '>' +
    (text == null ? '' : text) +
    '</span>'
  );
};
UI.tag = function (text, o) {
  o = o || {};
  return (
    '<span class="' +
    (o.tone === 'warn' ? 'tag warn' : 'tag') +
    '"' +
    attr('title', o.title) +
    '>' +
    text +
    '</span>'
  );
};
/* Nothing at zero — the rule that an empty Alerts badge is absent, not "0". */
UI.badge = function (count, label) {
  if (!count) return '';
  return (
    '<span class="badge"' + attr('title', label) + '>' + esc(count) + '</span>'
  );
};

/* ------------------------------------------------------ Avatar / Stack
     React sets `style` through the CSSOM, so the app's serialized DOM carries
     `rgb(162, 132, 94)` where the palette says `#a2845e`. Normalise, so a
     capture of the mock diffs clean against a capture of the app. */
UI.rgb = function (hex) {
  var m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  return m
    ? 'rgb(' +
        parseInt(m[1], 16) +
        ', ' +
        parseInt(m[2], 16) +
        ', ' +
        parseInt(m[3], 16) +
        ')'
    : hex;
};
UI.avatar = function (party, o) {
  o = o || {};
  var cls = o.className || 'av';
  var name = M.data.partyName(party);
  /* class, title, style — react-dom writes `style` through the CSSOM after
       every other attribute, so it is always last however the JSX orders it. */
  if (party.party_kind !== 'user') {
    return (
      '<span class="' +
      cls +
      ' team"' +
      attr('title', name) +
      ' style="background: var(--c-team);">' +
      M.icon('people') +
      '</span>'
    );
  }
  var colour = party.hue || M.data.hue(name);
  return (
    '<span class="' +
    cls +
    '"' +
    attr('title', name) +
    ' style="background: ' +
    UI.rgb(colour) +
    ';">' +
    esc(party.initials || M.data.initials(name)) +
    '</span>'
  );
};
UI.stack = function (parties, o) {
  o = o || {};
  parties = parties || [];
  var cls = ['stack', o.size && o.size !== 'md' ? o.size : '']
    .filter(Boolean)
    .join(' ');
  var inner = parties.length
    ? parties
        .slice(0, 2)
        .map(function (p) {
          return UI.avatar(p);
        })
        .join('')
    : /* No roster at all — the neutral glyph rather than a gap. */
      '<span class="av" style="background: var(--c-none);">' +
      M.icon('people') +
      '</span>';
  return (
    '<span class="' +
    cls +
    '"' +
    attr('title', o.title) +
    '>' +
    inner +
    '</span>'
  );
};

/* ------------------------------------------------ KindIcon / KindGlyph */
UI.kindIcon = function (kind, className) {
  return (
    '<span class="' +
    classes('kic', kind, className) +
    '">' +
    M.icon(M.data.KINDS[kind].icon) +
    '</span>'
  );
};
UI.kindGlyph = function (item, size) {
  if (M.data.isLogin(item)) {
    var site = M.data.nameOf(item.path);
    /* class, aria-hidden, style — `style` last, as everywhere React sets it. */
    return (
      '<span class="' +
      classes('kico', size) +
      '" aria-hidden="true" style="background: ' +
      UI.rgb(M.data.hue(site)) +
      ';">' +
      esc(site[0].toUpperCase()) +
      '</span>'
    );
  }
  var kind = M.data.kindOf(item);
  return (
    '<span class="' +
    classes('kico', size, kind) +
    '">' +
    M.icon(M.data.KINDS[kind].icon) +
    '</span>'
  );
};

/* ------------------------------------------------- Inset / InsetRow / Field */
UI.inset = function (rows, o) {
  o = o || {};
  var cls = [
    o.variant === 'preview' ? 'prev' : 'inset',
    o.off ? 'off' : '',
    o.className || '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    '<div class="' + cls + '"' + raw(o.attrs) + '>' + (rows || '') + '</div>'
  );
};
UI.insetRow = function (o) {
  o = o || {};
  var rowClass = classes(o.variant === 'preview' ? 'irow' : 'fr', o.className);
  var label =
    o.label === undefined
      ? ''
      : o.forId
        ? '<label class="k" for="' + esc(o.forId) + '">' + o.label + '</label>'
        : '<span class="k">' + o.label + '</span>';
  var value =
    o.value === undefined
      ? ''
      : '<span class="' +
        classes('v', o.valueClass) +
        '">' +
        o.value +
        '</span>';
  var action =
    o.action === undefined ? '' : '<span class="a">' + o.action + '</span>';
  return (
    '<div class="' +
    rowClass +
    '"' +
    raw(o.attrs) +
    '>' +
    label +
    value +
    action +
    '</div>'
  );
};
UI.field = function (o) {
  o = o || {};
  var id = o.id || nextId('f');
  /* Attribute order is the app's, from the captures: react-dom defers
       `type` and `value` on a controlled input and InsetRow's cloneElement
       adds `id`, so a Field serializes

         aria-label  class  placeholder  min  max  id  type  value

       — never `type` first, however components/field.tsx writes the JSX. */
  var input =
    '<input' +
    attr('aria-label', o.label) +
    (o.mono ? ' class="mono"' : '') +
    attr('placeholder', o.placeholder) +
    attr('min', o.min) +
    attr('max', o.max) +
    ' id="' +
    esc(id) +
    '"' +
    ' type="' +
    (o.type || 'text') +
    '"' +
    attr('value', o.value == null ? '' : o.value) +
    (o.bind
      ? ' data-bind="' +
        esc(o.bind) +
        '"' +
        (o.live === false ? '' : ' data-live')
      : '') +
    raw(o.inputAttrs) +
    '>';
  return UI.insetRow({
    label: esc(o.label),
    forId: id,
    valueClass: o.mono ? 'mono' : undefined,
    value:
      input + (o.hint === undefined ? '' : '<small>' + o.hint + '</small>'),
    action: o.action,
    className: o.className,
    attrs: o.attrs,
  });
};

/* ------------------------------------------------------- SectionLabel */
UI.sectionLabel = function (text, o) {
  o = o || {};
  if (o.as === 'side')
    return '<h6' + attr('class', o.className) + '>' + text + '</h6>';
  return (
    '<div class="' +
    classes('sec', o.className) +
    '">' +
    text +
    (o.action || '') +
    '</div>'
  );
};

/* --------------------------------------------------------- Notice / Band */
UI.notice = function (o) {
  o = o || {};
  var cls =
    o.severity === 'crit'
      ? 'notice stop'
      : o.severity === 'info'
        ? 'notice info'
        : 'notice';
  return (
    '<div class="' +
    cls +
    '"' +
    raw(o.attrs) +
    '>' +
    (o.eyebrow === undefined
      ? ''
      : '<div class="who">' + o.eyebrow + '</div>') +
    '<h2>' +
    o.title +
    '</h2>' +
    (o.body || '') +
    (o.footnote === undefined ? '' : '<p class="fn">' + o.footnote + '</p>') +
    (o.actions === undefined
      ? ''
      : '<div class="acts2">' + o.actions + '</div>') +
    '</div>'
  );
};
UI.band = function (o) {
  o = o || {};
  var cls =
    o.severity === 'crit'
      ? 'band stop'
      : o.severity === 'info'
        ? 'band info'
        : 'band';
  return (
    '<div class="' +
    cls +
    '"' +
    attr('title', o.title) +
    '>' +
    M.icon(o.severity === 'info' ? 'info' : 'alert') +
    '<span class="t">' +
    (o.label ? '<b>' + o.label + '</b> ' : '') +
    (o.text || '') +
    '</span>' +
    (o.action === undefined ? '' : '<span class="a">' + o.action + '</span>') +
    '</div>'
  );
};

/* ---------------------------------------------------- SegmentedControl
     Note the empty class="" on unselected buttons — React writes the
     attribute either way, and the captures show it. */
UI.segmented = function (o) {
  var on = o.value;
  return (
    '<span class="' +
    (o.variant === 'icon' ? 'seg' : 'seg txt') +
    '" role="group"' +
    attr('aria-label', o.label) +
    '>' +
    o.items
      .map(function (item) {
        var sel = item.id === on;
        var hook = o.key
          ? ' data-act="set" data-key="' +
            esc(o.key) +
            '" data-val="' +
            esc(item.id) +
            '"'
          : raw(item.attrs);
        return (
          '<button type="button" class="' +
          (sel ? 'on' : '') +
          '" aria-pressed="' +
          (sel ? 'true' : 'false') +
          '"' +
          attr('title', item.title) +
          hook +
          '>' +
          (item.icon ? M.icon(item.icon) : '') +
          (item.label === undefined ? '' : esc(item.label)) +
          '</button>'
        );
      })
      .join('') +
    '</span>'
  );
};

/* ----------------------------------------------- MenuButton / SplitButton
     The app portals every menu: `menus.tsx:39-62` wraps it in a
     `Popover className="menu-portal"`, so the tree is `.menuwrap > button`
     where the trigger stands and `#overlays > .menu-portal > .menu[role=menu]`
     wherever the popover lands (cap/shell/new-menu-open.html). There is no
     in-place form and no `.menu.right` — the app right-aligns by the portal's
     computed `left`.

     So `menuButton` and `splitButton` PORTAL THEMSELVES. A caller renders the
     trigger in `main` (or in a sheet body) exactly as before; when it passes
     `open:true` the helper stamps the trigger `data-menu="<id>"` and queues the
     menu on `M.pendingMenus`. The core empties that queue after the page's own
     overlay (`M.drainMenus()`), then positions each one against its trigger
     (`M.placeMenus()`). Nothing is asked of the page: no `overlay` entry, no
     `after()`, no trigger selector.

       M.ui.menuButton({ label:'Sort', open:s.v.menu==='sort', menuLabel:'Sort',
                         menuHtml: items, align:'end',
                         openAttrs:'data-act="set" data-key="v.menu" data-val="sort"',
                         closeAttrs:'data-act="set" data-key="v.menu" data-val=""' })

     `UI.menuPortal` stays for the one thing the queue cannot do: a menu whose
     trigger is not a menuButton (60-groups' hand-built rows, the card select),
     which still renders its portal from `overlay(s)` and calls
     `M.placeMenuPortal({trigger:'…', width:false})` from `after()`. */
/* `class`, then `aria-label`, then `role` — Menu spreads its props before
     writing role (cap/shell/new-menu-open.html). */
function menuBody(o) {
  return (
    '<div class="menu"' +
    attr('aria-label', o.menuLabel) +
    ' role="menu">' +
    (o.menuHtml || '') +
    '</div>'
  );
}
/* The portalled form of the same menu, for `overlay(s)`. */
UI.menuPortal = function (o) {
  o = o || {};
  return (
    '<div class="menu-portal"' +
    attr('style', o.style) +
    '>' +
    menuBody(o) +
    '</div>'
  );
};
/* Queue a menu for the core to portal, and the attribute that ties it to its
     trigger. `M.pendingMenus` is emptied by the core on every render. */
var menuSeq = 0;
function queueMenu(o) {
  var id = o.menuId || 'menu' + ++menuSeq;
  M.pendingMenus.push({
    id: id,
    align: o.align === 'end' ? 'end' : 'start',
    menuLabel: o.menuLabel,
    menuHtml: o.menuHtml,
  });
  return ' data-menu="' + esc(id) + '"';
}
UI.menuButton = function (o) {
  o = o || {};
  var open = !!o.open;
  /* menus.tsx:113-115 — an icon-only trigger falls back to its title for the
       accessible name. */
  var ariaLabel =
    o.ariaLabel !== undefined
      ? o.ariaLabel
      : o.label == null || o.label === ''
        ? o.title !== undefined
          ? o.title
          : o.menuLabel
        : undefined;
  var trigger = UI.btn(
    (o.label == null ? '' : o.label) +
      (o.trailingIcon === null
        ? ''
        : M.icon(o.trailingIcon || 'chev', null, { cls: 'chevron' })),
    {
      variant: o.variant,
      size: o.size,
      icon: o.icon,
      disabled: o.disabled,
      title: o.title,
      ariaLabel: ariaLabel,
      className: o.btnClass,
      attrs:
        'aria-haspopup="menu" aria-expanded="' +
        (open ? 'true' : 'false') +
        '"' +
        raw(open ? o.closeAttrs : o.openAttrs) +
        (open ? queueMenu(o) : ''),
    },
  );
  return (
    '<span class="' +
    classes('menuwrap', o.className) +
    '">' +
    trigger +
    '</span>'
  );
};
UI.splitButton = function (o) {
  o = o || {};
  var open = !!o.open;
  return (
    '<span class="menuwrap"><span class="split">' +
    UI.btn(o.label, {
      variant: 'primary',
      icon: o.icon,
      attrs: o.mainAttrs
        ? o.mainAttrs
        : 'aria-haspopup="menu" aria-expanded="' +
          (open ? 'true' : 'false') +
          '"' +
          raw(open ? o.closeAttrs : o.openAttrs),
    }) +
    UI.btn(M.icon('chev', null, { cls: 'chevron' }), {
      variant: 'primary',
      ariaLabel: o.menuLabel,
      attrs:
        'aria-haspopup="menu" aria-expanded="' +
        (open ? 'true' : 'false') +
        '"' +
        raw(o.toggleAttrs || (open ? o.closeAttrs : o.openAttrs)) +
        (open
          ? queueMenu({
              align: o.align || 'start',
              menuLabel: o.menuLabel,
              menuHtml: o.menuHtml,
              menuId: o.menuId,
            })
          : ''),
    }) +
    '</span></span>'
  );
};
/* --- the core's two hooks (10-core.js renderOverlays) ------------------- */
M.pendingMenus = [];
M.drainMenus = function () {
  return M.pendingMenus
    .map(function (m) {
      return (
        '<div class="menu-portal" data-menu-for="' +
        esc(m.id) +
        '">' +
        menuBody({ menuLabel: m.menuLabel, menuHtml: m.menuHtml }) +
        '</div>'
      );
    })
    .join('');
};
M.placeMenus = function () {
  M.pendingMenus.forEach(function (m) {
    M.placeMenuPortal({
      portal: '[data-menu-for="' + m.id + '"]',
      trigger: '[data-menu="' + m.id + '"]',
      width: false,
      align: m.align,
    });
  });
  /* Kept, not cleared: renderOverlays also runs on a toast, with no page
       render behind it, and the open menu must survive that. The core empties
       the queue at the top of every M.render. */
};

/* --------------------------------------------------------- SearchField */
UI.searchPlaceholder = function (title) {
  if (title === 'All items') return 'Search all items';
  var full = 'Search ' + title;
  return full.length > 18 ? 'Search' : full;
};
UI.searchField = function (o) {
  o = o || {};
  var ph = o.placeholder || 'Search';
  return (
    '<label class="search">' +
    M.icon('search') +
    '<input' +
    attr('placeholder', ph) +
    attr('aria-label', ph) +
    ' autocomplete="off" type="text"' +
    attr('value', o.value == null ? '' : o.value) +
    (o.bind ? ' data-bind="' + esc(o.bind) + '" data-live' : '') +
    '>' +
    '<kbd aria-hidden="true">⌘K</kbd></label>'
  );
};

/* ---------------------------------------------------------------- Sheet */
var SHEET_WIDTH = { base: '', mid: 'mid', wide: 'wide' };
UI.sheet = function (o) {
  o = o || {};
  var id = o.titleId || nextId('sh');
  var cls = ['sheet', SHEET_WIDTH[o.width || 'base']].filter(Boolean).join(' ');
  var panel =
    '<div class="' +
    cls +
    '" data-act="stop"><div class="hd">' +
    (o.glyph || '') +
    '<span class="t"><h2 id="' +
    esc(id) +
    '">' +
    o.title +
    '</h2>' +
    (o.subtitle === undefined ? '' : '<small>' + o.subtitle + '</small>') +
    '</span></div>' +
    '<div class="sb">' +
    (o.body || '') +
    '</div>' +
    (o.footer === undefined ? '' : '<div class="ft">' + o.footer + '</div>') +
    '</div>';
  return (
    '<div class="backdrop" role="' +
    (o.danger ? 'alertdialog' : 'dialog') +
    '" aria-modal="true" aria-labelledby="' +
    esc(id) +
    '" tabindex="-1"' +
    attr('aria-label', o.ariaLabel) +
    raw(o.backdropAttrs || o.closeAttrs) +
    '>' +
    panel +
    '</div>'
  );
};

/* ----------------------------------------------------------------- Tabs */
UI.tabs = function (o) {
  return (
    '<div class="tabs" role="tablist"' +
    attr('aria-label', o.label) +
    '>' +
    o.items
      .map(function (item) {
        var on = item.id === o.value;
        var hook = item.attrs
          ? raw(item.attrs)
          : item.page
            ? ' data-act="go" data-page="' + esc(item.page) + '"'
            : o.key
              ? ' data-act="set" data-key="' +
                esc(o.key) +
                '" data-val="' +
                esc(item.id) +
                '"'
              : '';
        return (
          '<button type="button" role="tab" class="' +
          (on ? 'tab on' : 'tab') +
          '" aria-selected="' +
          (on ? 'true' : 'false') +
          '"' +
          hook +
          '>' +
          esc(item.label) +
          (item.count === undefined
            ? ''
            : '<span class="' +
              (item.count ? 'n' : 'n zero') +
              '">' +
              esc(item.count) +
              '</span>') +
          '</button>'
        );
      })
      .join('') +
    '</div>'
  );
};

/* --------------------------------------------------------------- Toggle
     The lucide ChevronDown, pinned exactly as components/toggle.tsx draws it
     (stroke-width 2, 13px, its own xmlns) — not an M.icon. The body stays in
     the DOM when closed; only `hidden` changes. */
UI.toggle = function (o) {
  o = o || {};
  var id = o.id || nextId('tg');
  var open = !!o.open;
  return (
    '<div class="' +
    classes('toggle', o.className) +
    '">' +
    '<button type="button" class="toggle-trigger" aria-expanded="' +
    (open ? 'true' : 'false') +
    '" aria-controls="' +
    esc(id) +
    '"' +
    (o.disabled ? ' disabled' : '') +
    raw(o.attrs) +
    '>' +
    '<svg class="toggle-chevron" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="13" height="13" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>' +
    o.label +
    '</button>' +
    '<div id="' +
    esc(id) +
    '" class="toggle-body"' +
    (open ? '' : ' hidden') +
    '>' +
    (o.body || '') +
    '</div>' +
    '</div>'
  );
};

/* ------------------------------------------------- RadioGroup / RadioCard */
UI.radioGroup = function (cards, o) {
  o = o || {};
  return (
    '<div role="radiogroup"' +
    attr('aria-label', o.label) +
    ' class="' +
    classes('radios', o.className) +
    '">' +
    (Array.isArray(cards) ? cards.join('') : cards) +
    '</div>'
  );
};
UI.radioCard = function (o) {
  var cls = [
    'radio',
    o.className || '',
    o.selected ? 'on' : '',
    o.off ? 'off' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    '<button type="button" role="radio" aria-checked="' +
    (o.selected ? 'true' : 'false') +
    '" class="' +
    cls +
    '"' +
    (o.disabled || o.off ? ' disabled' : '') +
    raw(o.attrs) +
    '>' +
    '<span class="rb"></span><span class="t"><b>' +
    o.title +
    '</b>' +
    (o.detail === undefined ? '' : '<small>' + o.detail + '</small>') +
    '</span>' +
    (o.tail || '') +
    '</button>'
  );
};

/* ----------------------------------------------------------- CardSelect
     components/card-select.tsx portals its open list, because the sheet body
     it sits in scrolls and would clip it: the trigger stays in the sheet and
     `#overlays > .menu-portal > .card-select-menu[role=listbox]` is placed at
     runtime from the trigger's rect. So this is TWO helpers, exactly as the
     component is two nodes:

       cardSelect(o)        the `.card-select` + trigger, in the sheet body
       cardSelectMenu(o)    the portal + list, returned from the page's
                            overlay(s) AFTER the sheet
       M.placeMenuPortal()  called from the page's after(), which is the only
                            place the geometry exists

     A page therefore reads:

       overlay: function (s) { return sheetHtml + (open ? M.ui.cardSelectMenu(o) : ''); },
       after:   function ()  { M.placeMenuPortal(); }

     `.card-select*` and `.menu-portal` are app.css's, which 00-head.html
     links, so nothing here needs inline geometry beyond the three offsets
     the app itself writes. */
function cardText(opt) {
  return (
    '<span class="t"><b>' +
    opt.title +
    '</b>' +
    (opt.detail === undefined ? '' : '<small>' + opt.detail + '</small>') +
    '</span>'
  );
}
UI.cardSelect = function (o) {
  o = o || {};
  var open = !!o.open;
  var chosen = null;
  (o.options || []).forEach(function (opt) {
    if (opt.id === o.value) chosen = opt;
  });
  return (
    '<div class="card-select">' +
    '<button type="button" class="card-select-trigger" aria-haspopup="listbox" aria-expanded="' +
    (open ? 'true' : 'false') +
    '"' +
    attr('aria-label', o.label) +
    raw(open ? o.closeAttrs : o.triggerAttrs) +
    '>' +
    (chosen
      ? cardText(chosen)
      : '<span class="t"><b>' +
        (o.placeholder || 'Choose a vault') +
        '</b></span>') +
    M.icon('chev', null, { cls: 'card-select-chevron' }) +
    '</button></div>'
  );
};
UI.cardSelectMenu = function (o) {
  o = o || {};
  /* `class`, then `aria-label`, then `role` — card-select.tsx spreads its
       props before writing role, exactly as Menu does
       (cap/shell/new-store-open.html: `class="card-select-menu"
       aria-label="Save in" role="listbox"`). */
  return (
    '<div class="menu-portal">' +
    '<div class="card-select-menu"' +
    attr('aria-label', o.label) +
    ' role="listbox">' +
    (o.options || [])
      .map(function (opt) {
        var on = opt.id === o.value;
        return (
          '<button type="button" role="option" aria-selected="' +
          (on ? 'true' : 'false') +
          '"' +
          (opt.off ? ' aria-disabled="true" disabled' : '') +
          ' class="' +
          ['card-select-option', on ? 'on' : '', opt.off ? 'off' : '']
            .filter(Boolean)
            .join(' ') +
          '"' +
          (opt.off
            ? ''
            : ' tabindex="-1"' +
              raw(opt.attrs || (o.optionAttrs ? o.optionAttrs(opt.id) : ''))) +
          '>' +
          cardText(opt) +
          (on ? M.icon('check', null, { cls: 'card-select-check' }) : '') +
          '</button>'
        );
      })
      .join('') +
    '</div></div>'
  );
};
/* placeAnchoredMenu (components/menus.tsx) as the mock can run it: `.frame`
     carries a transform, so it is the containing block for the portal's
     `position:fixed`, and the app's own three offsets are what get written.
     Defaults suit a CardSelect (the portal matches the trigger's width);
     pass {width:false} for a `.menu`, {align:'end'} to right-align it. */
M.placeMenuPortal = function (o) {
  o = o || {};
  var portal = document.querySelector(o.portal || '#overlays .menu-portal');
  var trigger = document.querySelector(
    o.trigger || '#overlays .card-select-trigger',
  );
  var frame = document.getElementById('frame');
  if (!portal || !trigger || !frame) return;
  var a = trigger.getBoundingClientRect();
  var f = frame.getBoundingClientRect();
  if (o.width !== false) portal.style.width = a.width + 'px';
  portal.style.left =
    (o.align === 'end'
      ? a.right - f.left - (portal.offsetWidth || a.width)
      : a.left - f.left) + 'px';
  portal.style.top =
    a.bottom - f.top + (o.gap === undefined ? 4 : o.gap) + 'px';
  portal.style.visibility = 'visible';
};

/* -------------------------------------------------------------- CopyBox */
UI.copyBox = function (o) {
  o = o || {};
  var copy = UI.btn(esc(o.label || 'Copy'), {
    attrs:
      o.attrs ||
      'data-act="toast" data-text="' +
        esc((o.label || 'Copy') === 'Copy' ? 'Copied' : o.label) +
        '"',
  });
  var body = o.extra
    ? '<span class="copybox-actions">' + copy + o.extra + '</span>'
    : copy;
  return (
    '<div class="copybox"><span class="v">' +
    (o.display === undefined ? esc(o.text) : o.display) +
    '</span>' +
    body +
    '</div>'
  );
};

/* ----------------------------------------------------------- PageHeader */
UI.pageHeader = function (o) {
  o = o || {};
  var head =
    '<div class="loc' +
    (o.lead ? '' : ' text-only') +
    '">' +
    (o.lead || '') +
    '<div class="loc-copy"><h1>' +
    esc(o.title) +
    '</h1>' +
    (o.subtitle ? '<small>' + esc(o.subtitle) + '</small>' : '') +
    '</div></div>';
  var tail =
    o.tail || o.action
      ? '<div class="header-action">' +
        (o.tail || '') +
        (o.action || '') +
        '</div>'
      : '';
  var search = o.search
    ? UI.searchField({
        value: o.search.value,
        placeholder: o.search.placeholder || UI.searchPlaceholder(o.title),
        bind: o.search.bind,
      })
    : '';
  return '<div class="path">' + head + tail + search + '</div>';
};

/* -------------------------------------------------------------- Toolbar */
UI.toolbar = function (inner) {
  return '<div class="toolbar">' + (inner || '') + '</div>';
};
UI.spacer = function () {
  return '<span class="spacer"></span>';
};
UI.body = function (inner, o) {
  o = o || {};
  return (
    '<div class="' +
    classes('body', o.className) +
    '">' +
    (inner || '') +
    '</div>'
  );
};
UI.main = function (inner, o) {
  o = o || {};
  return (
    '<main class="' +
    classes('main', o.className) +
    '">' +
    (inner || '') +
    '</main>'
  );
};

/* -------------------------------------------------- list header and row */
UI.listHeader = function (o) {
  o = o || {};
  var key = o.key || 'v.sort';
  function head(label, sortKey) {
    var on = o.sort === sortKey;
    return (
      '<button type="button" class="' +
      (on ? 'on' : '') +
      '" data-act="set" data-key="' +
      esc(key) +
      '" data-val="' +
      esc(sortKey) +
      '">' +
      esc(label + (on ? ' ↓' : '')) +
      '</button>'
    );
  }
  return (
    '<div class="hdr"><span></span>' +
    head('Name', 'name') +
    '<span>Server</span><span>Readable by</span>' +
    head('Version', 'version') +
    '</div>'
  );
};
/* One `.row` of items-screen: kind mark, name (+ folder .pchip, store
     <small>, and the full path in a <code> while searching), server, the
     reader-count chip and the version. */
UI.listRow = function (o) {
  var item = o.item;
  var kind = M.data.kindOf(item);
  var prefix = M.data.prefixOf(item.path);
  var readers = M.data.readableBy(item);
  var count = readers.title === undefined ? 1 : M.data.readersOf(item).length;
  var storeName = o.storeName || M.data.whereOf(item);
  return (
    '<div class="' +
    (o.selected ? 'row sel' : 'row') +
    '" role="button" tabindex="0" aria-pressed="' +
    (o.selected ? 'true' : 'false') +
    '"' +
    raw(o.attrs) +
    '>' +
    UI.kindIcon(kind) +
    '<span class="name"><span class="tt"><span>' +
    esc(M.data.nameOf(item.path)) +
    '</span>' +
    (prefix ? '<span class="pchip">' + esc(prefix) + '</span>' : '') +
    '</span>' +
    '<small>' +
    esc(storeName) +
    (o.searching ? ' · <code>' + esc(item.path) + '</code>' : '') +
    '</small></span>' +
    '<span class="n server">' +
    esc(o.server || '') +
    '</span>' +
    '<span>' +
    UI.chip(esc(count), { title: readers.title }) +
    '</span>' +
    '<span class="n">' +
    esc(item.version) +
    '</span></div>'
  );
};

/* ------------------------------------------------------- empty states */
UI.emptyState = function (o) {
  o = o || {};
  var meta = o.kind ? M.data.KINDS[o.kind] : null;
  return (
    '<div class="empty"><div class="big">' +
    M.icon(meta ? meta.icon : 'key') +
    '</div>' +
    '<h2>' +
    esc(
      o.title || 'No ' + (meta ? meta.plural.toLowerCase() : 'items') + ' here',
    ) +
    '</h2>' +
    '<p>' +
    esc(o.body || (meta || M.data.KINDS.Password).blurb) +
    '</p>' +
    (o.action || '') +
    '</div>'
  );
};
UI.emptySearch = function (o) {
  o = o || {};
  return (
    '<div class="empty"><h2>Nothing ' +
    (o.store ? 'in ' + esc(o.store) : 'here') +
    ' matches “' +
    esc(o.query) +
    '”</h2>' +
    '<p>Search covers paths and store names only, and not private contents of items in the vault.</p></div>'
  );
};

/* ======================================================== the title bar
     app-root.tsx:590-625. data-tauri-drag-region="" is emitted even in the
     web build, so it is reproduced. The pill is display-only: `lost` does not
     touch it (nothing sets `.agent.lost`), it draws the .stopwrap instead. */
M.renderTitlebar = function (s) {
  s = s || M.s;
  var busy = s.refreshing === 'yes';
  var starting = s.agent === 'starting';
  return (
    '<div class="titlebar" data-tauri-drag-region="">' +
    '<span class="lights" aria-hidden="true"><span class="light r"></span><span class="light y"></span><span class="light g"></span></span>' +
    '<span class="brand" data-tauri-drag-region="">FOKS</span>' +
    '<span class="spacer" data-tauri-drag-region=""></span>' +
    '<span class="' +
    (starting ? 'agent warn' : 'agent') +
    '" data-tauri-drag-region=""><i data-tauri-drag-region=""></i>' +
    'Agent ' +
    (starting ? 'starting' : 'ready') +
    '</span>' +
    /* app-root.tsx:610-624 passes aria-label BEFORE title, so this one
         button serializes the other way round from every other Button. */
    UI.btn('', {
      variant: 'quiet',
      className: 'global-refresh',
      icon: 'again',
      ariaFirst: true,
      ariaLabel: busy ? 'Refreshing vaults and groups' : 'Refresh',
      title: busy
        ? 'Refreshing vaults and groups'
        : 'Refresh vaults and groups',
      disabled: busy,
      attrs: 'data-act="call" data-fn="refresh"',
    }) +
    '</div>'
  );
};
/* refreshAll (app-root.tsx:422-430): ignored while one is in flight, marks
     `refreshingWorld` (so the button goes disabled and both its labels become
     "Refreshing vaults and groups"), reloads the world, then toasts. */
M.fns.refresh = function () {
  if (M.s.refreshing === 'yes') return;
  M.set('refreshing', 'yes');
  setTimeout(function () {
    M.set('refreshing', 'no');
    M.toast('Vaults and groups refreshed');
  }, 320);
};

/* ========================================================== the sidebar
     shell/sidebar.tsx:184-246. Which row is `on` comes from page.nav:
     a StoreRef, or 'all' | 'alerts' | 'settings' | 'setup'. Every row
     navigates with data-act="go" to the ids below. */
M.storePages = {
  'acct:personal': 'store-personal',
  'acct:work': 'store-work',
  'team:eng': 'store-eng',
  'team:household': 'store-household',
  'team:homelab': 'store-homelab',
};
function navRow(o) {
  var cls = ['nav', o.active ? 'on' : '', o.dimmed ? 'off' : '']
    .filter(Boolean)
    .join(' ');
  return (
    '<button type="button" class="' +
    cls +
    '"' +
    attr('title', o.title) +
    (o.active ? ' aria-current="page"' : '') +
    raw(o.attrs) +
    '>' +
    (o.glyph || '') +
    '<span class="t">' +
    esc(o.name) +
    (o.caption ? '<small>' + esc(o.caption) + '</small>' : '') +
    '</span>' +
    (o.tail || '') +
    '</button>'
  );
}
M.navRow = navRow;
/* opts.alerts overrides the badge count (first run's own sidebar passes
     its own count rather than the world's). */
M.renderSidebar = function (s, page, opts) {
  opts = opts || {};
  s = s || M.s;
  var nav = (page && page.nav) || null;
  var w = M.data.world(s);
  function storeRow(store) {
    return navRow({
      active: nav === store.id,
      glyph: M.icon(store.kind === 'account' ? 'vault' : 'people'),
      name: store.name,
      caption: store.description,
      dimmed: store.state !== 'normal',
      tail: store.kind === 'team' ? UI.stack(store.parties) : undefined,
      attrs:
        'data-act="go" data-page="' +
        esc(M.storePages[store.id] || store.id) +
        '"',
    });
  }
  return (
    '<nav class="side" aria-label="Places">' +
    navRow({
      active: nav === 'all',
      glyph: M.icon('grid'),
      name: 'All items',
      attrs: 'data-act="go" data-page="all"',
    }) +
    UI.sectionLabel('Vaults', { as: 'side' }) +
    (w.vaults.length
      ? w.vaults.map(storeRow).join('')
      : '<p class="fn">No vaults yet</p>') +
    UI.sectionLabel('Groups', { as: 'side' }) +
    (w.groups.length
      ? w.groups.map(storeRow).join('')
      : '<p class="fn">No groups yet</p>') +
    /* The Shares section — heading and rows — is omitted entirely when
         there is no ad-hoc share. */
    (w.shares.length
      ? UI.sectionLabel('Shares', { as: 'side' }) +
        w.shares.map(storeRow).join('')
      : '') +
    (page && page.status
      ? typeof page.status === 'function'
        ? page.status(s)
        : page.status
      : '') +
    '<div class="foot">' +
    navRow({
      active: nav === 'alerts',
      glyph: M.icon('bell'),
      name: 'Alerts',
      tail: UI.badge(
        opts.alerts === undefined ? w.alertCount : opts.alerts,
        'Open alerts',
      ),
      attrs: 'data-act="go" data-page="alerts"',
    }) +
    navRow({
      active: nav === 'settings',
      glyph: M.icon('gear'),
      name: 'Settings',
      attrs: 'data-act="go" data-page="set-devices"',
    }) +
    '<div class="foot-separator"></div>' +
    navRow({
      active: nav === 'setup',
      glyph: M.icon('again'),
      name: 'Set up new vault',
      title: 'Set up a new vault without changing existing vaults',
      attrs: 'data-act="go" data-page="fr-who"',
    }) +
    '</div></nav>'
  );
};

/* ================================================= whole-window states
     All three replace the shell outright: `App` returns one of them INSTEAD of
     `<VaultShell>`, so there is no title bar and no sidebar under any of them
     (app-root.tsx:172-254). They are takeovers, never overlays.

     `.app-lock`, `.app-lock-card` and `.app-loading` are styled by app.css,
     which 00-head.html links, so none of them carries an inline style: the
     card, its heading, its paragraph and its stretched button are all the
     sheet's own rules. `.app-lock{flex:1}` works because `.window` is a flex
     column, exactly as `#root > .window` is in the app. */

/* app-root.tsx:172-223. The mock bridge's mechanism is "password". */
function appLockCard(s) {
  var mechanism =
    M.data.appLock.mechanism === 'biometry'
      ? 'Touch ID or your Mac password'
      : 'your operating-system password';
  return (
    '<div class="app-lock" role="dialog" aria-modal="true" aria-labelledby="app-lock-title">' +
    '<div class="app-lock-card">' +
    '<h1 id="app-lock-title">Unlock FOKS</h1>' +
    '<p>Authenticate with ' +
    mechanism +
    ' to allow FOKS to connect to servers and read vault data.</p>' +
    (s.v.lockError
      ? '<p class="app-lock-error" role="alert">' + esc(s.v.lockError) + '</p>'
      : '') +
    UI.btn('Unlock', {
      variant: 'primary',
      attrs: 'data-act="call" data-fn="unlockApp"',
    }) +
    '</div></div>'
  );
}

/* app-root.tsx:225-251 — note the curly apostrophe in "Couldn’t". */
function bootErrorCard() {
  return (
    '<div class="app-lock" role="alertdialog" aria-labelledby="app-boot-error-title">' +
    '<div class="app-lock-card">' +
    '<h1 id="app-boot-error-title">Couldn’t load FOKS</h1>' +
    '<p class="app-lock-error" role="alert">An injected world must include the bridge that answers its item actions.</p>' +
    UI.btn('Retry', {
      variant: 'primary',
      attrs: 'data-act="set" data-key="boot" data-val="ok"',
    }) +
    '</div></div>'
  );
}

/* write-workflows.tsx:788-822 — a sibling of `.app` inside `.window`,
     absolutely positioned inset:44px 0 0 0, so the title bar above it stays
     visible and live. */
M.stopwrap = function () {
  return (
    '<div aria-label="Local agent connection lost" class="stopwrap" role="alertdialog" aria-modal="true" tabindex="-1">' +
    UI.notice({
      severity: 'crit',
      eyebrow: 'This Mac · the local agent',
      title: 'The local agent connection was lost',
      body: '<p>We encountered a connection error with the local FOKS agent. Your data has been saved, and in-progress operations can be continued once a connection is restored.</p>',
      actions: UI.btn('Retry', {
        variant: 'primary',
        attrs: 'data-act="call" data-fn="agentRetry"',
      }),
    }) +
    '</div>'
  );
};

/**
 * The whole-window states that outrank whatever page is showing.
 *
 * Returns a render() result, or null when the page should draw itself.
 * Every page renderer must start with:
 *     var t = M.globalTakeover(s); if (t) return t;
 *
 *   lock=locked   the Unlock FOKS card instead of the shell. `App` reads
 *                 the lock BEFORE loadWorld and returns it in place of
 *                 <VaultShell>, so it outranks both boot screens and there
 *                 is no title bar under it — it is not an overlay.
 *   boot=error    the boot-error card instead of the shell
 *   boot=loading  the app-loading placeholder instead of the shell
 *
 * `agent=lost` is deliberately NOT here. The app keeps rendering the whole
 * screen and hangs `.stopwrap` off `.window` as a SIBLING of `.app`
 * (write-workflows.tsx:788 draws it as a Dialog; the capture in
 * /tmp/foks-mock/cap/world/agent-lost.html shows the full items list under
 * it), so returning a takeover would have thrown the page away. The core
 * appends `M.windowTail(s)` after `.app` instead, which means every page
 * gets the app's exact tree without touching its renderer.
 *
 */
M.globalTakeover = function (s) {
  s = s || M.s;
  /* app-root.tsx:172 — the lock is read before loadWorld, so it wins. */
  if (s.lock === 'locked') return { takeover: appLockCard(s) };
  if (s.boot === 'error') return { takeover: bootErrorCard() };
  if (s.boot === 'loading') {
    return {
      takeover: '<div class="app-loading">Connecting to the local agent…</div>',
    };
  }
  return null;
};
/* Appended by the core after `.app`, inside `.window`; a page may override
     it with `windowTail` in its own render result. */
M.windowTail = function (s) {
  s = s || M.s;
  return s.agent === 'lost' ? M.stopwrap() : '';
};

/* The core appends this to #overlays after the page's own overlay(s).
     Nothing global lives there any more — the app lock moved into
     globalTakeover, where the app puts it — but the hook stays so a future
     frame-wide overlay has somewhere to go. */
M.renderGlobalOverlay = function () {
  return '';
};

/* Retry: bridge.retryAgentConnection(), then `refresh('Connected to the
     local agent')` — which is refreshWorld() and then the toast, so the
     titlebar's refresh flag flickers exactly as the global Refresh does. The
     interrupted write is never replayed. */
M.fns.agentRetry = function () {
  M.patch({ agent: 'ready', refreshing: 'yes' });
  M.render();
  setTimeout(function () {
    M.set('refreshing', 'no');
    M.toast('Connected to the local agent');
  }, 260);
};
/* Unlocking drops the bridge and reboots the world load. */
M.fns.unlockApp = function () {
  M.set('v.lockError', null, { silent: true });
  M.set('lock', 'off');
};

/* ======================================================== Escape
     app-root.tsx:482-497 plus the dialogs that handle it first: a sheet, a
     menu or a confirmation closes; otherwise the search clears; otherwise the
     selection drops. A page with a different rule may replace M.fns.escape in
     its own after(). */
M.fns.escape = function () {
  var v = M.s.v;
  if (v.sheet) return M.set('v.sheet', null);
  if (v.menu) return M.set('v.menu', null);
  if (v.confirm) return M.set('v.confirm', null);
  if (v.search) return M.set('v.search', null);
  if (v.sel) return M.set('v.sel', null);
};

/* --------------------------------------------------------- placeholder
     So the assembled file boots before 40-items.js exists. M.page replaces by
     id, so the vault builder's own `all` overwrites this one. */
M.page({
  id: 'all',
  title: 'All items',
  path: ['Vault', 'All items'],
  nav: 'all',
  note: 'placeholder from 30-shell.js — 40-items.js re-registers this id.',
  render: function (s) {
    var t = M.globalTakeover(s);
    if (t) return t;
    return { main: '<main class="main"></main>' };
  },
});
