/**
 * Choosing the vault an item is written to and the roles it takes there,
 * shared by the new-item sheet and Copy to vault.
 */

import type { ReactNode } from 'react';
import {
  Field,
  Inset,
  RadioCard,
  RadioGroup,
  SectionLabel,
} from '../components';
import type { CardOption } from '../components';
import {
  canCreateInStore,
  leaseLapsed,
  partiesOf,
  partyName,
  readersOf,
  serverBlocked,
  serverDisplayLabelForStore,
  serverLeaseUnavailable,
  storeDescription,
} from '../model';
import type { AgentSnapshot, Item, RoleWire, Store } from '../model';
import type { KvRoleInput } from '../bridge';

/**
 * Formats a store option for a vault selector. A team's detail names its
 * server; a personal vault's does when `server` asks for it.
 */
export function storeOption(
  snapshot: AgentSnapshot,
  store: Store,
  server = false,
): CardOption {
  const description = storeDescription(snapshot, store);
  const label = serverDisplayLabelForStore(snapshot, store);
  return {
    id: store.id,
    title: store.name,
    detail:
      store.kind === 'account'
        ? `Personal vault · ${description}${server ? ` · ${label}` : ''}`
        : `Team vault · ${description} · ${label}`,
    off: !canCreateInStore(snapshot, store.id),
  };
}

/**
 * Returns a user-facing explanation if the selected store is read-only or blocked.
 */
export function writeBlockReason(
  snapshot: AgentSnapshot,
  store: Store,
): string | null {
  if (canCreateInStore(snapshot, store.id)) return null;
  if (leaseLapsed(snapshot, store.id))
    return 'Your server session has expired. Check in again to save changes.';
  if (serverBlocked(snapshot, store.id))
    return 'Server access is blocked. Cannot create items in this vault.';
  if (serverLeaseUnavailable(snapshot, store.id))
    return 'The server connection is unavailable. New items cannot be saved to this vault right now.';
  if (store.kind === 'team' && !store.active)
    return 'Team setup is incomplete. Complete setup before adding items.';
  if (store.kind === 'team')
    return 'You do not have write permissions for this team.';
  return 'You do not have permission to create items in this vault.';
}

export function AccessBlock({
  snapshot,
  store,
  readRole,
  writeRole,
  onReadRole,
  onWriteRole,
}: {
  snapshot: AgentSnapshot;
  store: Store;
  readRole: KvRoleInput;
  writeRole: KvRoleInput;
  onReadRole: (role: KvRoleInput) => void;
  onWriteRole: (role: KvRoleInput) => void;
}): ReactNode {
  if (store.kind !== 'team') {
    return null;
  }
  const roleWire = (role: KvRoleInput): RoleWire =>
    role === 'Owner' || role === 'Admin'
      ? { role }
      : { role: 'Member', visibility: Number(role.slice('Member:'.length)) };
  const candidate: Item = {
    store: store.id,
    path: '/preview',
    direntId: '0'.repeat(32),
    kind: 'Secret',
    size: 0,
    version: 0,
    read: roleWire(readRole),
    write: roleWire(writeRole),
  };
  const roster = partiesOf(snapshot, store.id);
  const admitted = readersOf(snapshot, candidate) ?? [];
  const excluded = roster.filter((party) => !admitted.includes(party));
  const changers =
    readersOf(snapshot, { ...candidate, read: candidate.write }) ?? [];
  const readVisibility = readRole.startsWith('Member:')
    ? Number(readRole.slice('Member:'.length))
    : 0;
  const choices: readonly [string, KvRoleInput, string][] = [
    ['Owner', 'Owner', 'Only owners can read.'],
    ['Admin', 'Admin', 'Admins and owners can read.'],
    [
      'Member',
      `Member:${readVisibility}`,
      'Members at this visibility level and above can read.',
    ],
  ];
  const roleChoices = (
    selected: KvRoleInput,
    choose: (role: KvRoleInput) => void,
    forWrite: boolean,
  ) =>
    choices.map(([label, role, detail]) => {
      const next =
        label === 'Member'
          ? (`Member:${selected.startsWith('Member:') ? Number(selected.slice('Member:'.length)) : 0}` as KvRoleInput)
          : role;
      const on =
        selected === next ||
        (label === 'Member' && selected.startsWith('Member:'));
      return (
        <RadioCard
          key={label}
          selected={on}
          onSelect={() => choose(next)}
          title={label}
          detail={
            forWrite
              ? 'Members with Admin or Owner roles can modify or delete this item.'
              : detail
          }
        />
      );
    });
  return (
    <>
      <SectionLabel
        action={
          <span className="pv">
            accessible to {admitted.length} of {roster.length}
          </span>
        }
      >
        Who can read
      </SectionLabel>
      <Inset>
        <RadioGroup label="Who can read">
          {roleChoices(readRole, onReadRole, false)}
        </RadioGroup>
        {readRole.startsWith('Member:') ? (
          <Field
            label="Member visibility"
            type="number"
            mono
            min={-32768}
            max={32767}
            value={String(readVisibility)}
            onChange={(val) => {
              const next = Number(val);
              if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                onReadRole(`Member:${next}`);
            }}
          />
        ) : null}
      </Inset>
      <p className="hint">
        At{' '}
        <b>
          {readRole === 'Owner' || readRole === 'Admin'
            ? readRole
            : `Member · visibility ${readVisibility}`}
        </b>{' '}
        this item will be readable by{' '}
        <b>
          {admitted.length} of {roster.length}
        </b>{' '}
        members in {store.name}: {admitted.map(partyName).join(', ') || 'none'}.
        {excluded.length
          ? ` ${excluded.map(partyName).join(', ')} cannot read this item under the selected role.`
          : ''}
      </p>
      <SectionLabel>
        Who can change{' '}
        <span className="pv">
          changeable by {changers.length} of {roster.length}
        </span>
      </SectionLabel>
      <Inset>
        <RadioGroup label="Who can change">
          {roleChoices(writeRole, onWriteRole, true)}
        </RadioGroup>
        {writeRole.startsWith('Member:') ? (
          <Field
            label="Member visibility"
            type="number"
            mono
            min={-32768}
            max={32767}
            value={writeRole.slice('Member:'.length)}
            onChange={(val) => {
              const next = Number(val);
              if (Number.isInteger(next) && next >= -32768 && next <= 32767)
                onWriteRole(`Member:${next}`);
            }}
          />
        ) : null}
      </Inset>
      <p className="hint">
        Read and write permissions are set separately. A member with write
        permission can update this item even if they cannot view its contents.
      </p>
    </>
  );
}
