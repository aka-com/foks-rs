import { createContext, useContext, useMemo } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import type {
  ChatAction,
  ChatReply,
  ChatResult,
  ChatScope,
} from '../chat-contract';
import type { HistoryPlacement, HistoryWindow } from './conversation-model';

/**
 * Accepts a verified history page into the team's history cache. A page
 * merged from several replies carries the replies' results in `sources`.
 */
export type AcceptHistory = (
  page: Extract<ChatResult, { kind: 'history' }>,
  before: string | null,
  placement?: HistoryPlacement,
  sources?: readonly ChatResult[],
) => HistoryWindow | void;

/**
 * What every channel of one team's conversation shares: the team, the
 * reader, and the guarded requests the pane issues for it. The pane provides
 * it once; the thread and its rows read it instead of receiving it as props.
 */
export interface ChatConversationValue {
  bridge: Bridge;
  /** The team's store id, as the location addresses it. */
  storeId: string;
  /** The reader's party identifier, or `null` while the scope is unknown. */
  actor: string | null;
  scope: ChatScope | null;
  /** Display names by party identifier. */
  senderNames: ReadonlyMap<string, string>;
  request: (action: ChatAction) => Promise<ChatReply>;
  refreshPending: () => Promise<void>;
  markRead: (channel: string, sequence: string) => Promise<void>;
  acceptHistory: AcceptHistory;
  /** Stops a channel whose history failed verification. */
  blockHistory: (channel: string) => void;
}

const Context = createContext<ChatConversationValue | null>(null);

export function ChatConversationProvider({
  bridge,
  storeId,
  actor,
  scope,
  senderNames,
  request,
  refreshPending,
  markRead,
  acceptHistory,
  blockHistory,
  children,
}: ChatConversationValue & { children: ReactNode }) {
  const value = useMemo(
    () => ({
      bridge,
      storeId,
      actor,
      scope,
      senderNames,
      request,
      refreshPending,
      markRead,
      acceptHistory,
      blockHistory,
    }),
    [
      bridge,
      storeId,
      actor,
      scope,
      senderNames,
      request,
      refreshPending,
      markRead,
      acceptHistory,
      blockHistory,
    ],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useChatConversationContext(): ChatConversationValue {
  const value = useContext(Context);
  if (!value) throw new Error('Chat requires the conversation provider.');
  return value;
}
