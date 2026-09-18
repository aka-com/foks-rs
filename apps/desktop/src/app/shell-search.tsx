import { useMemo, type ReactNode } from 'react';
import { useSidebarInbox } from '../chat/inbox-provider';
import type { Location } from '../location';
import type { AgentSnapshot } from '../model';
import { SearchPalette, type SearchChannel } from '../shell/search-palette';

/**
 * Mounts the search palette inside the chat inbox provider to share cached
 * channel names and keep search results consistent with rail unread badges.
 */
export function ShellSearch({
  snapshot,
  open,
  onClose,
  onNavigate,
  onOpenItem,
}: {
  snapshot: AgentSnapshot;
  open: boolean;
  onClose: () => void;
  onNavigate: (location: Location) => void;
  onOpenItem: (store: string, path: string) => void;
}): ReactNode {
  const inbox = useSidebarInbox();
  const channels = useMemo(() => {
    const found: SearchChannel[] = [];
    for (const [store, team] of inbox)
      for (const conversation of team.data?.conversations ?? []) {
        if (conversation.hidden) continue;
        found.push({
          store,
          name: conversation.channel.name,
          id: conversation.channel.id,
        });
      }
    return found;
  }, [inbox]);
  return (
    <SearchPalette
      snapshot={snapshot}
      open={open}
      onClose={onClose}
      onNavigate={onNavigate}
      onOpenItem={onOpenItem}
      channels={channels}
    />
  );
}
