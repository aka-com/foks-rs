import type { ReactNode, Ref } from 'react';
import { Button, Chip } from '../components';
import type { ChatChannel } from '../chat-contract';
import { plural } from '../model';
import { channelTitle } from './presentation';

/** The channel's name, its description or member count, and the ⓘ toggle. */
export function ThreadHeader({
  channel,
  memberCount,
  onInfo,
  infoOpen = false,
  infoRef,
}: {
  channel: ChatChannel;
  /** `undefined` while the team's roster has not arrived: no count is shown. */
  memberCount?: number;
  onInfo?: () => void;
  infoOpen?: boolean;
  infoRef?: Ref<HTMLButtonElement>;
}): ReactNode {
  const description = channel.description?.trim();
  return (
    <div className="chat-thread-header">
      <h2>{channelTitle(channel)}</h2>
      {(description || memberCount !== undefined) && (
        <p className="chat-thread-sub">
          {description ||
            (memberCount === undefined ? '' : plural(memberCount, 'member'))}
        </p>
      )}
      {channel.admin && <Chip>Admins</Chip>}
      {!channel.readable && <Chip tone="warn">Restricted</Chip>}
      <span className="chat-thread-spacer" />
      {onInfo && (
        <Button
          variant="quiet"
          className="chat-head-act"
          icon="info"
          title="Channel info"
          aria-label="Channel info"
          on={infoOpen}
          ref={infoRef}
          onClick={onInfo}
        />
      )}
    </div>
  );
}
