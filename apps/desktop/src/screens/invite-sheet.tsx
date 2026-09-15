/**
 * Invite someone to a server.
 *
 * An invitation is a message, not a link: FOKS grants nothing until an Owner
 * or an Admin adds the username someone replies with. The account is therefore
 * the first choice on the sheet — it decides the server the invitee signs up
 * on, the groups their username can later be added to, and every line of the
 * message — and the group below it only names a group in that message.
 *
 * The agent reports no signup code and no expiry for one, so neither is drawn:
 * the mock's signup-code panel would be stating a fact this Mac does not hold.
 * What the message cannot vouch for is said under it instead — the installer
 * comes from the sender, and a server may require a signup code no command
 * here can mint.
 */

import { useState } from 'react';
import type { ReactNode } from 'react';
import {
  Band,
  Button,
  CopyBox,
  Icon,
  Inset,
  InsetRow,
  RadioCard,
  RadioGroup,
  SectionLabel,
  SheetDialog,
} from '../components';
import { accountStores, serverName, storeReadable, usernameOf } from '../model';
import type {
  AccountStore,
  AgentSnapshot,
  StoreRef,
  TeamStore,
} from '../model';
import type { Bridge } from '../bridge';
import { useToast } from '/kit/toasts';
import { inviteUnavailableTitle } from './group-model';

/** The groups on one account's server that a username could be added to. */
function invitableGroups(
  snapshot: AgentSnapshot,
  account: AccountStore,
): TeamStore[] {
  return snapshot.stores.filter(
    (store): store is TeamStore =>
      store.kind === 'team' &&
      store.team_kind === 'named' &&
      store.active !== false &&
      store.server === account.server &&
      store.account === account.account,
  );
}

/**
 * The message, assembled from the account it is sent as and the group it
 * names. Nothing in it grants access: the last step is always the manual one.
 */
function inviteMessage(
  snapshot: AgentSnapshot,
  account: AccountStore,
  group: TeamStore | undefined,
): string {
  const host = serverName(snapshot, account);
  const address =
    snapshot.servers.find((server) => server.id === account.server)
      ?.configuredProbe ?? account.server;
  // The username as the server holds it: the whole of it is what the reader
  // types back, so it is not abbreviated here.
  const inviter = usernameOf(snapshot, account) ?? account.account;
  const lines = [
    group
      ? `I'd like to add you to ${group.name} on FOKS.`
      : `I'd like to invite you to FOKS on ${host}.`,
    '',
    '1. Ask me for the FOKS installer and install it',
    `2. When it asks for a server address, enter ${address}`,
    // No convention is invented for the username: the server decides what it
    // will accept, and this Mac holds no rule about it.
    '3. Create your account on the server',
    `4. Send your username to ${inviter}. FOKS does not use invite links.`,
    '',
    group
      ? `${group.name} appears for you once ${inviter} adds that username.`
      : `A group appears for you once ${inviter} adds that username to one.`,
  ];
  return lines.join('\n');
}

