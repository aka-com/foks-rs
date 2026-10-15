// @ts-check
import { readFileSync } from 'node:fs';
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

/**
 * The desktop's import layers, from apps/desktop/layers.json: each layer's
 * folders may not import from the folders it forbids. A relative specifier is
 * matched when the forbidden folder follows its leading `../` segments, so
 * `../../screens/x` from src/data/ is reported and `../chat-contract` is not.
 * apps/desktop/tests/module-boundaries.test.ts checks the same table against
 * resolved paths, type positions included.
 *
 * @type {{ layers: { description: string, paths: string[], forbid: string[], except?: string[] }[] }}
 */
const desktopLayers = JSON.parse(
  readFileSync(new URL('./apps/desktop/layers.json', import.meta.url), 'utf8'),
);

/** @param {string} text */
const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** @param {(typeof desktopLayers.layers)[number]} layer */
function desktopLayerRule(layer) {
  const folders = layer.forbid.map((path) => {
    const allowed = (layer.except ?? [])
      .filter((file) => file.startsWith(`${path}/`))
      .map((file) =>
        escapeRegExp(file.slice(path.length + 1).replace(/\.tsx?$/, '')),
      );
    const name = escapeRegExp(path.split('/').pop() ?? path);
    return allowed.length > 0
      ? `${name}(/(?!(${allowed.join('|')})$)|$)`
      : `${name}(/|$)`;
  });
  return {
    files: layer.paths.map((path) => `apps/desktop/${path}/**/*.{ts,tsx}`),
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: `^(\\.\\./)+(${folders.join('|')})`,
              message: layer.description,
            },
          ],
        },
      ],
    },
  };
}

export default tseslint.config(
  {
    ignores: [
      '**/node_modules/**',
      '**/dist/**',
      'target/**',
      '.claude/worktrees/**',
    ],
  },
  {
    languageOptions: {
      parserOptions: {
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['**/*.mjs'],
    languageOptions: {
      globals: globals.node,
      sourceType: 'module',
    },
  },
  {
    // The review mockup kit is a classic browser script inlined into each page.
    files: ['docs/review/mockups/_kit/kit.js'],
    languageOptions: {
      globals: globals.browser,
      sourceType: 'script',
    },
  },
  ...tseslint.configs.recommendedTypeChecked.map((config) => ({
    ...config,
    files: ['apps/desktop/**/*.{ts,tsx}'],
  })),
  {
    files: ['apps/desktop/**/*.{ts,tsx}'],
    languageOptions: {
      parserOptions: {
        projectService: true,
      },
    },
    plugins: {
      'react-hooks': reactHooks,
    },
    rules: {
      // Preserve the existing hooks checks independently of new compiler rules.
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      '@typescript-eslint/no-floating-promises': [
        'error',
        {
          allowForKnownSafeCalls: [
            {
              from: 'package',
              package: 'node:test',
              name: [
                'test',
                'describe',
                'it',
                'before',
                'after',
                'beforeEach',
                'afterEach',
              ],
            },
          ],
        },
      ],
      '@typescript-eslint/only-throw-error': 'off',
      '@typescript-eslint/require-await': 'off',
      '@typescript-eslint/no-base-to-string': 'off',
    },
  },
  {
    // Makes a desktop view module's growth past 800 lines visible, so that it
    // is split into model, sheet and view modules. The files already over the
    // limit warn until they are split.
    files: ['apps/desktop/src/**/*.tsx'],
    rules: {
      'max-lines': ['warn', { max: 800 }],
    },
  },
  {
    // Chat hooks and screens neither read nor write refs while rendering.
    // The rest of the desktop source is not yet held to this rule.
    files: [
      'apps/desktop/src/chat/**/*.{ts,tsx}',
      'apps/desktop/src/screens/chat-*.{ts,tsx}',
    ],
    rules: {
      'react-hooks/refs': 'error',
    },
  },
  ...desktopLayers.layers.map(desktopLayerRule),
);
