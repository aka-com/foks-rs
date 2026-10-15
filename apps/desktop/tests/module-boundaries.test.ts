import assert from 'node:assert/strict';
import test from 'node:test';
import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

import layering from '../layers.json';

const root = fileURLToPath(new URL('../src/', import.meta.url));
async function sources(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(
    entries.map((entry) =>
      entry.isDirectory()
        ? sources(join(directory, entry.name))
        : Promise.resolve(
            /\.tsx?$/.test(entry.name) ? [join(directory, entry.name)] : [],
          ),
    ),
  );
  return nested.flat();
}
async function imports(path: string): Promise<string[]> {
  const source = ts.createSourceFile(
    path,
    await readFile(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
  );
  return source.statements.flatMap((statement) => {
    if (ts.isImportDeclaration(statement)) {
      const clause = statement.importClause;
      if (
        clause?.isTypeOnly ||
        (clause?.namedBindings &&
          ts.isNamedImports(clause.namedBindings) &&
          clause.namedBindings.elements.length > 0 &&
          clause.namedBindings.elements.every((entry) => entry.isTypeOnly))
      )
        return [];
      return ts.isStringLiteral(statement.moduleSpecifier)
        ? [statement.moduleSpecifier.text]
        : [];
    }
    if (
      ts.isExportDeclaration(statement) &&
      !statement.isTypeOnly &&
      statement.moduleSpecifier &&
      ts.isStringLiteral(statement.moduleSpecifier)
    ) {
      if (
        statement.exportClause &&
        ts.isNamedExports(statement.exportClause) &&
        statement.exportClause.elements.length > 0 &&
        statement.exportClause.elements.every((entry) => entry.isTypeOnly)
      )
        return [];
      return [statement.moduleSpecifier.text];
    }
    return [];
  });
}

/**
 * Every module a file names, including type-only imports and re-exports,
 * `import('...')` types and dynamic `import()` calls.
 */
async function references(path: string): Promise<string[]> {
  const source = ts.createSourceFile(
    path,
    await readFile(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
  );
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier &&
      ts.isStringLiteral(node.moduleSpecifier)
    )
      found.push(node.moduleSpecifier.text);
    else if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments[0] &&
      ts.isStringLiteral(node.arguments[0])
    )
      found.push(node.arguments[0].text);
    else if (
      ts.isImportTypeNode(node) &&
      ts.isLiteralTypeNode(node.argument) &&
      ts.isStringLiteral(node.argument.literal)
    )
      found.push(node.argument.literal.text);
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

/**
 * The file a relative specifier names, as the bundler resolves it: the exact
 * path, then with a `.ts` or `.tsx` extension, then the folder's `index.ts`.
 * Package specifiers resolve to nothing.
 */
function resolveSpecifier(
  file: string,
  specifier: string,
  known: ReadonlySet<string>,
): string | undefined {
  if (!specifier.startsWith('.')) return undefined;
  const base = resolve(dirname(file), specifier);
  return [base, `${base}.ts`, `${base}.tsx`, join(base, 'index.ts')].find(
    (candidate) => known.has(candidate),
  );
}

/** The compatibility facade each folder has beside it at the src/ root. */
const FACADES: Readonly<Record<string, string>> = {
  bridge: 'bridge.ts',
  navigation: 'location.ts',
};

/**
 * Runtime import cycles that exist and are left for the chat restructuring:
 * the inbox provider renders the unread badge and the channel creation
 * provider, which read the inbox through the provider's own hooks.
 */
const KNOWN_CYCLE_EDGES = new Set(
  [
    'chat/channel-creation-provider.tsx -> chat/inbox-provider.tsx',
    'chat/unread-badge.tsx -> chat/inbox-provider.tsx',
  ].map((edge) =>
    edge
      .split(' -> ')
      .map((path) => join(root, path))
      .join(' -> '),
  ),
);

const folders = (await readdir(root, { withFileTypes: true }))
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name);

