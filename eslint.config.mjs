// @ts-check
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

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
);
