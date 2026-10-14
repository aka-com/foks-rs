import type { ChatAction, ChatOperation, ChatReply } from '../../chat-contract';
import { failure } from '../actions';
import { REQUEST_RETRY, retryDelay } from '../backoff';
import {
  awaitingAccess,
  intentHeld,
  operationOf,
  running,
  type OutgoingMessage,
} from './outgoing';
import type { Team } from './team';

/**
 * One request or drive a recovery may run. `key` names it across
 * recoveries, so its failures back off independently of the other jobs.
 */
export type RecoveryJob =
  /** Read the state of an operation whose outcome is unknown. */
  | {
      key: string;
      kind: 'status';
      operation: string;
      action: 'reconcile' | 'status';
    }
  /** Read back the text of a sent message that no row holds. */
  | { key: string; kind: 'body'; operation: string; channel: string }
  /**
   * Finalize a settled operation the agent reported for cleanup, first
   * reading back the text of a rejected one that no row holds.
   */
  | { key: string; kind: 'cleanup'; operation: ChatOperation; body: boolean }
  /** Clear the saved copy of a message that already has an operation. */
  | { key: string; kind: 'intent'; message: OutgoingMessage }
  /** Drive a saved or access-paused message until it has an operation. */
  | { key: string; kind: 'send'; message: OutgoingMessage };

/** How many jobs one recovery runs at most. */
const RECOVERY_JOBS = 16;

/**
 * The jobs the team has now. `obligations` are the settled operations the
 * agent reported for cleanup that have not been finalized, and
 * `deliverable(channel)` says whether chat is available and the channel
 * readable. Operations and messages a drive is working on are left to that
 * drive.
 */
export function planRecoveryJobs(
  team: Pick<Team, 'operations' | 'messages'>,
  obligations: ReadonlyMap<string, ChatOperation>,
  deliverable: (channel: string) => boolean,
): RecoveryJob[] {
  const jobs: RecoveryJob[] = [];
  for (const op of team.operations) {
    if (team.messages.some((m) => operationOf(m)?.id === op.id && running(m)))
      continue;
    if (op.statusUnknown || op.state === 'uncertain')
      jobs.push({
        key: `status:${op.id}`,
        kind: 'status',
        operation: op.id,
        action: deliverable(op.channel) ? 'reconcile' : 'status',
      });
    const message = team.messages.find((m) => operationOf(m)?.id === op.id);
    if (
      op.kind === 'send-message' &&
      !op.observed &&
      !op.bodyUnavailable &&
      op.text === undefined &&
      message?.text === undefined &&
      deliverable(op.channel)
    )
      jobs.push({
        key: `body:${op.id}`,
        kind: 'body',
        operation: op.id,
        channel: op.channel,
      });
  }
  for (const op of obligations.values())
    jobs.push({
      key: `cleanup:${op.id}`,
      kind: 'cleanup',
      operation: op,
      body:
        op.state === 'rejected' &&
        !team.messages.some((m) => operationOf(m)?.id === op.id) &&
        deliverable(op.channel),
    });
  for (const message of team.messages) {
    if (running(message)) continue;
    if (operationOf(message) && intentHeld(message))
      jobs.push({ key: `intent:${message.id}`, kind: 'intent', message });
    if (
      message.automatic &&
      ((!operationOf(message) && intentHeld(message)) ||
        awaitingAccess(message)) &&
      deliverable(message.channel)
    )
      jobs.push({ key: `send:${message.id}`, kind: 'send', message });
  }
  return jobs;
}

/**
 * The job to run next: of the jobs not yet run in this recovery and whose
 * backoff has passed, the one that has been due longest.
 */
export function nextRecoveryJob(
  jobs: readonly RecoveryJob[],
  checks: Team['checks'],
  visited: ReadonlySet<string>,
  now: number,
): RecoveryJob | undefined {
  const due = (job: RecoveryJob) => checks.get(job.key)?.due ?? 0;
  return jobs
    .filter((job) => !visited.has(job.key) && due(job) <= now)
    .sort((a, b) => due(a) - due(b))[0];
}

/**
 * Runs `work` unless the job's backoff has not passed, and records when it
 * may run again: after the base interval on success, after a growing delay
 * on failure.
 */
