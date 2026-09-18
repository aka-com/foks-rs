import assert from 'node:assert/strict';
import test from 'node:test';
import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

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

for (const domain of ['bridge', 'navigation'])
  test(`${domain} internal modules have no runtime cycles or imports of their public facade`, async () => {
    const files = await sources(join(root, domain));
    const facade = join(
      root,
      `${domain === 'navigation' ? 'location' : domain}.ts`,
    );
    const known = new Set([...files, facade]);
    const edges = new Map<string, string[]>();
    for (const file of files) {
      const dependencies = (await imports(file)).flatMap((specifier) => {
        if (!specifier.startsWith('.')) return [];
        const base = resolve(dirname(file), specifier);
        const target = [
          `${base}.ts`,
          `${base}.tsx`,
          join(base, 'index.ts'),
        ].find((candidate) => known.has(candidate));
        return target ? [target] : [];
      });
      if (file !== join(root, domain, 'index.ts')) {
        assert.ok(
          !dependencies.includes(facade),
          `${file} imports its compatibility facade`,
        );
        assert.ok(
          !dependencies.includes(join(root, domain, 'index.ts')),
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
        visit(dependency, [...stack, file]);
      done.add(file);
    };
    for (const file of files) visit(file, []);
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
