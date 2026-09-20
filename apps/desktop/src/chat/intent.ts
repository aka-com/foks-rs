import type { Bridge } from '../bridge';
import type { ChatScope } from '../chat-contract';
import { chatClient, integrity, type ChatWorkPriority } from './client';
import { channelWorkKey, sameScope } from './scope';

export type ChatIntent = { submission: string; text: string };
export interface ChatIntentPersistence {
  key: string;
  load(): Promise<ChatIntent | undefined>;
  save(intent: ChatIntent): Promise<void>;
  clear(submission: string): Promise<void>;
}

export function chatIntentPersistence(
  bridge: Bridge,
  storeId: string,
  scope: ChatScope,
  channel: string,
  owner?: ReturnType<typeof chatClient>,
  priority: ChatWorkPriority = 'foreground',
): ChatIntentPersistence {
  const target = { host: scope.host, actor: scope.actor, channel };
  const run = async (
    action:
      | ({ action: 'load-intent' } & typeof target)
      | ({ action: 'save-intent' } & typeof target & ChatIntent)
      | ({ action: 'clear-intent'; submission: string } & typeof target),
  ) => {
    const client = owner ?? chatClient(bridge, scope.store.profile, storeId);
    try {
      const reply = await client.request(
        action,
        undefined,
        undefined,
        priority,
      );
      if (!sameScope(reply.scope, scope) || reply.result.channel !== channel)
        throw integrity('Saved message identity changed.');
      return reply.result.intent;
    } finally {
      if (!owner) client.dispose();
    }
  };
  return {
    key: channelWorkKey(storeId, scope, channel),
    async load() {
      return (await run({ action: 'load-intent', ...target })) ?? undefined;
    },
    async save(intent) {
      const saved = await run({
        action: 'save-intent',
        ...target,
        submission: intent.submission,
        text: intent.text,
      });
      if (saved?.submission !== intent.submission || saved.text !== intent.text)
        throw integrity('Saved message content changed.');
    },
    async clear(submission) {
      await run({ action: 'clear-intent', ...target, submission });
    },
  };
}