for (const domain of folders)
  test(`${domain} internal modules have no runtime cycles or imports of their public facade or barrel`, async () => {
    const files = await sources(join(root, domain));
    const barrel = join(root, domain, 'index.ts');
    const facade = FACADES[domain] && join(root, FACADES[domain]);
    const known = new Set(facade ? [...files, facade] : files);
    const edges = new Map<string, string[]>();
    for (const file of files) {
      const dependencies = (await imports(file)).flatMap((specifier) => {
        const target = resolveSpecifier(file, specifier, known);
        return target ? [target] : [];
      });
      if (file !== barrel) {
        assert.ok(
          !facade || !dependencies.includes(facade),
          `${file} imports its compatibility facade`,
        );
        assert.ok(
          !dependencies.includes(barrel),
          `${file} imports its public barrel`,
        );
      }
      edges.set(file, dependencies);
    }
    const done = new Set<string>();
    const visit = (file: string, stack: string[]) => {
      assert.ok(
        !stack.includes(file),
        `Runtime import cycle: ${[...stack, file].join(' -> ')}`,
      );
      if (done.has(file)) return;
      for (const dependency of edges.get(file) ?? [])
        if (!KNOWN_CYCLE_EDGES.has(`${file} -> ${dependency}`))
          visit(dependency, [...stack, file]);
      done.add(file);
    };
    for (const file of files) visit(file, []);
  });

/**
 * Imports that layers.json forbids and that exist by design: the bridge
 * selection loads the mock bridge by dynamic import, which
 * react-boundary.test.ts requires to stay dynamic.
 */
const LAYER_EXCEPTION_EDGES = new Set(
  ['bridge/selection.ts -> mock/mock-bridge.ts'].map((edge) =>
    edge
      .split(' -> ')
      .map((path) => join(root, path))
      .join(' -> '),
  ),
);

test('no layer names a module in a folder layers.json forbids it', async () => {
  // eslint.config.mjs matches the same table against static import
  // specifiers. This resolves each specifier to a file, so any spelling of a
  // path into a forbidden folder is caught, and it covers `import('...')`
  // types and dynamic `import()` calls.
  const app = fileURLToPath(new URL('../', import.meta.url));
  const files = [
    ...(await sources(root)),
    ...(await sources(join(app, 'kit'))),
  ];
  const known = new Set(files);
  const under = (file: string, path: string) =>
    file.startsWith(join(app, path) + sep);
  const problems: string[] = [];
  for (const layer of layering.layers) {
    const except = new Set((layer.except ?? []).map((path) => join(app, path)));
    for (const file of files)
      if (layer.paths.some((path) => under(file, path)))
        for (const specifier of await references(file)) {
          const target = resolveSpecifier(file, specifier, known);
          if (
            target &&
            !except.has(target) &&
            !LAYER_EXCEPTION_EDGES.has(`${file} -> ${target}`) &&
            layer.forbid.some((path) => under(target, path))
          )
            problems.push(
              `${relative(app, file)} imports ${relative(app, target)}: ${layer.description}`,
            );
        }
  }
  assert.deepEqual(problems, []);
});

