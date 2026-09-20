import { diagnosticLog, hashId } from '../diagnostics/log';
import type { ChatInboxTiming } from './inbox-service';

/** Fixed event kinds and hashed scopes; no DTO content enters diagnostics. */
export function recordInboxTiming(event: ChatInboxTiming): void {
  switch (event.kind) {
    case 'poll':
      diagnosticLog.record({
        name: 'chat.poll',
        scope: `acct#${hashId(event.account)}`,
        ms: event.milliseconds,
        outcome: event.outcome,
        code: event.code,
        attrs: { bumped: event.bumped },
      });
      return;
    case 'sync':
      diagnosticLog.record({
        name: 'chat.sync',
        scope: `store#${hashId(event.store)}`,
        ms: event.milliseconds,
        outcome: event.outcome,
        code: event.code,
        attrs: { changed: event.changed, conversations: event.conversations },
      });
      return;
    case 'arrival':
      diagnosticLog.record({
        name: 'chat.arrival',
        scope: `store#${hashId(event.store)}`,
        ms: event.milliseconds,
        outcome: 'ok',
      });
      return;
    case 'publication':
      for (const [phase, ms] of [
        ['preparation', event.preparation],
        ['history-bindings', event.historyBindings],
        ['subscribers', event.subscribers],
      ] as const) {
        diagnosticLog.record({
          name: 'chat.publication',
          scope: `store#${hashId(event.store)}`,
          phase,
          ms,
          outcome: 'ok',
          attrs: {
            reason: event.reason,
            channels: event.channels,
            conversations: event.conversations,
            blockedChannels: event.blockedChannels,
            channelRevisions: event.channelRevisions,
            refreshRevisions: event.refreshRevisions,
            stores: event.stores,
            listeners: event.listeners,
          },
        });
      }
      return;
    default: {
      const exhaustive: never = event;
      return exhaustive;
    }
  }
}
