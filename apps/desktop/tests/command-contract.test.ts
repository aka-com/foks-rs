import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

// Every native command call in src/ is checked against the command names and
// argument keys in src-tauri/command-signatures.json, which a Rust test derives
// from the #[tauri::command] functions that generate_handler! registers. Tauri
// reads each argument from the camelCase key of its parameter name, so a
// renamed command or parameter otherwise fails only at runtime. A reference to
// invoke or a transport wrapper that is not the callee of a recorded call, such
// as a value passed on or `checked.call(...)`, fails the test. A Bridge method
// that only its wrapper and the mock bridge mention leaves its command
// reachable from the renderer with no feature behind it, which fails too.

type Presence = 'required' | 'optional';
type Keys = Map<string, Presence>;

interface Manifest {
  commands: Record<string, Record<string, Presence>>;
}

interface Site {
  /** Where the call or union member is, for messages. */
  at: string;
  command: string;
  /** The keys the payload has; `optional` when one may be absent. */
  keys: Keys | string;
}

const app = fileURLToPath(new URL('../', import.meta.url));
const source = join(app, 'src');
const transport = join(source, 'bridge', 'transport.ts');
const yubikey = join(source, 'bridge', 'yubikey.ts');
const contract = join(source, 'bridge', 'contract.ts');
const mock = join(source, 'mock');

async function manifest(): Promise<Manifest> {
  return JSON.parse(
    await readFile(
      new URL('../src-tauri/command-signatures.json', import.meta.url),
      'utf8',
    ),
  ) as Manifest;
}

function program(): ts.Program {
  const path = join(app, 'tsconfig.json');
  const config = ts.readConfigFile(path, (name) => ts.sys.readFile(name));
  assert.equal(config.error, undefined);
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, app);
  return ts.createProgram(
    parsed.fileNames.filter((file) => file.startsWith(source)),
    parsed.options,
  );
}

const absent = ts.TypeFlags.Undefined | ts.TypeFlags.Void;
const unchecked = ts.TypeFlags.Any | ts.TypeFlags.Unknown;

/** Whether a property of this type may be dropped from the JSON payload. */
function mayBeAbsent(type: ts.Type): boolean {
  return (type.isUnion() ? type.types : [type]).some(
    (member) => (member.flags & (absent | unchecked)) !== 0,
  );
}

/**
 * The keys an argument object of this type carries. A key is `required` only
 * when every member of a union has it, it is not optional and its value cannot
 * be `undefined`, which JSON serialization drops.
 */
function keysOf(
  checker: ts.TypeChecker,
  type: ts.Type,
  at: ts.Node,
): Keys | string {
  const members = (type.isUnion() ? type.types : [type]).map((member) => {
    const keys: Keys = new Map();
    if (member.flags & absent) return keys;
    if (!(member.flags & ts.TypeFlags.Object))
      return `arguments of type ${checker.typeToString(member)} have no known keys`;
    if (checker.getIndexInfosOfType(member).length > 0)
      return `arguments of type ${checker.typeToString(member)} have an index signature`;
    for (const property of checker.getPropertiesOfType(member))
      keys.set(
        property.name,
        property.flags & ts.SymbolFlags.Optional ||
          mayBeAbsent(checker.getTypeOfSymbolAtLocation(property, at))
          ? 'optional'
          : 'required',
      );
    return keys;
  });
  const failure = members.find((member) => typeof member === 'string');
  if (failure !== undefined) return failure;
  const maps = members as Keys[];
  const merged: Keys = new Map();
  for (const name of new Set(maps.flatMap((keys) => [...keys.keys()])))
    merged.set(
      name,
      maps.every((keys) => keys.get(name) === 'required')
        ? 'required'
        : 'optional',
    );
  return merged;
}

/** Whether the identifier names `invoke` or a transport wrapper around it. */
function isNative(checker: ts.TypeChecker, name: ts.Identifier): boolean {
  let symbol = checker.getSymbolAtLocation(name);
  if (symbol && symbol.flags & ts.SymbolFlags.Alias)
    symbol = checker.getAliasedSymbol(symbol);
  return (symbol?.declarations ?? []).some((declaration) => {
    const file = declaration.getSourceFile().fileName;
    return (
      (symbol?.name === 'invoke' &&
        file.includes('/node_modules/@tauri-apps/api/core')) ||
      (['invoke', 'checked', 'checkedMutation'].includes(symbol?.name ?? '') &&
        file === transport)
    );
  });
}

