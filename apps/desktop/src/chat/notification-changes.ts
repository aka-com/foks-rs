import type {
  ChatScope,
  ChatChannel,
  ChatConversation,
} from '../chat-contract';
import type { TeamInbox } from './inbox-service';
import { sameScope } from './scope';

// Lookup indexes refer to immutable shell DTOs; they are not channel authority.
// Weak keys let old snapshots (and their decrypted previews) be collected.
const indexes = new WeakMap<
  object,
  {
    channels: ReadonlyMap<string, ChatChannel>;
    conversations: ReadonlyMap<string, ChatConversation>;
  }
>();

/** Accepted notification metadata, never message bodies or previews. */
export interface NotificationView {
  scope: ChatScope;
  revision: number;
  authorization: number;
  eligible: boolean;
  readable: boolean;
  readRole: string;
  admin: boolean;
  muted: boolean;
  hidden: boolean;
  quarantined: boolean;
  preference: boolean;
  fresh: boolean;
  degraded: boolean;
}
export function notificationView(
  entry: TeamInbox | undefined,
  channel: string,
  preference = true,
): NotificationView | undefined {
  if (!entry?.scope || !entry.data) return;
  let index = indexes.get(entry.data);
  if (!index) {
    index = {
      channels: new Map(entry.data.channels.map((row) => [row.id, row])),
      conversations: new Map(
        entry.data.conversations.map((row) => [row.channel.id, row]),
      ),
    };
    indexes.set(entry.data, index);
  }
  const row = index.channels.get(channel);
  if (!row) return;
  const conversation = index.conversations.get(channel);
  return {
    scope: entry.scope,
    revision: entry.channelRevisions.get(channel) ?? 0,
    authorization: entry.authorizationRevision ?? 0,
    readable: row.readable,
    readRole: row.read_role,
    admin: row.admin,
    muted: !!conversation?.muted,
    hidden: !!conversation?.hidden,
    quarantined: entry.blockedChannels.has(channel),
    preference,
    eligible:
      row.readable &&
      !entry.blockedChannels.has(channel) &&
      !conversation?.muted &&
      !conversation?.hidden,
    fresh: entry.state === 'ready' && !entry.stale,
    degraded: entry.data.degraded,
  };
}
export interface NotificationChange {
  scope: boolean;
  authorization: boolean;
  membership: boolean;
  policy: boolean;
  preference: boolean;
  freshness: boolean;
  degraded: boolean;
  baseline: boolean;
  remove: boolean;
  content: boolean;
  fallback: boolean;
}
/** Status-only publications do not become history requests. */
export function classifyNotificationChange(
  previous: NotificationView | undefined,
  next: NotificationView | undefined,
): NotificationChange {
  const remove = !next || !next.eligible || !next.fresh;
  const authorization =
    !!previous &&
    !!next &&
    (previous.readRole !== next.readRole || previous.admin !== next.admin);
  const baseline =
    !remove &&
    (!previous ||
      !previous.eligible ||
      !previous.fresh ||
      !sameScope(previous.scope, next.scope) ||
      authorization);
  return {
    scope: !!previous && !!next && !sameScope(previous.scope, next.scope),
    authorization,
    membership: !!previous !== !!next,
    policy:
      !!previous &&
      !!next &&
      (previous.readable !== next.readable ||
        previous.muted !== next.muted ||
        previous.hidden !== next.hidden ||
        previous.quarantined !== next.quarantined),
    preference: !!previous && !!next && previous.preference !== next.preference,
    freshness: !!previous && !!next && previous.fresh !== next.fresh,
    degraded: !!previous && !!next && previous.degraded !== next.degraded,
    remove,
    baseline,
    content: !remove && !baseline && previous!.revision !== next.revision,
    fallback: !remove && !!next.degraded && !previous?.degraded,
  };
}
