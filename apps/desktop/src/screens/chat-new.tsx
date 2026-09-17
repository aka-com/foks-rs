import { useTabSheetState } from '../navigation-guard';
/**
 * New chat: pick a team, then pick or create a channel in it.
 *
 * Chat supports channels in named teams, not one-to-one conversations. The
 * default flow selects a team and then a channel. When opened for a specific
 * team, the sheet skips team selection, opens the create step, and shows
 * Cancel instead of Back. Unavailable teams remain listed with an explanation.
 * Channel creation uses the agent's durable preparation: one submission
 * identifier is retried rather than repeated, with the fields and audience
 * accepted by `prepare-channel`.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import {
  Button,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../components';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { ChatAction, ChatReply } from '../chat-contract';
import {
  CHAT_DESCRIPTION_MAX_CHARS,
  CHAT_DESCRIPTION_MIN_CHARS,
  CHAT_NAME_MAX_CHARS,
  CHAT_NAME_MIN_CHARS,
} from '../chat-limits';
import type { NavigationGuard } from '../location';
import { useNavigationGuard } from '../navigation-guard';
import { chatAvailable, serverDisplayName, storeDescription } from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  StoreRef,
  TeamStore,
} from '../model';
import { failure, preparationCanChange, submissionId } from '../chat/actions';
import { chatClient, integrity, sameScope } from '../chat/client';
import { useChatInbox } from '../chat/inbox-provider';
import {
  accessSummary,
  channelDescriptionProblem,
  channelMeta,
  channelNameProblem,
  channelTitle,
  listChannels,
  normalizeChannelName,
} from '../chat/presentation';
import { chatTeams, noChatReason, noChatTeams } from './chat-teams';
import { GroupMark } from './group-mark';

/** A team the sheet offers, and why it cannot be offered. */
interface TeamChoice {
  store: TeamStore;
  /** Empty when the team can be chatted in. */
  reason: string;
  detail: string;
}

const systemAccessNow = () => Date.now() / 1000;

