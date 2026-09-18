import type { StoreRef } from '../model/types';
import type { Location } from './types';

/** The chat location the Chat tab last opened, for the rail's Chat tab. */
let openedChat: Extract<Location, { kind: 'chat' }> | null = null;

/**
 * Where the rail's Chat tab goes: the team and channel the tab last had open,
 * so returning to Chat does not re-run the first-team fallback. The Chat tab
 * itself is the only writer, and it forgets a team that stopped having chat.
 */
export function chatTabLocation(): Location {
  return openedChat ?? { kind: 'chat' };
}

export function rememberChatLocation(
  location: Extract<Location, { kind: 'chat' }> | null,
): void {
  openedChat = location?.ref ? location : null;
}

/** The remembered team, for the writer deciding whether the memory still holds. */
export function rememberedChatRef(): StoreRef | undefined {
  return openedChat?.ref;
}
