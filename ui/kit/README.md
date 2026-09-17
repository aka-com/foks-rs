# `ui/kit` — the pieces AKA and FOKS actually share

Two Tauri apps live in this repository: `aka-desktop` (`ui/`, `src-tauri/`) and
`foks-desktop` (`foks-ui/`, `foks-tauri/`). They share the Vite/React
toolchain, the jsdom test harness, and this directory — nothing else.

Everything here is **app-neutral**: it imports React (and, for `icon.tsx`,
`lucide`'s types) and nothing from `ui/src/`. That is the admission rule. A
module that needs AKA's action vocabulary, state, or bridge is not kit.

## What moved here

| File | Depends on | Why it qualifies |
| --- | --- | --- |
| `virtual-list.ts` | nothing | Pure windowing arithmetic, no DOM, no React. |
| `menu-position.ts` | nothing | Pure anchored-menu geometry. |
| `toasts.tsx` | `react` | Controller + provider; no app vocabulary. |
| `icon.tsx` | `react`, `lucide` (types only) | `AppIcon` renders an `IconDefinition`; the definitions themselves stay per-app. |
| `overlay-primitives.tsx` | `react`, `react-dom`, `./menu-position` | `OverlayProvider`, `Dialog`, `Menu`, `Listbox`, `Popover`, `ContextMenu`. |
| `tokens.css` | — | See below. |

`menu-position.ts` was not on the original extraction list; it came along
because `overlay-primitives.tsx` stands on it and dragging `ui/src/` into the
kit would have broken the admission rule in the other direction.

## What deliberately did **not** move

- **`ui/src/sheet.tsx`.** It imports `ActionName` from `ui/src/actions.ts` (a
  290-line, AKA-specific click vocabulary that the compiler enforces through a
  `react` module augmentation) and `ActionDiv` from `ui/src/action-controls.tsx`.
  Cutting that is a refactor of AKA's typed-action contract, not a file move, so
  `Sheet` stays in `ui/src`. FOKS builds its sheets directly on the kit's
  `Dialog`. Revisit if the action vocabulary is ever parameterised.

## Old paths still work

Each moved file left a one-line re-export shim behind at its old
`ui/src/<name>` path:

```ts
export * from '../kit/overlay-primitives';
```

So no AKA import changed, and `ui/BUILD.bazel`'s `src/**` globs still see a
file at every path they saw one before. The Bazel globs were widened to
`kit/**` as well (`UI_SRCS`, `_TEST_DATA`) so the bundle, the dev server, the
type check and the tests reach the real sources. The shims are a migration
convenience, not a permanent seam: an AKA import may be repointed at
`/kit/<name>` at any time, and the shim deleted once the last one is.

`ui/tests/react-boundary.test.ts` reads sources directly rather than importing
them, so its recursive scan now covers `ui/kit/` too and its
`overlay-primitives` assertions read the kit path.

## Tokens, and the theme decision

`tokens.css` holds the **35** custom properties that `ui/styles.css` `:root`
and `dev/foks-desktop/iteration/wave6/shell.css` `:root` already declare under
the same name *and* with the same value. That number is measured, not
inherited from the plan: the mock declares 48 `:root` tokens, of which

- **35 are identical** — the contents of this file;
- **5 share a name but diverge in value** — `--faint` (`#707078` / `#8a8a92`),
  `--surface` (`rgba(246,246,249,.98)` / `#f6f6f9`), `--main-surface`
  (`rgba(249,249,252,.98)` / `#fff`), `--hover` (`rgba(0,0,0,.06)` /
  `rgba(0,0,0,.05)`), `--shadow-menu` (`…,.12)` / `…,.14)`);
- **8 are FOKS-only** — `--sans`, `--mono`, and the six kind tints
  `--c-password`, `--c-file`, `--c-resource`, `--c-link`, `--c-team`,
  `--c-none`.

Those thirteen are declared in `foks-ui/src/styles/shell.css`, after its
`@import` of this file, so the FOKS values win by cascade order. Nothing is
defined twice with two different values inside one app.

**Theme: forked at this seam.** `tokens.css` carries **light values only**.
FOKS is light-only in Phase 1. AKA keeps its `:root[data-theme="dark"]`
override block in `ui/styles.css`, stamped by `ui/public/theme.js` before
first paint. A dark block here would either give FOKS a theme it does not
have or make AKA's override depend on a file it does not load — see below.

**AKA does not import `tokens.css` yet.** `ui/styles.css` still declares its
own copy of all 35, and this was left alone on purpose: `ui/styles.css` is the
single stylesheet the Bazel `filegroup`/`copy_to_directory` rules ship flat,
and adding a cross-directory `@import` to it changes what has to be packaged.
Reconciling the two — AKA importing this file and deleting its duplicates — is
a **follow-up**, not part of this change. Until then, a token value edited here
must be edited in `ui/styles.css` too; the 35 names above are the list to check.
