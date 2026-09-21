/**
 * The text Copy diagnostics appends below the popover's status lines: a
 * timeline of recent events and per-kind summaries. Fixed width, sorted, and
 * pure, so two copies diff cleanly and the layout is held by a test.
 */

import type { TimingEvent, TimingValue } from './log';

export interface FormatOptions {
  /** Milliseconds since the epoch; events older than the window are omitted. */
  now: number;
  /** Lines printed under the heading: version, agent, window state. */
  header?: readonly string[];
  windowMilliseconds?: number;
  /** The output is cut to this many bytes, oldest timeline lines first. */
  maxBytes?: number;
}

const DEFAULT_WINDOW = 15 * 60_000;
const DEFAULT_MAX_BYTES = 512 * 1024;
/** Rows per summary table, the most frequent first. */
const MAX_ROWS = 64;
/**
 * Commands counted in the summaries but left out of the timeline: the
 * once-a-second connection-loss probe and the read the popover itself makes.
 */
const QUIET_COMMANDS: ReadonlySet<string> = new Set([
  'take_agent_connection_loss',
  'diagnostic_timings',
]);
function quiet(event: TimingEvent): boolean {
  return (
    event.name === 'invoke' &&
    QUIET_COMMANDS.has(String(attr(event, 'command')))
  );
}

export function formatMilliseconds(ms: number): string {
  if (!Number.isFinite(ms)) return '—';
  if (ms < 1_000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(2)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function clock(at: number): string {
  return new Date(at).toISOString().slice(11, 23);
}

function pad(value: string, width: number): string {
  return value.length >= width
    ? value
    : value + ' '.repeat(width - value.length);
}

function percentile(sorted: readonly number[], fraction: number): number {
  if (!sorted.length) return NaN;
  const index = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(sorted.length * fraction) - 1),
  );
  return sorted[index];
}

function attr(event: TimingEvent, key: string): TimingValue | undefined {
  return event.attrs?.[key];
}

function detail(event: TimingEvent): string {
  const parts: string[] = [];
  if (event.phase) parts.push(event.phase);
  if (event.outcome && event.outcome !== 'ok') parts.push(event.outcome);
  if (event.ms !== undefined) parts.push(formatMilliseconds(event.ms));
  if (event.code) parts.push(`code=${event.code}`);
  for (const [key, value] of Object.entries(event.attrs ?? {})) {
    if (
      typeof value === 'number' &&
      /(^|_)ms$|^(queue|pool|start|session|lock|body|late|wait|prepare|rescope)$/.test(
        key,
      )
    )
      parts.push(`${key}=${formatMilliseconds(value)}`);
    else parts.push(`${key}=${String(value)}`);
  }
  return parts.join(' ');
}

function timelineLine(event: TimingEvent): string {
  const indent = event.layer === 'renderer' ? '' : '  ';
  const label = `${indent}${event.name}`;
  const where = [event.scope, event.id].filter(Boolean).join(' · ');
  return `${clock(event.at)}  ${pad(label, 18)} ${pad(where, 24)} ${detail(event)}`.trimEnd();
}

interface Stat {
  n: number;
  ok: number;
  error: number;
  values: number[];
  last?: number;
  phases: Map<string, number[]>;
  flags: Map<string, { yes: number; n: number }>;
  labels: Map<string, number>;
}
function stat(): Stat {
  return {
    n: 0,
    ok: 0,
    error: 0,
    values: [],
    phases: new Map(),
    flags: new Map(),
    labels: new Map(),
  };
}
function add(into: Map<string, Stat>, key: string, event: TimingEvent): Stat {
  let entry = into.get(key);
  if (!entry) into.set(key, (entry = stat()));
  entry.n++;
  if (event.outcome === 'ok') entry.ok++;
  else if (event.outcome && event.outcome !== 'retired') entry.error++;
  if (event.ms !== undefined) {
    entry.values.push(event.ms);
    entry.last = event.ms;
  }
  for (const [name, value] of Object.entries(event.attrs ?? {})) {
    if (typeof value === 'number') {
      let series = entry.phases.get(name);
      if (!series) entry.phases.set(name, (series = []));
      series.push(value);
    } else if (typeof value === 'boolean') {
      let flag = entry.flags.get(name);
      if (!flag) entry.flags.set(name, (flag = { yes: 0, n: 0 }));
      flag.n++;
      if (value) flag.yes++;
    } else if (name === 'waited') {
      entry.labels.set(value, (entry.labels.get(value) ?? 0) + 1);
    }
  }
  return entry;
}
function p(values: readonly number[], fraction: number): string {
  return formatMilliseconds(
    percentile(
      [...values].sort((a, b) => a - b),
      fraction,
    ),
  );
}

