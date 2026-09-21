import assert from 'node:assert/strict';
import test from 'node:test';
import { installDom } from './lib/dom-harness';
import { mountBackInputs } from '../src/shell/back-inputs';
installDom({
  url: 'http://localhost/',
  body: '<input id="edit"><button id="button">Button</button>',
});

test('Back inputs navigate once, preserve editing shortcuts, and suppress native navigation when blocked', () => {
  let enabled = true;
  let backs = 0;
  let forwards = 0;
  const stop = mountBackInputs({
    enabled: () => enabled,
    back: () => backs++,
    forward: () => forwards++,
  });
  const button = document.getElementById('button')!;
  const input = document.getElementById('edit')!;
  const key = (
    key: string,
    options: KeyboardEventInit = {},
    target = button,
  ) => {
    const event = new window.KeyboardEvent('keydown', {
      key,
      bubbles: true,
      cancelable: true,
      ...options,
    });
    target.dispatchEvent(event);
    return event;
  };
  try {
    assert.equal(key('ArrowLeft', { altKey: true }).defaultPrevented, true);
    key('[', { metaKey: true });
    key('BrowserBack');
    assert.equal(backs, 3);
    key('BrowserBack', { repeat: true });
    key('[', { metaKey: true, shiftKey: true });
    assert.equal(
      key('ArrowLeft', { altKey: true }, input).defaultPrevented,
      false,
    );
    assert.equal(key('[', { metaKey: true }, input).defaultPrevented, false);
    key('Backspace');
    assert.equal(backs, 3);
    for (const type of ['mousedown', 'mouseup', 'auxclick']) {
      const event = new window.MouseEvent(type, {
        button: 3,
        bubbles: true,
        cancelable: true,
      });
      button.dispatchEvent(event);
      assert.equal(event.defaultPrevented, true);
    }
    assert.equal(backs, 4);
    key('ArrowRight', { altKey: true });
    key(']', { metaKey: true });
    key('BrowserForward');
    key('BrowserForward', { repeat: true });
    assert.equal(key(']', { metaKey: true }, input).defaultPrevented, false);
    assert.equal(
      key('ArrowRight', { altKey: true }, input).defaultPrevented,
      false,
    );
    assert.equal(forwards, 3);
    for (const type of ['mousedown', 'mouseup', 'auxclick']) {
      const event = new window.MouseEvent(type, {
        button: 4,
        bubbles: true,
        cancelable: true,
      });
      button.dispatchEvent(event);
      assert.equal(event.defaultPrevented, true);
    }
    assert.equal(forwards, 4);
    enabled = false;
    assert.equal(key('BrowserForward').defaultPrevented, true);
    key(']', { metaKey: true });
    button.dispatchEvent(
      new window.MouseEvent('mouseup', { button: 4, bubbles: true }),
    );
    assert.equal(forwards, 4);
    assert.equal(key('BrowserBack').defaultPrevented, true);
    assert.equal(key('[', { metaKey: true }).defaultPrevented, true);
    button.dispatchEvent(
      new window.MouseEvent('mouseup', { button: 3, bubbles: true }),
    );
    assert.equal(backs, 4);
  } finally {
    stop();
  }
  key('BrowserBack');
  key('BrowserForward');
  assert.equal(backs, 4);
  assert.equal(forwards, 4);
});
