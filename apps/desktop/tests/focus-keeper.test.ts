import assert from 'node:assert/strict';
import test from 'node:test';
import { installDom, settle } from './lib/dom-harness';
import {
  mountFocusKeeper,
  restoreFocusIfLost,
  setFocusContext,
} from '../kit/focus-keeper';
import { anyPopupOpen, inPopup } from '../kit/overlay-primitives';

installDom({ url: 'http://localhost/' });

function page(html: string): void {
  document.body.innerHTML = `<div class="app"><nav class="side"><button data-place="files">All Items</button></nav><main class="main" tabindex="-1">${html}</main></div>`;
}

async function tick(): Promise<void> {
  // The observer's callback, then the keeper's timer.
  await settle();
  await settle();
}

test('a swapped control hands the focus to its replacement with the same key', async () => {
  page('<button data-focus-key="reveal">Show</button>');
  const stop = mountFocusKeeper();
  try {
    document.querySelector<HTMLElement>('[data-focus-key]')!.focus();
    document.querySelector('.main')!.innerHTML =
      '<button>Copy</button><button data-focus-key="reveal">Hide</button>';
    await tick();
    assert.equal(document.activeElement?.textContent, 'Hide');
  } finally {
    stop();
  }
});

test('a removed row lands the focus on the list, a requested target first', async () => {
  page(
    '<div class="rows" role="listbox"><div role="option" tabindex="-1" data-row-key="a">a</div><div role="option" tabindex="0" data-row-key="b">b</div><div role="option" tabindex="-1" data-row-key="c">c</div></div>',
  );
  const stop = mountFocusKeeper();
  try {
    const rowB = document.querySelector<HTMLElement>('[data-row-key="b"]')!;
    rowB.focus();
    rowB.remove();
    await tick();
    // No request: the first row, as the list has no selected option left.
    assert.equal(document.activeElement?.getAttribute('data-row-key'), 'a');

    const rowA = document.activeElement as HTMLElement;
    restoreFocusIfLost('[data-row-key="c"]');
    rowA.remove();
    await tick();
    assert.equal(document.activeElement?.getAttribute('data-row-key'), 'c');
  } finally {
    stop();
  }
});

test('a click on blank space leaves the focus alone while its control is usable', async () => {
  page('<button id="kept">Kept</button>');
  const stop = mountFocusKeeper();
  try {
    const kept = document.getElementById('kept')!;
    kept.focus();
    kept.blur();
    await tick();
    assert.equal(document.activeElement, document.body);
  } finally {
    stop();
  }
});

test('returning to a place focuses the control last focused there', async () => {
  page(
    '<div class="rows"><div tabindex="0" data-row-key="one">one</div><div tabindex="0" data-row-key="two">two</div></div>',
  );
  const stop = mountFocusKeeper();
  try {
    setFocusContext('folder');
    await tick();
    document.querySelector<HTMLElement>('[data-row-key="two"]')!.focus();
    setFocusContext('subfolder');
    document.querySelector('.main')!.innerHTML =
      '<div class="rows"><div tabindex="0" data-row-key="inner">inner</div></div>';
    await tick();
    assert.equal(document.activeElement?.getAttribute('data-row-key'), 'inner');
    setFocusContext('folder');
    document.querySelector('.main')!.innerHTML =
      '<div class="rows"><div tabindex="0" data-row-key="one">one</div><div tabindex="0" data-row-key="two">two</div></div>';
    await tick();
    assert.equal(document.activeElement?.getAttribute('data-row-key'), 'two');
  } finally {
    stop();
  }
});

test('a page listbox is not a popup; a portaled popup or an expanded combobox is', () => {
  page('<div role="listbox"><div role="option">a</div></div>');
  assert.equal(anyPopupOpen(), false);
  document.body.insertAdjacentHTML(
    'beforeend',
    '<input role="combobox" aria-expanded="true" aria-controls="x">',
  );
  assert.equal(anyPopupOpen(), true);
  document.querySelector('[role="combobox"]')!.remove();
  document.body.insertAdjacentHTML(
    'beforeend',
    '<div data-popup=""><div role="menu"><button id="item">Item</button></div></div>',
  );
  assert.equal(inPopup(document.getElementById('item')!), true);
  assert.equal(
    inPopup(document.querySelector('.main [role="option"]')!),
    false,
  );
});

test('a window whose focus was never placed is left alone as its list renders', async () => {
  page('<div class="rows"></div>');
  // The shell names its first place before the keeper starts.
  setFocusContext('first');
  const stop = mountFocusKeeper();
  try {
    document.querySelector('.rows')!.innerHTML =
      '<div tabindex="0" data-row-key="one">one</div>';
    await tick();
    assert.equal(document.activeElement, document.body);
  } finally {
    stop();
  }
});

test('a place the window settles on before any input is not a navigation', async () => {
  page('<div class="rows"></div>');
  setFocusContext('opening');
  const stop = mountFocusKeeper();
  try {
    // The shell completes its first location once the snapshot arrives.
    setFocusContext('opening, completed');
    document.querySelector('.rows')!.innerHTML =
      '<div tabindex="0" data-row-key="one">one</div>';
    await tick();
    assert.equal(document.activeElement, document.body);
    // After a key press a change of place is a navigation again.
    document.dispatchEvent(new window.KeyboardEvent('keydown', { key: 'Tab' }));
    setFocusContext('next');
    await tick();
    assert.equal(document.activeElement?.getAttribute('data-row-key'), 'one');
  } finally {
    stop();
  }
});