/** The most frequent entries first, so a capped table keeps what matters. */
function frequent(stats: Map<string, Stat>): [string, Stat][] {
  return [...stats].sort((a, b) => b[1].n - a[1].n).slice(0, MAX_ROWS);
}

function table(
  title: string,
  columns: readonly string[],
  rows: readonly (readonly string[])[],
): string[] {
  if (!rows.length) return [];
  const widths = columns.map((column, index) =>
    Math.max(column.length, ...rows.map((row) => row[index]?.length ?? 0)),
  );
  const line = (cells: readonly string[]) =>
    cells
      .map((cell, index) => pad(cell, widths[index]))
      .join('  ')
      .trimEnd();
  return [`--- ${title} ---`, line(columns), ...rows.map(line), ''];
}

/** Recent events as text. Times are UTC so copies from two machines compare. */
export function formatTimings(
  events: readonly TimingEvent[],
  options: FormatOptions,
): string {
  const window = options.windowMilliseconds ?? DEFAULT_WINDOW;
  const maxBytes = options.maxBytes ?? DEFAULT_MAX_BYTES;
  const recent = events
    .filter((event) => options.now - event.at <= window)
    .sort((a, b) => a.at - b.at);
  const counts = { renderer: 0, backend: 0, agent: 0 };
  for (const event of recent) {
    counts[event.layer]++;
    // The agent's phases ride inside the backend's operation events.
    if (event.name === 'agent.op' && attr(event, 'body') !== undefined)
      counts.agent++;
  }
  const quieted = recent.filter(quiet).length;
  const heading = [
    `--- timing (last ${Math.round(window / 60_000)} min, ${recent.length} events, renderer ${counts.renderer} · backend ${counts.backend} · agent-timed ${counts.agent}, times UTC) ---`,
    ...(options.header ?? []),
    ...(quieted
      ? [
          `(${quieted} ${[...QUIET_COMMANDS].join(' and ')} calls counted below, not listed)`,
        ]
      : []),
    '',
  ];
  const timeline = recent.filter((event) => !quiet(event)).map(timelineLine);

  const jobs = new Map<string, Stat>();
  const steps = new Map<string, Stat>();
  const commands = new Map<string, Stat>();
  const ops = new Map<string, Stat>();
  const chat = new Map<string, Stat>();
  for (const event of recent) {
    if (event.ms === undefined) continue;
    if (event.name.startsWith('job.'))
      add(jobs, `${event.name.slice(4)}\t${event.scope ?? '—'}`, event);
    else if (event.name === 'invoke') {
      const command = String(attr(event, 'command') ?? '?');
      // A long-poll's wait is its purpose, not its cost.
      if (attr(event, 'action') !== 'poll-inbox') add(commands, command, event);
    } else if (event.name === 'agent.op') {
      const op = String(attr(event, 'op') ?? '?');
      // The long-poll's wait is its purpose, not its cost.
      if (op !== 'Chat/PollInbox') add(ops, op, event);
    } else if (event.name.startsWith('chat.')) add(chat, event.name, event);
    else add(steps, `${event.name}\t${event.scope ?? '—'}`, event);
  }
  const summaries: string[] = [
    ...table(
      'jobs',
      ['kind', 'scope', 'runs', 'ok', 'fail', 'p50', 'p95', 'max', 'last'],
      frequent(jobs).map(([key, entry]) => {
        const [kind, scope] = key.split('\t');
        return [
          kind,
          scope,
          String(entry.n),
          String(entry.ok),
          String(entry.error),
          p(entry.values, 0.5),
          p(entry.values, 0.95),
          p(entry.values, 1),
          formatMilliseconds(entry.last ?? NaN),
        ];
      }),
    ),
    ...table(
      'steps',
      ['step', 'scope', 'n', 'ok', 'fail', 'p50', 'p95', 'max'],
      frequent(steps).map(([key, entry]) => {
        const [name, scope] = key.split('\t');
        return [
          name,
          scope,
          String(entry.n),
          String(entry.ok),
          String(entry.error),
          p(entry.values, 0.5),
          p(entry.values, 0.95),
          p(entry.values, 1),
        ];
      }),
    ),
    ...table(
      'commands (renderer → backend, poll-inbox excluded)',
      ['command', 'n', 'ok', 'fail', 'p50', 'p95', 'max'],
      frequent(commands).map(([command, entry]) => [
        command,
        String(entry.n),
        String(entry.ok),
        String(entry.error),
        p(entry.values, 0.5),
        p(entry.values, 0.95),
        p(entry.values, 1),
      ]),
    ),
    ...table(
      'agent ops (via backend, Chat/PollInbox excluded)',
      [
        'op',
        'n',
        'fail',
        'p50',
        'p95',
        'queue p50',
        'lock p50',
        'body p50',
        'auth hit',
        'report hit',
        'waited behind',
      ],
      frequent(ops).map(([op, entry]) => {
        const hit = (name: string) => {
          const flag = entry.flags.get(name);
          return flag && flag.n
            ? `${Math.round((100 * flag.yes) / flag.n)}%`
            : '—';
        };
        const phase = (name: string) => {
          const series = entry.phases.get(name);
          return series ? p(series, 0.5) : '—';
        };
        return [
          op,
          String(entry.n),
          String(entry.error),
          p(entry.values, 0.5),
          p(entry.values, 0.95),
          phase('queue'),
          phase('lock'),
          phase('body'),
          hit('auth'),
          hit('report'),
          [...entry.labels].map(([label, n]) => `${label} ×${n}`).join(', ') ||
            '—',
        ];
      }),
    ),
  ];
  const chatLines: string[] = [];
  const chatStat = (name: string) => chat.get(name);
  const polls = chatStat('chat.poll');
  const syncs = chatStat('chat.sync');
  const arrivals = chatStat('chat.arrival');
  const sends = chatStat('chat.send');
  const history = chatStat('chat.history');
  const notify = chatStat('chat.notify');
  const frames = chatStat('chat.frame');
  if (polls)
    chatLines.push(
      `polls ${polls.n} (bumped ${polls.flags.get('bumped')?.yes ?? 0}, failed ${polls.error}) · wait p50 ${p(polls.values, 0.5)}`,
    );
  if (syncs)
    chatLines.push(
      `syncs ${syncs.n} (changed ${syncs.flags.get('changed')?.yes ?? 0}, failed ${syncs.error}) · p50 ${p(syncs.values, 0.5)} p95 ${p(syncs.values, 0.95)}`,
    );
  if (arrivals)
    chatLines.push(
      `arrivals ${arrivals.n} · bump→publish p50 ${p(arrivals.values, 0.5)} p95 ${p(arrivals.values, 0.95)}`,
    );
  if (frames)
    chatLines.push(
      `frames ${frames.n} · publish→frame p50 ${p(frames.values, 0.5)} p95 ${p(frames.values, 0.95)}`,
    );
  if (sends) {
    const byStep = new Map<string, number[]>();
    for (const event of recent)
      if (event.name === 'chat.send' && event.phase && event.ms !== undefined) {
        let series = byStep.get(event.phase);
        if (!series) byStep.set(event.phase, (series = []));
        series.push(event.ms);
      }
    chatLines.push(
      `send steps ${[...byStep]
        .map(
          ([step, values]) =>
            `${step} p50 ${p(values, 0.5)} (${values.length})`,
        )
        .join(' · ')}`,
    );
  }
  if (history) {
    const rows = history.phases.get('rows') ?? [];
    chatLines.push(
      `history loads ${history.n} (failed ${history.error}) · p50 ${p(history.values, 0.5)} p95 ${p(history.values, 0.95)}${
        rows.length
          ? ` · rows p50 ${Math.round(
              percentile(
                [...rows].sort((a, b) => a - b),
                0.5,
              ),
            )}`
          : ''
      }`,
    );
  }
  if (notify)
    chatLines.push(
      `notification passes ${notify.n} (failed ${notify.error}) · p50 ${p(notify.values, 0.5)} p95 ${p(notify.values, 0.95)}`,
    );
  if (chatLines.length) summaries.push('--- chat ---', ...chatLines, '');

  // The timeline gives way first, oldest lines first; the summaries are cut
  // from their end only when they alone exceed the budget.
  let footerLines = summaries;
  const headingBytes = byteLength(heading.join('\n'));
  while (
    footerLines.length &&
    headingBytes + byteLength(footerLines.join('\n')) + 2 > maxBytes
  )
    footerLines = footerLines.slice(0, -1);
  const footer = footerLines.join('\n').trimEnd();
  const assemble = (kept: readonly string[], dropped: number): string =>
    [
      ...heading,
      ...(dropped ? [`(${dropped} older lines omitted)`] : []),
      ...kept,
      ...(kept.length ? [''] : []),
      footer,
    ]
      .join('\n')
      .trimEnd();
  let kept = timeline;
  let dropped = 0;
  let text = assemble(kept, dropped);
  while (kept.length && byteLength(text) > maxBytes) {
    kept = kept.slice(1);
    dropped++;
    text = assemble(kept, dropped);
  }
  return text;
}

function byteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}
