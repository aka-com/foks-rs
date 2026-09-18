import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { Bridge } from '../src/bridge';
import { MetadataRepository } from '../src/metadata-repository';
import {
  MetadataRepositoryContext,
  QueryRepositoryContext,
  useMetadataQuery,
  useMetadataRepository,
} from '../src/query-hooks';
import { appInfoQuery } from '../src/resources/application';
import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/', timers: true, act: true });
let ui: typeof import('@testing-library/react');
test.before(async () => {
  ui = await import('@testing-library/react');
});
test.afterEach(() => ui.cleanup());

function View({ bridge }: { bridge: Pick<Bridge, 'appInfo'> }) {
  const repository = useMetadataRepository(bridge);
  const { data } = useMetadataQuery(appInfoQuery(repository, bridge));
  return createElement('output', null, data?.version ?? 'Loading');
}

test('legacy providers share the canonical resource across views and view remounts', async () => {
  let calls = 0;
  const repository = new MetadataRepository();
  const bridge = {
    appInfo: async () => {
      calls++;
      return { version: `Version ${calls}`, agentSocket: '/agent.sock' };
    },
  };
  const rendered = ui.render(
    createElement(
      QueryRepositoryContext.Provider,
      { value: repository },
      createElement(View, { bridge }),
      createElement(View, { bridge }),
    ),
  );
  await ui.act(async () => {});
  assert.equal(calls, 1);
  assert.equal(rendered.getAllByText('Version 1').length, 2);
  rendered.unmount();
  assert.equal(repository.retired, false);
  assert.equal(
    appInfoQuery(repository, bridge).getSnapshot().data?.version,
    'Version 1',
  );

  const remounted = ui.render(
    createElement(
      MetadataRepositoryContext.Provider,
      { value: repository },
      createElement(View, { bridge }),
    ),
  );
  await ui.act(async () => {});
  assert.equal(calls, 1);
  assert.ok(remounted.getByText('Version 1'));
  await ui.act(async () => {
    repository.clear();
  });
  assert.equal(calls, 2);
  assert.ok(remounted.getByText('Version 2'));
});
