import { useChannelCreation } from '../chat/channel-creation-provider';
import './chat-picker.css';
/**
 * New chat: search existing conversations or open one channel-creation form.
 *
 * Chat supports channels in named teams, not one-to-one conversations. Search
 * results open directly across teams. A team-specific entry opens the create
 * form and shows Cancel instead of Back. Unavailable teams remain listed with
 * an explanation. Unsubmitted forms survive same-session rail tab changes.
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

import {
  CHAT_DESCRIPTION_MAX_CHARS,
  CHAT_DESCRIPTION_MIN_CHARS,
  CHAT_NAME_MAX_CHARS,
  CHAT_NAME_MIN_CHARS,
} from '../chat-limits';
import type { NavigationGuard } from '../location';
import { useNavigationGuard, useTabSheetState } from '../navigation-guard';
import type { ChatScope } from '../chat-contract';
import { sameScope } from '../chat/client';
import { chatAvailable, serverDisplayName, storeDescription } from '../model';
import type {
  AgentSnapshot,
  AvailabilityOptions,
  StoreRef,
  TeamStore,
} from '../model';
import { failure } from '../chat/actions';
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
import {
  chatTeams,
  noChatReason,
  noChatTeams,
  pickerConversations,
} from './chat-teams';
import { GroupMark } from './group-mark';

/** A team the sheet offers, and why it cannot be offered. */
interface TeamChoice {
  store: TeamStore;
  /** Empty when the team can be chatted in. */
  reason: string;
  detail: string;
}

interface ChannelFormDraft {
  team?: StoreRef;
  chosen?: StoreRef;
  scope?: ChatScope;
  creating: boolean;
  name: string;
  description: string;
  admin: boolean;
}