export function InviteSheet({
  snapshot,
  bridge,
  account,
  group,
  onClose,
  onError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  /** The account the sheet opens as; the picker may change it. */
  account: AccountStore;
  /** The group the sheet opens on, when it was opened from one. */
  group?: TeamStore;
  onClose: () => void;
  onError: (error: unknown) => void;
}): ReactNode {
  const toasts = useToast();
  const accounts = accountStores(snapshot);
  const [actingId, setActingId] = useState<StoreRef>(account.id);
  const acting =
    accounts.find((candidate) => candidate.id === actingId) ?? account;
  const groups = invitableGroups(snapshot, acting);
  const [groupId, setGroupId] = useState<StoreRef | null>(group?.id ?? null);
  // A group belongs to the account that holds it, so switching account drops a
  // choice the new server has never heard of.
  const named = groups.find((candidate) => candidate.id === groupId);
  const host = serverName(snapshot, acting);
  const inviter = usernameOf(snapshot, acting) ?? acting.account;
  const readable = storeReadable(snapshot, acting.id);
  const message = inviteMessage(snapshot, acting, named);
  // The servers the invitee could *not* be added from: a second account of
  // this Mac's on the same server is not one of them, and naming it would
  // contradict the sentence above it.
  const others = [
    ...new Set(
      accounts
        .filter((candidate) => candidate.server !== acting.server)
        .map((candidate) => serverName(snapshot, candidate)),
    ),
  ];
  const copy = (text: string, done: string): void => {
    void bridge
      .copyText(text)
      .then(() => toasts.show(done))
      .catch(onError);
  };
  return (
    <SheetDialog
      onClose={onClose}
      width="wide"
      glyph={
        <span className="server-mark">
          <Icon name="server" />
        </span>
      }
      title="Invite someone"
      subtitle="Send setup instructions for one server. You add their username to a group afterwards."
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            icon="copy"
            disabled={!readable}
            title={readable ? undefined : inviteUnavailableTitle(host)}
            onClick={() => copy(message, 'Message copied.')}
          >
            Copy message
          </Button>
        </>
      }
    >
      <>
        <SectionLabel>Invite as</SectionLabel>
        <Inset>
          <RadioGroup label="Invite as">
            {accounts.map((candidate) => {
              const stopped = !storeReadable(snapshot, candidate.id);
              return (
                <RadioCard
                  key={candidate.id}
                  selected={candidate.id === acting.id}
                  onSelect={() => {
                    setActingId(candidate.id);
                    setGroupId(null);
                  }}
                  title={usernameOf(snapshot, candidate) ?? candidate.account}
                  detail={
                    stopped
                      ? inviteUnavailableTitle(serverName(snapshot, candidate))
                      : `${serverName(snapshot, candidate)} · ${candidate.account} account`
                  }
                />
              );
            })}
          </RadioGroup>
        </Inset>
        {/* The consequence of the choice above, in the one sentence that
            decides whether it was the right one. */}
        <Band
          severity="info"
          label={`You are inviting as ${inviter}, on ${host}`}
        >
          They create an account on {host}, and you add that username to groups
          on {host}.
          {others.length
            ? ` An account on ${others.join(
                ' or ',
              )} cannot be added to them; people on another server join only as part of an admitted group.`
            : ' People on another server join only as part of an admitted group.'}
        </Band>
        <SectionLabel>Which group you plan to add them to</SectionLabel>
        <Inset>
          {groups.length ? (
            <RadioGroup label="Which group you plan to add them to">
              <RadioCard
                selected={!named}
                onSelect={() => setGroupId(null)}
                title="No group yet"
                detail="The message names no group and asks only for the username."
              />
              {groups.map((candidate) => (
                <RadioCard
                  key={candidate.id}
                  selected={named?.id === candidate.id}
                  onSelect={() => setGroupId(candidate.id)}
                  title={candidate.name}
                  detail="Only names the group in the message. Nothing is reserved and no access is granted."
                />
              ))}
            </RadioGroup>
          ) : (
            <InsetRow label="Group">
              <span className="dim">No group on {host} yet</span>
            </InsetRow>
          )}
        </Inset>
        <SectionLabel>Message</SectionLabel>
        {/* The footer's button copies the message; this one copies the same
            text from where it is read, so it is named for the box, not for
            the sheet's own action. */}
        <CopyBox
          text={message}
          onCopy={(text) => copy(text, 'Message copied.')}
        />
        {/* The requirements this Mac cannot determine for another server. */}
        <p className="fn">
          Send the installer with these instructions. {host} may require a
          signup code; get that code from the server administrator.
        </p>
        <p className="fn">
          FOKS has no invite links. This message is plain text and grants
          nothing by itself: access begins only when an Owner or an Admin adds
          the username to a group.
        </p>
      </>
    </SheetDialog>
  );
}
