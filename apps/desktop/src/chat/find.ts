import type { ChatMessage } from '../chat-contract';
import { messageDate } from './presentation';

/**
 * Find in a conversation. The find bar matches the rows the thread draws from
 * its history window, which the agent has already decrypted and verified;
 * "Search older messages" reads older pages with the same history request as
 * Load older messages and keeps only a {@link FindReference} for each matching
 * row, so the scan never grows the window. Nothing here is stored: the agent
 * has no message index, and the find bar keeps no plaintext beyond the
 * references of the scan in progress, which go when the bar closes.
 */

/** Older history pages one "Search older messages" reads before it stops. */
export const FIND_SCAN_PAGES = 5;

/** The CSS highlight names every match and the current match are drawn with. */
export const FIND_HIGHLIGHT = 'chat-find';
export const FIND_CURRENT_HIGHLIGHT = 'chat-find-current';

/**
 * Case-folds `text` scalar by scalar, keeping the first scalar of each
 * mapping as {@link lowercaseChatText} does, and keeping a scalar unchanged
 * where its lowercase form has another UTF-16 length. Offsets into the folded
 * text are therefore offsets into `text`.
 */
export function foldFindText(text: string): string {
  let folded = '';
  for (const scalar of text) {
    const lower = [...scalar.toLowerCase()][0] ?? scalar;
    folded += lower.length === scalar.length ? lower : scalar;
  }
  return folded;
}

/**
 * The start offsets of the non-overlapping occurrences of `needle` in
 * `haystack`, both already folded with {@link foldFindText}.
 */
export function findOffsets(haystack: string, needle: string): number[] {
  const offsets: number[] = [];
  if (!needle) return offsets;
  for (
    let at = haystack.indexOf(needle);
    at >= 0;
    at = haystack.indexOf(needle, at + needle.length)
  )
    offsets.push(at);
  return offsets;
}

/**
 * The text find matches in a message: its Basic text. A row that did not
 * decrypt and verify, or whose type the desktop does not draw, has none.
 */
export function findableText(message: ChatMessage): string | null {
  return message.content.kind === 'text' ? message.content.text : null;
}

/**
 * A matching row an older-page scan found: enough to list it and to open the
 * channel around it, and no more than the row the agent returned.
 */
export interface FindReference {
  id: string;
  sequence: string;
  sender: string | null;
  insert_time: string;
  text: string;
}

/** The rows of a history page whose text contains `needle`, newest first. */
export function scanPage(
  messages: readonly ChatMessage[],
  needle: string,
): FindReference[] {
  const folded = foldFindText(needle);
  if (!folded) return [];
  const found: FindReference[] = [];
  for (const message of messages) {
    const text = findableText(message);
    if (text === null || !foldFindText(text).includes(folded)) continue;
    found.push({
      id: message.id,
      sequence: message.sequence,
      sender: message.sender,
      insert_time: message.insert_time,
      text,
    });
  }
  return found.sort((a, b) =>
    BigInt(a.sequence) > BigInt(b.sequence) ? -1 : 1,
  );
}

/**
 * The day a scan or a window reaches back to, as the find bar states it:
 * "Sep 30", with the year when it is not the current one.
 */