export function NewChatSheet({
  snapshot,
  team,
  accessOptions = {},
  accessGenerations,
  onClose,
  onOpen,
  onSubmitted,
  onDraft,
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
  onSubmitted?: () => void;
  onDraft?: () => void;
  /**
   * Legacy notification seam. Submission ownership now belongs to the
   * application controller; the sheet does not block navigation while a
   * preparation is unresolved.
   */
  onUnresolved?: (unresolved: boolean) => void;
}): ReactNode {
  const { snapshot: inbox } = useChatInbox();
  // A sheet opened on a team is that team's: there is no step behind it to go
  // back to, so it opens on the channel it was asked for — the create step —
  // and leaves by being cancelled.
  const fixedTeam = team !== undefined;
  const [saved, setSaved] = useTabSheetState<ChannelFormDraft | null>(
    'channel.form',
    null,
  );
  const restored = saved?.team === team ? saved : null;
  const [chosen, setChosen] = useState<StoreRef | undefined>(
    restored?.chosen ?? team,
  );
  const [creating, setCreating] = useState(restored?.creating ?? fixedTeam);
  const [name, setName] = useState(restored?.name ?? '');
  const [description, setDescription] = useState(restored?.description ?? '');
  const [admin, setAdmin] = useState(restored?.admin ?? false);
  const formScope = useRef({ store: restored?.chosen, scope: restored?.scope });
  const closing = useRef(false);
  const autoOpen = useRef(false);
  const submittedRef = useRef(onSubmitted);
  submittedRef.current = onSubmitted;
  const [filter, setFilter] = useState('');
  const [error, setError] = useState('');
  const { controller, creations } = useChannelCreation();
  const [creationId, setCreationId] = useState<string>();
  const active = creations.find((record) => record.id === creationId);
  const draftRef = useRef(onDraft);
  draftRef.current = onDraft;
  useEffect(() => {
    if (active?.state !== 'cancelled') return;
    setError(active.error);
    setCreationId(undefined);
    autoOpen.current = false;
    controller?.acknowledge(active.id);
    draftRef.current?.();
  }, [active, controller]);
  useEffect(() => {
    if (closing.current) return;
    if (creationId) {
      setSaved(null);
      return;
    }
    const scope = chosen ? inbox.get(chosen)?.scope : undefined;
    if (formScope.current.store !== chosen)
      formScope.current = { store: chosen, scope };
    if (
      formScope.current.scope &&
      scope &&
      !sameScope(formScope.current.scope, scope)
    ) {
      formScope.current = { store: chosen, scope };
      setName('');
      setDescription('');
      setAdmin(false);
      setSaved(null);
      setError(
        'The chat identity changed. Review a new channel form before submitting.',
      );
      return;
    }
    formScope.current = {
      store: chosen,
      scope: scope ?? formScope.current.scope,
    };
    setSaved({
      team,
      chosen,
      creating,
      name,
      description,
      admin,
      scope: formScope.current.scope
        ? structuredClone(formScope.current.scope)
        : undefined,
    });
  }, [
    team,
    chosen,
    creating,
    name,
    description,
    admin,
    creationId,
    inbox,
    setSaved,
  ]);
  const close = () => {
    closing.current = true;
    autoOpen.current = false;
    setSaved(null);
    onClose();
  };
  const open = (ref: StoreRef, channel?: string) => {
    closing.current = true;
    setSaved(null);
    onOpen(ref, channel);
  };
  const busy = active?.state === 'working';
  const outstanding = Boolean(
    active && active.state !== 'confirmed' && active.state !== 'cancelled',
  );
  // The preparation the agent has been given but has not finished: the
  // `prepare-channel` until it is accepted, then the operation identifier it
  // answered with. Recovering re-issues the same one, so a lost reply is
  // retried rather than turned into a second channel.
  const submission = creationId !== undefined && active?.state !== 'cancelled';
  // Whether the agent may already hold that submission: a request whose outcome
  // is unknown is one it may have taken, and from then on the only way to find
  // out is to re-issue the same one. A later refusal — of a request that never
  // reached the agent, say — does not make it discardable.
  const nameField = useRef<HTMLInputElement | null>(null);
  const body = useRef<HTMLDivElement | null>(null);
  const opened = useRef(false);
  // The controller checks current access around each request. This sheet
  // observes a confirmed result only while the initiating interaction is
  // still active; restoring an unsubmitted form never restores a redirect
  // subscription from an abandoned submission.
  const openedCreation = useRef<string | undefined>(undefined);
  const openRef = useRef(open);
  openRef.current = open;
  useEffect(() => {
    if (
      !autoOpen.current ||
      active?.state !== 'confirmed' ||
      !active.operation ||
      openedCreation.current === active.id
    )
      return;
    openedCreation.current = active.id;
    openRef.current(active.store.id, active.operation.channel);
    controller?.acknowledge(active.id);
  }, [active, controller]);
  useEffect(() => {
    return () => {
      // Submitted work outlives this sheet in the application controller.
      // Opening a later sheet does not restore this sheet's completion latch
      // or redirect the reader back to its original team.
      openedCreation.current = undefined;
    };
  }, []);
  useEffect(() => {
    for (const store of chatTeams(snapshot)) void controller?.discover(store);
  }, [controller, snapshot, inbox, accessGenerations]);
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
  }, [creating]);
  const choices: TeamChoice[] = [
    ...chatTeams(snapshot).map((store) => {
      const reachable =
        chatAvailable(snapshot, store, accessOptions) &&
        inbox.get(store.id)?.state !== 'blocked';
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
  // A length outside the limits is shown by the field alone — its border —
  // and the hint stays as it is; every other refusal is a sentence, since a
  // border cannot say that a name is taken.
  const nameLength = [...normalizeChannelName(name)].length;
  const nameOverLimit =
    nameLength > CHAT_NAME_MAX_CHARS ||
    (nameLength > 0 && nameLength < CHAT_NAME_MIN_CHARS);
  const descriptionOverLimit = descriptionProblem !== null;
  // Creation cannot be submitted until the team's channels are known: an empty
  // name is the general channel, and whether the team already has one is the
  // difference between creating it and being refused.
  const channelsKnown = Boolean(entry?.data);
  const locked = busy || outstanding;
  // What refuses a channel the sheet has not sent yet. A preparation the agent
  // has already taken is past all of it: its fields are frozen, so the only
  // thing that can stop a recovery is a recovery already running.
  const unsendable =
    !picked ||
    !controller?.readyFor(picked.store.id) ||
    problem !== null ||
    descriptionProblem !== null ||
    !channelsKnown ||
    Boolean(picked?.reason);
  const createDisabled =
    !controller ||
    busy ||
    active?.state === 'review' ||
    (!outstanding && unsendable);
  // Read by the guard when it is asked, so typing re-registers nothing.
  // Unsubmitted fields are restored on rail changes. Other destructive moves
  // can ask for discard; submitted work never holds navigation open.
  const guardState = useRef({ typed: false, name: '', locked: false });
  guardState.current = {
    typed:
      creating && !submission && Boolean(name.trim() || description.trim()),
    name: name.trim(),
    locked,
  };
  const closeRef = useRef(close);
  closeRef.current = close;
  const formGuard = useCallback<NavigationGuard>(() => {
    const { typed, name: typedName } = guardState.current;
    // The application owns submitted work independently of this sheet.
    // Only unsubmitted form fields need the destructive-navigation prompt.
    if (!typed) return null;
    return {
      verdict: 'prompt',
      title: 'Discard the new channel?',
      body: typedName
        ? `#${normalizeChannelName(typedName)} has not been created and will be lost.`
        : 'This channel has not been created and will be lost.',
      confirm: 'Discard',
      // Discard the saved tab form as well as allowing navigation, so a
      // later visit cannot restore fields the reader chose to discard.
      onConfirm: () => closeRef.current(),
    };
  }, []);
  useNavigationGuard(formGuard, [], true);
  const create = () => {
    if (!controller || createDisabled) return;
    setError('');
    if (outstanding && active) {
      autoOpen.current = true;
      if (
        !active.operation ||
        (active.operation.state === 'prepared' && active.input)
      )
        controller.retry(active.id);
      else controller.check(active.id);
      return;
    }
    if (!picked) return;
    try {
      const id = controller.submit(picked.store, { name, description, admin });
      autoOpen.current = true;
      setSaved(null);
      setCreationId(id);
      onSubmitted?.();
    } catch (cause) {
      setError(failure(cause));
    }
  };
  const back = () => {
    setCreating(false);
    setCreationId(undefined);
    setError('');
  };
  const recover = (id: string) => {
    const record = creations.find((candidate) => candidate.id === id);
    if (!record) return;
    setChosen(record.store.id);
    setName(record.input?.name ?? '');
    setDescription(record.input?.description ?? '');
    setAdmin(record.input?.admin ?? false);
    setCreationId(id);
    setSaved(null);
    autoOpen.current = true;
    onSubmitted?.();
    setCreating(true);
    setError('');
  };
  const pending = creations.filter(
    (record) => record.state !== 'confirmed' && record.state !== 'cancelled',
  );
  const query = filter.trim().toLowerCase();
  const results = pickerConversations(snapshot, inbox, query, accessOptions);
  useEffect(() => {
    if (!creating || creationId) return;
    const saved = creations.find(
      (record) =>
        record.store.id === chosen &&
        record.state !== 'confirmed' &&
        record.state !== 'cancelled',
    );
    if (!saved) return;
    setName(saved.input?.name ?? '');
    setDescription(saved.input?.description ?? '');
    setAdmin(saved.input?.admin ?? false);
    setCreationId(saved.id);
    setSaved(null);
    submittedRef.current?.();
  }, [creating, creationId, chosen, creations, setSaved]);
  return (
    <SheetDialog
      title={creating ? 'Create channel' : 'New chat'}
      onClose={close}
      dismissible
      footer={
        <>
          {/* Back belongs to the step behind this one. A sheet opened on one
              team has none — the cross-team picker is not where it came
              from — so its left button leaves instead. */}
          <Button
            onClick={!creating || fixedTeam || outstanding ? close : back}
          >
            {outstanding ? 'Close' : !creating || fixedTeam ? 'Cancel' : 'Back'}
          </Button>
          {creating ? (
            <Button
              variant="primary"
              busy={busy}
              disabled={createDisabled}
              onClick={create}
            >
              {busy
                ? 'Creating…'
                : outstanding
                  ? active?.operation
                    ? active.operation.state === 'prepared' && active.input
                      ? 'Retry creation'
                      : 'Check again'
                    : 'Retry channel creation'
                  : 'Create channel'}
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={() => {
                setCreating(true);
                setCreationId(undefined);
                setChosen(team);
                setName('');
                setDescription('');
                setAdmin(false);
              }}
            >
              Create channel
            </Button>
          )}
        </>
      }
    >
      <div ref={body}>
        {!creating ? (
          <div className="chat-picker">
            <input
              type="search"
              aria-label="Search conversations"
              placeholder="Search teams and channels"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              data-sheet-autofocus="true"
            />
            <div className="chat-picker-results" aria-label="Conversations">
              {results.map(({ store, entry: teamEntry, option, available }) => {
                // A channel the column draws as stopped, restricted, hidden or
                // muted says the same thing here: a picker that offered it as
                // an ordinary channel would be offering something else.
                const meta = channelMeta(option, teamEntry?.blockedChannels);
                const about =
                  option.channel.description || accessSummary(option.channel);
                return (
                  <button
                    type="button"
                    className="chat-picker-result"
                    key={`${store.id}:${option.channel.id}`}
                    disabled={!available}
                    onClick={() => open(store.id, option.channel.id)}
                  >
                    <GroupMark store={store} size="sm" />
                    <span>
                      <b>
                        {store.name} · {channelTitle(option.channel)}
                      </b>
                      <small>{meta ? `${meta} · ${about}` : about}</small>
                    </span>
                  </button>
                );
              })}
              {!results.length && (
                <p role="status">
                  No conversations match{query ? ` “${filter}”` : ''}.
                </p>
              )}
              {choices
                .filter(
                  (choice) =>
                    (!query ||
                      choice.store.name.toLowerCase().includes(query)) &&
                    (choice.reason || !inbox.get(choice.store.id)?.data),
                )
                .map((choice) => (
                  <p className="hint" key={choice.store.id}>
                    {choice.store.name}:{' '}
                    {choice.reason ||
                      inbox.get(choice.store.id)?.error ||
                      'Loading channels…'}
                  </p>
                ))}
            </div>
            {pending.length > 0 && (
              <section aria-label="Pending channel creation">
                <SectionLabel>Channel creation</SectionLabel>
                {pending.map((record) => (
                  <button
                    type="button"
                    className="chat-picker-result"
                    key={record.id}
                    onClick={() => recover(record.id)}
                  >
                    <span>
                      <b>
                        {record.store.name} ·{' '}
                        {record.input
                          ? `#${normalizeChannelName(record.input.name) || 'general'}`
                          : 'Saved channel creation'}
                      </b>
                      <small>
                        {record.state === 'working'
                          ? 'Creating…'
                          : record.error || 'Needs attention'}
                      </small>
                    </span>
                  </button>
                ))}
              </section>
            )}
          </div>
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
            {/* Choosing Create takes the form over: the channel list it was
                picked from is one Back away, not a second thing on screen. */}
            <Inset>
              <InsetRow label="Team">
                <select
                  aria-label="Team"
                  value={chosen ?? ''}
                  disabled={locked}
                  onChange={(event) => {
                    const next = event.target.value || undefined;
                    const saved = pending.find(
                      (record) => record.store.id === next,
                    );
                    if (saved) recover(saved.id);
                    else {
                      setChosen(next);
                      setCreationId(undefined);
                    }
                  }}
                >
                  <option value="">Choose a team</option>
                  {choices.map((choice) => (
                    <option
                      key={choice.store.id}
                      value={choice.store.id}
                      disabled={Boolean(choice.reason)}
                    >
                      {choice.store.name}
                      {choice.reason ? ` — ${choice.reason}` : ''}
                    </option>
                  ))}
                  {active &&
                    !choices.some(
                      (choice) => choice.store.id === active.store.id,
                    ) && (
                      <option value={active.store.id}>
                        {active.store.name} — Unavailable
                      </option>
                    )}
                </select>
                <small>{picked?.reason || picked?.detail}</small>
              </InsetRow>
            </Inset>
            {/* Until the team's channel list has arrived there is nothing to
                pick from and no way to tell whether a name is already taken. */}
            {picked && !channelsKnown && (
              <p className="hint" role="status">
                {entry?.error || 'Loading channels…'}
              </p>
            )}
            {!controller && (
              <p role="status" className="action-error">
                Channel creation is unavailable in this window.
              </p>
            )}
            {controller && picked && !controller.readyFor(picked.store.id) && (
              <p role="status">
                Saved channel creations must be checked first.{' '}
                <Button onClick={() => void controller.discover(picked.store)}>
                  Check saved creations
                </Button>
              </p>
            )}
            {active && !active.input && outstanding && (
              <p className="hint">
                The original channel fields are held by the agent. Check the
                saved operation or cancel it before creating another channel.
              </p>
            )}
            {active?.operation && outstanding && (
              <Button
                disabled={
                  busy ||
                  active.blocked ||
                  active.operation.state === 'uncertain'
                }
                onClick={() => {
                  if (active.operation?.state === 'rejected') {
                    controller?.review(active.id);
                    setCreationId(undefined);
                    autoOpen.current = false;
                    onDraft?.();
                  } else controller?.cancel(active.id);
                }}
              >
                {active.operation.state === 'rejected'
                  ? 'Review form'
                  : 'Cancel preparation'}
              </Button>
            )}
            {creating && (
              <>
                <Inset>
                  <InsetRow label="Channel name">
                    <input
                      aria-label="Channel name"
                      ref={nameField}
                      data-sheet-autofocus="true"
                      className={nameOverLimit ? 'over' : undefined}
                      aria-invalid={nameOverLimit || undefined}
                      value={name}
                      disabled={locked}
                      onChange={(event) => setName(event.target.value)}
                      placeholder="design"
                    />
                    {/* A refusal other than length is drawn in place of the
                      hint: an empty name is itself refused when the team
                      already has a general channel, and the hint that
                      describes an empty name would be saying it can be
                      created. */}
                    <small
                      className={
                        problem && !nameOverLimit ? 'action-error' : ''
                      }
                    >
                      {problem && !nameOverLimit
                        ? problem
                        : `Lowercase, ${CHAT_NAME_MIN_CHARS}–${CHAT_NAME_MAX_CHARS} characters. Leave empty to create the team’s #general channel.`}
                    </small>
                  </InsetRow>
                  <InsetRow label="Description">
                    <textarea
                      aria-label="Channel description"
                      className={descriptionOverLimit ? 'over' : undefined}
                      aria-invalid={descriptionOverLimit || undefined}
                      rows={2}
                      value={description}
                      disabled={locked}
                      onChange={(event) => setDescription(event.target.value)}
                      placeholder="What this channel is for"
                    />
                    <small>
                      Optional. Lowercase, {CHAT_DESCRIPTION_MIN_CHARS}–
                      {CHAT_DESCRIPTION_MAX_CHARS} characters.
                    </small>
                  </InsetRow>
                </Inset>
                <SectionLabel>Visibility</SectionLabel>
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
                      detail="Hidden from members."
                      selected={admin}
                      disabled={locked}
                      onSelect={() => setAdmin(true)}
                    />
                  </RadioGroup>
                </Inset>
                {outstanding && !busy && (
                  <p className="hint">
                    Channel creation is not confirmed. You can close this form
                    and check the saved creation later.
                  </p>
                )}
              </>
            )}
            {(error || active?.error) && (
              <p role="alert" className="action-error">
                {error || active?.error}
              </p>
            )}
          </form>
        )}
      </div>
    </SheetDialog>
  );
}
