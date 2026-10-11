/**
 * A test bridge whose chat replies pass the decoder the native bridge applies.
 */

import {
  decodeChatReplyFor,
  type ChatAction,
  type ChatReply,
  type ChatTeamRef,
} from '../../src/chat-contract';

type Chat<R> = (storeId: string, action: ChatAction, viewId?: string) => R;

/**
 * `fake` with each `chat` reply serialized as IPC would and decoded as the
 * native bridge decodes it, its scope checked against `team`. A reply the
 * agent could not send rejects with `chat-integrity`; thrown errors pass
 * through unchanged.
 */
export function decodedChatBridge<T extends { chat: unknown }>(
  fake: T,
  team: ChatTeamRef,
): Omit<T, 'chat'> & { chat: Chat<Promise<ChatReply>> } {
  const chat = fake.chat as Chat<unknown>;
  return {
    ...fake,
    chat: async (
      storeId: string,
      action: ChatAction,
      viewId?: string,
    ): Promise<ChatReply> => {
      // A reply with no JSON form, such as undefined, crosses IPC as null.
      const reply: unknown = JSON.parse(
        JSON.stringify(await chat(storeId, action, viewId)) ?? 'null',
      );
      return decodeChatReplyFor(reply, team, action);
    },
  };
}
