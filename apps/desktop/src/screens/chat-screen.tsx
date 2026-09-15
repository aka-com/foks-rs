import { ChatThread } from '../chat/chat-thread';
import { PendingRow } from '../chat/pending-row';
import { channelTitle, listChannels, partyNames } from '../chat/presentation';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode, Ref } from 'react';
import {
  Band,
  Button,
  Icon,
  Inset,
  InsetRow,
  Notice,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../components';
import type { Bridge } from '../bridge';
import type { ChatAction, ChatReply } from '../chat-contract';
import {
  chatAvailable,
  serverOf,
  storeAvailability,
  storeDescription,
  storeDescriptionState,
  storeOf,
} from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import type { Location } from '../location';
import { failure, preparationCanChange, submissionId } from '../chat/actions';
import { useChatConversation } from '../chat/use-chat-conversation';
import { chatTeams } from './chat-teams';
import './chat.css';

const systemAccessNow = () => Date.now() / 1000;

/**
 * One team's conversation: the pane beside the Chat tab's team column. The tab
 * keys this on the open team, so a switch mounts exactly one of these.
 */
export function ChatScreen({
  snapshot: agentSnapshot,
  bridge,
  location,
  onNavigate,
  accessNow = systemAccessNow,
  accessGeneration = 0,
  infoOpen = false,
  onToggleInfo,
  infoRef,
  creating = false,
  onCreating,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  location: Extract<Location, { kind: 'chat' }> & { ref: StoreRef };
  onNavigate: (location: Location) => void;
  accessNow?: () => number;
  accessGeneration?: number;
  /** The tab owns the info panel, because it owns the layout it sits in. */
  infoOpen?: boolean;
  onToggleInfo?: () => void;
  /** The ⓘ toggle, so the tab can hand focus back when the panel closes. */
  infoRef?: Ref<HTMLButtonElement>;
  /** The new-channel sheet, opened from here or from the column's `+`. */
  creating?: boolean;
  onCreating?: (open: boolean) => void;
}): ReactNode {
  const store = storeOf(agentSnapshot, location.ref);
  const access = useCallback(
    () =>
      store
        ? storeAvailability(agentSnapshot, store, { nowSeconds: accessNow() })
        : ({ available: false, reason: 'vault-unavailable' } as const),
    [accessNow, store, agentSnapshot],
  );
  const {
    channels,
    conversations,
    pending,
    error,
    syncError,
    degraded,
    loading,
    blocked,
    blockedChannels,
    channelRevisions,
    actor,
    request,
    refresh,
    refreshPending,
    markRead,
    acceptHistory,
    history,
    blockHistory,
  } = useChatConversation(
    bridge,
    store?.server ?? '',
    location.ref,
    access,
    accessGeneration,
  );
  const [created, setCreated] = useState<string | null>(null);
  const listed = listChannels(channels, conversations);
  const channel =
    listed.find(({ channel }) => channel.id === location.channel)?.channel ??
    (!location.channel ? listed[0]?.channel : undefined);
  const activeConversation = conversations.find(
    (conversation) => conversation.channel.id === channel?.id,
  );
  const channelNames = new Map(
    listed.map(({ channel }) => [channel.id, channelTitle(channel)]),
  );
  const storeId = store?.id ?? '';
  const senderNames = partyNames(agentSnapshot, storeId);
  const accessAvailable = useCallback(
    (): boolean => access().available,
    [access],
  );
  const guardedRequest = useCallback(
    (action: ChatAction): Promise<ChatReply> => request(action),
    [request],
  );
  const guardedRefresh = useCallback(
    (): Promise<void> => (accessAvailable() ? refresh() : Promise.resolve()),
    [accessAvailable, refresh],
  );
  const guardedRefreshPending = useCallback(
    (): Promise<void> =>
      accessAvailable() ? refreshPending() : Promise.resolve(),
    [accessAvailable, refreshPending],
  );
  const guardedMarkRead = useCallback(
    (channelId: string, sequence: string): Promise<void> =>
      accessAvailable() ? markRead(channelId, sequence) : Promise.resolve(),
    [accessAvailable, markRead],
  );
  useEffect(() => {
    if (!created || !storeId) return;
    if (!channels.some((channel) => channel.id === created)) return;
    setCreated(null);
    onNavigate({ kind: 'chat', ref: storeId, channel: created });
  }, [channels, created, onNavigate, storeId]);
  const available = accessAvailable();
  // The reason the locked pane states is read off the clock the lock decision
  // used, so the two cannot disagree about a check-in that expired this second.
  const describeOptions = { nowSeconds: accessNow() };
  const team =
    store &&
    store.kind === 'team' &&
    store.team_kind === 'named' &&
    store.active !== false
      ? store
      : undefined;
  const openCreate = () => onCreating?.(true);
  const empty = !loading && !listed.length && !error;
  const unfinished = pending.filter(
    (op) =>
      !op.observed &&
      (op.kind === 'create-channel' ||
        op.channel !== channel?.id ||
        !channel?.readable),
  );
  // Only a team that can actually be opened is worth offering: `chatTeams`
  // filters on the server's capability, not on whether it answers today.
  const elsewhere = chatTeams(agentSnapshot).find(
    (candidate) =>
      candidate.id !== store?.id &&
      chatAvailable(agentSnapshot, candidate, describeOptions),
  );
  // A lapsed check-in is renewable here; every other unavailable state is
  // stated and left alone, because no button in Chat resolves it.
  const lapsed =
    store &&
    ['check-in-expired', 'check-in-unavailable'].includes(
      storeDescriptionState(agentSnapshot, store, describeOptions),
    );
  const pane =
    !available && store ? (
      // A lapsed check-in locks a whole server: the column stays, and this pane
      // says which server and where to renew it.
      <div className="chat-locked">
        <span className="big">
          <Icon name="shield" />
        </span>
        <h2>{store.name} chat is locked</h2>
        <p role="alert">
          {storeDescription(agentSnapshot, store, describeOptions)} on{' '}
          {serverOf(agentSnapshot, store.id)?.name ?? store.server}.{' '}
          {lapsed
            ? 'Every store on that server is unavailable until the server is checked again. Messages already on this Mac are kept.'
            : 'Messages already on this Mac are kept.'}
        </p>
        <div className="chat-locked-actions">
          {lapsed && (
            <Button
              variant="primary"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'servers',
                  profile: store.server,
                })
              }
            >
              Check in
            </Button>
          )}
          {elsewhere && (
            <Button
              onClick={() => onNavigate({ kind: 'chat', ref: elsewhere.id })}
            >
              Open another team
            </Button>
          )}
        </div>
      </div>
    ) : blocked ? (
      <div className="chat-conversation-body">
        <Notice severity="crit" title="Chat stopped">
          <p role="alert">{blocked}</p>
          <p>
            Check the account and server, then lock and unlock the desktop to
            start a fresh chat session.
          </p>
        </Notice>
      </div>
    ) : !team ? (
      <div className="empty">
        <span className="big">
          <Icon name="people" />
        </span>
        <h2>Chat unavailable</h2>
        <p>
          Chat works in active named teams. Pick one in the column to open its
          conversations.
        </p>
      </div>
    ) : (
      <>
        {error && (
          <Band
            severity="crit"
            action={
              <Button
                size="sm"
                disabled={loading}
                onClick={() => void guardedRefresh()}
              >
                Retry
              </Button>
            }
          >
            <span role="alert">{error}</span>
          </Band>
        )}
        <div role="status" className="chat-status">
          {syncError && (
            <Band>
              Live updates paused
              <small>{syncError}</small>
            </Band>
          )}
          {degraded && (
            <Band severity="info">
              Some inbox changes could not be listed. Visible channels still
              refresh directly.
            </Band>
          )}
        </div>
        {channel && blockedChannels.has(channel.id) ? (
          <div className="empty">
            <h2>Channel stopped</h2>
            <p role="alert">
              Content in this channel could not be verified. Other channels
              remain available.
            </p>
            <p>
              Check the account and server, then lock and unlock the desktop to
              revalidate this channel.
            </p>
          </div>
        ) : channel ? (
          <ChatThread
            bridge={bridge}
            storeId={storeId}
            key={`${channel.id}:${channel.readable}`}
            channel={channel}
            teamName={team.name}
            onFiles={() => onNavigate({ kind: 'store', ref: team.id })}
            onInfo={onToggleInfo}
            infoOpen={infoOpen}
            infoRef={infoRef}
            actor={actor}
            senderNames={senderNames}
            request={guardedRequest}
            refreshPending={guardedRefreshPending}
            refreshInbox={guardedRefresh}
            revision={channelRevisions?.get(channel.id) ?? 0}
            readThrough={activeConversation?.read_through ?? null}
            markRead={guardedMarkRead}
            history={history}
            acceptHistory={acceptHistory}
            blockHistory={blockHistory}
            pending={pending.filter(
              (op) =>
                op.kind !== 'create-channel' &&
                !op.observed &&
                op.channel === channel.id,
            )}
          />
        ) : loading ? (
          <div className="empty" aria-busy="true">
            <p>Loading {team.name}…</p>
          </div>
        ) : location.channel ? (
          <div className="empty">
            <span className="big">
              <Icon name="alert" />
            </span>
            <h2>Channel unavailable</h2>
            <p>This channel is unavailable. Refresh to check your access.</p>
            <Button disabled={loading} onClick={() => void guardedRefresh()}>
              Refresh
            </Button>
          </div>
        ) : (
          <div className="empty">
            <span className="big">
              <Icon name="people" />
            </span>
            <h2>No conversations yet</h2>
            <p>
              Create the first channel for {team.name}. Every member with the
              right role can join in.
            </p>
            {empty && (
              <Button variant="primary" icon="plus" onClick={openCreate}>
                New channel
              </Button>
            )}
          </div>
        )}
      </>
    );
  return (
    <>
      {unfinished.length > 0 && (
        <section className="chat-recovery" aria-label="Needs attention">
          <SectionLabel>Needs attention</SectionLabel>
          <p className="chat-quiet">
            Saved work that has not finished. Nothing here is sent twice without
            your say-so.
          </p>
          {unfinished.map((op) => (
            <PendingRow
              key={op.id}
              operation={op}
              channelName={channelNames.get(op.channel)}
              request={guardedRequest}
              onChange={() => void guardedRefresh()}
            />
          ))}
        </section>
      )}
      {pane}
      {creating && (
        <ChannelCreateSheet
          team={team?.name ?? ''}
          request={guardedRequest}
          onClose={() => onCreating?.(false)}
          onCreated={async (channelId) => {
            setCreated(channelId);
            await guardedRefresh();
          }}
        />
      )}
    </>
  );
}

