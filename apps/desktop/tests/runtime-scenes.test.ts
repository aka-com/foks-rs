import assert from 'node:assert/strict';
import test from 'node:test';
import { initialScene, fixtureScenesAllowed } from '../src/app/scenes';
import { FIXTURE } from '../src/fixture';
import { installDom } from './lib/dom-harness';

installDom({ url: 'http://localhost/?state=show&lease=lapsed' });

test('native runtime URLs cannot force a fixture reveal or lease expiry', () => {
  const scene = initialScene({ native: true });
  assert.equal(scene.reveal, false);
  assert.equal(scene.demo, null);
  assert.equal(scene.lease, 'fresh');
  assert.equal(fixtureScenesAllowed(undefined), false);
});

test('an explicit mock capability preserves acceptance scenes', () => {
  const scene = initialScene({ native: false });
  assert.equal(scene.reveal, true);
  assert.equal(scene.demo, 'password');
  assert.equal(scene.lease, 'lapsed');
  assert.equal(
    fixtureScenesAllowed({ native: true, fixtureSnapshot: FIXTURE }),
    true,
  );
});
