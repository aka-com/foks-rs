import { normalizeCommandError } from '../bridge';
import type { ChatAction, ChatReply } from '../chat-contract';
import { diagnosticLog, hashId, outcomeForCode } from '../diagnostics/log';
import { integrity } from './client';
import type { ChatInboxService } from './inbox-service';

/**
 * Moves one channel's read pointer to `sequence` through the agent. The open
 * conversation and the Chat column's Mark as read both call this, with the
 * request path each already holds.
 *
 * A confirmed read is applied to the projection rather than resynchronized
 * into it: the reply states the pointer the server accepted, and the contract
 * has already checked that it names this request's channel and sequence. A
 * read that did not land leaves the local pointer unproven, so that path
 * invalidates and lets the next synchronization state it.
 */
export async function markChannelRead(
  request: (action: ChatAction) => Promise<ChatReply>,
  inbox: Pick<ChatInboxService, 'applyRead' | 'invalidate'>,
  storeId: string,
  channel: string,
  sequence: string,
): Promise<void> {
  const end = diagnosticLog.span('chat.read', {
    scope: `store#${hashId(storeId)}`,
  });
  try {
    const reply = await request({ action: 'mark-read', channel, sequence });
    if (reply.result.kind !== 'read') throw integrity();
    inbox.applyRead(storeId, reply.result.channel, reply.result.sequence);
    end('ok');
  } catch (error) {
    const code = normalizeCommandError(error).code;
    end(outcomeForCode(code), { code });
    inbox.invalidate(storeId);
    throw error;
  }
}
