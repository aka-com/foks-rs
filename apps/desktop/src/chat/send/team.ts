import type { ChatMessage, ChatReply, ChatScope } from '../../chat-contract';
import type { chatClient } from '../client';
import type { TrackedOperation } from '../operations';
import { move, type OutgoingMessage } from './outgoing';

/**
 * Everything the send service holds for one team identity. The gateway,
 * recovery and scheduler modules read and update it; only the service
 * creates and retires one.
 */
export interface Team {
  storeId: string;
  profile: string;
  scope: ChatScope | null;
  client: ReturnType<typeof chatClient>;
  /**
   * Incremented when the team is retired. Work started under an earlier
   * epoch finds it changed after its awaits and stops.
   */
  epoch: number;
  blocked: boolean;
  operations: TrackedOperation[];
  observations: Map<string, ChatMessage & { channel: string }>;
  messages: OutgoingMessage[];
  loaded: Set<string>;
  loading: Map<string, Promise<void>>;
  loadErrors: Map<string, string>;
  /** The next allowed run and the failure count of each recovery job. */
  checks: Map<string, { due: number; failures: number }>;
  /** Coalesced requests in flight, keyed by action. */
  requests: Map<string, Promise<ChatReply>>;
  /** Saved-copy clears in flight, keyed by message ID. */
  clears: Map<string, Promise<void>>;
  refresh?: Promise<void>;
  /** When the scheduler may next start a background pass for the team. */
  due: number;
  /**
   * Whether a recovery has completed for this identity. Only the agent's
   * pending list can say whether it holds operations from an earlier
   * session, so a team that has not run one yet always has work.
   */
  recovered: boolean;
  /** Whether the last recovery still had jobs to run when it stopped. */
  jobs: boolean;
  cleanupError: string;
}

export function newTeam(input: {
  storeId: string;
  profile: string;
  scope: ChatScope | null;
  client: ReturnType<typeof chatClient>;
}): Team {
  return {
    ...input,
    epoch: 0,
    blocked: false,
    operations: [],
    observations: new Map(),
    messages: [],
    loaded: new Set(),
    loading: new Map(),
    loadErrors: new Map(),
    checks: new Map(),
    requests: new Map(),
    clears: new Map(),
    due: 0,
    recovered: false,
    jobs: false,
    cleanupError: '',
  };
}

/**
 * Ends the team's current identity: the service stopped, or the team was
 * removed, replaced or rejected. Work in flight finds a new epoch and stops,
 * the client cancels its requests, every outgoing message is discarded with
 * its text, and everything the team learned about operations and channels
 * is dropped. A retired team stays blocked: the service either drops it or
 * keeps it, and a kept team refuses further work until the service stops.
 * The team's drafts live in the draft store; the caller removes them.
 */
export function retire(team: Team) {
  team.epoch++;
  team.blocked = true;
  team.client.dispose();
  for (const message of team.messages) move(message, { type: 'retired' });
  team.messages = [];
  team.operations = [];
  team.observations.clear();
  team.loaded.clear();
  team.loadErrors.clear();
  team.checks.clear();
  team.recovered = false;
  team.jobs = false;
  team.cleanupError = '';
}
