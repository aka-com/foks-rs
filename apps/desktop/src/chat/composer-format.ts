import { fmtSize } from '../model';

/**
 * Text edits the composer applies for its formatting keys and for an
 * `@name` completion, as pure functions of the draft and its selection. Each
 * returns the range to replace, the text that replaces it and the selection
 * afterwards, so the composer can apply it as one undoable insertion.
 */
export interface ComposerEdit {
  from: number;
  to: number;
  text: string;
  selectionStart: number;
  selectionEnd: number;
  /** The edit removed markers rather than adding them. */
  removed: boolean;
}

export type ComposerFormat = 'bold' | 'italic' | 'code' | 'codeBlock';

const MARKS = { bold: '**', italic: '*', code: '`' } as const;

/**
 * Whether `[start, end)` of `value` is enclosed in `mark`. A `*` enclosed in
 * `**` on both sides is bold, not italic, unless a third `*` precedes it.
 */
function enclosed(value: string, start: number, end: number, mark: string) {
  const width = mark.length;
  if (
    value.slice(start - width, start) !== mark ||
    value.slice(end, end + width) !== mark
  )
    return false;
  if (mark !== '*') return true;
  return (
    value.slice(start - 2, start) !== '**' ||
    value.slice(end, end + 2) !== '**' ||
    value.slice(start - 3, start) === '***'
  );
}

/**
 * Whether `core` starts and ends with `mark`. A `*` that starts `**` opens
 * bold, not italic, unless a third `*` follows it.
 */
function wrapped(core: string, mark: string): boolean {
  return (
    core.length > 2 * mark.length &&
    core.startsWith(mark) &&
    core.endsWith(mark) &&
    !(mark === '*' && core.startsWith('**') && !core.startsWith('***'))
  );
}

/**
 * A list item or quote marker, which `message-text.tsx` reads only at the
 * start of a line and which inline markers therefore must not precede.
 */
const BLOCK_MARKER = /^(?:[-*] |\d+\. |> )/;

/**
 * Wraps the selection in the format's markers, or removes them when the
 * selection is already wrapped in them; a caret between an empty pair
 * removes the pair. `message-text.tsx` reads an inline style within one line
 * only, so a selection across lines is handled line by line (see
 * {@link linesEdit}). Whitespace at the edges of the selection stays outside
 * the markers, as it belongs to the surrounding text. A code block puts the
 * selected lines, whole, between ``` fence lines.
 */
export function formatEdit(
  format: ComposerFormat,
  value: string,
  start: number,
  end: number,
): ComposerEdit {
  if (format === 'codeBlock') return fenceEdit(value, start, end);
  const mark = MARKS[format];
  const width = mark.length;
  const selected = value.slice(start, end);
  if (selected.includes('\n')) return linesEdit(value, start, end, mark);
  const coreStart = start + (selected.length - selected.trimStart().length);
  const coreEnd = Math.max(
    coreStart,
    end - (selected.length - selected.trimEnd().length),
  );
  const core = value.slice(coreStart, coreEnd);
  if (enclosed(value, coreStart, coreEnd, mark))
    return {
      from: coreStart - width,
      to: coreEnd + width,
      text: core,
      selectionStart: coreStart - width,
      selectionEnd: coreEnd - width,
      removed: true,
    };
  if (wrapped(core, mark)) {
    const inner = core.slice(width, -width);
    return {
      from: coreStart,
      to: coreEnd,
      text: inner,
      selectionStart: coreStart,
      selectionEnd: coreStart + inner.length,
      removed: true,
    };
  }
  return {
    from: coreStart,
    to: coreEnd,
    text: `${mark}${core}${mark}`,
    selectionStart: coreStart + width,
    selectionEnd: coreEnd + width,
    removed: false,
  };
}

/**
 * The inline format applied to each line of a selection across lines. Each
 * non-blank line is wrapped after any list item or quote marker that starts
 * it, so `- one` becomes `- **one**` and stays a list item. Fence lines and
 * the lines between them are left as they are, since a code block shows its
 * text as typed. When every line the format applies to is already wrapped in
 * the markers, they are removed from each line instead; otherwise lines that
 * are already wrapped are not wrapped again.
 */
