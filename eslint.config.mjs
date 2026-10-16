// @ts-check
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';

/**
 * The desktop's import layers, from apps/desktop/layers.json: each layer's
 * folders may not import from the folders it forbids. A relative specifier is
 * matched when the forbidden folder follows its leading `../` segments, so
 * `../../features/x` from src/data/ is reported and `../chat-contract` is not.
 * Its `features` entry names the folder whose subfolders are features: a
 * module outside a feature, other than in the composition root, names the
 * feature by its folder alone, so the import resolves to the feature's
 * index.ts. apps/desktop/tests/module-boundaries.test.ts checks the same
 * table against resolved paths, type positions included.
 *
 * @type {{
 *   features: { path: string, composer: string, description: string },
 *   layers: { description: string, paths: string[], forbid: string[], except?: string[] }[],
 * }}
 */
const desktopLayers = JSON.parse(
  readFileSync(new URL('./apps/desktop/layers.json', import.meta.url), 'utf8'),
);

/** @param {string} text */
const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const featuresPath = desktopLayers.features.path;
const featuresName = escapeRegExp(featuresPath.split('/').pop() ?? '');

/**
 * Each folder under the features folder, as its path segments below it; the
 * first segment is the feature.
 *
 * @returns {string[][]}
 */
function featureFolders() {
  /** @type {string[][]} */
  const found = [];
  /** @param {string[]} segments */
  const visit = (segments) => {
    const url = new URL(
      ['./apps/desktop', featuresPath, ...segments, ''].join('/'),
      import.meta.url,
    );
    if (!existsSync(url)) return;
    for (const entry of readdirSync(url, { withFileTypes: true }))
      if (entry.isDirectory()) {
        found.push([...segments, entry.name]);
        visit([...segments, entry.name]);
      }
  };
  visit([]);
  return found;
}

/**
 * The patterns that keep a layer's folders out of the folders it forbids, and
 * out of any feature's own modules unless the layer is the composition root
 * or forbids the features folder outright.
 *
 * @param {(typeof desktopLayers.layers)[number]} layer
 */
function desktopLayerPatterns(layer) {
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
  const patterns = [
    {
      regex: `^(\\.\\./)+(${folders.join('|')})`,
      message: layer.description,
    },
  ];
  if (
    !layer.paths.includes(desktopLayers.features.composer) &&
    !layer.forbid.includes(featuresPath)
  )
    patterns.push({
      regex: `^(\\.\\./)+${featuresName}/[^/]+/`,
      message: desktopLayers.features.description,
    });
  return patterns;
}

/** @param {(typeof desktopLayers.layers)[number]} layer */
function desktopLayerRule(layer) {
  return {
    files: layer.paths.map((path) => `apps/desktop/${path}/**/*.{ts,tsx}`),
    rules: {
      'no-restricted-imports': [
        'error',
        { patterns: desktopLayerPatterns(layer) },
      ],
    },
  };
}

/**
 * The rules for the modules directly in one folder under the features folder:
 * its layer's patterns, which a later block would otherwise replace, and one
 * that reports a path into another feature's modules. The pattern counts the
 * folder's depth, so `../files/store-access` from a feature's top folder is
 * reported, `../files` is not, and a subfolder that shares a feature's name
 * is not mistaken for it.
 *
 * @param {string[]} segments
 * @param {string[]} features
 */
function desktopFeatureRule(segments, features) {
  const layerPatterns = desktopLayers.layers
    .filter((layer) => layer.paths.includes(featuresPath))
    .flatMap(desktopLayerPatterns);
  const others = features
    .filter((feature) => feature !== segments[0])
    .map(escapeRegExp);
  // The feature's own folder, named relative to this one: `.` at the top,
  // `..` one level down, and by name from the features folder.
  const ownRoot =
    segments.length === 1 ? '\\.' : `\\.\\.(/\\.\\.){${segments.length - 2}}`;
  const ownName = `(\\.\\./){${segments.length}}${escapeRegExp(segments[0])}`;
  return {
    files: [`apps/desktop/${featuresPath}/${segments.join('/')}/*.{ts,tsx}`],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            ...layerPatterns,
            ...(others.length > 0
              ? [
                  {
                    regex: `^(\\.\\./){${segments.length}}(${others.join('|')})/`,
                    message: desktopLayers.features.description,
                  },
                ]
              : []),
            {
              regex: `^(${ownRoot}|${ownName})(/(index(\\.ts)?)?)?$`,
              message:
                "A feature's own modules import each other directly, not through its index.ts.",
            },
          ],
        },
      ],
    },
  };
}

const desktopFeatureFolders = featureFolders();
const desktopFeatures = desktopFeatureFolders
  .filter((segments) => segments.length === 1)
  .map(([feature]) => feature);

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
      'apps/desktop/src/features/chat/**/*.{ts,tsx}',
    ],
    rules: {
      'react-hooks/refs': 'error',
    },
  },
  ...desktopLayers.layers.map(desktopLayerRule),
  {
    // The modules at the root of src/ belong to no layer, and reach a feature
    // only through its index.ts.
    files: ['apps/desktop/src/*.{ts,tsx}'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              regex: `^\\./${featuresName}/[^/]+/`,
              message: desktopLayers.features.description,
            },
          ],
        },
      ],
    },
  },
  ...desktopFeatureFolders.map((segments) =>
    desktopFeatureRule(segments, desktopFeatures),
  ),
);
