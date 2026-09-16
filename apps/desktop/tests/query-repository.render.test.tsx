import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { installDom } from './lib/dom-harness';
import { QueryRepository } from '../src/query-repository';
import type { MetadataQuery } from '../src/query-repository';
import { useMetadataQuery } from '../src/query-hooks';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function View({
  query,
  error,
}: {
  query: MetadataQuery<string> | null;
  error: (e: unknown) => void;
}) {
  const state = useMetadataQuery(query, { onError: error });
  return createElement('output', null, state.data ?? 'No accepted metadata');
}

test('mounted subscribers share invalidation reloads and report one failed read', async () => {
  const repository = new QueryRepository();
  let calls = 0;
  let fail = false;
  const query = repository.query(['devices', 'profile'], async () => {
    calls++;
    if (fail) throw new Error('offline');
    return 'Accepted devices';
  });
  const errors: unknown[] = [];
  const error = (value: unknown) => {
    errors.push(value);
  };
  const rendered = ui.render(
    createElement(
      'div',
      null,
      createElement(View, { query, error }),
      createElement(View, { query, error }),
    ),
  );
  await ui.act(async () => {});
  assert.equal(calls, 1);
  assert.equal(rendered.getAllByText('Accepted devices').length, 2);
  fail = true;
  await ui.act(async () => {
    repository.invalidate(['devices']);
  });
  assert.equal(calls, 2);
  assert.equal(errors.length, 1);
  assert.equal(rendered.getAllByText('Accepted devices').length, 2);
});

test('changing identity clears the displayed old account before either late reply arrives', async () => {
  const repository = new QueryRepository();
  const old = deferred<string>();
  const next = deferred<string>();
  const a = repository.query(['devices', 'old'], () => old.promise);
  const b = repository.query(['devices', 'next'], () => next.promise);
  const error = () => {
    assert.fail('A read unexpectedly failed');
  };
  const rendered = ui.render(createElement(View, { query: a, error }));
  await ui.act(async () => {});
  rendered.rerender(createElement(View, { query: b, error }));
  assert.ok(rendered.getByText('No accepted metadata'));
  await ui.act(async () => {
    old.resolve('Old account');
  });
  assert.equal(rendered.queryByText('Old account'), null);
  await ui.act(async () => {
    next.resolve('Next account');
  });
  assert.ok(rendered.getByText('Next account'));
  rendered.rerender(createElement(View, { query: null, error }));
  assert.equal(
    rendered.queryByText('Next account'),
    null,
    'stopped access hides cached data immediately',
  );
});

test('an access reset reloads subscribers and discards a prior epoch failure', async () => {
  const repository = new QueryRepository();
  const old = deferred<string>();
  let calls = 0;
  const query = repository.query(['devices', 'profile'], () =>
    ++calls === 1 ? old.promise : Promise.resolve('Current epoch'),
  );
  const errors: unknown[] = [];
  const rendered = ui.render(
    createElement(View, {
      query,
      error: (value) => {
        errors.push(value);
      },
    }),
  );
  await ui.act(async () => {});
  await ui.act(async () => {
    repository.clear();
  });
  assert.equal(calls, 2);
  await ui.act(async () => {
    old.reject(new Error('late stale failure'));
  });
  assert.ok(rendered.getByText('Current epoch'));
  assert.deepEqual(errors, []);
});
