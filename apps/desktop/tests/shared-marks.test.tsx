/**
 * The store mark, role chip and party row that the Files list, the details
 * panel and the group page share from src/components.
 */

import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';

import { PartyRow, RoleChip, StoreMark } from '../src/components';
import { FIXTURE } from '../src/fixture';
import type { Party, Store } from '../src/model';

function store(id: string): Store {
  const found = FIXTURE.stores.find((candidate) => candidate.id === id);
  assert.ok(found, id);
  return found;
}

function party(username: string): Party {
  const found = FIXTURE.parties.find(
    (candidate) =>
      candidate.store === 'team:eng' && candidate.username === username,
  );
  assert.ok(found, username);
  return found;
}

test('a team store mark draws its initials over the hue the caller passes', () => {
  const html = renderToStaticMarkup(
    <StoreMark snapshot={FIXTURE} store={store('team:eng')} hue="#123456" />,
  );
  assert.equal(
    html,
    '<span class="av team" style="background:#123456" aria-hidden="true">E</span>',
  );
});

test('an account store mark is the account mark of its username', () => {
  const html = renderToStaticMarkup(
    <StoreMark snapshot={FIXTURE} store={store('acct:work')} hue="#123456" />,
  );
  assert.match(html, /^<span class="kico group account"/);
  assert.match(html, />V<\/span>$/);
});

test('a role chip writes the member band as a member row reads it', () => {
  assert.equal(
    renderToStaticMarkup(<RoleChip role={{ role: 'Member', visibility: 2 }} />),
    '<span class="rolecell"><span class="chip">Member · sees level 2</span></span>',
  );
  assert.match(renderToStaticMarkup(<RoleChip role={null} />), />—</);
});

test('a roster party row draws the mark, the caption, the role chip and the actions', () => {
  const html = renderToStaticMarkup(
    <PartyRow
      variant="roster"
      party={party('vitalik')}
      mark={<i className="mark" />}
      caption="person"
      actions={<button type="button">Actions</button>}
    />,
  );
  assert.equal(
    html,
    '<div class="prow"><span class="who2"><i class="mark"></i>' +
      '<span class="t"><b><span>vitalik</span><span class="chip you">you</span></b>' +
      '<small>person</small></span></span>' +
      '<span class="rowtail"><span class="rolecell"><span class="chip">Admin</span></span>' +
      '<button type="button">Actions</button></span></div>',
  );
});

test('a listing party row writes the role as the item access line does', () => {
  const html = renderToStaticMarkup(
    <PartyRow
      variant="listing"
      party={party('dana.okafor')}
      caption="No read access"
    />,
  );
  assert.equal(
    html,
    '<div class="party"><span class="t">dana.okafor<small>No read access</small></span>' +
      '<span class="chip">Member · visibility 0</span></div>',
  );
  const mine = renderToStaticMarkup(
    <PartyRow variant="listing" party={party('vitalik')} caption="" />,
  );
  assert.equal(
    mine,
    '<div class="party"><span class="t">vitalik <span class="chip you">you</span></span>' +
      '<span class="chip">Admin</span></div>',
  );
});
