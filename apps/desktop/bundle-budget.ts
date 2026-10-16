/**
 * The production build's startup JavaScript budget.
 *
 * The startup JavaScript is the entry chunk and every chunk it imports
 * statically: what the webview fetches, parses and evaluates before React
 * replaces the loading placeholder in index.html. The screens in
 * src/app/deferred-screens.tsx load later, in chunks of their own, and do
 * not count. A build whose startup JavaScript exceeds the budget fails, so
 * growth is a decision recorded here rather than a side effect of an import.
 * The budget does not measure the deferred chunks: vite.config.ts declares the
 * feature barrels free of side effects so that one screen's chunk does not
 * load the modules another feature's barrel re-exports for other screens.
 */

import type { Plugin } from 'vite';

/**
 * The budget in bytes of minified JavaScript. The startup JavaScript was about
 * 880 kB when the budget was set; before the deferred screens were split out,
 * one 1,332 kB chunk held the whole app.
 */
export const STARTUP_BUDGET_BYTES = 960_000;

/** The part of a Rollup/Rolldown output bundle the budget reads. */
export type BundleEntries = Readonly<
  Record<
    string,
    | {
        type: 'chunk';
        isEntry: boolean;
        imports: readonly string[];
        code: string;
      }
    | { type: 'asset' }
  >
>;

/**
 * The file names of the startup chunks, and their total size in bytes: every
 * entry chunk and the chunks reachable from one through static imports.
 */
export function startupJavaScript(bundle: BundleEntries): {
  files: string[];
  bytes: number;
} {
  const pending = Object.entries(bundle)
    .filter(([, output]) => output.type === 'chunk' && output.isEntry)
    .map(([fileName]) => fileName);
  const files: string[] = [];
  let bytes = 0;
  for (let fileName = pending.pop(); fileName; fileName = pending.pop()) {
    const output = bundle[fileName];
    if (files.includes(fileName) || output?.type !== 'chunk') continue;
    files.push(fileName);
    bytes += Buffer.byteLength(output.code);
    pending.push(...output.imports);
  }
  return { files: files.sort(), bytes };
}

/** Fails a production build whose startup JavaScript exceeds `budget`. */
export function startupBudget(budget = STARTUP_BUDGET_BYTES): Plugin {
  return {
    name: 'foks-startup-budget',
    apply: 'build',
    generateBundle(_options, bundle) {
      const { files, bytes } = startupJavaScript(bundle);
      const summary = `startup JavaScript is ${bytes} bytes in ${files.length} chunks; the budget is ${budget} bytes`;
      if (bytes > budget)
        this.error(
          `${summary}. Load the new code from a deferred screen, or raise STARTUP_BUDGET_BYTES in apps/desktop/bundle-budget.ts.`,
        );
      this.info(summary);
    },
  };
}
