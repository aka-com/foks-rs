import { normalizeCommandError } from '../../bridge';
import type { ChatAction, ChatReply, ChatScope } from '../../chat-contract';
import { CHAT_ACTIONS } from '../../chat-contract';
import { channelIntegrity, integrity, type ChatWorkPriority } from '../client';
import { accessChanged, cancelled } from '../errors';
import { sameScope } from '../scope';
import type { Team } from './team';

/** What the send service knows about access when a request is authorized. */
export interface TeamAccess {
  /**
   * The service is running, the team is the one it holds under `epoch` (its
   * current epoch when omitted), and the team is not blocked.
   */
  current(team: Team, epoch?: number): boolean;
  /** Chat is available for the team's store under the agent's snapshot. */
  available(team: Team): boolean;
  /** The inbox lists the channel as readable and has not blocked it. */
  readable(team: Team, channel: string): boolean;
  /** The inbox lists the channel as writable. */
  writable(team: Team, channel: string): boolean;
  /** The inbox has blocked the channel after an integrity failure. */
  channelBlocked(team: Team, channel: string): boolean;
  /**
   * The profile's access generation, which advances when the profile's
   * access identity changes or its lease expires.
   */
  generation(profile: string): number;
  /** The agent reports itself ready. */
  agentReady(): boolean;
}

export interface GatewayHost extends TeamAccess {
  inbox: {
    /** The scope the inbox verified for the store, if it has one. */
    scope(storeId: string): ChatScope | undefined;
    handleError(storeId: string, error: unknown, channel?: string): unknown;
    invalidate(storeId: string): void;
  };
  /** Reconciles a reply whose scope has been checked into the team. */
  apply(team: Team, reply: ChatReply, action: ChatAction): void;
  /** Records a failed request of a current team, after the inbox has seen it. */
  failed(team: Team, action: ChatAction): void;
}

/**
 * Shares one run of `start` among callers that ask under the same key while
 * it is in flight. The entry is removed when the run settles.
 */
export function coalesce<T>(
  inflight: Map<string, Promise<T>>,
  key: string,
  start: () => Promise<T>,
): Promise<T> {
  const current = inflight.get(key);
  if (current) return current;
  const work = start();
  inflight.set(key, work);
  void work
    .finally(() => {
      if (inflight.get(key) === work) inflight.delete(key);
    })
    .catch(() => {});
  return work;
}

/** The channel an action addresses, directly or through its operation. */
export function actionChannel(
  team: Team,
  action: ChatAction,
): string | undefined {
  return 'channel' in action
    ? action.channel
    : 'operation' in action
      ? team.operations.find((op) => op.id === action.operation)?.channel
      : undefined;
}

/**
 * The check the chat client runs before a request is sent and after its
 * reply arrives. It fails when the team was retired or blocked; when a
 * non-local action finds chat unavailable or the profile's access
 * generation changed since `generation`, or a submit finds its channel no
 * longer readable and writable; when the agent is not ready; and when a
 * non-local action addresses a channel the inbox has blocked.
 */
export function authorizer(
  access: TeamAccess,
  team: Team,
  action: ChatAction,
  epoch: number,
  generation: number,
  channel: string | undefined,
): (phase: 'before' | 'after') => void {
  const local = CHAT_ACTIONS[action.action].local;
  return (phase) => {
    if (!access.current(team, epoch)) throw cancelled();
    if (
      !local &&
      (!access.available(team) ||
        generation !== access.generation(team.profile) ||
        (phase === 'before' &&
          action.action === 'submit-message' &&
          (!access.readable(team, action.channel) ||
            !access.writable(team, action.channel))))
    )
      throw accessChanged(
        'Chat access changed. Saved work is kept until access returns.',
        phase,
      );
    if (!access.agentReady()) throw cancelled();
    if (channel && !local && access.channelBlocked(team, channel))
      throw channelIntegrity();
  };
}

/**
 * Every chat request the send service and the conversation make for a team
 * goes through here: identical requests of a coalescing action share one
 * run, each request is authorized against current access before it is sent
 * and after it returns, the reply's scope is checked against the team's and
 * the inbox's, and the reply is reconciled into the team before the caller
 * sees it.
 */
export class ChatRequestGateway {
  constructor(private host: GatewayHost) {}

  request(
    team: Team,
    action: ChatAction,
    priority: ChatWorkPriority,
  ): Promise<ChatReply> {
    if (!CHAT_ACTIONS[action.action].coalesce)
      return this.perform(team, action, priority);
    return coalesce(team.requests, JSON.stringify(action), () =>
      this.perform(team, action, priority),
    );
  }

  private async perform(
    team: Team,
    action: ChatAction,
    priority: ChatWorkPriority,
  ): Promise<ChatReply> {
    const host = this.host;
    const epoch = team.epoch;
    const channel = actionChannel(team, action);
    const authorize = authorizer(
      host,
      team,
      action,
      epoch,
      host.generation(team.profile),
      channel,
    );
    try {
      const reply = await team.client.request(
        action,
        undefined,
        authorize,
        priority,
      );
      if (!host.current(team, epoch)) throw cancelled();
      const trusted = host.inbox.scope(team.storeId);
      if (
        (team.scope && !sameScope(team.scope, reply.scope)) ||
        (trusted && !sameScope(trusted, reply.scope))
      )
        throw integrity();
      team.scope = reply.scope;
      host.apply(team, reply, action);
      return reply;
    } catch (error) {
      if (host.current(team, epoch)) {
        host.inbox.handleError(team.storeId, error, channel);
        // Immediately invalidate the team after an interactive send is denied,
        // matching conversation-history reads. Denials found by background
        // synchronization retain the inbox retry delay.
        if (normalizeCommandError(error).code === 'chat-access-denied')
          host.inbox.invalidate(team.storeId);
        host.failed(team, action);
      }
      throw error;
    }
  }
}
