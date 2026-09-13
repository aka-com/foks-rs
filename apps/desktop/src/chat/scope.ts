import type { ChatScope } from '../chat-contract';
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
export function channelWorkKey(
  storeId: string,
  scope: ChatScope,
  channel: string,
): string {
  return JSON.stringify([
    storeId,
    scope.host,
    scope.actor,
    scope.store.profile,
    scope.store.account_alias,
    scope.store.team_alias,
    scope.store.team_id,
    channel,
  ]);
}
