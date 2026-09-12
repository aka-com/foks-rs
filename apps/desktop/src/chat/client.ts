import { enqueueProfileWork } from '../bridge';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatReply,
  ChatResult,
  ChatScope,
} from '../chat-contract';
import { submissionId } from './actions';

export const cancelled = () =>
  Object.assign(new Error('Conversation closed.'), {
    code: 'cancelled',
    message: 'Conversation closed.',
    retryable: false,
    fatal: false,
    ambiguous: false,
  });
export const integrity = (message = 'The chat identity changed.') => ({
  code: 'chat-integrity',
  message,
  retryable: false,
  fatal: true,
  ambiguous: false,
});
export const channelIntegrity = (
  message = 'Channel content could not be verified.',
) => ({
  code: 'chat-channel-integrity',
  message,
  retryable: false,
  fatal: true,
  ambiguous: false,
});
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
} as const;
type ResultFor<A extends ChatAction> = Extract<
  ChatResult,
  { kind: (typeof kinds)[A['action']] }
>;

/** One request lifetime. Disposing cannot cancel a different owner or a later generation. */
export function chatClient(bridge: Bridge, profile: string, storeId: string) {
  const view = submissionId();
  let closed = false;
  let scope: ChatScope | null = null;
  const request = async (action: ChatAction): Promise<ChatReply> => {
    const work = async () => {
      if (closed) throw cancelled();
      const reply = await bridge.chat(storeId, action, view);
      if (closed) throw cancelled();
      if (scope && !sameScope(scope, reply.scope)) throw integrity();
      if (reply.result.kind !== kinds[action.action])
        throw integrity('Unexpected chat response.');
      scope = reply.scope;
      return reply;
    };
    return action.action === 'poll-inbox'
      ? work()
      : enqueueProfileWork(bridge, profile, work);
  };
  return {
    request,
    async run<A extends ChatAction>(action: A): Promise<ResultFor<A>> {
      return (await request(action)).result as ResultFor<A>;
    },
    dispose() {
      if (closed) return;
      closed = true;
      void bridge.cancelChat(view).catch(() => {});
    },
  };
}