function ChannelCreateSheet({
  team,
  request,
  onClose,
  onCreated,
}: {
  /** The open team; a channel can only be created in the team that is open. */
  team: string;
  request: (a: ChatAction) => Promise<ChatReply>;
  onClose: () => void;
  onCreated: (channelId: string) => Promise<void>;
}): ReactNode {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [admin, setAdmin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submission = useRef<ChatAction | null>(null);
  const sending = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      submission.current = null;
    };
  }, []);
  const locked = busy || submission.current !== null;
  const create = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    setError('');
    submission.current ??= {
      action: 'prepare-channel',
      submission: submissionId(),
      name,
      description,
      admin,
    };
    let preparedId: string | null = null;
    try {
      const prepared = await request(submission.current);
      if (!alive.current) return;
      if (prepared.result.kind !== 'operation')
        throw new Error('Invalid channel preparation.');
      const op = prepared.result.operation;
      preparedId = op.id;
      submission.current = null;
      setName('');
      setDescription('');
      const attempted = await request({ action: 'attempt', operation: op.id });
      if (!alive.current) return;
      const channelId =
        attempted.result.kind === 'operation'
          ? attempted.result.operation.channel
          : op.channel;
      await onCreated(channelId);
      if (alive.current) onClose();
    } catch (e) {
      if (alive.current) {
        if (submission.current && preparationCanChange(e))
          submission.current = null;
        setError(failure(e));
        if (preparedId) {
          try {
            await request({ action: 'status', operation: preparedId });
          } catch {
            /* Keep the durable identity visible if status is unavailable. */
          }
        }
      }
    } finally {
      sending.current = false;
      if (alive.current) setBusy(false);
    }
  };
  return (
    <SheetDialog
      title="New channel"
      onClose={onClose}
      dismissible={!locked}
      footer={
        <>
          <Button disabled={locked} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => void create()}
          >
            {busy
              ? 'Creating…'
              : submission.current
                ? 'Recover preparation'
                : 'Create channel'}
          </Button>
        </>
      }
    >
      <form
        className="chat-create"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        {team && <p className="hint">This channel is created in {team}.</p>}
        <Inset>
          <InsetRow label="Channel name">
            <input
              aria-label="Channel name"
              data-sheet-autofocus="true"
              value={name}
              disabled={locked}
              onChange={(e) => setName(e.target.value)}
              placeholder="design"
              maxLength={32}
            />
            <small>
              3–32 characters, lowercased automatically. The one exception is an
              empty name, which creates the team&apos;s general channel.
            </small>
          </InsetRow>
          <InsetRow label="Description">
            <textarea
              aria-label="Channel description"
              value={description}
              disabled={locked}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What this channel is for"
              maxLength={1024}
            />
            <small>
              Optional, 3–512 characters. Descriptions are lowercased
              automatically.
            </small>
          </InsetRow>
        </Inset>
        <SectionLabel>Who can take part</SectionLabel>
        <Inset>
          <RadioGroup label="Channel audience">
            <RadioCard
              title="Everyone on the team"
              detail="Members and above can read and write."
              selected={!admin}
              disabled={locked}
              onSelect={() => setAdmin(false)}
            />
            <RadioCard
              title="Admins and owners"
              detail="Hidden from members. Only admins and owners can read or write."
              selected={admin}
              disabled={locked}
              onSelect={() => setAdmin(true)}
            />
          </RadioGroup>
        </Inset>
        {submission.current && !busy && (
          <p className="hint">
            The server reply was lost. Recover retries the same request so the
            channel is not created twice.
          </p>
        )}
        {error && (
          <p role="alert" className="action-error">
            {error}
          </p>
        )}
      </form>
    </SheetDialog>
  );
}