export function NewChatSheet({
  snapshot,
  bridge,
  team,
  accessOptions = {},
  accessNow = systemAccessNow,
  accessGenerations,
  onClose,
  onOpen,
  onUnresolved,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The team the sheet opens on, when it was opened from one. */
  team?: StoreRef;
  accessOptions?: AvailabilityOptions;
  /** The shell's availability clock, read afresh around every request. */
  accessNow?: () => number;
  /** The shell's access generation per server, as the conversation reads it. */
  accessGenerations?: ReadonlyMap<string, number>;
  onClose: () => void;
  onOpen: (ref: StoreRef, channel?: string) => void;
  /**
   * Whether the agent holds a submission this sheet has not settled. The tab
   * keeps the sheet mounted while that is true, because a submission has to be
   * settled where it was made.
   */
  onUnresolved?: (unresolved: boolean) => void;
}): ReactNode {
  const { service, snapshot: inbox } = useChatInbox();
  // A sheet opened on a team is that team's: there is no step behind it to go
  // back to, so it opens on the channel it was asked for — the create step —
  // and leaves by being cancelled.
  const fixedTeam = team !== undefined;
  const [chosen, setChosen] = useTabSheetState<StoreRef | undefined>(
    'channel.chosen',
    team,
  );
  const [step, setStep] = useTabSheetState<'team' | 'channel'>(
    'channel.step',
    team ? 'channel' : 'team',
  );
  const [channel, setChannel] = useTabSheetState<string | undefined>(
    'channel.channel',
    undefined,
  );
  const [creating, setCreating] = useTabSheetState(
    'channel.creating',
    fixedTeam,
  );
  const [name, setName] = useTabSheetState('channel.name', '');
  const [description, setDescription] = useTabSheetState(
    'channel.description',
    '',
  );
  const [admin, setAdmin] = useTabSheetState('channel.admin', false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  // The preparation the agent has been given but has not finished: the
  // `prepare-channel` until it is accepted, then the operation identifier it
  // answered with. Recovering re-issues the same one, so a lost reply is
  // retried rather than turned into a second channel.
  const submission = useRef<ChatAction | null>(null);
  // Whether the agent may already hold that submission: a request whose outcome
  // is unknown is one it may have taken, and from then on the only way to find
  // out is to re-issue the same one. A later refusal — of a request that never
  // reached the agent, say — does not make it discardable.
  const held = useRef(false);
  const prepared = useRef<{ operation: string; channel: string } | null>(null);
  const [outstanding, setOutstanding] = useState(false);
  const sending = useRef(false);
  const alive = useRef(true);
  const nameField = useRef<HTMLInputElement | null>(null);
  const body = useRef<HTMLDivElement | null>(null);
  const opened = useRef(false);
  // A request in flight has to be judged against what the shell knows now, not
  // against what it knew when the request started: the snapshot, the
  // availability options and the access generations are read out of refs
  // assigned every render, the way `use-chat-conversation.ts` reads its own.
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;
  const accessOptionsRef = useRef(accessOptions);
  accessOptionsRef.current = accessOptions;
  const generationsRef = useRef(accessGenerations);
  generationsRef.current = accessGenerations;
  const unresolvedRef = useRef(onUnresolved);
  unresolvedRef.current = onUnresolved;
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      submission.current = null;
      held.current = false;
      prepared.current = null;
      // The tab holds the sheet open for an unresolved submission. A sheet that
      // is gone holds nothing, so the latch leaves with it rather than keeping
      // the next New chat pinned to a team switch that already happened.
      unresolvedRef.current?.(false);
    };
  }, []);
  useEffect(() => {
    onUnresolved?.(outstanding);
  }, [outstanding, onUnresolved]);
  // A step replaces the whole body, so focus follows it rather than staying on
  // a control that is no longer there — and the create form is a step of its
  // own, whose field is the name rather than its first control. The dialog
  // places focus when it opens, so only an actual change moves it.
  useEffect(() => {
    if (!opened.current) {
      opened.current = true;
      return;
    }
    if (creating) {
      nameField.current?.focus();
      return;
    }
    const region = body.current;
    const first = region?.querySelector<HTMLElement>(
      'input:not([disabled]), textarea:not([disabled]), button:not([disabled])',
    );
    first?.focus();
  }, [step, creating]);
  const choices: TeamChoice[] = [
    ...chatTeams(snapshot).map((store) => {
      const reachable = chatAvailable(snapshot, store, accessOptions);
      const entry = inbox.get(store.id);
      // A hidden conversation is listed in the sheet with the rest, but it is
      // not one of the channels this line offers the reader.
      const channels = entry?.data
        ? listChannels(entry.data.channels, entry.data.conversations).filter(
            ({ conversation }) => !conversation?.hidden,
          ).length
        : null;
      const serverEntry = snapshot.servers.find(
        (candidate) => candidate.id === store.server,
      );
      const server = serverEntry
        ? serverDisplayName(serverEntry)
        : store.server;
      return {
        store,
        reason: reachable
          ? ''
          : storeDescription(snapshot, store, accessOptions),
        detail:
          channels === null
            ? server
            : `${channels} ${channels === 1 ? 'channel' : 'channels'} · ${server}`,
      };
    }),
    ...noChatTeams(snapshot).map((store) => ({
      store,
      reason: noChatReason(snapshot, store),
      detail: '',
    })),
  ];
  const picked = choices.find((choice) => choice.store.id === chosen);
  const entry = picked ? inbox.get(picked.store.id) : undefined;
  const listed = entry?.data
    ? listChannels(entry.data.channels, entry.data.conversations)
    : [];
  const problem = channelNameProblem(
    name,
    listed.map(({ channel }) => channel.name),
  );
  const descriptionProblem = channelDescriptionProblem(description);
  // Step two cannot be answered until the team's channels are known: an empty
  // name is the general channel, and whether the team already has one is the
  // difference between creating it and being refused.
  const channelsKnown = Boolean(entry?.data);
  const locked = busy || outstanding;
  // What refuses a channel the sheet has not sent yet. A preparation the agent
  // has already taken is past all of it: its fields are frozen, so the only
  // thing that can stop a recovery is a recovery already running.
  const unsendable =
    problem !== null ||
    descriptionProblem !== null ||
    !channelsKnown ||
    Boolean(picked?.reason);
  const createDisabled = busy || (!outstanding && unsendable);
  // Read by the guard when it is asked, so typing re-registers nothing. The
  // latch above keeps the sheet mounted through a team switch; the guard is
  // the same answer, given to the moves that would take the whole tab away.
  const guardState = useRef({ typed: false, name: '', locked: false });
  guardState.current = {
    typed: creating && Boolean(name.trim() || description.trim()),
    name: name.trim(),
    locked,
  };
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const formGuard = useCallback<NavigationGuard>(() => {
    const { typed, name: typedName, locked: inFlight } = guardState.current;
    // A submission the agent may already hold has to be settled in the sheet
    // that made it: this is the one state the reader cannot choose to drop.
    if (inFlight)
      return {
        verdict: 'refuse',
        reason: 'Wait for the channel to finish being created.',
      };
    if (!typed) return null;
    return {
      verdict: 'prompt',
      title: 'Discard the new channel?',
      body: typedName
        ? `#${normalizeChannelName(typedName)} has not been created and will be lost.`
        : 'This channel has not been created and will be lost.',
      confirm: 'Discard',
      // Discard the form as well as allowing navigation. The sheet remains
      // mounted through team switches and would otherwise retain the text.
      onConfirm: () => closeRef.current(),
    };
  }, []);
  useNavigationGuard(formGuard, [], !locked);
  const create = async () => {
    if (sending.current || !picked) return;
    const store = picked.store;
    sending.current = true;
    setBusy(true);
    setError('');
    // The conversation guards every request on the shell's availability and on
    // the access generation the server is on; a channel created from this sheet
    // is the same kind of write and is guarded the same way.
    const generation = generationsRef.current?.get(store.server) ?? 0;
    const authorize = (phase: 'before' | 'after'): void => {
      const available = chatAvailable(snapshotRef.current, store, {
        ...accessOptionsRef.current,
        nowSeconds: accessNow(),
      });
      if (
        available &&
        generation === (generationsRef.current?.get(store.server) ?? 0)
      )
        return;
      throw {
        code: available ? 'access-changed' : 'chat-access-denied',
        message:
          phase === 'after'
            ? 'Access changed after the channel was sent. Its saved preparation is kept; recover it once access is back.'
            : 'Access changed before the channel was sent.',
        fatal: false,
        // A request that may already have been taken is not retryable on its
        // own: the saved preparation is what settles it.
        ambiguous: phase === 'after',
        retryable: phase === 'before',
      };
    };
    const client = chatClient(bridge, store.server, store.id);
    // The service holds the scope this team's chat has been established under.
    // A reply under a different identity is not this team's, so the account is
    // quarantined rather than trusted for one more request.
    const trust = (reply: ChatReply): void => {
      const trusted = service.getSnapshot().get(store.id)?.scope;
      if (trusted && !sameScope(trusted, reply.scope)) {
        service.block(store.id, 'The chat identity changed.');
        throw integrity();
      }
    };
    try {
      if (!prepared.current) {
        submission.current ??= {
          action: 'prepare-channel',
          submission: submissionId(),
          name,
          description,
          admin,
        };
        setOutstanding(true);
        const reply: ChatReply = await client.request(
          submission.current,
          undefined,
          authorize,
        );
        if (!alive.current) return;
        trust(reply);
        if (reply.result.kind !== 'operation')
          throw new Error('Invalid channel preparation.');
        prepared.current = {
          operation: reply.result.operation.id,
          channel: reply.result.operation.channel,
        };
      }
      const attempted: ChatReply = await client.request(
        { action: 'attempt', operation: prepared.current.operation },
        undefined,
        authorize,
      );
      if (!alive.current) return;
      trust(attempted);
      const created =
        attempted.result.kind === 'operation'
          ? attempted.result.operation.channel
          : prepared.current.channel;
      // Only a settled attempt retires the submission: until then it is what a
      // recovery re-issues.
      submission.current = null;
      held.current = false;
      prepared.current = null;
      setOutstanding(false);
      // The column reads the service, so the new channel is listed only once
      // the team has been synchronized again.
      service.invalidate(store.id);
      onOpen(store.id, created);
    } catch (cause) {
      if (alive.current) {
        const failed = normalizeCommandError(cause);
        // An ambiguous failure — the request may have been taken, the reply was
        // lost — leaves the submission where it is, and marks it as one the
        // agent may hold from here on.
        if (failed.ambiguous) held.current = true;
        // A preparation the agent refused on its content can be changed and
        // sent again; one it may have taken cannot, and is recovered as it
        // stands. That includes a later refusal of a request that never
        // reached the agent: it says nothing about the one that did.
        if (
          !prepared.current &&
          submission.current &&
          !held.current &&
          preparationCanChange(cause)
        ) {
          submission.current = null;
          setOutstanding(false);
        }
        // A fatal failure ends the session this submission was made in: there
        // is nothing left to recover it with, so the sheet states the reason
        // and can be closed without preventing the user from navigating away.
        if (failed.fatal) {
          submission.current = null;
          held.current = false;
          prepared.current = null;
          setOutstanding(false);
        }
        setError(failure(cause));
        // Whatever the agent is still holding belongs in the conversation's
        // "Needs attention" as well, in case the sheet is closed on it.
        if (prepared.current) service.invalidate(store.id);
      }
    } finally {
      client.dispose();
      sending.current = false;
      if (alive.current) setBusy(false);
    }
  };
  const back = () => {
    if (creating) {
      setCreating(false);
      setError('');
      return;
    }
    setStep('team');
    setChannel(undefined);
  };
  return (
    <SheetDialog
      title={
        creating
          ? 'New channel'
          : step === 'team'
            ? 'New chat'
            : 'Pick a channel'
      }
      onClose={onClose}
      dismissible={!locked}
      footer={
        <>
          {/* Back belongs to the step behind this one. A sheet opened on one
              team has none — the cross-team picker is not where it came
              from — so its left button leaves instead. */}
          <Button
            disabled={locked}
            onClick={step === 'team' || fixedTeam ? onClose : back}
          >
            {step === 'team' || fixedTeam ? 'Cancel' : 'Back'}
          </Button>
          {step === 'team' ? (
            <Button
              variant="primary"
              disabled={!picked || Boolean(picked.reason) || !channelsKnown}
              onClick={() => setStep('channel')}
            >
              Continue
            </Button>
          ) : creating ? (
            <Button
              variant="primary"
              busy={busy}
              disabled={createDisabled}
              onClick={() => void create()}
            >
              {busy
                ? 'Creating…'
                : outstanding
                  ? 'Retry channel creation'
                  : 'Create channel'}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={!channel || Boolean(picked?.reason)}
              onClick={() =>
                picked && channel && onOpen(picked.store.id, channel)
              }
            >
              Open chat
            </Button>
          )}
        </>
      }
    >
      <div ref={body}>
        {step === 'team' ? (
          <RadioGroup label="Team" className="chat-pick">
            {choices.map((choice) => (
              <RadioCard
                key={choice.store.id}
                title={
                  <>
                    <GroupMark store={choice.store} size="sm" />
                    {choice.store.name}
                  </>
                }
                detail={choice.reason || choice.detail}
                selected={choice.store.id === chosen}
                off={Boolean(choice.reason)}
                onSelect={() => {
                  setChosen(choice.store.id);
                  setChannel(undefined);
                }}
              />
            ))}
          </RadioGroup>
        ) : (
          <form
            className="chat-create"
            // Submit on Enter only when the Create button is enabled.
            onSubmit={(event) => {
              event.preventDefault();
              if (creating && !createDisabled) void create();
            }}
          >
            {/* A team that loses chat while this step is open is not submitted
              against: the reason stands where the channels would be. */}
            {picked?.reason && (
              <p role="alert" className="action-error">
                Chat is currently unavailable for {picked.store.name}:{' '}
                {picked.reason}
              </p>
            )}
            <RadioGroup label="Channel" className="chat-pick">
              {listed.map((option) => {
                // A channel the column draws as stopped, restricted, hidden or
                // muted says the same thing here: a picker that offered it as
                // an ordinary channel would be offering something else.
                const meta = channelMeta(option, entry?.blockedChannels);
                const about =
                  option.channel.description || accessSummary(option.channel);
                return (
                  <RadioCard
                    key={option.channel.id}
                    title={channelTitle(option.channel)}
                    detail={meta ? `${meta} · ${about}` : about}
                    selected={!creating && option.channel.id === channel}
                    disabled={locked || Boolean(picked?.reason)}
                    onSelect={() => {
                      setCreating(false);
                      setChannel(option.channel.id);
                    }}
                  />
                );
              })}
              {/* Until the team's channel list has arrived there is nothing to
                pick from and no way to tell whether a name is already taken. */}
              {channelsKnown ? (
                <RadioCard
                  title="Create a channel"
                  detail="A new channel in this team"
                  selected={creating}
                  disabled={locked || Boolean(picked?.reason)}
                  onSelect={() => {
                    setChannel(undefined);
                    setCreating(true);
                  }}
                />
              ) : (
                <p className="hint" role="status">
                  Loading channels…
                </p>
              )}
            </RadioGroup>
            {creating && (
              <>
                <Inset>
                  <InsetRow label="Channel name">
                    <input
                      aria-label="Channel name"
                      ref={nameField}
                      data-sheet-autofocus="true"
                      value={name}
                      disabled={locked}
                      onChange={(event) => setName(event.target.value)}
                      placeholder="design"
                    />
                    {/* The problem is drawn whenever there is one: an empty
                      name is itself refused when the team already has a general
                      channel, and the hint that describes an empty name would
                      be saying it can be created. */}
                    <small className={problem ? 'action-error' : ''}>
                      {problem ??
                        (name
                          ? `Created as #${normalizeChannelName(name)}.`
                          : `${CHAT_NAME_MIN_CHARS}–${CHAT_NAME_MAX_CHARS} characters, lowercased automatically. An empty name creates the team’s general channel.`)}
                    </small>
                  </InsetRow>
                  <InsetRow label="Description">
                    <textarea
                      aria-label="Channel description"
                      value={description}
                      disabled={locked}
                      onChange={(event) => setDescription(event.target.value)}
                      placeholder="What this channel is for"
                    />
                    <small className={descriptionProblem ? 'action-error' : ''}>
                      {descriptionProblem ??
                        `Optional, ${CHAT_DESCRIPTION_MIN_CHARS}–${CHAT_DESCRIPTION_MAX_CHARS} characters. Descriptions are lowercased automatically.`}
                    </small>
                  </InsetRow>
                </Inset>
                <SectionLabel>Who can take part</SectionLabel>
                <Inset>
                  <RadioGroup label="Channel audience">
                    <RadioCard
                      title="Everyone on the team"
                      detail="Members and administrators can view and post messages."
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
                {outstanding && !busy && (
                  <p className="hint">
                    The server did not respond. Select Retry to resend the
                    request without creating a duplicate.
                  </p>
                )}
              </>
            )}
            {error && (
              <p role="alert" className="action-error">
                {error}
              </p>
            )}
          </form>
        )}
      </div>
    </SheetDialog>
  );
}
