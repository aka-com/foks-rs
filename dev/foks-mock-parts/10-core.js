  /* ------------------------------------------------------------- the core
     M is the mock runtime. Parts register pages, flows, app-wide state groups
     and data on it; the core owns the state object, the control deck, the
     address bar and the render loop.

       M.s                       the state: { page, <app keys…>, v: { <view keys…> } }
       M.app({key,label,values,note})     an app-wide state group (right, top)
       M.page({id,title,path,nav,controls,render,overlay,note,side,titlebar,frameless})
       M.flow({id,title,steps:[{label,page,set,note}]})
       M.set(key, value)         'lease' or 'v.sheet'; re-renders and writes the URL
       M.go(pageId, patch)       navigate, optionally patching state first
       M.toast(text)             a toast in the app's toast stack
       M.hint(text)              the line under the controls
       M.showDim                 deck preference: are other pages' state groups
                                 listed? (localStorage `foksMockDim`)
       M.fns.name = fn           handlers for data-act="call" data-fn="name" data-arg="…"
       M.esc, M.icon, M.h        helpers

     Markup conventions for anything the renderers emit:
       data-act="set" data-key="v.kind" data-val="Password"   set a key
       data-act="go"  data-page="eng-people" [data-set='{"v.tab":"people"}']
       data-act="call" data-fn="removeItem" data-arg="…"
       data-act="toast" data-text="…"
     A value of "" via data-val clears the key (null). */

  var M = window.FOKS_MOCK = {
    s: { page: 'all', v: {} },
    pages: {}, pageOrder: [], flows: [], flowOrder: [], appGroups: [],
    fns: {}, data: {}, icons: {},
    currentFlow: null, step: 0,
    /* The deck's own preference: are the other pages' (dimmed) state groups
       listed? Open by default, remembered per browser rather than in the URL —
       a deep link should describe the app, not the deck. */
    showDim: true,
  };
  try { M.showDim = localStorage.getItem('foksMockDim') !== 'hidden'; } catch (e) { /* file:// */ }

  /* ------------------------------------------------------------ helpers */
  function esc(s) {
    return String(s == null ? '' : s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }
  M.esc = esc;
  /* Join html fragments, dropping falsy ones. */
  M.h = function () {
    var out = '';
    for (var i = 0; i < arguments.length; i++) {
      var a = arguments[i];
      if (Array.isArray(a)) out += M.h.apply(null, a);
      else if (a) out += a;
    }
    return out;
  };
  /* <Icon name size /> as components/icon.tsx emits it: an svg.ic with the
     shell's stroke defaults; the paths come from M.icons (20-data.js). */
  M.icon = function (name, size, extra) {
    var body = M.icons[name];
    if (!body) return '<svg class="ic" viewBox="0 0 24 24" aria-hidden="true" data-missing-icon="' + esc(name) + '"></svg>';
    var attrs = 'class="ic' + (extra && extra.cls ? ' ' + extra.cls : '') + '" viewBox="0 0 24 24" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"';
    if (size) attrs += ' width="' + size + '" height="' + size + '"';
    return '<svg ' + attrs + '>' + body + '</svg>';
  };
  var hintTimer = null;
  M.hint = function (text) {
    var el = document.getElementById('hint');
    el.textContent = text || '';
    el.style.opacity = 1;
    clearTimeout(hintTimer);
    if (text) hintTimer = setTimeout(function () { el.style.opacity = 0; }, 6000);
  };
  M.toasts = [];
  var toastSeq = 0;
  /* ui/kit/toasts.tsx's two constants, to the millisecond: a toast lives
     DEFAULT_DURATION_MS, then drops `.show` (the CSS transition out) and is
     removed EXIT_DURATION_MS later. So a toast is on screen for 2.6s and
     fading for a further 0.3s — not one 4.2s pill. An actionable toast with no
     explicit durationMs is persistent (toasts.tsx:88-92): it waits for the
     action or the dismiss button. Five at most (MAX_VISIBLE_TOASTS), oldest
     dropped, and a repeat of the same message revives the entry in place
     rather than stacking a second one (the dedupeKey path). */
  M.TOAST_MS = 2600;
  M.TOAST_EXIT_MS = 300;
  M.TOAST_MAX = 5;
  M.toast = function (text, opts) {
    opts = opts || {};
    var key = opts.dedupeKey || text;
    var live = M.toasts.filter(function (t) { return (t.dedupeKey || t.text) === key; })[0];
    var entry;
    if (live) {
      clearTimeout(live.timer); clearTimeout(live.exitTimer);
      entry = live;
      entry.text = text; entry.tone = opts.tone === 'warning' ? 'warning' : 'info';
      entry.action = opts.action || null; entry.show = true;
    } else {
      entry = {
        id: ++toastSeq, text: text, dedupeKey: opts.dedupeKey || null,
        tone: opts.tone === 'warning' ? 'warning' : 'info', action: opts.action || null, show: true,
      };
      M.toasts.push(entry);
      /* MAX_VISIBLE_TOASTS: `.slice(-5)` drops the oldest. */
      while (M.toasts.length > M.TOAST_MAX) { var gone = M.toasts.shift(); clearTimeout(gone.timer); clearTimeout(gone.exitTimer); }
    }
    var persistent = entry.action && opts.durationMs === undefined;
    if (!persistent) {
      var ms = Math.max(0, opts.durationMs === undefined ? M.TOAST_MS : opts.durationMs);
      entry.timer = setTimeout(function () { entry.show = false; renderOverlays(); }, ms);
      entry.exitTimer = setTimeout(function () { M.dismissToast(entry.id); }, ms + M.TOAST_EXIT_MS);
    }
    renderOverlays();
    return entry.id;
  };
  M.dismissToast = function (id) {
    M.toasts = M.toasts.filter(function (t) {
      if (String(t.id) === String(id)) { clearTimeout(t.timer); clearTimeout(t.exitTimer); return false; }
      return true;
    });
    renderOverlays();
  };
  M.fns.toastDismiss = function (id) { M.dismissToast(id); };
  M.fns.toastAction = function (id) {
    var t = M.toasts.filter(function (x) { return String(x.id) === String(id); })[0];
    try { if (t && t.action && t.action.onAction) t.action.onAction(); } finally { M.dismissToast(id); }
  };

  /* ---------------------------------------------------------- registry */
  M.app = function (group) {
    if (!group.key) throw new Error('app group needs a key');
    M.appGroups = M.appGroups.filter(function (g) { return g.key !== group.key; });
    M.appGroups.push(group);
    if (M.s[group.key] === undefined) M.s[group.key] = group.values[0].v;
    return group;
  };
  M.page = function (page) {
    if (!page.id) throw new Error('page needs an id');
    if (!M.pages[page.id]) M.pageOrder.push(page.id);
    page.path = page.path || [page.title];
    page.controls = page.controls || [];
    M.pages[page.id] = page;
    return page;
  };
  M.flow = function (flow) {
    if (!M.flows.some(function (f) { return f.id === flow.id; })) M.flowOrder.push(flow.id);
    M.flows = M.flows.filter(function (f) { return f.id !== flow.id; });
    M.flows.push(flow);
    return flow;
  };
  M.current = function () { return M.pages[M.s.page] || M.pages[M.pageOrder[0]]; };
  /* Every view control group known to any page, keyed, first declaration wins
     for label/values; a page's own declaration is used while it is current. */
  function viewGroups() {
    var seen = {}, out = [];
    var cur = M.current();
    /* The current page's groups apply; every other page's groups are shown
       dimmed, deduped by key AND label so a key two parts share with different
       meanings is still listed once per meaning. */
    var live = {};
    (cur ? cur.controls : []).forEach(function (g) { live[g.key] = true; out.push({ g: g, applies: true }); });
    M.pageOrder.forEach(function (id) {
      M.pages[id].controls.forEach(function (g) {
        var sig = g.key + '|' + g.label;
        if (live[g.key] || seen[sig]) return;
        seen[sig] = true;
        out.push({ g: g, applies: false });
      });
    });
    return out;
  }
  M.applies = function (key) {
    var cur = M.current();
    return !!cur && cur.controls.some(function (g) { return g.key === key; });
  };

  /* --------------------------------------------------------- state ops */
  function getKey(key) {
    if (key.indexOf('v.') === 0) return M.s.v[key.slice(2)];
    return M.s[key];
  }
  function putKey(key, val) {
    if (val === '' || val === undefined) val = null;
    if (key.indexOf('v.') === 0) M.s.v[key.slice(2)] = val;
    else M.s[key] = val;
  }
  M.get = getKey;
  M.set = function (key, val, opts) {
    putKey(key, val);
    if (!(opts && opts.silent)) M.render();
  };
  M.patch = function (obj) {
    Object.keys(obj || {}).forEach(function (k) { putKey(k, obj[k]); });
  };
  /* Leaving a page drops what the app's location transition drops: the
     selection, a reveal, and any open sheet/menu/confirm (location.ts
     `transition`). A page may add its own `leave(s, nextId)`. */
  M.leaveKeys = ['sel', 'reveal', 'sheet', 'menu', 'confirm'];
  M.go = function (pageId, patch) {
    if (pageId !== M.s.page && M.pages[pageId]) {
      var leaving = M.current();
      if (leaving && leaving.leave) leaving.leave(M.s, pageId);
      M.leaveKeys.forEach(function (k) { M.s.v[k] = null; });
      /* …and every view key the destination page does not declare: the
         app remounts the screen, so drafts and sheet state do not travel. */
      var keeps = {};
      M.pages[pageId].controls.forEach(function (g) { keeps[g.key] = true; });
      (M.pages[pageId].keeps || []).forEach(function (k) { keeps[k] = true; });
      Object.keys(M.s.v).forEach(function (k) { if (!keeps[k]) M.s.v[k] = null; });
    }
    if (patch) M.patch(patch);
    if (M.pages[pageId]) M.s.page = pageId;
    else M.hint('No page "' + pageId + '" is registered yet.');
    M.render();
  };
  /* Reset every view key the current page declares to its first value; used
     by flows so a step starts from the page's own default. */
  M.resetView = function () { M.s.v = {}; };

  /* --------------------------------------------------------- the URL */
  function encodeState() {
    var p = new URLSearchParams();
    p.set('page', M.s.page);
    M.appGroups.forEach(function (g) {
      if (M.s[g.key] != null && M.s[g.key] !== g.values[0].v) p.set(g.key, M.s[g.key]);
    });
    Object.keys(M.s.v).forEach(function (k) {
      if (M.s.v[k] != null) p.set(k, M.s.v[k]);
    });
    if (M.currentFlow) { p.set('flow', M.currentFlow.id); p.set('step', String(M.step + 1)); }
    return '?' + p.toString();
  }
  function decodeState() {
    var p = new URLSearchParams(location.search);
    var appKeys = {};
    M.appGroups.forEach(function (g) { appKeys[g.key] = true; });
    var flowId = p.get('flow');
    p.forEach(function (val, key) {
      if (key === 'page') { if (M.pages[val]) M.s.page = val; }
      else if (key === 'flow' || key === 'step') { /* below */ }
      else if (appKeys[key]) M.s[key] = val;
      else M.s.v[key] = val;
    });
    if (flowId) {
      var f = M.flows.filter(function (x) { return x.id === flowId; })[0];
      if (f) {
        M.currentFlow = f;
        M.step = Math.max(0, Math.min(f.steps.length - 1, (parseInt(p.get('step') || '1', 10) || 1) - 1));
        if (!p.get('page')) applyStep(false);
      }
    }
  }
  function writeUrl() {
    try { history.replaceState(null, '', encodeState()); } catch (e) { /* file:// */ }
  }

  /* ------------------------------------------------------------- flows */
  function applyStep(render) {
    var st = M.currentFlow.steps[M.step];
    if (!st) return;
    if (st.reset !== false) M.resetView();
    M.patch(st.set || {});
    if (st.page && M.pages[st.page]) M.s.page = st.page;
    /* The step's note is drawn beside ‹ Previous / Next ›, not repeated in the hint line. */
    if (render !== false) M.render();
  }
  M.startFlow = function (id, step) {
    var f = M.flows.filter(function (x) { return x.id === id; })[0];
    if (!f) return;
    M.currentFlow = f; M.step = step || 0;
    applyStep();
  };
  M.stopFlow = function () { M.currentFlow = null; M.step = 0; M.render(); };
  /* ↑ / ↓: the previous / next walkthrough in the deck's order, from its
     first step. With none running, ↓ starts the first and ↑ the last. */
  M.flowMove = function (delta) {
    if (!M.flowOrder.length) return;
    var at = M.currentFlow ? M.flowOrder.indexOf(M.currentFlow.id) : (delta > 0 ? -1 : M.flowOrder.length);
    var next = at + delta;
    if (next < 0 || next >= M.flowOrder.length) return;
    M.startFlow(M.flowOrder[next], 0);
  };
  M.flowStep = function (delta) {
    if (!M.currentFlow) return;
    var n = M.step + delta;
    if (n < 0 || n >= M.currentFlow.steps.length) return;
    M.step = n;
    applyStep();
  };

  /* ---------------------------------------------------------- controls */
  function chip(label, on, attrs, extraCls) {
    return '<button class="mock-chip' + (on ? ' on' : '') + (extraCls ? ' ' + extraCls : '') + '" ' +
      'aria-pressed="' + (on ? 'true' : 'false') + '" ' + attrs + '>' + esc(label) + '</button>';
  }
  function groupHTML(g, applies, current, prefix) {
    prefix = prefix || '';
    var cls = 'mc-group' + (applies ? '' : ' dim');
    var html = '<div class="' + cls + '" data-group="' + esc(g.key) + '"><span class="mc-label" title="' + esc(g.note || '') + '">' + esc(g.label) + '</span>';
    var lit = false;
    g.values.forEach(function (o) {
      var on = (current == null ? (o.v == null || o.v === '') : String(current) === String(o.v));
      if (on) lit = true;
      html += chip(o.label, on, 'data-act="set" data-key="' + esc(prefix + g.key) + '" data-val="' + esc(o.v == null ? '' : o.v) + '"' + (o.hint ? ' title="' + esc(o.hint) + '"' : '') + (applies ? '' : ' disabled'));
    });
    /* The value that is actually set matches no chip — a free-text draft, a
       flow step's token list, or a typo in a deep link. Show it rather than
       leaving the whole group dark, which reads as "unset". */
    if (!lit && current != null && current !== '') {
      var text = String(current);
      html += chip(text.length > 34 ? text.slice(0, 32) + '…' : text, true,
        'data-act="set" data-key="' + esc(prefix + g.key) + '" data-val="' + esc(current) + '" title="' +
        esc(text + ' — set to a value this group does not declare.') + '"' + (applies ? '' : ' disabled'), 'custom');
    }
    return html + '</div>';
  }
  /* A capped scroller with more below the fold gets `.over`, which fades its
     last row (00-head.html) — otherwise a half-shown chip row reads as a
     clipping bug. Recomputed after every render and on scroll. */
  function markScroll(el) {
    if (el) el.classList.toggle('over', el.scrollHeight - el.scrollTop - el.clientHeight > 4);
  }
  document.addEventListener('scroll', function (ev) {
    var el = ev.target;
    if (el && el.classList && el.classList.contains('mc-scroll')) markScroll(el);
  }, true);
  function renderControls() {
    var cur = M.current();
    /* app-wide */
    document.getElementById('mcAppGroups').innerHTML = M.appGroups.map(function (g) {
      var applies = !g.appliesTo || g.appliesTo(cur, M.s);
      return groupHTML({ key: g.key, label: g.label, values: g.values, note: g.note }, applies, M.s[g.key]);
    }).join('');
    /* view-specific */
    var groups = viewGroups();
    var applying = groups.filter(function (x) { return x.applies; });
    var rest = groups.filter(function (x) { return !x.applies; });
    var html = applying.map(function (x) { return groupHTML(x.g, true, M.s.v[x.g.key], 'v.'); }).join('');
    if (!applying.length) html += '<div class="mc-note">This page has no view-specific state.</div>';
    /* Keys the state carries that this page does not declare: a deep link or a
       flow step set them, the page may well read them (first run's `path`), but
       no chip above can show them — so name them instead of hiding them. */
    var mine = {};
    (cur ? cur.controls : []).forEach(function (g) { mine[g.key] = true; });
    var strays = Object.keys(M.s.v).filter(function (k) { return M.s.v[k] != null && M.s.v[k] !== '' && !mine[k]; });
    if (strays.length) {
      html += '<div class="mc-note">Also set, but not declared by this page: ' +
        strays.map(function (k) { return '<code>' + esc(k) + '=' + esc(M.s.v[k]) + '</code>'; }).join(', ') +
        ' — the page may still read it.</div>';
    }
    /* Every other page's groups, dimmed, behind a toggle: on a page with a
       dozen knobs of its own the two dozen it does not read are noise. Open by
       default; the choice is remembered per browser, not in the URL, so a deep
       link still means one thing. */
    var dimBtn = document.getElementById('mcDimToggle');
    if (rest.length && M.showDim) {
      html += '<hr class="mc-divider" />' +
        '<div class="mc-note">Groups other pages read — chips disabled while they do not apply.</div>' +
        rest.map(function (x) { return groupHTML(x.g, false, M.s.v[x.g.key], 'v.'); }).join('');
    }
    dimBtn.hidden = !rest.length;
    dimBtn.textContent = (M.showDim ? 'Hide ' : 'Show ') + rest.length + ' groups other pages read';
    dimBtn.setAttribute('aria-expanded', String(!!M.showDim));
    dimBtn.setAttribute('data-act', 'dim-toggle');
    document.getElementById('mcViewGroups').innerHTML = html;
    /* Flows: 1 2 3 ... for each walkthrough in declaration order. */
    document.getElementById('mcFlowList').innerHTML = M.flowOrder.map(function (id, i) {
      var f = M.flows.filter(function (x) { return x.id === id; })[0];
      var num = String(i + 1);
      var tip = num + ' · ' + (f.group ? '[' + f.group + '] ' : '') + f.title + (f.note ? ' — ' + f.note : '');
      return chip(num, M.currentFlow && M.currentFlow.id === f.id,
        'data-act="flow" data-flow="' + esc(f.id) + '" title="' + esc(tip) + '" aria-label="' + esc(tip) + '"');
    }).join('');
    var stepsEl = document.getElementById('mcSteps');
    var nav = document.getElementById('mcStepNav');
    /* The rule, the Selected row, the nav row and the note row show only
       while a walkthrough is running. */
    ['mcFlowRule', 'mcStepsRow', 'mcStepNoteRow'].forEach(function (id) {
      var el = document.getElementById(id); if (el) el.hidden = !M.currentFlow;
    });
    if (M.currentFlow) {
      /* Numbered circles, nothing else. The label and the page path are the
         circle's `title`/`aria-label`, and the chosen step's own sentence is
         printed once, beside ‹ Previous / Next ›. */
      stepsEl.innerHTML = M.currentFlow.steps.map(function (st, i) {
        var pg = M.pages[st.page];
        var where = (i + 1) + ' · ' + st.label + ' — ' + (pg ? pg.path.join(' › ') : st.page);
        return '<button class="mc-step' + (i === M.step ? ' on' : '') + '" data-act="flow-step" data-step="' + i + '"' +
          ' title="' + esc(where) + '" aria-label="' + esc(where) + '"' +
          (i === M.step ? ' aria-current="step"' : '') + '>' + (i + 1) + '</button>';
      }).join('');
      nav.hidden = false;
      /* The ‹ › icons go dim at either end, as the keys stop there too. */
      nav.querySelector('[data-act="flow-prev"]').disabled = M.step <= 0;
      nav.querySelector('[data-act="flow-next"]').disabled = M.step >= M.currentFlow.steps.length - 1;
      document.getElementById('mcStepNote').textContent = (M.currentFlow.steps[M.step] && M.currentFlow.steps[M.step].note) || '';
      var flowIdx = M.flowOrder.indexOf(M.currentFlow.id) + 1;
      document.getElementById('mcFlowSub').innerHTML = (flowIdx ? '#' + flowIdx + ' · ' : '') + esc(M.currentFlow.title) + ' · step ' + (M.step + 1) + ' of ' + M.currentFlow.steps.length + ' <button class="mock-chip sub" data-act="flow-stop">leave</button>';
    } else {
      stepsEl.innerHTML = '';
      nav.hidden = true;
      document.getElementById('mcFlowSub').textContent = 'pick one to step through it';
    }
    /* page picker */
    var btn = document.getElementById('mcPickerBtn');
    var path = cur ? cur.path : ['—'];
    btn.innerHTML = path.slice(0, -1).map(function (c) { return '<span class="crumb">' + esc(c) + '</span>'; }).join('') + '<span class="cur">' + esc(path[path.length - 1]) + '</span><span class="caret">▾</span>';
    document.getElementById('mcPageSub').textContent = cur ? cur.id : '';
    document.getElementById('mcPageNote').textContent = (cur && cur.note) || '';
    renderTree();
    /* Siblings as quick chips — the pages one level of the tree holds, so
       "next door" is one click without opening the picker. Each chip carries
       its page id after the leaf label: it is the id a deep link needs, and it
       keeps every deck label distinct from the frame's own button text
       (capture.mjs `clickText` takes the first match in DOM order, and the
       deck sits above the frame — a chip reading exactly "All items" would
       swallow every click meant for the sidebar). */
    var quick = document.getElementById('mcPagesQuick');
    var sibs = cur ? M.pageOrder.filter(function (id) {
      var p = M.pages[id];
      return p.path.length === cur.path.length && p.path.slice(0, -1).join('/') === cur.path.slice(0, -1).join('/');
    }) : [];
    quick.innerHTML = sibs.length > 1 ? sibs.map(function (id) {
      var p = M.pages[id];
      return '<button class="mock-chip' + (id === cur.id ? ' on' : '') + '" data-act="go" data-page="' + esc(id) + '">' +
        esc(p.path[p.path.length - 1]) + '<span class="qid">' + esc(id) + '</span></button>';
    }).join('') : '';
    ['mcAppGroups', 'mcViewGroups'].forEach(function (id) { markScroll(document.getElementById(id)); });
  }
  /* The tree: pages grouped by their path prefixes, any depth. Every branch is
     open — the picker is the map of the deck, and a collapsed branch hides
     pages a reader has no other way of knowing about. The current page's leaf
     is the highlighted one, and the popover scrolls to it when it opens. */
  function renderTree() {
    var root = { kids: {}, order: [], pages: [] };
    M.pageOrder.forEach(function (id) {
      var p = M.pages[id], node = root;
      p.path.slice(0, -1).forEach(function (seg) {
        if (!node.kids[seg]) { node.kids[seg] = { kids: {}, order: [], pages: [] }; node.order.push(seg); }
        node = node.kids[seg];
      });
      node.pages.push(p);
    });
    var cur = M.current();
    function walk(node) {
      var html = '';
      node.pages.forEach(function (p) {
        html += '<button class="leaf' + (cur && p.id === cur.id ? ' on' : '') + '" data-act="go" data-page="' + esc(p.id) + '"><span>' + esc(p.path[p.path.length - 1]) + '</span><span class="id">' + esc(p.id) + '</span></button>';
      });
      node.order.forEach(function (seg) {
        html += '<details open><summary>' + esc(seg) + '</summary><div class="lvl">' + walk(node.kids[seg]) + '</div></details>';
      });
      return html;
    }
    document.getElementById('mcTree').innerHTML = walk(root);
  }

  /* ------------------------------------------------------------ render */
  /* Where #toasts sits inside #overlays is React mount order, not z-order, and
     the app's captures show both answers:

       cap/shell/new.html, exists.html   .backdrop  then  #toasts
       cap/shell/remove-confirm.html     #toasts    then  .backdrop
       cap/shell/new-store-open.html     .backdrop  then  #toasts  then  .menu-portal

     ToastProvider renders {children} before its own portalled host
     (toasts.tsx:202-206), and a portal's DOM node is appended at commit time —
     so anything the screen already had open on the FIRST paint (a deep-linked
     sheet) lands in #overlays before the toast host, and anything mounted
     afterwards (a sheet or menu opened by clicking) is appended after it.

     `M.overlayFirst` is that first-paint bit: true while the overlay the page
     returned is the same one it returned on the very first render, false for
     ever after it has closed once. A page may force either answer with
     `overlayOrder: 'before' | 'after'` (or `out.overlayFirst`) — first run's
     `existing` cards, say, which mount with the screen.

     Menu portals are always last: a `.menu` mounts when its trigger is
     clicked, which is by definition after the first paint, so the split below
     keeps a `.menu-portal` the page returned from `overlay(s)` (the CardSelect
     list) behind #toasts even when the sheet in front of it is a deep link. */
  var firstPaintDone = false;
  M.overlayFirst = false;
  function splitPortals(html) {
    var at = html.indexOf('<div class="menu-portal"');
    return at < 0 ? [html, ''] : [html.slice(0, at), html.slice(at)];
  }
  function renderOverlays() {
    var cur = M.current();
    var toasts = '<div id="toasts" class="toasts" role="status" aria-live="polite" aria-atomic="false">' + M.toasts.map(function (t) {
      var warn = t.tone === 'warning';
      return '<div class="toast toast-owned' + (t.show === false ? '' : ' show') + '" data-toast-id="' + t.id + '" data-toast-tone="' + t.tone + '"' + (warn ? ' role="alert" aria-live="assertive"' : '') + '>' +
        '<span class="toast-message">' + esc(t.text) + '</span>' +
        (t.action ? '<button type="button" class="toast-action" data-act="call" data-fn="toastAction" data-arg="' + t.id + '">' + esc(t.action.label) + '</button>' : '') +
        '<button type="button" class="toast-dismiss" aria-label="Dismiss notification" data-toast-dismiss="true" data-act="call" data-fn="toastDismiss" data-arg="' + t.id + '">×</button></div>';
    }).join('') + '</div>';
    /* A takeover replaces the shell (the app lock, the two boot screens, a
       page's own full-window stop), and the app returns it INSTEAD of
       <VaultShell> — so the screen underneath does not exist and neither do
       its sheets. Suppress the page overlay there; toasts still stack.
       (agent=lost is the opposite case and keeps its sheet: it is not a
       takeover but a `.stopwrap` hung off .window by M.windowTail.) */
    var pageOverlay = (cur && cur.overlay && !M.inTakeover) ? (cur.overlay(M.s) || '') : '';
    /* The first-paint bit, recomputed on every pass: an overlay that was up on
       the first render keeps its place ahead of #toasts until it closes, and
       an overlay that opens later never gets it. */
    if (!firstPaintDone) { M.overlayFirst = !!pageOverlay; firstPaintDone = true; }
    else if (!pageOverlay) M.overlayFirst = false;
    var forced = M.overlayOrderOverride !== undefined ? M.overlayOrderOverride
      : (cur && cur.overlayOrder !== undefined ? cur.overlayOrder : undefined);
    var first = forced === 'before' ? true : forced === 'after' ? false : M.overlayFirst;
    var parts = splitPortals(pageOverlay);
    var html = (first ? parts[0] : '') + toasts + (first ? '' : parts[0]) + parts[1];
    /* Menus queued by M.ui.menuButton / splitButton during the page render:
       the app portals every menu into the overlay layer, so the helper puts it
       here rather than inside `.menuwrap` (30-shell.js). */
    html += (M.drainMenus ? M.drainMenus() : '');
    if (M.renderGlobalOverlay) html += M.renderGlobalOverlay(M.s) || '';
    document.getElementById('overlays').innerHTML = html;
    if (M.placeMenus) M.placeMenus();
    /* The kit's Dialog marks the app inert while a modal is up — including a
       Dialog drawn outside #overlays, which is how the agent-loss `.stopwrap`
       reaches `.app` (see the app's own capture: `<div class="app" inert=""
       aria-hidden="true">` under it). */
    var app = document.querySelector('#win > .app');
    if (app) {
      /* The app's click-opened sheets always isolate the shell; only a
         sheet present at first paint misses it (the kit's Dialog reads a
         background ref that is still null on the mounting commit — an app
         first-paint quirk, not a rule). The mock keeps the steady state:
         inert whenever a .backdrop is up. `modalInert: false` opts a page out. */
      var modal = M.appInert || !!document.querySelector('#win > [aria-modal="true"]') ||
        (cur && cur.modalInert !== false && /class="backdrop/.test(pageOverlay));
      if (modal) { app.setAttribute('inert', ''); app.setAttribute('aria-hidden', 'true'); }
      else { app.removeAttribute('inert'); app.removeAttribute('aria-hidden'); }
    }
  }
  M.renderOverlays = renderOverlays;
  M.render = function () {
    var cur = M.current();
    if (!cur) { document.getElementById('win').innerHTML = '<div style="padding:20px">No pages registered.</div>'; return; }
    var win = document.getElementById('win');
    M.pendingMenus = [];
    var out = cur.render(M.s) || {};
    if (typeof out === 'string') out = { main: out };
    M.inTakeover = !!out.takeover;
    /* `out.overlayFirst` (true/false) forces this render's #overlays order; see
       renderOverlays. Undefined leaves the first-paint rule in charge. */
    M.overlayOrderOverride = out.overlayFirst === undefined ? undefined
      : (out.overlayFirst ? 'before' : 'after');
    var html = '';
    if (out.takeover) {
      /* The whole window minus nothing: full-window stops, the lock. */
      html = out.takeover;
    } else {
      html += (out.titlebar !== undefined ? out.titlebar : (M.renderTitlebar ? M.renderTitlebar(M.s, cur) : ''));
      html += '<div class="app' + (out.appClass ? ' ' + out.appClass : '') + '">';
      html += (out.side !== undefined ? out.side : (M.renderSidebar ? M.renderSidebar(M.s, cur) : ''));
      html += out.main || '';
      html += out.details || '';
      html += '</div>';
      /* A sibling of `.app` inside `.window` — where the app hangs the
         agent-loss `.stopwrap`, which covers the page without replacing it.
         The page may name its own; otherwise the shell's M.windowTail(s)
         decides, so no page renderer has to know about it. */
      html += (out.windowTail !== undefined ? out.windowTail
        : (M.windowTail ? M.windowTail(M.s, cur) : '')) || '';
    }
    /* Keep focus where the app keeps it: on the control just used (its
       `.on` ring after a toolbar click), or on a newly opened sheet's first
       control. The active element is re-found by a stable signature. */
    var active = document.activeElement, sig = null;
    if (active && win.contains(active)) {
      sig = active.getAttribute('data-key') != null
        ? '[data-key="' + active.getAttribute('data-key') + '"][data-val="' + (active.getAttribute('data-val') || '') + '"]'
        : active.getAttribute('aria-label') ? '[aria-label="' + active.getAttribute('aria-label').replace(/"/g, '\\"') + '"]'
        : active.id ? '#' + active.id : null;
    }
    var hadBackdrop = !!document.querySelector('#overlays .backdrop, #win .backdrop');
    win.innerHTML = html;
    M._refocus = function () {
      var backdrop = document.querySelector('#overlays .backdrop, #win .backdrop');
      if (backdrop && !hadBackdrop) {
        var first = backdrop.querySelector('input:not([disabled]), textarea, select, button:not([disabled]), [tabindex="0"]');
        if (first) { try { first.focus({ preventScroll: true }); } catch (e) {} }
        return;
      }
      if (!sig) return;
      var again = win.querySelector(sig);
      if (again && again !== document.activeElement) { try { again.focus({ preventScroll: true }); } catch (e) {} }
    };
    win.className = 'window web-mock-window' + (out.windowClass ? ' ' + out.windowClass : '');
    renderOverlays();
    renderControls();
    /* The amber band under the deck is the mock talking about itself, never the
       app: a page's own `band`, plus a line for every app-wide value that says
       something the screen cannot (a state the running app cannot be driven
       into, say). App values carry it as `band` on the value (20-data.js). */
    var band = document.getElementById('mockBand');
    var lines = [out.band || (cur.band && cur.band(M.s)) || ''];
    M.appGroups.forEach(function (g) {
      var val = g.values.filter(function (o) { return String(o.v) === String(M.s[g.key]); })[0];
      if (val && val.band) lines.push(val.band);
    });
    var bandText = lines.filter(Boolean).join('<br />');
    band.hidden = !bandText; band.innerHTML = bandText || '';
    writeUrl();
    if (cur.after) cur.after(M.s);
    if (M._refocus) M._refocus();
  };

  /* ------------------------------------------------------------ events */
  /* Backdrops dismiss on mousedown, as the kit's DismissibleDialog does
     (overlay-primitives.tsx:230-239: `event.target === event.currentTarget`),
     so a drag that starts inside the sheet and ends over the backdrop does
     not close it. The click that follows lands on whatever the re-render put
     under the pointer, which is never the (now gone) backdrop. */
  document.addEventListener('mousedown', function (ev) {
    var t = ev.target;
    if (!t || !t.classList || !t.classList.contains('backdrop') || !t.getAttribute('data-act')) return;
    if (ev.button !== 0) return;
    ev.preventDefault();
    t.__dismissed = true;
    runAct(t, ev);
  });
  function runAct(t, ev) {
    var act = t.getAttribute('data-act');
    if (act === 'set') M.set(t.getAttribute('data-key'), t.getAttribute('data-val'));
    else if (act === 'call') { var fn = M.fns[t.getAttribute('data-fn')]; if (fn) fn(t.getAttribute('data-arg'), t, ev); }
    else if (act === 'go') { var patch = t.getAttribute('data-set'); M.go(t.getAttribute('data-page'), patch ? JSON.parse(patch) : null); }
  }
  document.addEventListener('click', function (ev) {
    if (ev.target && ev.target.classList && ev.target.classList.contains('backdrop') && ev.target.__dismissed) return;
    var t = ev.target.closest('[data-act]');
    var picker = document.getElementById('mcPicker');
    if (!ev.target.closest('#mcPicker')) {
      document.getElementById('mcTree').hidden = true;
      document.getElementById('mcPickerBtn').setAttribute('aria-expanded', 'false');
    }
    if (ev.target.closest('#mcPickerBtn')) {
      var tree = document.getElementById('mcTree');
      tree.hidden = !tree.hidden;
      document.getElementById('mcPickerBtn').setAttribute('aria-expanded', String(!tree.hidden));
      /* Open on the current page: the tree is 40-odd leaves deep. */
      if (!tree.hidden) {
        var here = tree.querySelector('.leaf.on');
        if (here) tree.scrollTop = Math.max(0, here.offsetTop - tree.offsetTop - tree.clientHeight / 2);
      }
      return;
    }
    if (!t || t.disabled) return;
    var act = t.getAttribute('data-act');
    if (act === 'set') {
      M.set(t.getAttribute('data-key'), t.getAttribute('data-val'));
    } else if (act === 'go') {
      var patch = t.getAttribute('data-set');
      M.go(t.getAttribute('data-page'), patch ? JSON.parse(patch) : null);
      document.getElementById('mcTree').hidden = true;
    } else if (act === 'call') {
      var fn = M.fns[t.getAttribute('data-fn')];
      if (fn) fn(t.getAttribute('data-arg'), t, ev); else M.hint('No handler "' + t.getAttribute('data-fn') + '".');
    } else if (act === 'stop') {
      /* a panel shielding its backdrop's close hook */
    } else if (act === 'toast') {
      M.toast(t.getAttribute('data-text'));
    } else if (act === 'hint') {
      M.hint(t.getAttribute('data-text'));
    } else if (act === 'flow') {
      var id = t.getAttribute('data-flow');
      if (M.currentFlow && M.currentFlow.id === id) M.stopFlow(); else M.startFlow(id, 0);
    } else if (act === 'dim-toggle') {
      M.showDim = !M.showDim;
      try { localStorage.setItem('foksMockDim', M.showDim ? 'shown' : 'hidden'); } catch (e) { /* file:// */ }
      renderControls();
    } else if (act === 'flow-step') {
      M.step = parseInt(t.getAttribute('data-step'), 10); applyStep();
    } else if (act === 'flow-prev') M.flowStep(-1);
    else if (act === 'flow-next') M.flowStep(1);
    else if (act === 'flow-stop') M.stopFlow();
    if (t.closest('#mcTree')) { /* keep the tree open only for summaries */ }
  });
  /* Keyboard: ← → step the running flow, ↑ ↓ move between flows, Escape
     closes what a page marks closable. */
  document.addEventListener('keydown', function (ev) {
    if (ev.target && /INPUT|TEXTAREA|SELECT/.test(ev.target.tagName)) return;
    if (M.currentFlow && ev.key === 'ArrowRight') { M.flowStep(1); ev.preventDefault(); }
    else if (M.currentFlow && ev.key === 'ArrowLeft') { M.flowStep(-1); ev.preventDefault(); }
    else if (ev.key === 'ArrowDown') { M.flowMove(1); ev.preventDefault(); }
    else if (ev.key === 'ArrowUp') { M.flowMove(-1); ev.preventDefault(); }
    else if (ev.key === 'Escape' && M.fns.escape) M.fns.escape();
  });
  /* Text inputs inside the frame: data-bind="v.search" keeps the state. */
  document.addEventListener('input', function (ev) {
    var key = ev.target.getAttribute && ev.target.getAttribute('data-bind');
    if (!key) return;
    /* A bound field keeps '' as '' (a cleared field is not an unset one);
       only chips fold '' to null. */
    if (key.indexOf('v.') === 0) M.s.v[key.slice(2)] = ev.target.value; else M.s[key] = ev.target.value;
    if (ev.target.getAttribute('data-live') != null) {
      var active = ev.target, pos = active.selectionStart;
      M.render();
      var again = document.querySelector('[data-bind="' + key + '"]');
      if (again) { again.focus(); try { again.setSelectionRange(pos, pos); } catch (e) {} }
    } else writeUrl();
  });
  document.addEventListener('change', function (ev) {
    var key = ev.target.getAttribute && ev.target.getAttribute('data-bind');
    if (key && ev.target.tagName === 'SELECT') M.set(key, ev.target.value);
    if (key && ev.target.type === 'checkbox') M.set(key, ev.target.checked ? '1' : '');
  });

  /* --------------------------------------------------------------- boot */
  M.boot = function () {
    decodeState();
    if (!M.pages[M.s.page]) M.s.page = M.pageOrder[0];
    M.render();
  };
  window.addEventListener('DOMContentLoaded', function () { if (!M.booted) { M.booted = true; M.boot(); } });
