import { enqueueProfileWork } from '../bridge';
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
export function sameScope(a: ChatScope, b: ChatScope): boolean {
  return (
    a.host === b.host &&
    a.actor === b.actor &&
    a.store.profile === b.store.profile &&
    a.store.account_alias === b.store.account_alias &&
    a.store.team_alias === b.store.team_alias &&
    a.store.team_id === b.store.team_id
  );
}
const kinds = {
  'operation-body': 'operation-body',
  channels: 'channels',
  history: 'history',
  inbox: 'inbox',
  'sync-inbox': 'inbox',
  'poll-inbox': 'poll',
  'mark-read': 'read',
  pending: 'pending',
  'prepare-channel': 'operation',
  'prepare-message': 'operation',
  status: 'operation',
  attempt: 'operation',
  cancel: 'operation',
  finalize: 'operation',
} as const satisfies Record<ChatAction['action'], ChatResult['kind']>;
type ResultFor<A extends ChatAction> = Extract<
  ChatResult,
  { kind: (typeof kinds)[A['action']] }
>;
export type ReplyFor<A extends ChatAction> = Omit<ChatReply, 'result'> & {
  result: ResultFor<A>;
};

/** One request lifetime. Disposing cannot cancel a different owner or a later generation. */
export function chatClient(bridge: Bridge, profile: string, storeId: string) {
  const view = submissionId();
  let closed = false;
  let scope: ChatScope | null = null;
  const request = async <A extends ChatAction>(
    action: A,
  ): Promise<ReplyFor<A>> => {
    const work = async () => {
      if (closed) throw cancelled();
      const reply = await bridge.chat(storeId, action, view);
      if (closed) throw cancelled();
      if (scope && !sameScope(scope, reply.scope)) throw integrity();
      if (reply.result.kind !== kinds[action.action])
        throw integrity('Unexpected chat response.');
      scope = reply.scope;
      // Both scope and the action/result correlation have been checked above.
      return reply as ReplyFor<A>;
    };
    return action.action === 'poll-inbox'
      ? work()
      : enqueueProfileWork(bridge, profile, work);
  };
  return {
    request,
    async run<A extends ChatAction>(action: A): Promise<ResultFor<A>> {
      return (await request(action)).result;
    },
    dispose() {
      if (closed) return;
      closed = true;
      void bridge.cancelChat(view).catch(() => {});
    },
  };
}
