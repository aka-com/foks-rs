/**
 * A layout for message lists in JSDOM, which lays nothing out: every
 * `.chat-messages` element is a 400px viewport over 100px rows, one per
 * `[data-message]` element. A row's place follows from its `data-sequence`,
 * so the rows a list holds must have consecutive sequences.
 *
 * Returns a function that restores the DOM's own layout.
 */
export function layOutLists(): () => void {
  const prototype = window.HTMLElement.prototype;
  const rect = Object.getOwnPropertyDescriptor(
    window.Element.prototype,
    'getBoundingClientRect',
  )!;
  const isList = (element: Element) =>
    element.classList.contains('chat-messages');
  Object.defineProperty(prototype, 'getBoundingClientRect', {
    configurable: true,
    value(this: HTMLElement) {
      if (isList(this)) return { top: 0, bottom: 400, height: 400 } as DOMRect;
      const list = this.dataset.message
        ? this.closest<HTMLElement>('.chat-messages')
        : null;
      const first = list?.querySelector<HTMLElement>('[data-message]');
      if (!list || !first) return (rect.value as () => DOMRect).call(this);
      const top =
        (Number(this.dataset.sequence) - Number(first.dataset.sequence)) * 100 -
        list.scrollTop;
      return { top, bottom: top + 100, height: 100 } as DOMRect;
    },
  });
  Object.defineProperty(prototype, 'scrollHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return isList(this)
        ? this.querySelectorAll('[data-message]').length * 100
        : 0;
    },
  });
  Object.defineProperty(prototype, 'clientHeight', {
    configurable: true,
    get(this: HTMLElement) {
      return isList(this) ? 400 : 0;
    },
  });
  return () => {
    for (const key of ['getBoundingClientRect', 'scrollHeight', 'clientHeight'])
      delete (prototype as unknown as Record<string, unknown>)[key];
  };
}
