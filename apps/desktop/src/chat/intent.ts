import type { Bridge } from '../bridge';
import type { ChatScope } from '../chat-contract';
import type { SavedChatIntent } from './local-contract';
import { channelWorkKey, sameScope } from './scope';

export type ChatIntent = Pick<SavedChatIntent, 'submission' | 'text'>;
export interface ChatIntentPersistence {
  key: string;
  load(): Promise<ChatIntent | undefined>;
  save(intent: ChatIntent): Promise<void>;
  clear(submission: string): Promise<void>;
}

export function chatIntentPersistence(
  bridge: Pick<Bridge, 'chatLocal'>,
  storeId: string,
  scope: ChatScope,
  channel: string,
): ChatIntentPersistence {
  const target = { storeId, scope, channel };
  const invalid = () =>
    Object.assign(
      new Error('Saved message identity or content did not match.'),
      {
        code: 'chat-intent',
        retryable: false,
        ambiguous: false,
        fatal: false,
      },
    );
  const checked = (intent: SavedChatIntent): ChatIntent => {
    if (
      intent.storeId !== storeId ||
      intent.channel !== channel ||
      !sameScope(intent.scope, scope)
    )
      throw invalid();
    return { submission: intent.submission, text: intent.text };
  };
  return {
    key: channelWorkKey(storeId, scope, channel),
    async load() {
      const result = await bridge.chatLocal({
        action: 'load-intent',
        ...target,
      });
      return result.intent ? checked(result.intent) : undefined;
    },
    async save(intent) {
      const result = await bridge.chatLocal({
        action: 'save-intent',
        ...target,
        submission: intent.submission,
        text: intent.text,
      });
      if (!result.intent) throw invalid();
      const saved = checked(result.intent);
      if (saved.submission !== intent.submission || saved.text !== intent.text)
        throw invalid();
    },
    async clear(submission) {
      await bridge.chatLocal({ action: 'clear-intent', ...target, submission });
    },
  };
}
