import type { ReactNode, Ref } from 'react';
import { Button, Chip } from '../components';
import type { ChatChannel } from '../chat-contract';
import { audienceLine, type ChannelAudience } from './audience';
import { channelTitle } from './presentation';

/**
 * The channel's name, its description and audience (`audienceLine`), and the
 * ⓘ toggle.
 */
export function ThreadHeader({
  channel,
  audience,
  onInfo,
  infoOpen = false,
  infoRef,
}: {
  channel: ChatChannel;
  /** `undefined` while the team's roster has not arrived: no count is shown. */
  audience?: ChannelAudience;
  onInfo?: () => void;
  infoOpen?: boolean;
  infoRef?: Ref<HTMLButtonElement>;
}): ReactNode {
  const line = audienceLine(audience, channel.description?.trim());
  return (
    <div className="chat-thread-header">
      <h2>{channelTitle(channel)}</h2>
      {line && <p className="chat-thread-sub">{line}</p>}
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