test('metadata resource services do not call secret-bearing or mutation bridge APIs', async () => {
  const files = await sources(join(root, 'resources'));
  const forbidden = new Set([
    'readItem',
    'copyItem',
    'createTextItem',
    'editTextItem',
    'prepareOwnerBackup',
    'commitOwnerBackup',
    'verifyRecoveryPhrase',
    'runYubi',
    'setPassphrase',
    'changePassphrase',
  ]);
  for (const file of files) {
    const source = ts.createSourceFile(
      file,
      await readFile(file, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
    );
    const visit = (node: ts.Node) => {
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression)
      )
        assert.ok(
          !forbidden.has(node.expression.name.text),
          `${file} invokes ${node.expression.name.text}`,
        );
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
});

test('every chat module is reachable from the application entry point', async () => {
  // A module that only tests import is not what the application runs.
  const files = await sources(root);
  const known = new Set(files);
  const reached = new Set<string>();
  const pending = [join(root, 'main.tsx')];
  for (let file = pending.pop(); file; file = pending.pop()) {
    if (reached.has(file)) continue;
    reached.add(file);
    for (const specifier of await references(file)) {
      if (!specifier.startsWith('.')) continue;
      const base = resolve(dirname(file), specifier);
      const target = [
        base,
        `${base}.ts`,
        `${base}.tsx`,
        join(base, 'index.ts'),
      ].find((candidate) => known.has(candidate));
      if (target) pending.push(target);
    }
  }
  assert.deepEqual(
    (await sources(join(root, 'chat'))).filter((file) => !reached.has(file)),
    [],
  );
});

test('the startup module graph statically imports no screen the shell loads on demand', async () => {
  // src/app/deferred-screens.tsx loads each of these by dynamic import, so the
  // build puts it in a chunk of its own. A static import of one from anywhere
  // the entry point reaches puts it, and what it imports, back in the
  // startup chunk without failing the build.
  const files = await sources(root);
  const known = new Set(files);
  const deferredScreens = join(root, 'app', 'deferred-screens.tsx');
  const resolved = async (
    file: string,
    specifiers: Promise<string[]>,
  ): Promise<string[]> =>
    (await specifiers).flatMap((specifier) => {
      const target = resolveSpecifier(file, specifier, known);
      return target ? [target] : [];
    });
  const staticTargets = new Set(
    await resolved(deferredScreens, imports(deferredScreens)),
  );
  const deferred = (
    await resolved(deferredScreens, references(deferredScreens))
  ).filter((target) => !staticTargets.has(target));
  assert.equal(deferred.length, 6, 'deferred-screens.tsx loads six screens');
  const reached = new Set<string>();
  const pending = [join(root, 'main.tsx')];
  for (let file = pending.pop(); file; file = pending.pop()) {
    if (reached.has(file)) continue;
    reached.add(file);
    pending.push(...(await resolved(file, imports(file))));
  }
  assert.deepEqual(
    deferred
      .filter((file) => reached.has(file))
      .map((file) => relative(root, file)),
    [],
  );
});

test('application modules name other modules by relative path, so node loads them without Vite', async () => {
  const kit = fileURLToPath(new URL('../kit/', import.meta.url));
  for (const file of [...(await sources(root)), ...(await sources(kit))])
    for (const specifier of await references(file))
      assert.ok(
        !specifier.startsWith('/'),
        `${file} imports the root-absolute path ${specifier}`,
      );
});

test('tests import application modules directly; only the entry-point test starts Vite', async () => {
  // A module loaded through `vite.ssrLoadModule` is typed only by a cast that
  // can name a different path, and is a second copy of any module the test
  // also imports. `app-root.render.test.tsx` boots `src/main.tsx`, which
  // imports the stylesheet, so it alone still needs Vite.
  const tests = fileURLToPath(new URL('./', import.meta.url));
  const users: string[] = [];
  for (const file of await sources(tests))
    if ((await references(file)).includes('vite'))
      users.push(file.slice(tests.length));
  assert.deepEqual(users, ['app-root.render.test.tsx']);
});

/** The DOM globals and timers that `installDom` defines or replaces. */
const DOM_GLOBALS = new Set([
  'window',
  'document',
  'navigator',
  'localStorage',
  'sessionStorage',
  'matchMedia',
  'getComputedStyle',
  'requestAnimationFrame',
  'setTimeout',
  'setInterval',
]);

/**
 * Each place a module names a DOM global or timer in code that runs when the
 * module is evaluated: outside function bodies, types and instance members.
 * A call made at evaluation into a function that names one is not found.
 */
async function evaluationDomReads(path: string): Promise<string[]> {
  const source = ts.createSourceFile(
    path,
    await readFile(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
  );
  const found: string[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isFunctionLike(node) ||
      ts.isTypeNode(node) ||
      ts.isInterfaceDeclaration(node) ||
      ts.isTypeAliasDeclaration(node) ||
      ts.isModuleDeclaration(node) ||
      (ts.isPropertyDeclaration(node) &&
        !node.modifiers?.some(
          (modifier) => modifier.kind === ts.SyntaxKind.StaticKeyword,
        ))
    )
      return;
    if (ts.isIdentifier(node) && DOM_GLOBALS.has(node.text)) {
      const parent = node.parent;
      const named =
        (ts.isPropertyAccessExpression(parent) && parent.name === node) ||
        ((ts.isPropertyAssignment(parent) ||
          ts.isPropertyDeclaration(parent)) &&
          parent.name === node) ||
        ts.isImportSpecifier(parent);
      if (!named)
        found.push(
          `${node.text} at line ${source.getLineAndCharacterOfPosition(node.getStart()).line + 1}`,
        );
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

test('tests that install the DOM import statically no module that reads it at evaluation', async () => {
  // A test file's static imports are evaluated before its first statement, so
  // a module they reach runs before `installDom`, and a later `await import()`
  // of it returns that instance. Such a module is loaded by `await import()`
  // after `installDom` instead.
  const app = fileURLToPath(new URL('../', import.meta.url));
  const tests = fileURLToPath(new URL('./', import.meta.url));
  const testFiles = await sources(tests);
  const known = new Set([
    ...(await sources(root)),
    ...(await sources(join(app, 'kit'))),
    ...testFiles,
  ]);
  const problems: string[] = [];
  for (const file of testFiles) {
    if (
      !/\.test\.tsx?$/.test(file) ||
      !(await readFile(file, 'utf8')).includes('installDom(')
    )
      continue;
    const reached = new Set<string>();
    const pending = [[file]];
    for (let chain = pending.pop(); chain; chain = pending.pop()) {
      const module = chain[chain.length - 1];
      if (reached.has(module)) continue;
      reached.add(module);
      const reads = await evaluationDomReads(module);
      if (reads.length > 0)
        problems.push(
          `${chain.map((path) => relative(app, path)).join(' -> ')} names ${reads.join(', ')}`,
        );
      for (const specifier of await imports(module)) {
        const target = resolveSpecifier(module, specifier, known);
        if (target) pending.push([...chain, target]);
      }
    }
  }
  assert.deepEqual(problems, []);
});

test('screen and shell model modules load without React or a view module', async () => {
  // These hold the pure logic split out of view modules, so they can be
  // unit-tested in node and read without the views around them.
  const models = [
    'shell/sync-summary.ts',
    'screens/devices/credential-actions.ts',
    'model/group-model.ts',
    'screens/servers/server-model.ts',
    'screens/files/write-workflow-model.ts',
    'screens/details-model.ts',
  ].map((path) => join(root, path));
  const files = new Set(await sources(root));
  const reached = new Set<string>();
  const pending = models.map((model) => [model]);
  for (let chain = pending.pop(); chain; chain = pending.pop()) {
    const file = chain[chain.length - 1];
    if (reached.has(file)) continue;
    reached.add(file);
    assert.ok(
      !file.endsWith('.tsx'),
      `A model module loads a view: ${chain.join(' -> ')}`,
    );
    for (const specifier of await imports(file)) {
      assert.ok(
        specifier !== 'react' && !specifier.startsWith('react-dom'),
        `${chain.join(' -> ')} imports ${specifier}`,
      );
      if (!specifier.startsWith('.')) continue;
      const base = resolve(dirname(file), specifier);
      const target = [`${base}.ts`, `${base}.tsx`, join(base, 'index.ts')].find(
        (candidate) => files.has(candidate),
      );
      if (target) pending.push([...chain, target]);
    }
  }
});