/**
 * The identifiers through which this callee calls `invoke` or a transport
 * wrapper: a name, a property such as `core.invoke`, or either branch of a
 * conditional.
 */
function nativeCallees(
  checker: ts.TypeChecker,
  callee: ts.Expression,
): ts.Identifier[] {
  let expression = callee;
  while (ts.isParenthesizedExpression(expression))
    expression = expression.expression;
  if (ts.isConditionalExpression(expression))
    return [
      ...nativeCallees(checker, expression.whenTrue),
      ...nativeCallees(checker, expression.whenFalse),
    ];
  const name = ts.isPropertyAccessExpression(expression)
    ? expression.name
    : expression;
  return ts.isIdentifier(name) && isNative(checker, name) ? [name] : [];
}

/**
 * Whether the identifier declares, imports or exports a name, including a
 * member of the transport interfaces or of the object that implements them.
 */
function isBinding(name: ts.Identifier): boolean {
  const parent = name.parent;
  return (
    ts.isImportSpecifier(parent) ||
    ts.isExportSpecifier(parent) ||
    ((ts.isFunctionDeclaration(parent) ||
      ts.isPropertySignature(parent) ||
      ts.isPropertyAssignment(parent)) &&
      parent.name === name)
  );
}

function location(node: ts.Node): string {
  const file = node.getSourceFile();
  const { line } = file.getLineAndCharacterOfPosition(node.getStart());
  return `${relative(source, file.fileName)}:${line + 1}`;
}

/**
 * The native calls in src/: those that name their command with a literal, and
 * the files of those that pass a command chosen at runtime. `unchecked` lists
 * the other references to `invoke` and the transport wrappers.
 */
function callSites(compiled: ts.Program): {
  literal: Site[];
  dynamic: string[];
  yubi: Site[];
  unchecked: string[];
} {
  const checker = compiled.getTypeChecker();
  const literal: Site[] = [];
  const dynamic: string[] = [];
  const unchecked: string[] = [];
  const callees = new Set<ts.Node>();
  for (const file of compiled.getSourceFiles()) {
    if (!file.fileName.startsWith(source)) continue;
    const visit = (node: ts.Node): void => {
      if (
        ts.isIdentifier(node) &&
        !callees.has(node) &&
        !isBinding(node) &&
        isNative(checker, node)
      )
        unchecked.push(`${location(node)}: ${node.text}`);
      const native = ts.isCallExpression(node)
        ? nativeCallees(checker, node.expression)
        : [];
      if (ts.isCallExpression(node) && native.length > 0) {
        for (const callee of native) callees.add(callee);
        const [name, args] = node.arguments;
        if (name && ts.isStringLiteralLike(name))
          literal.push({
            at: location(node),
            command: name.text,
            keys: args
              ? keysOf(checker, checker.getTypeAtLocation(args), args)
              : new Map(),
          });
        else dynamic.push(relative(source, file.fileName));
      }
      ts.forEachChild(node, visit);
    };
    visit(file);
  }

  // runYubi passes `command` and `args` from one member of this union.
  const declarations = compiled.getSourceFile(yubikey)!.statements;
  const alias = declarations.find(
    (statement): statement is ts.TypeAliasDeclaration =>
      ts.isTypeAliasDeclaration(statement) &&
      statement.name.text === 'YubiCommand',
  );
  assert.ok(alias, 'bridge/yubikey.ts declares YubiCommand');
  const union = checker.getTypeAtLocation(alias.name);
  assert.ok(union.isUnion());
  const yubi = union.types.flatMap((member) => {
    const property = (name: string) => {
      const symbol = checker.getPropertyOfType(member, name);
      assert.ok(symbol, `YubiCommand members have ${name}`);
      return checker.getTypeOfSymbolAtLocation(symbol, alias);
    };
    const commands = property('command');
    const keys = keysOf(checker, property('args'), alias);
    return (commands.isUnion() ? commands.types : [commands]).map((command) => {
      assert.ok(command.isStringLiteral(), 'YubiCommand commands are literals');
      return {
        at: `YubiCommand ${command.value}`,
        command: command.value,
        keys,
      };
    });
  });
  return { literal, dynamic: dynamic.sort(), yubi, unchecked };
}

