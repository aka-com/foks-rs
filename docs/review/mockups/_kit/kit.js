/* FOKS mockup kit · kit.js
   ---------------------------------------------------------------------------
   No dependencies; exposes window.Kit. inline.mjs copies this file into each
   mockup between the KIT:JS markers; edit it here and re-run inline.mjs.

   Load it in <head>. It applies the stored theme and notes preference before
   first paint, then wires the page on DOMContentLoaded. Page scripts placed
   at the end of <body> run before that, so markup they render synchronously
   is wired too; call Kit.refresh(element) after injecting markup later.

   Behaviour is declarative (data attributes, see README.md) and delegated
   from the document, so it applies to markup added at any time. */
(function () {
  'use strict';

  var doc = document;
  var root = doc.documentElement;

  /* ----------------------------------------------------------- utilities */

  var store = {
    get: function (key) {
      try {
        return window.localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    set: function (key, value) {
      try {
        window.localStorage.setItem(key, value);
      } catch {
        /* Storage can be blocked (private windows, file:// policies). */
      }
    },
  };

  function param(name) {
    try {
      return new URLSearchParams(window.location.search).get(name);
    } catch {
      return null;
    }
  }

  function esc(value) {
    return String(value == null ? '' : value)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function tokens(value) {
    return String(value || '')
      .trim()
      .split(/\s+/)
      .filter(Boolean);
  }

  function emit(name, detail) {
    doc.dispatchEvent(new CustomEvent(name, { detail: detail }));
  }

  function fragment(html) {
    var template = doc.createElement('template');
    template.innerHTML = html;
    return template.content;
  }

  var FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), ' +
    'select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"]), ' +
    '[contenteditable="true"]';

  function isShown(el) {
    if (!el || el.closest('[hidden]')) return false;
    var style = window.getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none') return false;
    return el.getClientRects().length > 0;
  }

  function focusables(scope) {
    return Array.prototype.filter.call(
      scope.querySelectorAll(FOCUSABLE),
      isShown,
    );
  }

  function focus(el) {
    if (!el) return;
    try {
      el.focus({ preventScroll: true });
    } catch {
      el.focus();
    }
  }

  function windowOf(el) {
    return (el && el.closest && el.closest('.window')) || null;
  }

  /* --------------------------------------------------------------- icons
     Inner SVG markup copied from the lucide package the app uses
     (node_modules/lucide). Regenerate with `node icons.mjs --kit`. */

  var ICONS = {
    /* ICONS:BEGIN */
    archive:
      '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
    'arrow-down': '<path d="M12 5v14"/><path d="m19 12-7 7-7-7"/>',
    'arrow-left': '<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>',
    'arrow-up': '<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>',
    'arrow-up-down':
      '<path d="m21 16-4 4-4-4"/><path d="M17 20V4"/><path d="m3 8 4-4 4 4"/><path d="M7 4v16"/>',
    'at-sign':
      '<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>',
    bell: '<path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M3.262 15.326A1 1 0 0 0 4 17h16a1 1 0 0 0 .74-1.673C19.41 13.956 18 12.499 18 8A6 6 0 0 0 6 8c0 4.499-1.411 5.956-2.738 7.326"/>',
    'bell-off':
      '<path d="M10.268 21a2 2 0 0 0 3.464 0"/><path d="M17 17H4a1 1 0 0 1-.74-1.673C4.59 13.956 6 12.499 6 8a6 6 0 0 1 .258-1.742"/><path d="m2 2 20 20"/><path d="M8.668 3.01A6 6 0 0 1 18 8c0 2.687.77 4.653 1.707 6.05"/>',
    bold: '<path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/>',
    bot: '<path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/>',
    check: '<path d="M20 6 9 17l-5-5"/>',
    'check-check':
      '<path d="M18 6 7 17l-5-5"/><path d="m22 10-7.5 7.5L13 16"/>',
    'chevron-down': '<path d="m6 9 6 6 6-6"/>',
    'chevron-left': '<path d="m15 18-6-6 6-6"/>',
    'chevron-right': '<path d="m9 18 6-6-6-6"/>',
    'chevron-up': '<path d="m18 15-6-6-6 6"/>',
    'circle-alert':
      '<circle cx="12" cy="12" r="10"/><line x1="12" x2="12" y1="8" y2="12"/><line x1="12" x2="12.01" y1="16" y2="16"/>',
    'circle-check':
      '<circle cx="12" cy="12" r="10"/><path d="m16 9-5.5 5.5L8 12"/>',
    'circle-help':
      '<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>',
    clock: '<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>',
    code: '<path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/>',
    copy: '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>',
    dices:
      '<rect width="12" height="12" x="2" y="10" rx="2" ry="2"/><path d="m17.92 14 3.5-3.5a2.24 2.24 0 0 0 0-3l-5-4.92a2.24 2.24 0 0 0-3 0L10 6"/><path d="M6 18h.01"/><path d="M10 14h.01"/><path d="M15 6h.01"/><path d="M18 9h.01"/>',
    'door-open':
      '<path d="M10 21H2"/><path d="M10 4a2 2 0 012.36-1.968l5.41.992A1.5 1.5 0 0119 4.5V21l-7.876.992A1 1 0 0110 21z"/><path d="M10.268 3H7a2 2 0 00-2 2v16"/><path d="M14 12h.01"/><path d="M22 21h-3"/>',
    download:
      '<path d="M12 15V3"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/>',
    ellipsis:
      '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
    'external-link':
      '<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>',
    eye: '<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>',
    'eye-off':
      '<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>',
    file: '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/>',
    'file-text':
      '<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"/><path d="M14 2v5a1 1 0 0 0 1 1h5"/><path d="M10 9H8"/><path d="M16 13H8"/><path d="M16 17H8"/>',
    fingerprint:
      '<path d="M12 10a2 2 0 0 0-2 2c0 1.02-.1 2.51-.26 4"/><path d="M14 13.12c0 2.38 0 6.38-1 8.88"/><path d="M17.29 21.02c.12-.6.43-2.3.5-3.02"/><path d="M2 12a10 10 0 0 1 18-6"/><path d="M2 16h.01"/><path d="M21.8 16c.2-2 .131-5.354 0-6"/><path d="M5 19.5C5.5 18 6 15 6 12a6 6 0 0 1 .34-2"/><path d="M8.65 22c.21-.66.45-1.32.57-2"/><path d="M9 6.8a6 6 0 0 1 9 5.2v2"/>',
    flag: '<path d="M4 22V4a1 1 0 0 1 .4-.8A6 6 0 0 1 8 2c3 0 5 2 7.333 2q2 0 3.067-.8A1 1 0 0 1 20 4v10a1 1 0 0 1-.4.8A6 6 0 0 1 16 16c-3 0-5-2-8-2a6 6 0 0 0-4 1.528"/>',
    folder:
      '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>',
    'folder-input':
      '<path d="M2 9V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-1"/><path d="M2 13h10"/><path d="m9 16 3-3-3-3"/>',
    'folder-open':
      '<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>',
    globe:
      '<circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/>',
    hash: '<line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/>',
    history:
      '<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>',
    inbox:
      '<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>',
    info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>',
    italic:
      '<line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/>',
    key: '<path d="m2 21 9.6-9.6"/><path d="m7.5 15.5 2.3 2.3a1 1 0 0 1 0 1.4l-2.1 2.1a1 1 0 0 1-1.4 0L4 19"/><circle cx="15.5" cy="7.5" r="5.5"/>',
    'key-round':
      '<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"/><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"/>',
    keyboard:
      '<path d="M10 8h.01"/><path d="M12 12h.01"/><path d="M14 8h.01"/><path d="M16 12h.01"/><path d="M18 8h.01"/><path d="M6 8h.01"/><path d="M7 16h10"/><path d="M8 12h.01"/><rect width="20" height="16" x="2" y="4" rx="2"/>',
    laptop:
      '<path d="M18 5a2 2 0 0 1 2 2v8.526a2 2 0 0 0 .212.897l1.068 2.127a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45l1.068-2.127A2 2 0 0 0 4 15.526V7a2 2 0 0 1 2-2z"/><path d="M20.054 15.987H3.946"/>',
    'layout-grid':
      '<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>',
    link: '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>',
    list: '<path d="M3 5h.01"/><path d="M3 12h.01"/><path d="M3 19h.01"/><path d="M8 5h13"/><path d="M8 12h13"/><path d="M8 19h13"/>',
    'list-filter': '<path d="M2 5h20"/><path d="M6 12h12"/><path d="M9 19h6"/>',
    'loader-circle': '<path d="M21 12a9 9 0 1 1-6.219-8.56"/>',
    lock: '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>',
    'log-out':
      '<path d="m16 17 5-5-5-5"/><path d="M21 12H9"/><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>',
    mail: '<path d="m22 7-8.991 5.727a2 2 0 0 1-2.009 0L2 7"/><rect x="2" y="4" width="20" height="16" rx="2"/>',
    'message-square':
      '<path d="M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z"/>',
    monitor:
      '<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>',
    moon: '<path d="M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401"/>',
    'panel-left':
      '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/>',
    'panel-left-close':
      '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="m16 15-3-3 3-3"/>',
    'panel-left-open':
      '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="m14 9 3 3-3 3"/>',
    paperclip:
      '<path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"/>',
    'paw-print':
      '<circle cx="11" cy="4" r="2"/><circle cx="18" cy="8" r="2"/><circle cx="20" cy="16" r="2"/><path d="M9 10a5 5 0 0 1 5 5v3.5a3.5 3.5 0 0 1-6.84 1.045Q6.52 17.48 4.46 16.84A3.5 3.5 0 0 1 5.5 10Z"/>',
    pencil:
      '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>',
    pin: '<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>',
    plug: '<path d="M12 22v-5"/><path d="M15 8V2"/><path d="M17 8a1 1 0 0 1 1 1v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1z"/><path d="M9 8V2"/>',
    plus: '<path d="M5 12h14"/><path d="M12 5v14"/>',
    quote:
      '<path d="M16 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z"/><path d="M5 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z"/>',
    'refresh-cw':
      '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    reply: '<path d="M20 18v-2a4 4 0 0 0-4-4H4"/><path d="m9 17-5-5 5-5"/>',
    search: '<path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/>',
    send: '<path d="M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z"/><path d="m21.854 2.147-10.94 10.939"/>',
    server:
      '<rect width="20" height="8" x="2" y="2" rx="2" ry="2"/><rect width="20" height="8" x="2" y="14" rx="2" ry="2"/><line x1="6" x2="6.01" y1="6" y2="6"/><line x1="6" x2="6.01" y1="18" y2="18"/>',
    settings:
      '<path d="M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"/><circle cx="12" cy="12" r="3"/>',
    shield:
      '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>',
    'shield-alert':
      '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="M12 8v4"/><path d="M12 16h.01"/>',
    'shield-check':
      '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/><path d="m9 12 2 2 4-4"/>',
    smartphone:
      '<rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/>',
    smile:
      '<path d="M15 10V9"/><path d="M16.472 15a6 6 0 01-8.943 0"/><path d="M9 10V9"/><circle cx="12" cy="12" r="10"/>',
    'square-check':
      '<rect width="18" height="18" x="3" y="3" rx="2"/><path d="m16 9-5.5 5.5L8 12"/>',
    star: '<path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2"/><path d="M12 20v2"/><path d="m4.93 4.93 1.41 1.41"/><path d="m17.66 17.66 1.41 1.41"/><path d="M2 12h2"/><path d="M20 12h2"/><path d="m6.34 17.66-1.41 1.41"/><path d="m19.07 4.93-1.41 1.41"/>',
    terminal: '<path d="M12 19h8"/><path d="m4 17 6-6-6-6"/>',
    trash:
      '<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
    'trash-2':
      '<path d="M10 11v6"/><path d="M14 11v6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>',
    'triangle-alert':
      '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>',
    'undo-2':
      '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>',
    unlock:
      '<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>',
    upload:
      '<path d="M12 3v12"/><path d="m17 8-5-5-5 5"/><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>',
    user: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
    'user-plus':
      '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" x2="19" y1="8" y2="14"/><line x1="22" x2="16" y1="11" y2="11"/>',
    users:
      '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><path d="M16 3.128a4 4 0 0 1 0 7.744"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><circle cx="9" cy="7" r="4"/>',
    vault:
      '<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/><path d="m7.9 7.9 2.7 2.7"/><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"/><path d="m13.4 10.6 2.7-2.7"/><circle cx="7.5" cy="16.5" r=".5" fill="currentColor"/><path d="m7.9 16.1 2.7-2.7"/><circle cx="16.5" cy="16.5" r=".5" fill="currentColor"/><path d="m13.4 13.4 2.7 2.7"/><circle cx="12" cy="12" r="2"/>',
    'wand-sparkles':
      '<path d="m21.64 3.64-1.28-1.28a1.21 1.21 0 0 0-1.72 0L2.36 18.64a1.21 1.21 0 0 0 0 1.72l1.28 1.28a1.2 1.2 0 0 0 1.72 0L21.64 5.36a1.2 1.2 0 0 0 0-1.72"/><path d="m14 7 3 3"/><path d="M5 6v4"/><path d="M19 14v4"/><path d="M10 2v2"/><path d="M7 8H3"/><path d="M21 16h-4"/><path d="M11 3H9"/>',
    x: '<path d="M18 6 6 18"/><path d="m6 6 12 12"/>',
    /* ICONS:END */
  };

  function icon(name, size, extraClass) {
    var body = ICONS[name];
    if (body == null) {
      if (window.console) console.warn('Kit.icon: unknown icon "' + name + '"');
      body = '';
    }
    var style = size
      ? ' style="width:' + size + 'px;height:' + size + 'px"'
      : '';
    var cls = 'ic' + (extraClass ? ' ' + extraClass : '');
    return (
      '<svg class="' +
      esc(cls) +
      '" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
      'stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" ' +
      'focusable="false"' +
      style +
      '>' +
      body +
      '</svg>'
    );
  }

  function registerIcons(map) {
    Object.keys(map || {}).forEach(function (name) {
      ICONS[name] = map[name];
    });
  }

  /* <i data-icon="folder" data-size="15" class="extra"></i> becomes an SVG. */
  function hydrateIcons(scope) {
    var nodes = (scope || doc).querySelectorAll('[data-icon]');
    Array.prototype.forEach.call(nodes, function (node) {
      if (node.tagName.toLowerCase() === 'svg') return;
      var size = node.getAttribute('data-size');
      var svg = fragment(
        icon(
          node.getAttribute('data-icon'),
          size ? Number(size) : 0,
          node.className,
        ),
      ).firstChild;
      var label = node.getAttribute('aria-label');
      if (label) {
        svg.removeAttribute('aria-hidden');
        svg.setAttribute('role', 'img');
        svg.setAttribute('aria-label', label);
      }
      if (node.title)
        svg.insertAdjacentHTML(
          'afterbegin',
          '<title>' + esc(node.title) + '</title>',
        );
      node.replaceWith(svg);
    });
  }

  /* <kbd data-kit-kbd="K"></kbd> reads "⌘K" on macOS and "Ctrl K" elsewhere,
     as the app's search field does. */
  var IS_MAC = /Mac|iPhone|iPad/i.test(
    (navigator.platform || '') + ' ' + (navigator.userAgent || ''),
  );

  function hydrateKbd(scope) {
    Array.prototype.forEach.call(
      (scope || doc).querySelectorAll('[data-kit-kbd]'),
      function (el) {
        var key = el.getAttribute('data-kit-kbd');
        el.textContent = IS_MAC ? '\u2318' + key : 'Ctrl ' + key;
      },
    );
  }

  /* --------------------------------------------------------------- theme */

  var THEME_KEY = 'foks-kit:theme';
  var THEMES = ['light', 'dark', 'system'];
  var themeChoice = 'system';

  function applyTheme(choice) {
    themeChoice = THEMES.indexOf(choice) >= 0 ? choice : 'system';
    if (themeChoice === 'system') delete root.dataset.theme;
    else root.dataset.theme = themeChoice;
    var buttons = doc.querySelectorAll('[data-theme-choice]');
    Array.prototype.forEach.call(buttons, function (button) {
      var on = button.getAttribute('data-theme-choice') === themeChoice;
      button.setAttribute('aria-pressed', on ? 'true' : 'false');
      button.classList.toggle('on', on);
    });
  }

  function setTheme(choice, options) {
    applyTheme(choice);
    if (!options || options.persist !== false)
      store.set(THEME_KEY, themeChoice);
    emit('kit:theme', { theme: themeChoice, resolved: resolvedTheme() });
  }

  function resolvedTheme() {
    if (themeChoice !== 'system') return themeChoice;
    try {
      return window.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light';
    } catch {
      return 'light';
    }
  }

  applyTheme(param('theme') || store.get(THEME_KEY) || 'system');

  /* --------------------------------------------------------------- notes */

  var NOTES_KEY = 'foks-kit:notes';

  function notesOpen() {
    return root.dataset.notes !== 'closed';
  }

  function applyNotes(open) {
    root.dataset.notes = open ? 'open' : 'closed';
    var toggles = doc.querySelectorAll('[data-notes-toggle]');
    Array.prototype.forEach.call(toggles, function (button) {
      button.setAttribute('aria-expanded', open ? 'true' : 'false');
    });
  }

  function setNotes(open, options) {
    applyNotes(!!open);
    if (!options || options.persist !== false)
      store.set(NOTES_KEY, open ? 'open' : 'closed');
    emit('kit:notes', { open: !!open });
  }

  applyNotes((param('notes') || store.get(NOTES_KEY) || 'open') !== 'closed');

  /* --------------------------------------------------------------- state */

  var currentState = null;

  function stateTabs() {
    return Array.prototype.slice.call(doc.querySelectorAll('[data-state-tab]'));
  }

  function stateNames() {
    return stateTabs().map(function (tab) {
      return tab.getAttribute('data-state-tab');
    });
  }

  function inList(el, attr, state) {
    var list = tokens(el.getAttribute(attr));
    return list.indexOf(state) >= 0 || list.indexOf('*') >= 0;
  }

  function applyStateVisibility(scope) {
    if (currentState == null) return;
    var base = scope || doc;
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-show-state]'),
      function (el) {
        el.hidden = !inList(el, 'data-show-state', currentState);
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-hide-state]'),
      function (el) {
        el.hidden = inList(el, 'data-hide-state', currentState);
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-notes-for]'),
      function (el) {
        el.hidden = !inList(el, 'data-notes-for', currentState);
      },
    );
    /* data-state-class="files:with-details chat,files:other" */
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-state-class]'),
      function (el) {
        tokens(el.getAttribute('data-state-class')).forEach(function (pair) {
          var at = pair.indexOf(':');
          if (at < 0) return;
          var states = pair.slice(0, at).split(',');
          el.classList.toggle(
            pair.slice(at + 1),
            states.indexOf(currentState) >= 0 || states.indexOf('*') >= 0,
          );
        });
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-on-state]'),
      function (el) {
        var on = inList(el, 'data-on-state', currentState);
        el.classList.toggle('on', on);
        if (on) el.setAttribute('aria-current', 'page');
        else el.removeAttribute('aria-current');
      },
    );
  }

  function setState(name, options) {
    options = options || {};
    var names = stateNames();
    if (names.length && names.indexOf(name) < 0) return false;
    var previous = currentState;
    currentState = name;
    if (doc.body) doc.body.dataset.state = name;
    stateTabs().forEach(function (tab) {
      var on = tab.getAttribute('data-state-tab') === name;
      tab.setAttribute('aria-selected', on ? 'true' : 'false');
      tab.tabIndex = on ? 0 : -1;
      if (on && options.focus) focus(tab);
      if (on && tab.scrollIntoView && options.focus) {
        tab.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      }
    });
    if (previous !== name) {
      closeFloating();
      closeAllDialogs();
      closeDrawers();
    }
    applyStateVisibility(doc);
    if (options.hash !== false) {
      var hash = '#' + name;
      if (window.location.hash !== hash) {
        try {
          window.history.replaceState(window.history.state, '', hash);
        } catch {
          window.location.hash = hash;
        }
      }
    }
    if (previous !== name)
      emit('kit:state', { state: name, previous: previous });
    return true;
  }

  function initialState() {
    var names = stateNames();
    var fromHash;
    try {
      fromHash = decodeURIComponent(window.location.hash.slice(1));
    } catch {
      fromHash = '';
    }
    if (fromHash && (names.indexOf(fromHash) >= 0 || !names.length))
      return fromHash;
    var fromBody = doc.body && doc.body.dataset.state;
    if (fromBody && (names.indexOf(fromBody) >= 0 || !names.length))
      return fromBody;
    return names[0] || null;
  }

  function onTabKey(event, tab) {
    var tabs = stateTabs().filter(isShown);
    var index = tabs.indexOf(tab);
    var next = null;
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown')
      next = tabs[(index + 1) % tabs.length];
    else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp')
      next = tabs[(index - 1 + tabs.length) % tabs.length];
    else if (event.key === 'Home') next = tabs[0];
    else if (event.key === 'End') next = tabs[tabs.length - 1];
    if (!next) return;
    event.preventDefault();
    setState(next.getAttribute('data-state-tab'), { focus: true });
  }

  /* --------------------------------------------------------------- toast */

  function toastHost(within) {
    if (within) return within;
    var windows = Array.prototype.filter.call(
      doc.querySelectorAll('.window'),
      isShown,
    );
    var active = windowOf(doc.activeElement);
    if (active && windows.indexOf(active) >= 0) return active;
    return windows[0] || doc.body;
  }

  function toast(text, options) {
    options = options || {};
    var host = toastHost(options.within);
    var region = null;
    for (var i = 0; i < host.children.length; i++) {
      if (host.children[i].classList.contains('toasts'))
        region = host.children[i];
    }
    if (!region) {
      region = doc.createElement('div');
      region.className = 'toasts';
      host.appendChild(region);
    }
    var tone = options.tone === 'warning' ? 'warning' : 'info';
    var el = doc.createElement('div');
    el.className = 'toast';
    el.setAttribute('data-toast-tone', tone);
    el.setAttribute('role', tone === 'warning' ? 'alert' : 'status');
    el.setAttribute('aria-atomic', 'true');
    var message = doc.createElement('span');
    message.className = 'toast-message';
    message.textContent = text;
    el.appendChild(message);
    var timer = null;
    var closed = false;
    function close() {
      if (closed) return;
      closed = true;
      if (timer) clearTimeout(timer);
      el.classList.remove('show');
      setTimeout(function () {
        el.remove();
      }, 260);
    }
    if (options.action && options.action.label) {
      var action = doc.createElement('button');
      action.type = 'button';
      action.className = 'toast-action';
      action.textContent = options.action.label;
      action.addEventListener('click', function () {
        if (typeof options.action.onClick === 'function')
          options.action.onClick();
        close();
      });
      el.appendChild(action);
    }
    var dismiss = doc.createElement('button');
    dismiss.type = 'button';
    dismiss.className = 'toast-dismiss';
    dismiss.setAttribute('aria-label', 'Dismiss notification');
    dismiss.textContent = '×';
    dismiss.addEventListener('click', close);
    el.appendChild(dismiss);
    region.appendChild(el);
    var timeout =
      options.timeout == null
        ? options.action
          ? 8000
          : 5000
        : options.timeout;
    function arm() {
      if (timeout > 0 && isFinite(timeout)) timer = setTimeout(close, timeout);
    }
    el.addEventListener('mouseenter', function () {
      if (timer) clearTimeout(timer);
    });
    el.addEventListener('mouseleave', arm);
    el.addEventListener('focusin', function () {
      if (timer) clearTimeout(timer);
    });
    el.addEventListener('focusout', arm);
    requestAnimationFrame(function () {
      el.classList.add('show');
    });
    arm();
    return { element: el, close: close };
  }

  /* ---------------------------------------------- menus and popovers
     One floating panel is open at a time. On open, the panel moves into the
     trigger's window (so it is clipped like the app's portal) and returns to
     its place on close. */

  var open = null;
  var ITEM =
    '[role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"], [role="option"]';

  function menuItems(panel) {
    var items = panel.querySelectorAll(ITEM);
    if (!items.length) items = panel.querySelectorAll('button, a[href]');
    return Array.prototype.filter.call(items, function (item) {
      return !item.disabled && isShown(item);
    });
  }

  function isMenuLike(panel) {
    var role = panel.getAttribute('role');
    return (
      role === 'menu' ||
      role === 'listbox' ||
      !!panel.querySelector('[role="listbox"], [role="menu"]')
    );
  }

  function place(panel, host, anchor, point, align) {
    panel.style.left = '0px';
    panel.style.top = '0px';
    panel.style.right = 'auto';
    panel.style.bottom = 'auto';
    var isBody = host === doc.body;
    var box;
    if (isBody) {
      box = {
        left: 0,
        top: 0,
        right: root.clientWidth,
        bottom: window.innerHeight,
      };
    } else {
      var hr = host.getBoundingClientRect();
      box = {
        left: hr.left + host.clientLeft,
        top: hr.top + host.clientTop,
        right: hr.left + host.clientLeft + host.clientWidth,
        bottom: hr.top + host.clientTop + host.clientHeight,
      };
    }
    var width = panel.offsetWidth;
    var height = panel.offsetHeight;
    var x;
    var y;
    var above;
    if (point) {
      x = point.x;
      y = point.y;
      above = point.y - height;
    } else if (!anchor) {
      x = (box.left + box.right - width) / 2;
      y = box.top + 60;
      above = y;
    } else {
      var r = anchor.getBoundingClientRect();
      x = align === 'end' ? r.right - width : r.left;
      y = r.bottom + 4;
      above = r.top - 4 - height;
    }
    var pad = 8;
    if (y + height > box.bottom - pad && above >= box.top + pad) y = above;
    x = Math.max(box.left + pad, Math.min(x, box.right - pad - width));
    y = Math.max(box.top + pad, Math.min(y, box.bottom - pad - height));
    if (isBody) {
      panel.style.left = Math.round(x + window.scrollX) + 'px';
      panel.style.top = Math.round(y + window.scrollY) + 'px';
    } else {
      panel.style.left = Math.round(x - box.left) + 'px';
      panel.style.top = Math.round(y - box.top) + 'px';
    }
  }

  function openFloating(panel, trigger, options) {
    if (typeof panel === 'string') panel = doc.getElementById(panel);
    if (!panel) return;
    options = options || {};
    closeFloating();
    var host = windowOf(trigger) || windowOf(panel) || doc.body;
    var placeholder = null;
    if (panel.parentNode !== host) {
      placeholder = doc.createComment('kit-floating');
      panel.parentNode.insertBefore(placeholder, panel);
      host.appendChild(panel);
    }
    panel.hidden = false;
    panel.classList.add('kit-floating');
    var align =
      options.align ||
      (trigger && trigger.getAttribute('data-align')) ||
      (panel.classList.contains('right') ? 'end' : 'start');
    place(panel, host, trigger, options.point, align);
    if (trigger && trigger.hasAttribute('aria-expanded'))
      trigger.setAttribute('aria-expanded', 'true');
    open = { panel: panel, trigger: trigger, placeholder: placeholder };
    if (isMenuLike(panel)) {
      var items = menuItems(panel);
      items.forEach(function (item) {
        item.tabIndex = -1;
      });
      var selected = items.filter(function (item) {
        return (
          item.getAttribute('aria-selected') === 'true' ||
          item.classList.contains('on')
        );
      })[0];
      focus(options.last ? items[items.length - 1] : selected || items[0]);
    } else {
      if (!panel.hasAttribute('tabindex')) panel.tabIndex = -1;
      focus(
        panel.querySelector('[data-autofocus]') ||
          focusables(panel)[0] ||
          panel,
      );
    }
    emit('kit:open', { panel: panel, trigger: trigger });
  }

  function closeFloating(options) {
    if (!open) return;
    var current = open;
    open = null;
    var panel = current.panel;
    panel.hidden = true;
    panel.classList.remove('kit-floating');
    panel.style.left =
      panel.style.top =
      panel.style.right =
      panel.style.bottom =
        '';
    if (current.placeholder && current.placeholder.parentNode) {
      current.placeholder.parentNode.insertBefore(panel, current.placeholder);
      current.placeholder.remove();
    }
    var trigger = current.trigger;
    if (trigger && trigger.hasAttribute('aria-expanded'))
      trigger.setAttribute('aria-expanded', 'false');
    if (options && options.focus && trigger) focus(trigger);
    emit('kit:close', { panel: panel, trigger: trigger });
  }

  function onMenuKey(event) {
    var panel = open.panel;
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      closeFloating({ focus: true });
      return true;
    }
    if (event.key === 'Tab') {
      if (isMenuLike(panel)) {
        event.preventDefault();
        closeFloating({ focus: true });
        return true;
      }
      return false;
    }
    if (!isMenuLike(panel)) return false;
    var items = menuItems(panel);
    if (!items.length) return false;
    var index = items.indexOf(doc.activeElement);
    var next = null;
    if (event.key === 'ArrowDown') next = items[(index + 1) % items.length];
    else if (event.key === 'ArrowUp')
      next = items[(index - 1 + items.length) % items.length];
    else if (event.key === 'Home') next = items[0];
    else if (event.key === 'End') next = items[items.length - 1];
    else if (event.key.length === 1 && /\S/.test(event.key)) {
      var key = event.key.toLowerCase();
      var ordered = items.slice(index + 1).concat(items.slice(0, index + 1));
      next = ordered.filter(function (item) {
        return item.textContent.trim().toLowerCase().indexOf(key) === 0;
      })[0];
    }
    if (!next) return false;
    event.preventDefault();
    focus(next);
    return true;
  }

  /* A card-select option copies its label into the trigger. */
  function chooseOption(option) {
    var listbox = option.closest('[role="listbox"]');
    if (!listbox) return;
    Array.prototype.forEach.call(
      listbox.querySelectorAll('[role="option"]'),
      function (item) {
        var on = item === option;
        item.setAttribute('aria-selected', on ? 'true' : 'false');
        item.classList.toggle('on', on);
      },
    );
    var trigger = open && open.trigger;
    var label = option.querySelector('.t');
    var target = trigger && trigger.querySelector('.t');
    if (label && target) target.innerHTML = label.innerHTML;
  }

  /* ------------------------------------------------------------- dialogs */

  var dialogs = [];

  function openDialog(dialog, trigger) {
    if (typeof dialog === 'string') dialog = doc.getElementById(dialog);
    if (!dialog) return;
    closeFloating();
    var host = windowOf(trigger);
    var placeholder = null;
    if (host && !host.contains(dialog)) {
      placeholder = doc.createComment('kit-dialog');
      dialog.parentNode.insertBefore(placeholder, dialog);
      host.appendChild(dialog);
    }
    dialog.hidden = false;
    if (!dialog.hasAttribute('role')) dialog.setAttribute('role', 'dialog');
    dialog.setAttribute('aria-modal', 'true');
    if (!dialog.hasAttribute('tabindex')) dialog.tabIndex = -1;
    dialogs.push({
      dialog: dialog,
      trigger: trigger || null,
      returnTo: doc.activeElement,
      placeholder: placeholder,
    });
    var panel = dialog.querySelector('.sheet, .pal, .tcard') || dialog;
    focus(
      dialog.querySelector('[data-autofocus]') ||
        focusables(panel)[0] ||
        dialog,
    );
    emit('kit:dialog', { dialog: dialog, open: true });
  }

  function closeDialog(dialog) {
    if (typeof dialog === 'string') dialog = doc.getElementById(dialog);
    var index = -1;
    for (var i = dialogs.length - 1; i >= 0; i--) {
      if (!dialog || dialogs[i].dialog === dialog) {
        index = i;
        break;
      }
    }
    if (index < 0) return;
    var entry = dialogs.splice(index, 1)[0];
    entry.dialog.hidden = true;
    if (entry.placeholder && entry.placeholder.parentNode) {
      entry.placeholder.parentNode.insertBefore(
        entry.dialog,
        entry.placeholder,
      );
      entry.placeholder.remove();
    }
    var back = entry.trigger || entry.returnTo;
    if (back && doc.contains(back)) focus(back);
    emit('kit:dialog', { dialog: entry.dialog, open: false });
  }

  function closeAllDialogs() {
    while (dialogs.length) closeDialog(dialogs[dialogs.length - 1].dialog);
  }

  function topDialog() {
    return dialogs.length ? dialogs[dialogs.length - 1].dialog : null;
  }

  function trapTab(event, dialog) {
    var items = focusables(dialog);
    if (!items.length) {
      event.preventDefault();
      focus(dialog);
      return;
    }
    var first = items[0];
    var last = items[items.length - 1];
    var active = doc.activeElement;
    if (!dialog.contains(active)) {
      event.preventDefault();
      focus(first);
    } else if (event.shiftKey && (active === first || active === dialog)) {
      event.preventDefault();
      focus(last);
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      focus(first);
    }
  }

  /* ------------------------------------------------------------- drawers
     Below 760px of window width, the side column of a split view is a
     drawer. The topbar's [data-kit-drawer] button opens it. */

  var DRAWERS =
    '.chat-inbox, .folder-split > .tpane, .subnav-side, .kit-split > .kit-side';

  function drawerOf(win, button) {
    var id = button && button.getAttribute('aria-controls');
    var byId = id && doc.getElementById(id);
    if (byId) return byId;
    return Array.prototype.filter.call(
      win.querySelectorAll(DRAWERS),
      function (el) {
        return !el.closest('[hidden]');
      },
    )[0];
  }

  function setDrawer(win, openIt, button) {
    if (!win) return;
    button = button || win.querySelector('[data-kit-drawer]');
    var drawer = drawerOf(win, button);
    if (openIt) {
      if (!drawer) return;
      win.dataset.drawer = 'open';
      var host = drawer.parentElement;
      if (!host.querySelector(':scope > .kit-scrim')) {
        var scrim = doc.createElement('div');
        scrim.className = 'kit-scrim';
        scrim.setAttribute('data-drawer-close', '');
        scrim.setAttribute('aria-hidden', 'true');
        host.appendChild(scrim);
      }
      Array.prototype.forEach.call(
        win.querySelectorAll('[data-kit-drawer]'),
        function (b) {
          b.setAttribute('aria-expanded', 'true');
        },
      );
      focus(
        drawer.querySelector('.on, [aria-current]') || focusables(drawer)[0],
      );
    } else {
      var hadFocus = drawer && drawer.contains(doc.activeElement);
      delete win.dataset.drawer;
      Array.prototype.forEach.call(
        win.querySelectorAll('[data-kit-drawer]'),
        function (b) {
          b.setAttribute('aria-expanded', 'false');
        },
      );
      if (hadFocus && button && isShown(button)) focus(button);
    }
  }

  function closeDrawers() {
    Array.prototype.forEach.call(
      doc.querySelectorAll('.window[data-drawer="open"]'),
      function (win) {
        setDrawer(win, false);
      },
    );
  }

  function drawerIsOpen() {
    return !!doc.querySelector('.window[data-drawer="open"]');
  }

  /* --------------------------------------------------------- wiring */

  function prepare(scope) {
    var base = scope || doc;
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-menu], [data-popover]'),
      function (trigger) {
        var id =
          trigger.getAttribute('data-menu') ||
          trigger.getAttribute('data-popover');
        var panel = doc.getElementById(id);
        if (!trigger.hasAttribute('aria-expanded'))
          trigger.setAttribute('aria-expanded', 'false');
        if (!trigger.hasAttribute('aria-controls'))
          trigger.setAttribute('aria-controls', id);
        if (
          trigger.hasAttribute('data-menu') &&
          !trigger.hasAttribute('aria-haspopup')
        ) {
          var listbox =
            panel &&
            (panel.getAttribute('role') === 'listbox' ||
              panel.querySelector('[role="listbox"]'));
          trigger.setAttribute('aria-haspopup', listbox ? 'listbox' : 'menu');
        }
        if (panel && !(open && open.panel === panel)) panel.hidden = true;
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-dialog]'),
      function (trigger) {
        if (!trigger.hasAttribute('aria-haspopup'))
          trigger.setAttribute('aria-haspopup', 'dialog');
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-kit-drawer]'),
      function (button) {
        if (!button.hasAttribute('aria-expanded'))
          button.setAttribute('aria-expanded', 'false');
      },
    );
    Array.prototype.forEach.call(
      base.querySelectorAll('[data-notes-toggle]'),
      function (button) {
        button.setAttribute('aria-expanded', notesOpen() ? 'true' : 'false');
      },
    );
  }

  function refresh(scope) {
    hydrateIcons(scope);
    hydrateKbd(scope);
    prepare(scope);
    applyStateVisibility(scope);
    applyTheme(themeChoice);
  }

  function onClick(event) {
    var target = event.target;
    if (!(target instanceof Element)) return;

    var el = target.closest('[data-theme-choice]');
    if (el) return setTheme(el.getAttribute('data-theme-choice'));

    el = target.closest('[data-notes-toggle]');
    if (el) return setNotes(!notesOpen());

    el = target.closest('[data-state-tab]');
    if (el) return setState(el.getAttribute('data-state-tab'));

    el = target.closest('[data-goto-state]');
    if (el) {
      event.preventDefault();
      setState(el.getAttribute('data-goto-state'));
      return;
    }

    /* Clicks inside the open panel. */
    if (open && open.panel.contains(target)) {
      var option = target.closest('[role="option"]');
      if (option && option.getAttribute('aria-disabled') !== 'true')
        chooseOption(option);
      var item = target.closest(ITEM + ', button, a[href]');
      if (
        item &&
        !item.disabled &&
        item.getAttribute('aria-disabled') !== 'true' &&
        !item.closest('[data-keep-open]') &&
        isMenuLike(open.panel)
      ) {
        var menuTrigger = open.trigger;
        maybeToast(item);
        closeFloating({ focus: true });
        if (item.hasAttribute('data-dialog'))
          openDialog(item.getAttribute('data-dialog'), menuTrigger);
        return;
      }
    }

    el = target.closest('[data-menu], [data-popover]');
    if (el) {
      var id = el.getAttribute('data-menu') || el.getAttribute('data-popover');
      if (open && open.trigger === el) closeFloating({ focus: true });
      else openFloating(id, el);
      return;
    }

    el = target.closest('[data-dialog]');
    if (el) return openDialog(el.getAttribute('data-dialog'), el);

    el = target.closest('[data-dialog-close]');
    if (el) {
      var dialog = el.closest('[aria-modal="true"]') || topDialog();
      maybeToast(el);
      return closeDialog(dialog);
    }

    var top = topDialog();
    if (
      top &&
      target === top &&
      top.getAttribute('data-dismissible') !== 'false'
    ) {
      return closeDialog(top);
    }

    el = target.closest('[data-kit-drawer]');
    if (el) {
      var win = windowOf(el);
      return setDrawer(win, !(win && win.dataset.drawer === 'open'), el);
    }

    el = target.closest('[data-drawer-close]');
    if (el) {
      setDrawer(windowOf(el), false);
    } else if (drawerIsOpen()) {
      var hit = target.closest(
        '.chat-channel, .fselect, .subnav-side .tab, .kit-side .nav',
      );
      if (hit && hit.closest(DRAWERS)) setDrawer(windowOf(hit), false);
    }

    el = target.closest('[data-kit-seg] button');
    if (el && !el.disabled) {
      Array.prototype.forEach.call(
        el.parentElement.children,
        function (button) {
          var on = button === el;
          button.classList.toggle('on', on);
          button.setAttribute('aria-pressed', on ? 'true' : 'false');
        },
      );
    }

    el = target.closest('[role="radiogroup"] [role="radio"]');
    if (el && !el.disabled && el.getAttribute('aria-disabled') !== 'true')
      selectRadio(el);

    el = target.closest('[role="switch"]');
    if (el && !el.disabled && el.getAttribute('aria-disabled') !== 'true') {
      el.setAttribute(
        'aria-checked',
        el.getAttribute('aria-checked') === 'true' ? 'false' : 'true',
      );
    }

    el = target.closest('[data-kit-disclosure]');
    if (el) toggleDisclosure(el);

    el = target.closest('[data-kit-close]');
    if (el) closePanel(el.getAttribute('data-kit-close'));

    el = target.closest('[data-toast]');
    if (el && !(open && open.panel.contains(el))) maybeToast(el);
  }

  function maybeToast(el) {
    var text = el.getAttribute && el.getAttribute('data-toast');
    if (!text) return;
    var label = el.getAttribute('data-toast-action');
    toast(text, {
      tone: el.getAttribute('data-toast-tone') || 'info',
      within: windowOf(el) || undefined,
      action: label ? { label: label } : undefined,
    });
  }

  function selectRadio(radio) {
    var group = radio.closest('[role="radiogroup"]');
    Array.prototype.forEach.call(
      group.querySelectorAll('[role="radio"]'),
      function (item) {
        var on = item === radio;
        item.setAttribute('aria-checked', on ? 'true' : 'false');
        item.classList.toggle('on', on);
        item.tabIndex = on ? 0 : -1;
      },
    );
  }

  function controllersOf(id) {
    return Array.prototype.slice.call(
      doc.querySelectorAll('[data-kit-disclosure][aria-controls="' + id + '"]'),
    );
  }

  function syncDisclosure(id, expanded) {
    controllersOf(id).forEach(function (button) {
      button.setAttribute('aria-expanded', expanded ? 'true' : 'false');
      button.classList.toggle('open', expanded);
      var team = button.closest('.chat-team');
      if (team) team.classList.toggle('collapsed', !expanded);
    });
  }

  function toggleDisclosure(button) {
    var expanded = button.getAttribute('aria-expanded') !== 'true';
    var id = button.getAttribute('aria-controls');
    var panel = id && doc.getElementById(id);
    if (panel) panel.hidden = !expanded;
    if (id) syncDisclosure(id, expanded);
    else {
      button.setAttribute('aria-expanded', expanded ? 'true' : 'false');
      button.classList.toggle('open', expanded);
    }
  }

  /* data-kit-close="panel-id": hide a panel opened by a disclosure. */
  function closePanel(id) {
    var panel = doc.getElementById(id);
    if (!panel) return;
    var hadFocus = panel.contains(doc.activeElement);
    panel.hidden = true;
    syncDisclosure(id, false);
    if (hadFocus) focus(controllersOf(id).filter(isShown)[0]);
  }

  function onKeydown(event) {
    var target = event.target;

    if (open && (open.panel.contains(target) || target === open.trigger)) {
      if (onMenuKey(event)) return;
    } else if (open && event.key === 'Escape') {
      event.preventDefault();
      closeFloating({ focus: true });
      return;
    }

    if (target instanceof Element) {
      var trigger = target.closest('[data-menu]');
      if (
        trigger &&
        !open &&
        (event.key === 'ArrowDown' || event.key === 'ArrowUp')
      ) {
        event.preventDefault();
        openFloating(trigger.getAttribute('data-menu'), trigger, {
          last: event.key === 'ArrowUp',
        });
        return;
      }

      if (
        (event.key === 'F10' && event.shiftKey) ||
        event.key === 'ContextMenu'
      ) {
        var host = target.closest('[data-context-menu]');
        if (host) {
          event.preventDefault();
          var r = host.getBoundingClientRect();
          openFloating(host.getAttribute('data-context-menu'), host, {
            point: { x: r.left + Math.min(24, r.width / 2), y: r.bottom - 2 },
          });
          return;
        }
      }

      var tab = target.closest('[data-state-tab]');
      if (tab) return onTabKey(event, tab);

      var radio = target.closest('[role="radiogroup"] [role="radio"]');
      if (radio && /^Arrow/.test(event.key)) {
        var radios = Array.prototype.filter.call(
          radio
            .closest('[role="radiogroup"]')
            .querySelectorAll('[role="radio"]'),
          function (item) {
            return (
              !item.disabled && item.getAttribute('aria-disabled') !== 'true'
            );
          },
        );
        var at = radios.indexOf(radio);
        var step =
          event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1 : -1;
        var nextRadio = radios[(at + step + radios.length) % radios.length];
        if (nextRadio) {
          event.preventDefault();
          selectRadio(nextRadio);
          focus(nextRadio);
        }
        return;
      }
    }

    var dialog = topDialog();
    if (dialog) {
      if (
        event.key === 'Escape' &&
        dialog.getAttribute('data-dismissible') !== 'false'
      ) {
        event.preventDefault();
        closeDialog(dialog);
        return;
      }
      if (event.key === 'Tab') {
        trapTab(event, dialog);
        return;
      }
    }

    if (event.key === 'Escape' && drawerIsOpen()) {
      event.preventDefault();
      var win = doc.querySelector('.window[data-drawer="open"]');
      setDrawer(win, false, win.querySelector('[data-kit-drawer]'));
      focus(win.querySelector('[data-kit-drawer]'));
    }
  }

  function onContextMenu(event) {
    var target = event.target;
    if (!(target instanceof Element)) return;
    var host = target.closest('[data-context-menu]');
    if (!host) return;
    event.preventDefault();
    openFloating(host.getAttribute('data-context-menu'), host, {
      point: { x: event.clientX, y: event.clientY },
    });
  }

  function onPointerDown(event) {
    if (!open) return;
    var target = event.target;
    if (open.panel.contains(target)) return;
    if (open.trigger && open.trigger.contains(target)) return;
    closeFloating();
  }

  function onFocusIn(event) {
    var dialog = topDialog();
    if (open && open.panel.contains(event.target)) return;
    if (dialog && !dialog.contains(event.target)) {
      var items = focusables(dialog);
      focus(items[0] || dialog);
    }
  }

  function onHashChange() {
    var name;
    try {
      name = decodeURIComponent(window.location.hash.slice(1));
    } catch {
      return;
    }
    if (name && name !== currentState) setState(name, { hash: false });
  }

  function init() {
    hydrateIcons(doc);
    hydrateKbd(doc);
    prepare(doc);
    applyTheme(themeChoice);
    applyNotes(notesOpen());
    var first = initialState();
    if (first) setState(first, { hash: stateNames().length > 0 });
    doc.addEventListener('click', onClick);
    doc.addEventListener('keydown', onKeydown);
    doc.addEventListener('contextmenu', onContextMenu);
    doc.addEventListener('pointerdown', onPointerDown, true);
    doc.addEventListener('focusin', onFocusIn);
    window.addEventListener('hashchange', onHashChange);
    window.addEventListener('resize', function () {
      if (open && open.trigger && !open.panel.hidden) closeFloating();
    });
    emit('kit:ready', { state: currentState });
  }

  /* ---------------------------------------------------------------- data
     The canonical sample data every mockup uses. Dates are local times on
     Saturday 3 and Sunday 4 October 2026; "today" is Sunday. */

  var HUES = [
    'indigo',
    'orange',
    'teal',
    'red',
    'green',
    'purple',
    'blue',
    'brown',
  ];

  /* The app's hue(name): the sum of the name's char codes, modulo 8. */
  function hue(name) {
    var sum = 0;
    var chars = Array.from(String(name));
    for (var i = 0; i < chars.length; i++) sum += chars[i].charCodeAt(0);
    return HUES[sum % HUES.length];
  }

  /* The app's single-letter mark (AccountMark, GroupMark). */
  function initial(name) {
    return String(name || '?')
      .slice(0, 1)
      .toUpperCase();
  }

  /* The app's two-letter initials(): sam.ortiz -> SO, deploy-bot -> DB. */
  function initials(name) {
    return String(name || '')
      .replace(/@.*/, '')
      .split(/[^\p{L}\p{N}]+/u)
      .filter(Boolean)
      .slice(0, 2)
      .map(function (word) {
        return word[0].toUpperCase();
      })
      .join('');
  }

  function person(username, host, extra) {
    var p = {
      username: username,
      host: host,
      hue: hue(username),
      initial: initial(username),
    };
    Object.keys(extra || {}).forEach(function (key) {
      p[key] = extra[key];
    });
    return p;
  }

  var PERSONAL = 'foks.example.net';
  var ACME = 'acme.example';

  var data = {
    today: '2026-10-04',

    me: {
      username: 'satoshi',
      host: PERSONAL,
      server: 'Personal server',
      alias: 'personal',
      hue: 'red',
      initial: 'S',
    },

    /* Local accounts on this device. The second is the real mock's work
       account, which owns the "Work (Acme)" vault; show it only where the
       account switcher appears. */
    accounts: [
      {
        username: 'satoshi',
        alias: 'personal',
        host: PERSONAL,
        server: 'Personal server',
        current: true,
        hue: 'red',
      },
      {
        username: 'vitalik',
        alias: 'work',
        host: ACME,
        server: 'Acme',
        current: false,
        hue: 'green',
      },
    ],

    servers: [
      { host: PERSONAL, label: 'Personal server', verified: true },
      { host: ACME, label: 'Acme', verified: true },
    ],

    people: {
      satoshi: person('satoshi', PERSONAL, { you: true }),
      hal: person('hal', PERSONAL),
      'priya.n': person('priya.n', ACME),
      vitalik: person('vitalik', ACME),
      'sam.ortiz': person('sam.ortiz', ACME),
      lee: person('lee', ACME),
      'deploy-bot': person('deploy-bot', ACME, { bot: true }),
    },

    /* hue: the team mark in Teams and Chat (the app's hue(name)).
       fileHue: the store mark in the Files tree, location cells and details. */
    teams: [
      {
        id: 'household',
        name: 'Household',
        host: PERSONAL,
        server: 'Personal server',
        hue: 'red',
        fileHue: 'green',
        initial: 'H',
        yourRole: 'owner',
        members: [
          { username: 'satoshi', role: 'owner' },
          { username: 'hal', role: 'member' },
        ],
        chat: { enabled: true },
        channels: [
          {
            id: 'general',
            name: 'general',
            description: 'A place for the whole team.',
            audience: 'everyone',
            unread: 0,
            newFrom: 'hh-g-6',
          },
          {
            id: 'groceries',
            name: 'groceries',
            description: '',
            audience: 'everyone',
            unread: 0,
          },
        ],
      },
      {
        id: 'engineering',
        name: 'Engineering',
        host: ACME,
        server: 'Acme',
        hue: 'red',
        fileHue: 'red',
        initial: 'E',
        yourRole: 'owner',
        members: [
          { username: 'satoshi', role: 'owner' },
          { username: 'priya.n', role: 'admin' },
          { username: 'vitalik', role: 'admin' },
          { username: 'sam.ortiz', role: 'member' },
          { username: 'lee', role: 'member' },
          { username: 'deploy-bot', role: 'member', bot: true },
        ],
        chat: { enabled: true },
        channels: [
          {
            id: 'general',
            name: 'general',
            description: '',
            audience: 'everyone',
            unread: 0,
          },
          {
            id: 'deploys',
            name: 'deploys',
            description: 'Release and rollout announcements',
            audience: 'everyone',
            unread: 2,
            newFrom: 'en-d-4',
          },
          {
            id: 'incidents',
            name: 'incidents',
            description: 'Pager rotations and postmortems',
            audience: 'admins',
            readers: ['satoshi', 'priya.n', 'vitalik'],
            unread: 0,
          },
          {
            id: 'random',
            name: 'random',
            description: '',
            audience: 'everyone',
            unread: 0,
          },
        ],
      },
      {
        id: 'homelab',
        name: 'Homelab',
        host: PERSONAL,
        server: 'Personal server',
        hue: 'indigo',
        fileHue: 'teal',
        initial: 'H',
        yourRole: 'owner',
        members: [{ username: 'satoshi', role: 'owner' }],
        chat: {
          enabled: false,
          reason: 'Chat is not enabled on Personal server',
        },
        channels: [],
      },
    ],

    /* Vaults as the Files tree lists them. Account vaults are marked with
       the account's initial. Counts are derived from items. */
    vaults: [
      {
        id: 'personal',
        name: 'Personal',
        kind: 'account',
        account: 'satoshi',
        host: PERSONAL,
        fileHue: 'red',
        initial: 'S',
        folders: ['agents', 'documents', 'env', 'env/prod', 'logins', 'ssh'],
      },
      {
        id: 'work',
        name: 'Work (Acme)',
        kind: 'account',
        account: 'vitalik',
        host: ACME,
        fileHue: 'green',
        initial: 'V',
        folders: [],
      },
      {
        id: 'household',
        name: 'Household',
        kind: 'team',
        team: 'household',
        host: PERSONAL,
        fileHue: 'green',
        initial: 'H',
        folders: ['documents', 'streaming', 'wifi'],
      },
      {
        id: 'homelab',
        name: 'Homelab',
        kind: 'team',
        team: 'homelab',
        host: PERSONAL,
        fileHue: 'teal',
        initial: 'H',
        folders: [],
      },
      {
        id: 'engineering',
        name: 'Engineering',
        kind: 'team',
        team: 'engineering',
        host: ACME,
        fileHue: 'red',
        initial: 'E',
        folders: ['deploy', 'onboarding', 'release'],
      },
    ],

    /* Items, sorted by name as "All items" lists them. */
    items: [
      {
        id: 'anthropic-api-key',
        name: 'anthropic-api-key',
        kind: 'Document',
        vault: 'personal',
        folder: 'agents',
        size: '108 B',
        bytes: 108,
      },
      {
        id: 'bundle.tar',
        name: 'bundle.tar',
        kind: 'Document',
        vault: 'engineering',
        folder: 'release',
        size: '84.4 MB',
        bytes: 84400000,
      },
      {
        id: 'DATABASE_URL',
        name: 'DATABASE_URL',
        kind: 'Document',
        vault: 'personal',
        folder: 'env/prod',
        size: '96 B',
        bytes: 96,
      },
      {
        id: 'emergency.pdf',
        name: 'emergency.pdf',
        kind: 'Document',
        vault: 'household',
        folder: 'documents',
        size: '2.8 MB',
        bytes: 2800000,
      },
      {
        id: 'fastmail.com',
        name: 'fastmail.com',
        kind: 'Password',
        vault: 'personal',
        folder: 'logins',
        size: '96 B',
        bytes: 96,
      },
      {
        id: 'github.com',
        name: 'github.com',
        kind: 'Password',
        vault: 'personal',
        folder: 'logins',
        size: '142 B',
        bytes: 142,
        version: 9,
        fields: {
          username: 'satoshi',
          password: '•'.repeat(12),
          website: 'https://github.com/login',
        },
        access: { read: 'Owner', write: 'Owner' },
        sharing: 'No one else has access to this item.',
      },
      {
        id: 'guest-password',
        name: 'guest-password',
        kind: 'Password',
        vault: 'household',
        folder: 'wifi',
        size: '64 B',
        bytes: 64,
      },
      {
        id: 'id_ed25519',
        name: 'id_ed25519',
        kind: 'Document',
        vault: 'personal',
        folder: 'ssh',
        size: '419 B',
        bytes: 419,
      },
      {
        id: 'netflix',
        name: 'netflix',
        kind: 'Password',
        vault: 'household',
        folder: 'streaming',
        size: '70 B',
        bytes: 70,
      },
      {
        id: 'passport-scan.pdf',
        name: 'passport-scan.pdf',
        kind: 'Document',
        vault: 'personal',
        folder: 'documents',
        size: '2.8 MB',
        bytes: 2800000,
      },
      {
        id: 'production-token',
        name: 'production-token',
        kind: 'Document',
        vault: 'engineering',
        folder: 'deploy',
        size: '88 B',
        bytes: 88,
      },
      {
        id: 'README.md',
        name: 'README.md',
        kind: 'Document',
        vault: 'engineering',
        folder: 'onboarding',
        size: '5.1 KB',
        bytes: 5100,
      },
      {
        id: 'staging-token',
        name: 'staging-token',
        kind: 'Document',
        vault: 'engineering',
        folder: 'deploy',
        size: '88 B',
        bytes: 88,
      },
    ],

    /* Outstanding requests to join, by team (the Teams rail count). */
    requests: { engineering: 2, household: 2 },

    devices: [
      {
        id: 'macbook-pro',
        name: 'MacBook Pro',
        kind: 'computer',
        label: 'Computer',
        icon: 'laptop',
        current: true,
      },
      {
        id: 'travel-mac',
        name: 'Travel Mac',
        kind: 'computer',
        label: 'Computer',
        icon: 'laptop',
      },
      {
        id: 'yubikey-5c',
        name: 'YubiKey 5C',
        kind: 'security-key',
        label: 'Security key',
        icon: 'key-round',
      },
      {
        id: 'cage-32',
        name: 'cage 32',
        kind: 'recovery-phrase',
        label: 'Recovery phrase',
        icon: 'file',
      },
    ],

    /* Message library, keyed "team/channel". Text uses the app's message
       markup: `code`, **bold**, *italic*, [label](https://…), ``` blocks,
       "- " and "1. " lists, "> " quotes. */
    messages: {
      'household/general': [
        {
          id: 'hh-g-1',
          author: 'hal',
          at: '2026-10-03T17:47',
          text: 'Groceries are sorted for the week. The list is in #groceries.',
        },
        {
          id: 'hh-g-2',
          author: 'hal',
          at: '2026-10-03T17:49',
          text: 'Also, the router rebooted twice this afternoon. Probably the firmware update.',
        },
        {
          id: 'hh-g-3',
          author: 'satoshi',
          at: '2026-10-04T09:12',
          text: 'Has anyone rotated the wifi password yet?',
        },
        {
          id: 'hh-g-4',
          author: 'satoshi',
          at: '2026-10-04T09:13',
          text: 'I can do it tonight. Will update the guest-password item in the vault.',
        },
        {
          id: 'hh-g-5',
          author: 'satoshi',
          at: '2026-10-04T09:14',
          text: 'Here is the snippet:\n`nmcli dev wifi show-password`',
        },
        {
          id: 'hh-g-6',
          author: 'hal',
          at: '2026-10-04T09:31',
          text: 'Thanks. Can you also update the card on the fridge? Guests still read it from there.',
        },
      ],
      'household/groceries': [
        {
          id: 'hh-r-1',
          author: 'hal',
          at: '2026-10-03T10:05',
          text: 'This week:\n- oat milk\n- eggs\n- coffee beans\n- dish soap',
        },
        {
          id: 'hh-r-2',
          author: 'satoshi',
          at: '2026-10-03T10:31',
          text: 'Added bread and tomatoes.',
        },
        {
          id: 'hh-r-3',
          author: 'hal',
          at: '2026-10-04T08:41',
          text: 'Farmers market closes at 1 today, I will go at noon.',
        },
      ],
      'engineering/general': [
        {
          id: 'en-g-1',
          author: 'priya.n',
          at: '2026-10-03T16:10',
          text: 'Can someone review the agent socket change?',
        },
        {
          id: 'en-g-2',
          author: 'priya.n',
          at: '2026-10-03T16:11',
          text: 'It moves the socket under `$XDG_RUNTIME_DIR` and checks the mode is 0600 before connecting.',
        },
        {
          id: 'en-g-3',
          author: 'vitalik',
          at: '2026-10-03T16:24',
          text: 'On it after lunch.',
        },
        {
          id: 'en-g-4',
          author: 'lee',
          at: '2026-10-04T09:05',
          text: 'Left two comments on the fallback path. Otherwise **looks good**.',
        },
      ],
      'engineering/deploys': [
        {
          id: 'en-d-1',
          author: 'deploy-bot',
          at: '2026-10-03T14:00',
          text: 'deploy #811 succeeded (prod-us)',
        },
        {
          id: 'en-d-2',
          author: 'deploy-bot',
          at: '2026-10-03T14:14',
          text: 'deploy #812 succeeded (prod-eu)',
        },
        {
          id: 'en-d-3',
          author: 'sam.ortiz',
          at: '2026-10-04T09:30',
          text: 'Rollout of 0.4.2 is at 50%',
        },
        {
          id: 'en-d-4',
          author: 'sam.ortiz',
          at: '2026-10-04T09:32',
          text: 'Holding there until the error rate settles. To follow along:\n```\nkubectl rollout status deploy/foks-server -n prod\n```',
        },
        {
          id: 'en-d-5',
          author: 'deploy-bot',
          at: '2026-10-04T10:02',
          text: 'deploy #813 started (prod-us, 0.4.2)',
        },
      ],
      'engineering/incidents': [
        {
          id: 'en-i-1',
          author: 'vitalik',
          at: '2026-10-03T23:48',
          text: 'Paging: elevated 502s on the acme.example API. Investigating.',
        },
        {
          id: 'en-i-2',
          author: 'priya.n',
          at: '2026-10-04T00:05',
          text: 'Root cause: the staging-token in the deploy vault had expired. Rotated it; the 502s are gone.',
        },
        {
          id: 'en-i-3',
          author: 'vitalik',
          at: '2026-10-04T08:15',
          text: 'Postmortem draft is up. Please add your timeline notes by Monday.',
        },
      ],
      'engineering/random': [
        {
          id: 'en-r-1',
          author: 'lee',
          at: '2026-10-03T13:15',
          text: 'Anyone up for the climbing gym on Wednesday?',
        },
        {
          id: 'en-r-2',
          author: 'sam.ortiz',
          at: '2026-10-03T13:20',
          text: 'I am in.',
        },
        {
          id: 'en-r-3',
          author: 'satoshi',
          at: '2026-10-04T09:00',
          text: 'Count me in too.',
        },
      ],
    },
  };

  function byId(list, id) {
    return list.filter(function (entry) {
      return entry.id === id;
    })[0];
  }

  data.team = function (id) {
    return byId(data.teams, id);
  };
  data.vault = function (id) {
    return byId(data.vaults, id);
  };
  data.item = function (id) {
    return byId(data.items, id);
  };
  data.device = function (id) {
    return byId(data.devices, id);
  };
  data.person = function (username) {
    return data.people[username] || person(username, '');
  };
  /* data.channel('household/general') -> { team, channel, messages } */
  data.channel = function (key) {
    var parts = String(key).split('/');
    var team = data.team(parts[0]);
    var channel = team && byId(team.channels, parts[1]);
    return team && channel
      ? { team: team, channel: channel, messages: data.messages[key] || [] }
      : null;
  };
  data.itemsIn = function (vaultId, folder) {
    return data.items.filter(function (item) {
      return (
        item.vault === vaultId &&
        (!folder ||
          item.folder === folder ||
          item.folder.indexOf(folder + '/') === 0)
      );
    });
  };
  data.unreadTotal = function () {
    return data.teams.reduce(function (sum, team) {
      return (
        sum +
        team.channels.reduce(function (n, channel) {
          return n + (channel.unread || 0);
        }, 0)
      );
    }, 0);
  };

  /* ---------------------------------------------------------- formatting */

  var MONTHS = [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ];
  var MONTHS_SHORT = MONTHS.map(function (m) {
    return m.slice(0, 3);
  });
  var WEEKDAYS = [
    'Sunday',
    'Monday',
    'Tuesday',
    'Wednesday',
    'Thursday',
    'Friday',
    'Saturday',
  ];

  function parse(iso) {
    var m = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?/.exec(String(iso));
    if (!m) return null;
    return new Date(+m[1], +m[2] - 1, +m[3], +(m[4] || 0), +(m[5] || 0));
  }

  var fmt = {
    /* "9:12 AM", the clock beside a message. */
    time: function (iso) {
      var d = parse(iso);
      if (!d) return '';
      var h = d.getHours();
      var mm = String(d.getMinutes()).padStart(2, '0');
      return (h % 12 || 12) + ':' + mm + ' ' + (h < 12 ? 'AM' : 'PM');
    },
    /* "Today", "Yesterday", a weekday within the past week, else the date. */
    day: function (iso, today) {
      var d = parse(iso);
      var t = parse(today || data.today);
      if (!d || !t) return '';
      var days = Math.round(
        (new Date(t.getFullYear(), t.getMonth(), t.getDate()) -
          new Date(d.getFullYear(), d.getMonth(), d.getDate())) /
          86400000,
      );
      if (days === 0) return 'Today';
      if (days === 1) return 'Yesterday';
      if (days > 1 && days < 7) return WEEKDAYS[d.getDay()];
      return MONTHS[d.getMonth()] + ' ' + d.getDate() + ', ' + d.getFullYear();
    },
    /* "Oct 4, 2026, 9:12 AM", the tooltip on a grouped message. */
    stamp: function (iso) {
      var d = parse(iso);
      if (!d) return '';
      return (
        MONTHS_SHORT[d.getMonth()] +
        ' ' +
        d.getDate() +
        ', ' +
        d.getFullYear() +
        ', ' +
        fmt.time(iso)
      );
    },
    dayKey: function (iso) {
      return String(iso).slice(0, 10);
    },
    plural: function (n, word) {
      return n + ' ' + word + (n === 1 ? '' : 's');
    },
  };

  /* ----------------------------------------------------------- rendering
     HTML-string helpers that emit the app's markup, for mockups that build
     threads or marks from Kit.data. */

  function safeLink(url) {
    return /^https?:\/\/[^\s\\]+$/i.test(url) ? url : null;
  }

  function inline(text) {
    var out = '';
    var last = 0;
    var pattern =
      /(`[^`\n]+`|\*\*[^*\n]+\*\*|\*[^*\n]+\*|\[[^\]\n]+\]\([^\s)]+\))/g;
    var match;
    while ((match = pattern.exec(text))) {
      out += esc(text.slice(last, match.index));
      var token = match[0];
      if (token[0] === '`')
        out += '<code>' + esc(token.slice(1, -1)) + '</code>';
      else if (token.slice(0, 2) === '**')
        out += '<strong>' + esc(token.slice(2, -2)) + '</strong>';
      else if (token[0] === '*')
        out += '<em>' + esc(token.slice(1, -1)) + '</em>';
      else {
        var split = token.indexOf('](');
        var url = safeLink(token.slice(split + 2, -1));
        out += url
          ? '<a href="' +
            esc(url) +
            '" rel="noreferrer noopener">' +
            esc(token.slice(1, split)) +
            '</a>'
          : esc(token);
      }
      last = match.index + token.length;
    }
    return out + esc(text.slice(last));
  }

  /* The app's MessageText, as an HTML string. */
  function messageText(text) {
    var lines = String(text).split('\n');
    var blocks = [];
    for (var i = 0; i < lines.length; i++) {
      var line = lines[i];
      if (line.indexOf('```') === 0) {
        var body = [];
        var start = i;
        while (++i < lines.length && lines[i].indexOf('```') !== 0)
          body.push(lines[i]);
        if (i === lines.length) {
          blocks.push('<p>' + esc(lines.slice(start).join('\n')) + '</p>');
          break;
        }
        blocks.push(
          '<div><pre><code>' +
            esc(body.join('\n')) +
            '</code></pre>' +
            '<button type="button" class="btn cap" data-toast="Copied">Copy code</button>' +
            '<span role="status"></span></div>',
        );
      } else if (/^[-*] /.test(line) || /^\d+\. /.test(line)) {
        var ordered = /^\d+\. /.test(line);
        var marker = ordered ? /^\d+\. / : /^[-*] /;
        var items = [];
        do {
          items.push('<li>' + inline(lines[i].replace(marker, '')) + '</li>');
          i++;
        } while (i < lines.length && marker.test(lines[i]));
        i--;
        blocks.push(
          ordered
            ? '<ol>' + items.join('') + '</ol>'
            : '<ul>' + items.join('') + '</ul>',
        );
      } else if (line.indexOf('> ') === 0) {
        blocks.push('<blockquote>' + inline(line.slice(2)) + '</blockquote>');
      } else if (line) {
        blocks.push('<p>' + inline(line) + '</p>');
      }
    }
    return (
      '<div class="chat-message-text">' +
      (blocks.join('') || '<p>' + esc(text) + '</p>') +
      '</div>'
    );
  }

  /* Account mark: <span class="kico group account hue-red">S</span>.
     size: 'sm' (26px, default), 'md' (30px square), 'round' (30px), 'big'. */
  function avatar(name, options) {
    options = options || {};
    var size = options.size && options.size !== 'sm' ? ' ' + options.size : '';
    var extra = options.className ? ' ' + options.className : '';
    var tone = options.hue || hue(name);
    return (
      '<span class="kico' +
      size +
      ' group account' +
      extra +
      ' hue-' +
      tone +
      '" aria-hidden="true">' +
      esc(initial(name)) +
      '</span>'
    );
  }

  /* Team mark (GroupMark): a rounded square with the team's initial. */
  function teamMark(team, options) {
    options = options || {};
    var t = typeof team === 'string' ? data.team(team) : team;
    var size = options.size && options.size !== 'sm' ? ' ' + options.size : '';
    var extra = options.className ? ' ' + options.className : '';
    var tone = options.hue || (t && t.hue) || hue(t ? t.name : String(team));
    return (
      '<span class="kico' +
      size +
      ' group' +
      extra +
      ' hue-' +
      tone +
      '" aria-hidden="true">' +
      esc(initial(t ? t.name : team)) +
      '</span>'
    );
  }

  /* The 16px store mark used by the Files tree, location cells and details. */
  function storeMark(vault) {
    var v = typeof vault === 'string' ? data.vault(vault) : vault;
    if (!v) return '';
    return (
      '<span class="smallmark hue-' +
      v.fileHue +
      '" aria-hidden="true">' +
      esc(v.initial) +
      '</span>'
    );
  }

  function message(m, options) {
    options = options || {};
    var you = options.you || data.me.username;
    var own = m.author === you;
    var grouped = !!options.grouped;
    var who = data.person(m.author);
    var parts = [];
    parts.push(
      '<article class="chat-message' +
        (grouped ? ' grouped' : '') +
        '" data-message="' +
        esc(m.id || '') +
        '"' +
        (grouped ? ' title="' + esc(fmt.stamp(m.at)) + '"' : '') +
        '>',
    );
    parts.push(avatar(m.author, { className: 'chat-avatar', hue: who.hue }));
    parts.push('<div class="chat-message-body">');
    parts.push(
      '<header' +
        (grouped ? ' class="offscreen"' : '') +
        '>' +
        '<span class="chat-sender' +
        (own ? ' you' : '') +
        '">' +
        esc(own ? 'You' : m.author) +
        '</span>' +
        '<time datetime="' +
        esc(m.at) +
        '">' +
        esc(fmt.time(m.at)) +
        '</time></header>',
    );
    parts.push(messageText(m.text));
    parts.push('</div></article>');
    return parts.join('');
  }

  /* A message list with day separators, the NEW divider and 5-minute
     grouping, as the app draws it. source: a "team/channel" key or an array.
     options: { you, newFrom, edge (true: "Beginning of the channel") }. */
  function thread(source, options) {
    options = options || {};
    var list =
      typeof source === 'string' ? data.messages[source] || [] : source || [];
    var meta = typeof source === 'string' ? data.channel(source) : null;
    var newFrom =
      options.newFrom !== undefined
        ? options.newFrom
        : meta && meta.channel.newFrom;
    var you = options.you || data.me.username;
    var out = [];
    if (options.edge !== false)
      out.push(
        '<div class="chat-history-edge"><small>Beginning of the channel</small></div>',
      );
    list.forEach(function (m, index) {
      var prev = list[index - 1];
      var newDay = !prev || fmt.dayKey(prev.at) !== fmt.dayKey(m.at);
      var isNew = !!newFrom && m.id === newFrom && m.author !== you;
      var grouped =
        !isNew &&
        !newDay &&
        prev &&
        prev.author === m.author &&
        parse(m.at) - parse(prev.at) <= 5 * 60 * 1000;
      if (newDay)
        out.push(
          '<div class="chat-daysep" role="separator"><span>' +
            esc(fmt.day(m.at)) +
            '</span></div>',
        );
      if (isNew)
        out.push(
          '<div class="chat-divider" role="separator" aria-label="New messages"><span>New</span></div>',
        );
      out.push(message(m, { grouped: grouped, you: you }));
    });
    return out.join('');
  }

  /* A Files list row (All items layout: name, folder path, kind, location,
     size, and the hover actions). options: { selected, location (default
     true), path (default true), contextMenu: menu id }. */
  function itemRow(item, options) {
    options = options || {};
    var it = typeof item === 'string' ? data.item(item) : item;
    var vault = data.vault(it.vault);
    var password = it.kind === 'Password';
    var withLocation = options.location !== false;
    var cls =
      'cols' +
      (withLocation ? ' loc' : '') +
      ' row one' +
      (options.selected ? ' sel' : '');
    return (
      '<div class="' +
      cls +
      '" role="button" tabindex="0" aria-pressed="' +
      (options.selected ? 'true' : 'false') +
      '" data-item="' +
      esc(it.id) +
      '"' +
      (options.contextMenu
        ? ' data-context-menu="' + esc(options.contextMenu) + '"'
        : '') +
      '>' +
      '<span class="name">' +
      icon(password ? 'key-round' : 'file', 0, password ? 'k' : 'f') +
      '<span class="nm" title="' +
      esc(it.name) +
      '">' +
      esc(it.name) +
      '</span>' +
      (options.path === false
        ? ''
        : '<span class="fpath">/' + esc(it.folder) + '</span>') +
      '</span>' +
      '<span class="cell kind">' +
      esc(it.kind) +
      '</span>' +
      (withLocation
        ? '<span class="cell mark">' +
          storeMark(vault) +
          '<span>' +
          esc(vault.name) +
          '</span></span>'
        : '') +
      '<span class="cell num">' +
      esc(it.size) +
      '</span>' +
      '<span class="acts"><button type="button" title="Copy" aria-label="Copy ' +
      esc(it.name) +
      '" data-toast="' +
      (password ? 'Password copied' : 'Copied') +
      '">' +
      icon('copy') +
      '</button>' +
      '<button type="button" title="Reveal" aria-label="Reveal ' +
      esc(it.name) +
      '">' +
      icon('eye') +
      '</button></span></div>'
    );
  }

  /* --------------------------------------------------------------- export */

  window.Kit = {
    version: '1.0.0',
    isMac: IS_MAC,
    icon: icon,
    icons: function () {
      return Object.keys(ICONS).sort();
    },
    registerIcons: registerIcons,
    refresh: refresh,
    setTheme: setTheme,
    theme: function () {
      return themeChoice;
    },
    resolvedTheme: resolvedTheme,
    setState: setState,
    state: function () {
      return currentState;
    },
    setNotes: setNotes,
    toast: toast,
    openMenu: openFloating,
    openPopover: openFloating,
    closeMenu: closeFloating,
    openDialog: openDialog,
    closeDialog: closeDialog,
    setDrawer: function (win, openIt) {
      setDrawer(
        typeof win === 'string' ? doc.getElementById(win) : win,
        openIt,
      );
    },
    on: function (name, handler) {
      doc.addEventListener('kit:' + name, function (event) {
        handler(event.detail);
      });
    },
    data: data,
    hue: hue,
    initial: initial,
    initials: initials,
    esc: esc,
    fmt: fmt,
    render: {
      avatar: avatar,
      teamMark: teamMark,
      storeMark: storeMark,
      message: message,
      messageText: messageText,
      thread: thread,
      itemRow: itemRow,
    },
  };

  if (doc.readyState === 'loading')
    doc.addEventListener('DOMContentLoaded', init);
  else init();
})();
