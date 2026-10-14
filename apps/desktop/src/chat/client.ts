import { normalizeMutationError } from '../bridge';
import { CHAT_ACTIONS, chatActionMutates } from '../chat-contract';
import {
  scheduleProfileWork,
  type BackgroundHistoryWork,
} from '../scheduling/profile-work';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatReply,
  ChatResult,
  ChatScope,
} from '../chat-contract';
import { submissionId } from './actions';

import { cancelled, integrity } from './errors';
export { cancelled, integrity, channelIntegrity } from './errors';
import { sameScope } from './scope';
export { sameScope } from './scope';

type ResultFor<A extends ChatAction> = Extract<
  ChatResult,
  { kind: (typeof CHAT_ACTIONS)[A['action']]['result'] }
>;
export type ReplyFor<A extends ChatAction> = Omit<ChatReply, 'result'> & {
  result: ResultFor<A>;
};
export type ChatWorkPriority = 'foreground' | 'background';

/** One request lifetime. Disposing cannot cancel a different owner or a later generation. */
export function chatClient(bridge: Bridge, profile: string, storeId: string) {
  const view = submissionId();
  let closed = false;
  const lifetime = new AbortController();
  let scope: ChatScope | null = null;
  const request = async <A extends ChatAction>(
    action: A,
    background?: BackgroundHistoryWork,
    authorize?: (phase: 'before' | 'after') => void,
    priority: ChatWorkPriority = 'foreground',
  ): Promise<ReplyFor<A>> => {
    if (background && action.action !== 'notification-history')
      throw integrity(
        'Only notification history may use background scheduling.',
      );
    const work = async () => {
      if (closed) throw cancelled();
      authorize?.('before');
      const reply = await bridge
        .chat(storeId, action, view)
        .catch((cause: unknown) => {
          throw chatActionMutates(action)
            ? normalizeMutationError(cause)
            : cause;
        });
      if (closed) throw cancelled();
      authorize?.('after');
      if (scope && !sameScope(scope, reply.scope)) throw integrity();
      if (reply.result.kind !== CHAT_ACTIONS[action.action].result)
        throw integrity('Unexpected chat response.');
      scope = reply.scope;
      // Both scope and the action/result correlation have been checked above.
      return reply as ReplyFor<A>;
    };
    const scheduling =
      background ??
      (priority === 'background' || action.action === 'sync-inbox'
        ? {
            key: JSON.stringify([storeId, action.action]),
            owner: {},
            generation: 0,
            signal: lifetime.signal,
            current: () => !closed,
            cancel: () => {},
            preemptible: false as const,
          }
        : undefined);
    // The chat lane, so a request does not wait behind a catalog walk or an
    // invitation read for the same profile. Chat requests still serialize
    // against each other here, and against everything else in the agent.
    return action.action === 'poll-inbox'
      ? work()
      : scheduleProfileWork(
          bridge,
          profile,
          work,
          scheduling,
          `chat:${action.action}`,
          'chat',
        );
  };
  return {
    request,
    async run<A extends ChatAction>(action: A): Promise<ResultFor<A>> {
      return (await request(action)).result;
    },
    dispose() {
      if (closed) return;
      closed = true;
      lifetime.abort();
      void bridge.cancelChat(view).catch(() => {});
    },
  };
}