/** Every disagreement between the call sites and the manifest. */
function problems(sites: Site[], { commands }: Manifest): string[] {
  return sites.flatMap(({ at, command, keys }) => {
    const signature = commands[command];
    if (!signature) return [`${at}: no command ${command}`];
    if (typeof keys === 'string') return [`${at}: ${command}: ${keys}`];
    return [
      ...[...keys.keys()]
        .filter((key) => !Object.hasOwn(signature, key))
        .map((key) => `${at}: ${command} has no argument ${key}`),
      ...Object.entries(signature)
        .filter(
          ([key, presence]) =>
            presence === 'required' && keys.get(key) !== 'required',
        )
        .map(([key]) =>
          keys.has(key)
            ? `${at}: ${command} may omit required argument ${key}`
            : `${at}: ${command} omits required argument ${key}`,
        ),
    ];
  });
}

/**
 * The Bridge methods that no file in src/ refers to apart from the contract,
 * the bridge/commands-*.ts wrappers and the mock bridge in src/mock/. A method
 * is referred to when an identifier in another file resolves to its
 * declaration in the Bridge interface, as the property in
 * `bridge.readItem(...)` does.
 */
function uncalledBridgeMethods(compiled: ts.Program): string[] {
  const checker = compiled.getTypeChecker();
  const bridge = compiled
    .getSourceFile(contract)!
    .statements.find(
      (statement): statement is ts.InterfaceDeclaration =>
        ts.isInterfaceDeclaration(statement) &&
        statement.name.text === 'Bridge',
    );
  assert.ok(bridge, 'bridge/contract.ts declares Bridge');
  const methods = new Set<ts.Node>(bridge.members);
  const called = new Set<ts.Node>();
  for (const file of compiled.getSourceFiles()) {
    const name = file.fileName;
    if (
      !name.startsWith(source) ||
      name === contract ||
      name.startsWith(`${mock}/`) ||
      /\/bridge\/commands-[^/]+\.ts$/.test(name)
    )
      continue;
    const visit = (node: ts.Node): void => {
      if (ts.isIdentifier(node))
        for (const declaration of checker.getSymbolAtLocation(node)
          ?.declarations ?? [])
          if (methods.has(declaration)) called.add(declaration);
      ts.forEachChild(node, visit);
    };
    visit(file);
  }
  return bridge.members
    .filter((member) => !called.has(member))
    .map((member) => member.name?.getText() ?? '');
}

const compiled = program();
const sites = callSites(compiled);

test('only the transport and runYubi choose a native command at runtime', () => {
  // The transport's four: its unchecked invoke, checked, checkedMutation
  // calling checked, and the Tauri host's invoke.
  assert.deepEqual(sites.dynamic, [
    'bridge/commands-yubikey.ts',
    'bridge/transport.ts',
    'bridge/transport.ts',
    'bridge/transport.ts',
    'bridge/transport.ts',
  ]);
});

test('invoke and the transport wrappers are referenced only as callees', () => {
  assert.deepEqual(sites.unchecked, []);
});

test('every native call names a registered command with its argument keys', async () => {
  assert.deepEqual(
    problems([...sites.literal, ...sites.yubi], await manifest()),
    [],
  );
});

test('every registered command is called from the renderer', async () => {
  const called = new Set(
    [...sites.literal, ...sites.yubi].map((site) => site.command),
  );
  assert.deepEqual(
    Object.keys((await manifest()).commands).filter(
      (command) => !called.has(command),
    ),
    [],
  );
});

test('every bridge method has a caller besides its wrapper and the mock', () => {
  // These predate the check. Remove an entry when its method gains a caller
  // or is deleted together with its native command.
  assert.deepEqual(uncalledBridgeMethods(compiled), [
    'listParties',
    'listFederation',
    'verifyAccountPassphrase',
  ]);
});

test('a renamed command, parameter or optional parameter is reported', async () => {
  const renamed = await manifest();
  const { storeId, ...readItem } = renamed.commands.read_item;
  renamed.commands.read_item = { ...readItem, store: storeId };
  renamed.commands.create_text_item.readRole = 'required';
  delete renamed.commands.yubi_pin_status;
  const reported = problems([...sites.literal, ...sites.yubi], renamed);
  for (const expected of [
    /read_item has no argument storeId$/,
    /read_item omits required argument store$/,
    /create_text_item may omit required argument readRole$/,
    /^YubiCommand yubi_pin_status: no command yubi_pin_status$/,
  ])
    assert.ok(
      reported.some((problem) => expected.test(problem)),
      `${expected} in ${reported.join('\n')}`,
    );
});
