# `ui/kit` — the pieces AKA and FOKS actually share

Two Tauri apps live in this repository: `aka-desktop` (`ui/`, `src-tauri/`) and
`foks-desktop` (`foks-ui/`, `foks-tauri/`). They share the Vite/React
toolchain, the jsdom test harness, and this directory — nothing else.

Everything here is **app-neutral**: it imports React (and, for `icon.tsx`,
`lucide`'s types) and nothing from `ui/src/`. That is the admission rule. A
module that needs AKA's action vocabulary, state, or bridge is not kit.

## What moved here

| File                     | Depends on                              | Why it qualifies                                                                |
| ------------------------ | --------------------------------------- | ------------------------------------------------------------------------------- |
| `virtual-list.ts`        | nothing                                 | Pure windowing arithmetic, no DOM, no React.                                    |
| `menu-position.ts`       | nothing                                 | Pure anchored-menu geometry.                                                    |
| `toasts.tsx`             | `react`                                 | Controller + provider; no app vocabulary.                                       |
| `icon.tsx`               | `react`, `lucide` (types only)          | `AppIcon` renders an `IconDefinition`; the definitions themselves stay per-app. |
| `overlay-primitives.tsx` | `react`, `react-dom`, `./menu-position` | `OverlayProvider`, `Dialog`, `Menu`, `Listbox`, `Popover`, `ContextMenu`.       |
| `tokens.css`             | —                                       | See below.                                                                      |

`menu-position.ts` provides viewport-aware anchor positioning calculations used by `overlay-primitives.tsx`.

## What deliberately did **not** move

- **`ui/src/sheet.tsx`:** Retained in `ui/src` due to tight coupling with `ActionName` and `ActionDiv`. Use `Dialog` from the kit for generic overlays.

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
the same name _and_ with the same value. That number is measured, not
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

**Theming:** `tokens.css` defines base light theme tokens. Dark theme overrides are managed in `ui/styles.css` via `:root[data-theme="dark"]`.

**Token Synchronization:** Shared tokens are imported by `ui/styles.css` via `@import './kit/tokens.css';`. Keep definitions unified in `tokens.css`.