function linesEdit(
  value: string,
  start: number,
  end: number,
  mark: string,
): ComposerEdit {
  const width = mark.length;
  const segments = value.slice(start, end).split('\n');
  const firstLine = value.lastIndexOf('\n', start - 1) + 1;
  // `message-text.tsx` opens and closes a fence at each line that starts
  // with ```, so the fence lines before the selection say whether its first
  // line is inside one.
  let fenced =
    value
      .slice(0, firstLine)
      .split('\n')
      .filter((line) => line.startsWith('```')).length %
      2 ===
    1;
  let at = start;
  const lines = segments.map((segment) => {
    const segmentStart = at;
    at += segment.length + 1;
    const lineStart = value.lastIndexOf('\n', segmentStart - 1) + 1;
    if (value.startsWith('```', lineStart)) {
      fenced = !fenced;
      return null;
    }
    if (fenced) return null;
    // A selection that starts within a line takes no marker from it.
    const block =
      segmentStart === lineStart ? (BLOCK_MARKER.exec(segment)?.[0] ?? '') : '';
    const rest = segment.slice(block.length);
    const core = rest.trim();
    if (!core) return null;
    return { lead: block.length + rest.indexOf(core), core };
  });
  const applied = lines.filter((line) => line !== null);
  const removed =
    applied.length > 0 && applied.every(({ core }) => wrapped(core, mark));
  const text = segments
    .map((segment, i) => {
      const line = lines[i];
      if (!line) return segment;
      const { lead, core } = line;
      const head = segment.slice(0, lead);
      const tail = segment.slice(lead + core.length);
      if (removed) return `${head}${core.slice(width, -width)}${tail}`;
      if (wrapped(core, mark)) return segment;
      return `${head}${mark}${core}${mark}${tail}`;
    })
    .join('\n');
  return {
    from: start,
    to: end,
    text,
    selectionStart: start,
    selectionEnd: start + text.length,
    removed,
  };
}

function fenceEdit(value: string, start: number, end: number): ComposerEdit {
  const from = value.lastIndexOf('\n', start - 1) + 1;
  // A selection that ends at the start of a line does not take that line.
  const last = end > start && value[end - 1] === '\n' ? end - 1 : end;
  const newline = value.indexOf('\n', last);
  const to = newline < 0 ? value.length : newline;
  const body = value.slice(from, to);
  // The fence starts on a line of its own, as `message-text.tsx` requires.
  const text = '```\n' + body + '\n```';
  return {
    from,
    to,
    text,
    selectionStart: from + 4,
    selectionEnd: from + 4 + body.length,
    removed: false,
  };
}

/** The `@name` being typed at the caret: its range and the text after `@`. */
export interface MentionToken {
  start: number;
  end: number;
  query: string;
}

/**
 * The `@name` that ends at a collapsed caret, read as `message-text.tsx`
 * reads a mention: an `@` that does not continue a word, a name or an email
 * address. The caret must be at the end of the word, so completing never
 * splits text after it. `null` when there is none.
 */
export function mentionToken(
  value: string,
  start: number,
  end: number,
): MentionToken | null {
  if (start !== end) return null;
  const match = /(?<![\p{L}\p{M}\p{N}_@.])@([\p{L}\p{M}\p{N}._-]*)$/u.exec(
    value.slice(0, start),
  );
  if (!match) return null;
  if (start < value.length && !/\s/u.test(value[start])) return null;
  return {
    start: start - match[1].length - 1,
    end: start,
    query: match[1],
  };
}

/** Replaces the typed `@name` with the chosen username and a space. */
export function mentionEdit(
  token: MentionToken,
  username: string,
): ComposerEdit {
  const text = `@${username} `;
  return {
    from: token.start,
    to: token.end,
    text,
    selectionStart: token.start + text.length,
    selectionEnd: token.start + text.length,
    removed: false,
  };
}

/**
 * The share of the text limit at which the composer starts stating the space
 * left in the message.
 */
export const TEXT_BUDGET_SHOWN = 0.9;

export interface TextBudget {
  over: boolean;
  /** What the composer draws: `3.3 KB left` or `120 bytes over the limit`. */
  label: string;
  /** The exact count, for the tooltip. */
  detail: string;
}

function bytesLabel(bytes: number): string {
  // fmtSize writes "B"; below a kilobyte the unit is spelled out.
  return bytes < 1000
    ? `${bytes} ${bytes === 1 ? 'byte' : 'bytes'}`
    : fmtSize(bytes);
}

/**
 * The space left in a message of `bytes` UTF-8 bytes against `limit`, once
 * it has used {@link TEXT_BUDGET_SHOWN} of it; `null` before then. The count
 * is in bytes because the limit is: a character takes one to four bytes, so
 * no character count can be derived from it.
 */
export function textBudget(bytes: number, limit: number): TextBudget | null {
  if (bytes < limit * TEXT_BUDGET_SHOWN) return null;
  const left = limit - bytes;
  const total = limit.toLocaleString('en-US');
  return left >= 0
    ? {
        over: false,
        label: `${bytesLabel(left)} left`,
        detail: `${left.toLocaleString('en-US')} of ${total} bytes left. Accented letters, symbols and emoji take 2 to 4 bytes each.`,
      }
    : {
        over: true,
        label: `${bytesLabel(-left)} over the limit`,
        detail: `${(-left).toLocaleString('en-US')} bytes over the ${total}-byte limit. Accented letters, symbols and emoji take 2 to 4 bytes each.`,
      };
}

/**
 * Whether the platform's command modifier is ⌘ (Meta) rather than Ctrl, as
 * the formatting keys read it.
 */
export function applePlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /Mac|iPhone|iPad/i.test(
    `${navigator.platform} ${navigator.userAgent}`,
  );
}

/** A key with the platform's command modifier, as key labels write it. */
export function commandKey(key: string, shift = false): string {
  return applePlatform()
    ? `${shift ? '⇧' : ''}⌘${key}`
    : `Ctrl+${shift ? 'Shift+' : ''}${key}`;
}
