import { inlineTokens } from './mentions';

/**
 * The bounded Markdown subset of Basic message text, read into blocks and
 * inline parts. `message-text.tsx` draws these, and
 * {@link messagePlainText} reads the same parts as the text that drawing
 * shows, which find in a conversation matches.
 */

/** Literal absolute HTTP(S) only; never decode entities into executable URLs. */
export function safeChatLink(value: string): string | null {
  if (
    value.length > 2048 ||
    Array.from(value).some(
      (c) =>
        /[\s\\]/u.test(c) || c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127,
    )
  )
    return null;
  if (!/^https?:\/\//i.test(value)) return null;
  try {
    const url = new URL(value);
    return url.hostname && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}

/** Closing brackets a bare URL may end in, each with its opening bracket. */
const URL_BRACKETS = new Map([
  [')', '('],
  [']', '['],
  ['\uFF09', '\uFF08'], // full-width parentheses
  ['\uFF3D', '\uFF3B'], // full-width square brackets
  ['\u300D', '\u300C'], // corner brackets
  ['\u300F', '\u300E'], // white corner brackets
  ['\u3011', '\u3010'], // black lenticular brackets
]);

/**
 * A bare URL without the punctuation that ends the sentence around it: any
 * trailing `.`, `,`, `;`, `:`, `!`, `?`, `'` or `"`, the CJK sentence
 * punctuation `。`, `、`, `，`, `．`, `！`, `？`, `：` and `；`, a trailing `_`
 * when the URL follows a `_` (`_https://example.com_`), and a trailing
 * closing bracket (ASCII, full-width or CJK corner and lenticular) that closes
 * nothing the URL opened (`(see https://example.com/a_(b))` keeps the URL's
 * own `)`). Linear in the URL's length.
 */
export function trimBareUrl(url: string, underscored = false): string {
  // Per opening bracket: openings minus closings in the whole URL.
  const open = new Map<string, number>();
  for (const opener of URL_BRACKETS.values()) open.set(opener, 0);
  for (const c of url) {
    const opener = URL_BRACKETS.get(c);
    if (opener !== undefined) open.set(opener, (open.get(opener) ?? 0) - 1);
    else if (open.has(c)) open.set(c, (open.get(c) ?? 0) + 1);
  }
  const trailing = `.,;:!?'"\u3002\u3001\uFF0C\uFF0E\uFF01\uFF1F\uFF1A\uFF1B${
    underscored ? '_' : ''
  }`;
  let end = url.length;
  while (end > 0) {
    const c = url[end - 1];
    const opener = URL_BRACKETS.get(c);
    if (trailing.includes(c)) end--;
    else if (opener !== undefined && (open.get(opener) ?? 0) < 0) {
      open.set(opener, (open.get(opener) ?? 0) + 1);
      end--;
    } else break;
  }
  return url.slice(0, end);
}

// The bidirectional formatting characters: U+061C, U+200E, U+200F,
// U+202A..U+202E and U+2066..U+2069.
export const BIDI_CONTROL = /\p{Bidi_Control}/gu;

/** A link label as drawn: without bidirectional formatting characters. */
export function drawnLinkLabel(label: string): string {
  return label.replace(BIDI_CONTROL, '');
}

/**
 * A run of one line's inline syntax. `at` is where the token starts in the
 * line; a link's `url` is a {@link safeChatLink} result.
 */
export type InlinePart =
  | { kind: 'text'; text: string }
  | { kind: 'code' | 'strong' | 'em'; at: number; text: string }
  | { kind: 'link'; at: number; label: string; url: string }
  | { kind: 'mention'; at: number; name: string };

/**
 * The inline parts of one line: code spans, bold, italics, links, bare URLs
 * and `@name` tokens, with the text between them. Deliberately nonrecursive
 * and bounded: unsupported or nested syntax remains text, and past 256 parts
 * the rest of the line is text. `mentionsReader` reads the same tokens.
 */
export function inlineParts(text: string): InlinePart[] {
  const parts: InlinePart[] = [];
  const pattern = inlineTokens();
  let last = 0;
  for (let match; (match = pattern.exec(text));) {
    if (parts.length >= 256) break;
    const token = match[0],
      at = match.index;
    let used = token.length;
    parts.push({ kind: 'text', text: text.slice(last, at) });
    if (token.startsWith('`'))
      parts.push({ kind: 'code', at, text: token.slice(1, -1) });
    else if (token.startsWith('**'))
      parts.push({ kind: 'strong', at, text: token.slice(2, -2) });
    else if (token.startsWith('*'))
      parts.push({ kind: 'em', at, text: token.slice(1, -1) });
    else if (token.startsWith('[')) {
      const split = token.indexOf('](');
      const url = safeChatLink(token.slice(split + 2, -1));
      parts.push(
        url
          ? { kind: 'link', at, label: token.slice(1, split), url }
          : { kind: 'text', text: token },
      );
    } else if (token.startsWith('@')) {
      // A name does not end in `.`, `_` or `-`; those end the sentence. A
      // backward scan, since a trailing-separator regex backtracks on a long
      // separator run followed by a name character.
      let end = token.length;
      while (end > 2 && '._-'.includes(token[end - 1])) end--;
      used = end;
      parts.push({ kind: 'mention', at, name: token.slice(1, end) });
    } else {
      const bare = trimBareUrl(token, text[at - 1] === '_');
      used = bare.length;
      const url = safeChatLink(bare);
      parts.push(
        url
          ? { kind: 'link', at, label: bare, url }
          : { kind: 'text', text: bare },
      );
    }
    last = at + used;
    pattern.lastIndex = last;
  }
  parts.push({ kind: 'text', text: text.slice(last) });
  return parts;
}

/** A list item or quote line without its marker, and its line number. */
export interface MessageLine {
  line: number;
  text: string;
}

/**
 * A block of message text. `start` is its first line; `gap` is set when blank
 * lines separate it from the block before, which draws a paragraph gap.
 */
export type MessageBlock = { start: number; gap: boolean } & (
  | { kind: 'code'; code: string }
  /** A fence left open: the rest of the message, drawn as written. */
  | { kind: 'literal'; text: string }
  | { kind: 'list'; ordered: boolean; items: MessageLine[] }
  /** Consecutive quote lines, which are one quote. */
  | { kind: 'quote'; lines: MessageLine[] }
  | { kind: 'paragraph'; text: string }
);

/** Text longer than this, or with more lines, is drawn as written. */
const MAX_FORMATTED_TEXT = 65536;
const MAX_FORMATTED_LINES = 512;

/**
 * The blocks of `text`. Empty when the text is past the formatting bounds or
 * holds only blank lines; such text is drawn as written.
 */
export function messageBlocks(text: string): MessageBlock[] {
  const lines = text.split(/\r?\n/);
  const blocks: MessageBlock[] = [];
  if (text.length > MAX_FORMATTED_TEXT || lines.length > MAX_FORMATTED_LINES)
    return blocks;
  // Set by blank lines between blocks: the next block is drawn after a
  // paragraph gap. Blank lines before the first block draw nothing.
  let gap = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const start = i;
    if (line.startsWith('```')) {
      const body: string[] = [];
      while (++i < lines.length && !lines[i].startsWith('```'))
        body.push(lines[i]);
      if (i === lines.length) {
        blocks.push({
          kind: 'literal',
          start,
          gap,
          text: lines.slice(start).join('\n'),
        });
        break;
      }
      blocks.push({ kind: 'code', start, gap, code: body.join('\n') });
    } else if (/^[-*] /.test(line) || /^\d+\. /.test(line)) {
      const ordered = /^\d+\. /.test(line);
      const marker = ordered ? /^\d+\. / : /^[-*] /;
      const items: MessageLine[] = [];
      do {
        items.push({ line: i, text: lines[i].replace(marker, '') });
        i++;
      } while (i < lines.length && marker.test(lines[i]));
      i--;
      blocks.push({ kind: 'list', start, gap, ordered, items });
    } else if (line.startsWith('> ')) {
      const quoted: MessageLine[] = [];
      do {
        quoted.push({ line: i, text: lines[i].slice(2) });
        i++;
      } while (i < lines.length && lines[i].startsWith('> '));
      i--;
      blocks.push({ kind: 'quote', start, gap, lines: quoted });
    } else if (line.trim())
      blocks.push({ kind: 'paragraph', start, gap, text: line });
    else {
      gap = blocks.length > 0;
      continue;
    }
    gap = false;
  }
  return blocks;
}

/** The text one line's inline parts draw. */
function plainInline(text: string): string {
  let plain = '';
  for (const part of inlineParts(text)) {
    if (part.kind === 'link') plain += drawnLinkLabel(part.label);
    else if (part.kind === 'mention') plain += `@${part.name}`;
    else plain += part.text;
  }
  return plain;
}

/**
 * The text `MessageText` draws for `text`, with every line of a code block,
 * including the lines a collapsed block hides: no inline markers, list or
 * quote markers, or link destinations, and each block, list item and quote
 * line on a line of its own. The screen-reader-only text and the destination
 * host drawn beside a link are not part of it.
 */
export function messagePlainText(text: string): string {
  const blocks = messageBlocks(text);
  if (!blocks.length) return text;
  return blocks
    .map((block) => {
      switch (block.kind) {
        case 'code':
          return block.code;
        case 'literal':
          return block.text;
        case 'list':
          return block.items.map((item) => plainInline(item.text)).join('\n');
        case 'quote':
          return block.lines.map((line) => plainInline(line.text)).join('\n');
        case 'paragraph':
          return plainInline(block.text);
      }
    })
    .join('\n');
}
