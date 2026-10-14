import type { ReactNode, Ref } from 'react';
import { Button, Chip } from '../components';
import type { ChatChannel } from '../chat-contract';
import { audienceLine, type ChannelAudience } from './audience';
import { channelTitle } from './presentation';
import { applePlatform, commandKey } from './composer-format';

/**
 * The channel's name, its description and audience (`audienceLine`), the
 * team's unsent-work chip, the find toggle and the ⓘ toggle.
 */
export function ThreadHeader({
  channel,
  audience,
  onInfo,
  infoOpen = false,
  infoRef,
  attention,
  onFind,
  findOpen = false,
}: {
  channel: ChatChannel;
  /** `undefined` while the team's roster has not arrived: no count is shown. */
  audience?: ChannelAudience;
  onInfo?: () => void;
  infoOpen?: boolean;
  infoRef?: Ref<HTMLButtonElement>;
  /** The team's `UnsentChip`, drawn before the ⓘ. */
  attention?: ReactNode;
  /** Opens or closes the find bar; omitted where the channel is unreadable. */
  onFind?: () => void;
  findOpen?: boolean;
}): ReactNode {
  const line = audienceLine(audience, channel.description?.trim());
  return (
    <div className="chat-thread-header">
      <h2>{channelTitle(channel)}</h2>
      {line && <p className="chat-thread-sub">{line}</p>}
      {channel.admin && <Chip>Admins</Chip>}
      {!channel.readable && <Chip tone="warn">Restricted</Chip>}
      <span className="chat-thread-spacer" />
      {attention}
      {onFind && (
        <Button
          variant="quiet"
          className={findOpen ? 'chat-head-act on' : 'chat-head-act'}
          icon="search"
          title={`Find in ${channelTitle(channel)} (${commandKey('F')})`}
          aria-label="Find in channel"
          aria-expanded={findOpen}
          aria-keyshortcuts={applePlatform() ? 'Meta+F' : 'Control+F'}
          onClick={onFind}
        />
      )}
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