export function findDay(
  milliseconds: string,
  nowMilliseconds: number = Date.now(),
): string {
  const date = messageDate(milliseconds);
  if (!date) return `time ${milliseconds}`;
  return new Intl.DateTimeFormat(undefined, {
    year:
      date.getFullYear() === new Date(nowMilliseconds).getFullYear()
        ? undefined
        : 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

/** One line of a match in context, with the match itself apart. */
export interface FindSnippet {
  before: string;
  match: string;
  after: string;
}

/**
 * The first occurrence of `needle` in `text` with up to `context` characters
 * on either side, on one line: runs of whitespace read as one space, and a
 * cut end is marked with an ellipsis. `null` when `text` does not contain it.
 */
export function findSnippet(
  text: string,
  needle: string,
  context = 40,
): FindSnippet | null {
  const folded = foldFindText(needle);
  const at = folded ? foldFindText(text).indexOf(folded) : -1;
  if (at < 0) return null;
  const end = at + folded.length;
  const start = Math.max(0, at - context);
  const stop = Math.min(text.length, end + context);
  const line = (part: string) => part.replace(/\s+/g, ' ');
  return {
    before: (start > 0 ? '…' : '') + line(text.slice(start, at)).trimStart(),
    match: line(text.slice(at, end)),
    after:
      line(text.slice(end, stop)).trimEnd() + (stop < text.length ? '…' : ''),
  };
}

/** A match drawn in the message list: its row and the text it covers. */
export interface FindMatch {
  /** The message row, an `article` carrying `data-message`. */
  row: HTMLElement;
  range: Range;
}

const SHOW_TEXT = 0x4;
/** Text inside these is not part of what the message says. */
const SKIPPED = 'button, .offscreen, [aria-hidden="true"]';
/** A change of block between two text nodes ends a run a match can span. */
const BLOCK = 'p, li, pre, blockquote, div';

/**
 * The occurrences of `needle` in the message text the history rows under
 * `root` draw, oldest first. Each row's text is matched as one string, so a
 * match may span formatting such as a bold word or a mention, but not two
 * paragraphs. Screen-reader-only text and the controls inside a message (a
 * code block's Copy, for one) are not matched.
 */
export function findRanges(root: ParentNode, needle: string): FindMatch[] {
  const folded = foldFindText(needle);
  const matches: FindMatch[] = [];
  if (!folded) return matches;
  for (const row of root.querySelectorAll<HTMLElement>('[data-message]')) {
    const body = row.querySelector<HTMLElement>('.chat-message-text');
    if (!body) continue;
    const document = body.ownerDocument;
    const walker = document.createTreeWalker(body, SHOW_TEXT);
    const nodes: { node: Text; start: number }[] = [];
    let text = '';
    let block: Element | null = null;
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const parent = node.parentElement;
      const skipped = parent?.closest(SKIPPED);
      if (!parent || (skipped && body.contains(skipped))) continue;
      const value = node.nodeValue ?? '';
      if (!value) continue;
      const here = parent.closest(BLOCK);
      if (nodes.length > 0 && here !== block) text += '\n';
      block = here;
      nodes.push({ node: node as Text, start: text.length });
      text += value;
    }
    const position = (offset: number, end: boolean) => {
      // The last node starting at or before the offset; an end offset at a
      // node's start belongs to the node before it.
      let index = nodes.length - 1;
      while (
        index > 0 &&
        (end ? nodes[index].start >= offset : nodes[index].start > offset)
      )
        index--;
      return { node: nodes[index].node, offset: offset - nodes[index].start };
    };
    for (const at of findOffsets(foldFindText(text), folded)) {
      const start = position(at, false);
      const end = position(at + folded.length, true);
      const range = document.createRange();
      range.setStart(start.node, start.offset);
      range.setEnd(end.node, end.offset);
      matches.push({ row, range });
    }
  }
  return matches;
}

/**
 * Draws `matches` with the CSS Custom Highlight API, the current one apart,
 * without changing the rows' DOM. A webview without the API draws no
 * highlight; the current row is marked by its `data-find` attribute either
 * way.
 */
export function paintFindHighlights(
  matches: readonly FindMatch[],
  current: FindMatch | null,
): void {
  const registry = (globalThis as { CSS?: { highlights?: HighlightRegistry } })
    .CSS?.highlights;
  const Paint = (globalThis as { Highlight?: typeof Highlight }).Highlight;
  if (!registry || !Paint) return;
  const others = matches.filter((match) => match !== current);
  if (others.length)
    registry.set(FIND_HIGHLIGHT, new Paint(...others.map((m) => m.range)));
  else registry.delete(FIND_HIGHLIGHT);
  if (current) registry.set(FIND_CURRENT_HIGHLIGHT, new Paint(current.range));
  else registry.delete(FIND_CURRENT_HIGHLIGHT);
}

/**
 * The window header's search field reaches the mounted thread through this:
 * typing there while its badge names the open channel opens the thread's find
 * bar with the text. Returns false when no thread is mounted to take it.
 */
let findRequest: ((text: string) => void) | null = null;

export function requestChatFind(text: string): boolean {
  if (!findRequest) return false;
  findRequest(text);
  return true;
}

/**
 * Makes `handler` the receiver of {@link requestChatFind}; the returned
 * function removes it again if it is still the receiver.
 */
export function receiveChatFind(handler: (text: string) => void): () => void {
  findRequest = handler;
  return () => {
    if (findRequest === handler) findRequest = null;
  };
}