export async function runChecked(
  checks: Team['checks'],
  key: string,
  now: () => number,
  work: () => Promise<unknown>,
) {
  const record = checks.get(key);
  if (record && record.due > now()) return;
  let failed = false;
  try {
    await work();
  } catch {
    failed = true;
  }
  const failures = failed ? Math.min((record?.failures ?? 0) + 1, 6) : 0;
  checks.set(key, {
    failures,
    due:
      now() +
      (failed ? retryDelay(failures, REQUEST_RETRY) : REQUEST_RETRY.base),
  });
}

/** Removes the backoff of jobs whose operation or message is gone. */
export function pruneChecks(
  team: Pick<Team, 'checks' | 'operations' | 'messages'>,
) {
  const live = new Set([
    ...team.operations.map((op) => op.id),
    ...team.messages.map((m) => m.id),
  ]);
  for (const key of team.checks.keys())
    if (!live.has(key.slice(key.indexOf(':') + 1))) team.checks.delete(key);
}

/** What a recovery needs from the send service. */
export interface RecoveryHost {
  /** The team is still the one the service holds under `epoch`. */
  current(epoch: number): boolean;
  /** Chat is available and the channel readable. */
  deliverable(channel: string): boolean;
  now(): number;
  request(action: ChatAction): Promise<ChatReply>;
  /** Starts the channel's next queued message if the channel is free. */
  drain(channel: string): void;
  drive(message: OutgoingMessage): Promise<void>;
  clearIntent(message: OutgoingMessage): Promise<void>;
  publish(): void;
}

/**
 * One recovery of a team: read the agent's pending operations and the
 * settled operations it holds for cleanup, then run up to
 * {@link RECOVERY_JOBS} jobs, planning again before each one, since every
 * job can change what the others have to do.
 */
export async function recover(team: Team, host: RecoveryHost) {
  const epoch = team.epoch;
  if (!host.current(epoch)) return;
  await host.request({ action: 'pending' });
  if (!host.current(epoch)) return;
  const cleanup = await host.request({ action: 'cleanup-pending' });
  if (!host.current(epoch)) return;
  const obligations = new Map(
    cleanup.result.kind === 'cleanup-pending'
      ? cleanup.result.operations.map((op) => [op.id, op])
      : [],
  );
  const visited = new Set<string>();
  for (let count = 0; count < RECOVERY_JOBS && host.current(epoch); count++) {
    // A queue whose head settled between publications still has to move;
    // the drain is a no-op for a channel that is already working.
    for (const channel of new Set(
      team.messages
        .filter((m) => m.delivery.stage === 'queued')
        .map((m) => m.channel),
    ))
      host.drain(channel);
    const jobs = planRecoveryJobs(team, obligations, (channel) =>
      host.deliverable(channel),
    );
    // What the tick still has to come back for. A job whose check is not
    // due yet is work in hand, not an idle team.
    team.jobs = jobs.length > 0;
    const job = nextRecoveryJob(jobs, team.checks, visited, host.now());
    if (!job) break;
    visited.add(job.key);
    await runChecked(
      team.checks,
      job.key,
      () => host.now(),
      () => run(team, job, obligations, host),
    );
  }
  pruneChecks(team);
  if (host.current(epoch)) {
    team.recovered = true;
    host.publish();
  }
}

/** Runs one job. A job that leaves its work unfinished rejects. */
async function run(
  team: Team,
  job: RecoveryJob,
  obligations: Map<string, ChatOperation>,
  host: RecoveryHost,
) {
  switch (job.kind) {
    case 'status':
      await host.request({ action: job.action, operation: job.operation });
      return;
    case 'body':
      await host.request({
        action: 'operation-body',
        operation: job.operation,
        channel: job.channel,
      });
      return;
    case 'cleanup': {
      const op = job.operation;
      try {
        if (job.body)
          await host.request({
            action: 'operation-body',
            operation: op.id,
            channel: op.channel,
          });
        await host.request({ action: 'finalize', operation: op.id });
        obligations.delete(op.id);
        if (!obligations.size) team.cleanupError = '';
      } catch (error) {
        team.cleanupError = failure(error);
        throw error;
      }
      return;
    }
    case 'intent':
      await host.clearIntent(job.message);
      if (intentHeld(job.message))
        throw new Error('Local intent cleanup is pending.');
      return;
    case 'send':
      await host.drive(job.message);
      if (!operationOf(job.message))
        throw new Error('Preparation is still pending.');
      return;
  }
}
