import assert from 'node:assert/strict';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

export function parseUniqueJson(text, source = 'JSON') {
  const value = JSON.parse(text);
  const tokens = text.match(/"(?:[^"\\]|\\.)*"|[{}[\]:,]|[^\s{}[\]:,]+/g) ?? [];
  const stack = [];
  for (let index = 0; index < tokens.length; index++) {
    const token = tokens[index];
    if (token === '{') stack.push(new Set());
    else if (token === '[') stack.push(null);
    else if (token === '}' || token === ']') stack.pop();
    else if (token.startsWith('"') && tokens[index + 1] === ':') {
      const key = JSON.parse(token);
      const keys = stack.at(-1);
      assert.ok(keys && !keys.has(key), `${source}: duplicate key ${key}`);
      keys.add(key);
    }
  }
  return value;
}

export function mergeDomains(domains) {
  const merged = {};
  for (const [domain, records] of Object.entries(domains)) {
    assert.ok(
      records && typeof records === 'object' && !Array.isArray(records),
    );
    for (const [key, value] of Object.entries(records)) {
      assert.ok(
        !Object.hasOwn(merged, key),
        `${domain}: duplicate record ${key}`,
      );
      Object.defineProperty(merged, key, {
        value,
        enumerable: true,
        configurable: true,
        writable: true,
      });
    }
  }
  return merged;
}

export async function loadDomains(directory = new URL('./', import.meta.url)) {
  const read = async (name) =>
    parseUniqueJson(await readFile(new URL(name, directory), 'utf8'), name);
  const inventory = await read('inventory.json');
  const expectedFiles = Object.keys(inventory.domains).map(
    (name) => `${name}.json`,
  );
  const files = (await readdir(directory)).filter(
    (name) => name.endsWith('.json') && name !== 'inventory.json',
  );
  assert.deepEqual(
    files.sort(),
    expectedFiles.sort(),
    'Domain inventory must list every fixture file',
  );
  const domains = {};
  for (const [name, keys] of Object.entries(inventory.domains)) {
    const records = await read(`${name}.json`);
    assert.deepEqual(
      Object.keys(records).sort(),
      [...keys].sort(),
      `${name}: record inventory drift`,
    );
    domains[name] = records;
  }
  return { inventory, domains, aggregate: mergeDomains(domains) };
}

async function main() {
  const mode = process.argv[2] ?? '--check';
  assert.ok(['--check', '--write'].includes(mode), 'Use --check or --write');
  const { aggregate } = await loadDomains();
  const target = new URL('../wire-contract.json', import.meta.url);
  if (mode === '--write') {
    await writeFile(target, `${JSON.stringify(aggregate, null, 2)}\n`);
  } else {
    const existing = parseUniqueJson(
      await readFile(target, 'utf8'),
      'wire-contract.json',
    );
    assert.deepEqual(
      aggregate,
      existing,
      'Reassemble wire-contract.json from the domain fixtures',
    );
  }
}

if (
  process.argv[1] &&
  pathToFileURL(process.argv[1]).href === import.meta.url
) {
  await main();
}
