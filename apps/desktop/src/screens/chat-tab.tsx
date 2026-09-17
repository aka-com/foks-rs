/**
 * The Chat tab: a strip of team chips over the conversation.
 *
 * The rail used to carry one chat row per group. Those rows are this strip, so
 * Chat can be a tab. `{kind:'chat'}` — the tab with no team chosen — opens the
 * first team that has chat, and says so when there is none.
 */

import { useEffect } from 'react';
import type { ReactNode } from 'react';
import { Icon } from '../components';
import { storeHues, storeNavigationOrder } from '../model';
import type { AgentSnapshot, StoreRef, TeamStore } from '../model';
import type { Bridge } from '../bridge';
import type { Location } from '../location';
import { PageHeader } from '../shell/page-header';
import { useSidebarInbox } from '../chat/inbox-provider';
import { teamUnread } from '../chat/unread';
import { chatAvailable } from '../shell/sidebar';
import { ChatScreen } from './chat-screen';

/** The named, active groups the strip offers, in navigation order. */
export function chatTeams(snapshot: AgentSnapshot): TeamStore[] {
  return storeNavigationOrder(snapshot).filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      store.active !== false,
  );
}

/** The first team whose chat this Mac can open, if any. */
export function firstChatTeam(snapshot: AgentSnapshot): TeamStore | undefined {
  return chatTeams(snapshot).find((store) => chatAvailable(snapshot, store));
}

export function ChatTeamStrip({
  snapshot,
  active,
  onSelect,
}: {
  snapshot: AgentSnapshot;
  active?: StoreRef;
  onSelect: (ref: StoreRef) => void;
}): ReactNode {
  const inbox = useSidebarInbox();
  const stores = storeNavigationOrder(snapshot);
  const hues = storeHues(stores);
  const teams = chatTeams(snapshot);
  if (!teams.length) return null;
  return (
    <div className="team-strip" role="group" aria-label="Team chats">
      {teams.map((store) => {
        const available = chatAvailable(snapshot, store);
        const unread = available ? teamUnread(inbox.get(store.id)) : null;
        return (
          <button
            type="button"
            key={store.id}
            className={store.id === active ? 'chipbtn on' : 'chipbtn'}
            disabled={!available}
            aria-label={`${store.name} chat`}
            title={
              available
                ? `${store.name} chat`
                : 'Chat is not available for this group'
            }
            onClick={() => onSelect(store.id)}
          >
            <span
              className="sq"
              style={{ background: hues.get(store.id) }}
              aria-hidden="true"
            />
            {store.name}
            {unread ? (
              <span
                className="n"
                aria-label={unread.description}
                title={unread.description}
              >
                {unread.label}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}

export interface ChatTabProps {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'chat' | 'team-chat' }>;
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
  accessGeneration?: number;
}

export function ChatTab({
  snapshot,
  bridge,
  location,
  onNavigate,
  accessNow,
  accessGeneration,
}: ChatTabProps): ReactNode {
  const first = firstChatTeam(snapshot);
  const chosen = location.kind === 'team-chat' ? location.ref : undefined;
  // The tab with no team chosen is not a place to stay: it resolves to the
  // first team that has chat.
  useEffect(() => {
    if (location.kind === 'chat' && first)
      onNavigate({ kind: 'team-chat', ref: first.id });
  }, [first, location.kind, onNavigate]);
  return (
    <div className="chat-tab">
      <ChatTeamStrip
        snapshot={snapshot}
        active={chosen}
        onSelect={(ref) => onNavigate({ kind: 'team-chat', ref })}
      />
      {location.kind === 'team-chat' ? (
        <ChatScreen
          snapshot={snapshot}
          bridge={bridge}
          location={location}
          onNavigate={onNavigate}
          accessNow={accessNow}
          accessGeneration={accessGeneration}
        />
      ) : (
        <>
          <PageHeader title="Chat" subtitle="Group conversations" />
          <div className="body chat-tab-empty">
            <div className="empty">
              <span className="big">
                <Icon name="chat" />
              </span>
              <h2>No group chat available</h2>
              <p>
                Chat opens on a group whose server offers it. Join or create a
                group on a server with chat, then come back.
              </p>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
