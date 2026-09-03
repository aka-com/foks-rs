  /* ---------------------------------------------------------- 72-alerts.js
     The Alerts pane — screens/alerts-screen.tsx, all 85 lines of it.

     `notesNow(world)` decides the list, and 20-data.js already applies it:
     `M.data.world(s).alerts` is 2 entries in the fresh world and 3 under
     `acme=lapsed`, where the critical `foks.acme-corp.com is locked` card
     joins them and the sidebar badge goes 2 → 3. Nothing here re-derives it.

     A card's button is enabled only when `note.action === 'Retry'` and
     `note.id` starts with `catalog-` (alerts-screen.tsx:27); every fixture
     note fails that test, so all three buttons are disabled and carry
     `title="Open the screen that owns this action: Servers or Groups."`.

     View key:
       empty   no | yes | retry
               `yes`   the empty pane, `<p class="hint">Nothing needs
                       attention on this Mac.</p>` — code-derived: the fixture
                       cannot reach it, because `team-homelab` and
                       `fed-homelab` are unconditional in notesNow (map §8.1).
               `retry` one extra card whose button is live — also
                       code-derived, from bridge.ts:1746: a catalog failure
                       becomes `{ id:'catalog-<scope>-<n>', title:'Could not
                       load <source> on <profile>', action:'Retry' }` when the
                       error is retryable. The fixture has no catalog failure,
                       so this is the only way to see an enabled card.        */

  (function () {
    var esc = M.esc, UI = M.ui, D = M.data;

    /* bridge.ts:1746-1757, with a retryable non-fatal catalog failure on the
       Acme profile. Not in the fixture — see the `empty=retry` note above. */
    var RETRYABLE = {
      id: 'catalog-catalog-0',
      severity: 'warn',
      title: 'Could not load catalog on acme',
      detail: 'The catalog request to foks.acme-corp.com did not complete.',
      action: 'Retry',
    };
    var ACTIONS_ARE_LATER = 'Open the screen that owns this action: Servers or Groups.';

    /* alerts-screen.tsx:27. `Resume creation` and `Restore access` name work
       that lives on the Groups screens, and `Wait for the agent` names work
       only the agent can do, so none of them is wired here — the app renders
       all three disabled, with the title below. Resuming Homelab is reached
       by the three affordances that do own it (the Homelab vault takeover,
       the group page band, Settings › Groups › Open). */
    function canRetry(note) {
      return note.action === 'Retry' && note.id.indexOf('catalog-') === 0;
    }
    function card(note) {
      var live = canRetry(note);
      return '<div class="card"><span class="sev ' + esc(note.severity) + '"></span>' +
        '<div><h3>' + esc(note.title) + '</h3><p>' + esc(note.detail) + '</p></div>' +
        UI.btn(esc(note.action), {
          disabled: !live,
          title: live ? 'Try the catalog load again' : ACTIONS_ARE_LATER,
          attrs: live ? 'data-act="call" data-fn="alertsRetry" data-arg="' + esc(note.id) + '"' : undefined,
        }) + '</div>';
    }

    function notes(s) {
      var mode = s.v.empty || 'no';
      if (mode === 'yes') return [];
      var list = D.world(s).alerts.slice();
      if (mode === 'retry') list = list.concat([RETRYABLE]);
      return list;
    }

    /* onRefreshWorld() — the catalog load is retried; the note stays until a
       load answers differently, and nothing is toasted. */
    M.fns.alertsRetry = function () {
      M.hint('onRefreshWorld() — the catalog load is retried. No toast; the card stays until a load answers differently.');
    };

    M.page({
      id: 'alerts',
      title: 'Alerts',
      path: ['Alerts'],
      nav: 'alerts',
      note: 'notesNow(world): 2 cards fresh, 3 under acme=lapsed (the critical foks.acme-corp.com is locked). Every fixture button is inert.',
      controls: [
        {
          key: 'empty', label: 'Alerts list',
          note: 'The two states the fixture cannot reach (map §8.1), both derived from the code rather than transcribed from a capture.',
          values: [
            { v: 'no', label: 'Fixture', hint: 'notesNow(world) — 2 entries, or 3 while Acme’s check-in is lapsed.' },
            { v: 'yes', label: 'Empty', hint: 'no notes at all: <p class="hint">Nothing needs attention on this Mac.</p>' },
            { v: 'retry', label: '+ retryable', hint: 'one catalog-* note whose action is Retry, so its button is the only live one on the pane.' },
          ],
        },
      ],
      render: function (s) {
        var t = M.globalTakeover(s);
        if (t) return t;
        var list = notes(s);
        return {
          /* The sidebar badge IS this list — `notesNow(world).length`
             (shell/sidebar.tsx) — so the two can never disagree in the app.
             `v.empty` is the mock's own knob for the two lists the fixture
             cannot produce, and the world it reads does not know about it, so
             this pane hands the count down rather than letting the badge say
             2 over an empty pane. */
          side: M.renderSidebar(s, M.current(), { alerts: list.length }),
          main: UI.main(
            UI.pageHeader({ title: 'Alerts', subtitle: '' }) +
            UI.body(list.length
              ? '<div class="cards">' + list.map(card).join('') + '</div>'
              : '<p class="hint">Nothing needs attention on this Mac.</p>')),
        };
      },
    });
  })();
