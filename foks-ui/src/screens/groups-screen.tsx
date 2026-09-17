import { useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { Dialog } from '/kit/overlay-primitives';
import { Band, Button, Chip, Inset, InsetRow, Notice, Stack } from '../components';
import {
  admissionActive,
  actionableGroupMember,
  catalog,
  formatRole,
  hue,
  kindOf,
  leaseLapsed,
  parseRole,
  partiesOf,
  partyName,
  peopleGroups,
  readersOf,
  safestRemovalTarget,
  serverBlocked,
  serverLeaseUnavailable,
  serverOf,
  storeDescriptionState,
  storeOf,
} from '../model';
import type { AccountStore, Item, Party, Store, StoreRef, World } from '../model';
import { normalizeCommandError } from '../bridge';
import type { Bridge } from '../bridge';
import type { RoleDto } from '../bridge';
import type { Location } from '../location';
import { StoreAccessTakeover } from './store-access';

type Tab = 'people' | 'federation' | 'store' | 'danger';
type Sheet = 'manage' | 'invite' | 'add' | 'demote' | 'remove' | 'admit' | 'create' | null;

const VIS_MIN = -32768;
const VIS_MAX = 32767;
const stateName = (): string =>
  typeof window === 'undefined' ? '' : new URLSearchParams(window.location.search).get('state') ?? '';

function shortId(value: string): string {
  return value.length > 14 ? `${value.slice(0, 10)}…${value.slice(-4)}` : value;
}

function itemsOf(world: World, store: string): Item[] {
  return catalog(world).filter((item) => item.store === store);
}

function readsOf(world: World, party: Party): Item[] {
  return itemsOf(world, party.store).filter((item) =>
    readersOf(world, item)?.some((candidate) => candidate.party_id_hex === party.party_id_hex),
  );
}

function canTarget(world: World, party: Party): boolean {
  return actionableGroupMember(world, party);
}

function demotionFor(party: Party): RoleDto | null {
  const role = parseRole(party.destination_role);
  if (!role) return null;
  if (role.kind === 'owner' || role.kind === 'admin') return { role: 'Member', visibility: 0 };
  const visibility = role.visibility ?? 0;
  return visibility > VIS_MIN ? { role: 'Member', visibility: visibility - 1 } : null;
}

function roleText(party: Party): string {
  return fmtRole(party.destination_role);
}

function partyShortName(party: Party): string {
  return party.party_kind === 'user'
    ? partyName(party)
    : party.team_name?.split(' @')[0] ?? partyName(party);
}

function fmtRole(role: Item['read']): string {
  const parsed = parseRole(role);
  return parsed ? formatRole(parsed) : typeof role === 'string' ? role : role.role;
}

function GroupMark({ store }: { store: Store }): ReactNode {
  return <span className="kico md group" style={{ background: store.kind === 'team' && store.active === false ? 'var(--c-none)' : hue(store.name) }}>{store.name.slice(0, 1)}</span>;
}

function GroupsList({ world, onNavigate, onSheet, onResume, onCopy }: {
  world: World;
  onNavigate: (location: Location) => void;
  onSheet: (sheet: Sheet) => void;
  onResume: (storeId: string) => void;
  onCopy?: (text: string) => void;
}): ReactNode {
  const groups = world.stores.filter((store) => store.kind === 'team');
  const canCreate = world.stores.some((store) =>
    store.kind === 'account' &&
    !leaseLapsed(world, store.id) &&
    !serverBlocked(world, store.id) &&
    !serverLeaseUnavailable(world, store.id) &&
    world.accounts.some((account) =>
      account.store === store.id || (account.alias === store.account && account.server === store.server),
    ),
  );
  return <>
    <div className="path groups-path"><div className="loc text-only"><h1>Groups</h1><small>{groups.length} groups</small><Button icon="plus" disabled={!canCreate} title={canCreate ? undefined : 'No available account can create a group'} onClick={() => onSheet('create')}>Create group</Button></div></div>
    <div className="body"><div className="groups-wrap"><div className="gcards">
      {groups.map((group) => {
        const parties = partiesOf(world, group.id);
        const server = serverOf(world, group.id);
        const account = world.accounts.find((candidate) => candidate.alias === group.account && candidate.server === group.server);
        const mine = parties.find((party) => party.label === 'you');
        const unavailable = leaseLapsed(world, group.id) || serverBlocked(world, group.id) || serverLeaseUnavailable(world, group.id);
        return <div className="gcard" key={group.id}>
          <div className="ghead"><GroupMark store={group} /><span className="t"><b>{group.name}</b><small>{server?.name} · {account?.username ?? group.account}</small></span>{mine && group.active ? <Chip>{roleText(mine)}</Chip> : null}<Chip tone={group.active ? undefined : 'warn'}>{group.active ? group.team_kind === 'adhoc' ? 'ad-hoc' : 'named' : 'Inactive'}</Chip></div>
          {unavailable ? <div className="party"><span className="t"><span className="hint">Roster unavailable while server access is stopped; no cached membership is shown.</span></span><Button size="sm" onClick={() => onNavigate({ kind: 'group-admin', ref: group.id })}>Review</Button></div>
            : group.active ? <div className="party"><Stack parties={parties} /><span className="t">{peopleGroups(parties)}</span><Button size="sm" onClick={() => onNavigate({ kind: 'group-admin', ref: group.id })}>Open</Button></div>
            : <div className="party"><span className="t"><span className="hint">Creation interrupted</span></span><Button size="sm" onClick={() => onResume(group.id)}>Resume creation</Button></div>}
        </div>;
      })}
    </div>
    <div className="sec">Your usernames</div><Inset className="rows group-usernames">{world.accounts.map((account) => <InsetRow key={`${account.server}|${account.alias}`} label="Your account on" action={onCopy ? <Button size="sm" onClick={() => onCopy(account.username)}>Copy</Button> : undefined}>{account.store ? serverOf(world, account.store)?.name ?? account.server : world.servers.find((server) => server.id === account.server)?.name ?? account.server} · {account.username}</InsetRow>)}</Inset>
    <p className="hint">Your own vaults are not groups; they live under Vaults.</p>
    <Band label="Group discovery is not available">This Mac currently lists only groups it created or added as members. Groups that someone else adds you to will not appear yet.</Band>
    </div></div>
  </>;
}

function PartyRow({ world, party, selected, onSelect, onSheet, onCopy, manageable }: {
  world: World;
  party: Party;
  selected: boolean;
  onSelect: () => void;
  onSheet: (sheet: Sheet, party?: Party) => void;
  onCopy: (value: string) => void;
  manageable: boolean;
}): ReactNode {
  const reads = readsOf(world, party);
  const total = itemsOf(world, party.store).length;
  const active = admissionActive(world, party, party.store);
  const source = fmtRole(party.source_role);
  const destination = roleText(party);
  const sourceDiffers = source !== destination;
  const machine = party.party_kind === 'user' && Boolean(party.note?.includes('service account'));
  return <div className={`row${selected ? ' sel' : ''}`} onClick={onSelect}>
    <span className="name"><span className="tt"><Stack parties={[party]} /><span>{partyShortName(party)}</span>{party.label ? <Chip>you</Chip> : null}{party.party_kind !== 'user' ? <span className="tag">group</span> : null}</span><small>{party.party_kind !== 'user' && party.team_name?.includes(' @') ? `@ ${party.team_name.split('@ ')[1]} · ` : ''}<code>{shortId(party.party_id_hex)}</code> <Button size="sm" onClick={(event) => { event.stopPropagation(); onCopy(party.party_id_hex); }}>Copy</Button>{!active ? <span className="lapsed"> membership inactive · no access here</span> : null}</small></span>
    <span>{party.party_kind !== 'user' ? 'group' : machine ? <><span>machine</span><span className="hint">label kept on this Mac</span></> : 'person'}</span><span><Chip>{destination}</Chip></span><span className="opt">{sourceDiffers ? <Chip>{source}</Chip> : <span className="dim">—</span>}</span><span className="n opt">{party.generation}</span><span className={`n${active ? '' : ' strike lapsed'}`}>{reads.length} of {total}</span>
    <span className="acts2">{selected ? <span className="hint">shown in the panel</span> : manageable && canTarget(world, party) ? <>{demotionFor(party) ? <Button size="sm" onClick={(event) => { event.stopPropagation(); onSheet('demote', party); }}>Change role</Button> : null}<Button size="sm" variant="danger" onClick={(event) => { event.stopPropagation(); onSheet('remove', party); }}>Remove…</Button></> : <span className="hint">{!manageable ? 'ad-hoc group management unavailable' : party.label ? 'your own membership — see Danger' : party.party_kind !== 'user' ? 'a member group — see Federation' : 'not a unique locally manageable username'}</span>}</span>
  </div>;
}

function PeopleTab({ world, store, selected, onSelect, onSheet, onRefresh, onCopy, manageable }: { world: World; store: Store; selected: Party | null; onSelect: (party: Party | null) => void; onSheet: (sheet: Sheet, party?: Party) => void; onRefresh: () => void; onCopy: (value: string) => void; manageable: boolean }): ReactNode {
  const parties = partiesOf(world, store.id);
  return <><div className="party"><span className="t"><b>{peopleGroups(parties)}</b> — each with the role it holds here and what that role lets it read</span><Button size="sm" onClick={onRefresh}>Re-read the roster</Button><Button size="sm" icon="plus" disabled={!manageable} onClick={() => onSheet('add')}>Add someone</Button></div>
    <div className={`rt${selected ? ' narrow' : ''}`}><div className="hdr"><span>Party</span><span>Kind</span><span>Role here</span><span className="opt">Role at source</span><span className="opt">Generation</span><span>Reads · of {itemsOf(world, store.id).length}</span><span>Actions</span></div>{parties.map((party) => <PartyRow key={party.party_id_hex} world={world} party={party} selected={selected?.party_id_hex === party.party_id_hex} onSelect={() => onSelect(selected?.party_id_hex === party.party_id_hex ? null : party)} onSheet={onSheet} onCopy={onCopy} manageable={manageable} />)}</div>
    <p className="hint">A party is a person or a group. Each party carries its source role and the role it holds here. Reads counts are computed from item read roles and this roster.</p></>;
}

function PartyPanel({ world, party, onClose, onSheet, onFederation, onCopy, manageable }: { world: World; party: Party; onClose: () => void; onSheet: (sheet: Sheet, party?: Party) => void; onFederation: () => void; onCopy: (text: string) => void; manageable: boolean }): ReactNode {
  const group = storeOf(world, party.store);
  const server = serverOf(world, party.store);
  const items = itemsOf(world, party.store);
  const readable = new Set(readsOf(world, party));
  const ordered = [...items].sort((left, right) => Number(readable.has(right)) - Number(readable.has(left)));
  const machine = party.party_kind === 'user' && Boolean(party.note?.includes('service account'));
  const source = fmtRole(party.source_role);
  const here = roleText(party);
  return <aside className="details"><div className="dh"><Stack parties={[party]} size="lg" /><span className="t"><h2>{partyShortName(party)} {party.label ? <Chip>you</Chip> : null}</h2><small>{party.party_kind !== 'user' ? `a member group on ${party.team_name?.split('@ ')[1] ?? 'another server'}` : machine ? 'labelled a machine on this Mac' : `${server?.name ?? group?.server}`} · {shortId(party.party_id_hex)} · gen {party.generation}</small></span><button className="x" title="Close" onClick={onClose}>×</button></div><div className="scroll"><div className="sec">Identity</div><Inset className="rows"><InsetRow label="Party id"><code>{shortId(party.party_id_hex)}</code><Button size="sm" onClick={() => onCopy(party.party_id_hex)}>Copy</Button></InsetRow><InsetRow label="Generation"><span>{party.generation}<span className="hint">The group-key generation this membership belongs to.</span></span></InsetRow><InsetRow label="Role here"><Chip>{here}</Chip><span className="dim">{source === here ? 'same at source' : `at source: ${source}`}</span></InsetRow><InsetRow label="Change here"><span>{canTarget(world, party) ? `yes — a unique user of ${server?.name ?? group?.server}` : 'no'}<span className="hint">The roster does not prove sufficient authority; the signed edit is checked when it lands and can be refused.</span></span></InsetRow>{party.party_kind === 'user' ? <InsetRow label="Machine"><span>{machine ? 'On' : 'Off'}<span className="hint">A label kept on this Mac, not a server fact.</span></span></InsetRow> : null}</Inset>
    <div className="sec">What {partyShortName(party)} can read<span className="lnk">{readable.size} of {items.length}</span></div><Inset className="rows">{ordered.map((item) => <InsetRow key={item.path} label={readable.has(item) ? '✓' : '✗'}><code>{item.path}</code> <span className="tag">{kindOf(item)} · v{item.version}</span><span className="hint">{readable.has(item) ? `read role ${fmtRole(item.read)} admits ${here}` : `needs ${fmtRole(item.read)}; ${partyShortName(party)} holds ${here}. Raising it means remove and re-add — two operations — and rekey; lowering the item role changes every reader at that band.`}</span><Chip>{fmtRole(item.read)}</Chip></InsetRow>)}<p className="txt">There are no per-item grants. The item read role and this party’s role decide; change either and this list changes with it.{party.party_kind !== 'user' ? ` Every member of ${partyShortName(party)} reads through this party at the same role.` : ''}{admissionActive(world, party, party.store) ? '' : ' This group membership is inactive and cannot access any items.'}</p></Inset>{machine ? <><div className="sec">Connect an agent</div><Inset><p className="txt">{partyShortName(party)} uses its own account and device credentials on {server?.name}. The local agent socket is limited to this desktop account. The group role defines the machine’s access.</p></Inset><p className="pfn">Removing this machine revokes future access and rekeys the group. Rotate any secrets it may already have copied.</p></> : null}{party.party_kind !== 'user' ? <p className="pfn">Group memberships can only be removed from Federation, which is not available on this Mac.</p> : null}</div><div className="dfoot">{manageable && canTarget(world, party) ? <>{demotionFor(party) ? <Button onClick={() => onSheet('demote', party)}>Change role</Button> : null}<Button variant="danger" onClick={() => onSheet('remove', party)}>Remove</Button></> : party.party_kind !== 'user' ? <Button onClick={onFederation}>Federation →</Button> : <><Button disabled>Change role</Button><Button variant="danger" disabled>Remove</Button></>}</div></aside>;
}

function FederationTab({ world, store, onSheet, onRerun, onCopy, manageable }: { world: World; store: Store; onSheet: (sheet: Sheet) => void; onRerun: (operationId: string) => void; onCopy: (text: string) => void; manageable: boolean }): ReactNode {
  const entries = world.federation.filter((entry) => entry.store === store.id);
  return <><div className="party"><span className="t"><b>{entries.length} member {entries.length === 1 ? 'group' : 'groups'}</b> — each is one party in this roster</span><Button size="sm" icon="plus" disabled={!manageable} onClick={() => onSheet('admit')}>Add a group</Button></div>{entries.map((entry) => {
    const party = partiesOf(world, store.id).find((candidate) => candidate.party_id_hex === entry.remote_team_id_hex);
    const pinned = world.servers.find((server) => server.name === entry.remote_profile || server.id === entry.remote_profile);
    const pinMatches = Boolean(pinned?.host_id && pinned.host_id === entry.remote_host_id_hex);
    const pinStatus = pinMatches
      ? 'matches this Mac’s pin'
      : pinned?.host_id
        ? 'differs from this Mac’s pin'
        : 'local pin fact unavailable';
    return <div className="fcard" key={`${entry.remote_host_id_hex}|${entry.remote_team_id_hex}`}><div className="ghead">{party ? <Stack parties={[party]} size="lg" /> : <span className="kico md">G</span>}<span className="t"><b>{entry.remote_team_alias} @ {entry.remote_profile}</b><small>{store.name} includes this group as one member. {entry.active ? 'Its members have the role shown below.' : 'Its members have no access while the group membership is inactive.'}</small></span><Chip tone={entry.active ? undefined : 'warn'}>{entry.active ? 'Active' : 'Inactive'}</Chip>{!entry.active && entry.operation_id_hex ? <Button variant="primary" size="sm" disabled={!manageable} onClick={() => onRerun(entry.operation_id_hex!)}>Restore access</Button> : null}</div>{!entry.active ? <Band title="Group membership inactive">Restore access continues operation {entry.operation_id_hex ?? 'whose id is unavailable'} after both server check-ins are current.</Band> : null}<Inset className="rows"><InsetRow label="Remote host id"><code>{entry.remote_host_id_hex}</code><Chip tone={pinMatches ? undefined : 'warn'}>{pinStatus}</Chip><span className="hint">The host ID recorded when this group was added, compared with this Mac’s pinned host ID.</span></InsetRow><InsetRow label="Remote group id"><code>{shortId(entry.remote_team_id_hex)}</code><span className="hint">This ID also identifies the group under People &amp; groups.</span><Button size="sm" onClick={() => onCopy(entry.remote_team_id_hex)}>Copy</Button></InsetRow><InsetRow label="Role here"><Chip>{fmtRole(entry.destination)}</Chip>{party ? <span className="dim">at source: {fmtRole(party.source_role)}</span> : null}<span className="hint">Member groups can only be Members and cannot manage this roster.</span></InsetRow>{entry.operation_id_hex ? <InsetRow label="Membership operation"><code>{entry.operation_id_hex}</code><span className="hint">Restore access continues this operation.</span></InsetRow> : null}</Inset><div className="dz"><div className="sec">Take it back</div><div className="rowend"><span className="t">Removal is unavailable on this Mac.<span className="hint">A member group is identified by group ID and host, not by username. Inactive memberships have no access; active memberships are limited to the role above.</span></span><span className="a"><Button variant="danger" disabled>Take it back</Button><Chip>not offered from this Mac</Chip></span></div></div></div>;
  })}<p className="hint">A member group appears under People &amp; groups with its source and destination roles. Both servers must be checked, and the remote group must be active. Select Refresh to update this information.</p></>;
}

function StoreTab({ world, store }: { world: World; store: Store }): ReactNode {
  const items = itemsOf(world, store.id);
  const roles = [...new Set(items.map((item) => fmtRole(item.read)))];
  const parties = partiesOf(world, store.id);
  const inactive = parties.filter((party) => !admissionActive(world, party, store.id));
  return <><div className="party"><span className="t"><b>{items.length} items</b> in {store.name}’s store — who can read what, by read role</span></div><Inset className="rows">{roles.map((role) => { const list = items.filter((item) => fmtRole(item.read) === role); const first = list[0]; const readers = first ? readersOf(world, first) ?? [] : []; return <InsetRow key={role} label={<Chip>{role}</Chip>}><span>{list.map((item) => <code key={item.path}>{item.path} </code>)}</span><span className="hint">{list.length} {list.length === 1 ? 'item' : 'items'} · readable by {readers.length} of {parties.length}: {readers.map((party) => party.party_kind === 'user' ? partyName(party) : `${partyName(party)} (as one party)`).join(', ')}</span><Stack parties={readers} /></InsetRow>; })}<p className="txt">Everything here is readable by every party its read role admits. There is no per-item share: putting something here is the share, and removing a party is what un-shares it. Values stay masked until Show in the vault reads the exact version; nothing is kept on this Mac.{inactive.length ? ` ${inactive.map(partyName).join(', ')} is a member but inactive, so it has no access and is not counted.` : ''}</p></Inset></>;
}

function DangerTab({ world, store, onSheet, manageable }: { world: World; store: Store; onSheet: (sheet: Sheet, party?: Party) => void; manageable: boolean }): ReactNode {
  const mine = partiesOf(world, store.id).find((party) => party.label === 'you');
  const server = serverOf(world, store.id);
  const seniors = partiesOf(world, store.id).filter((party) => party.party_kind === 'user' && !party.label && parseRole(party.destination_role)?.kind !== 'member');
  const removable = manageable ? partiesOf(world, store.id).filter((party) => canTarget(world, party)) : [];
  const safest = manageable ? safestRemovalTarget(world, store.id) : undefined;
  return <><Inset className="rows"><InsetRow label=""><span>Remove me from {store.name}<span className="hint">You cannot remove yourself from this Mac{mine ? ` — you are ${roleText(mine)} here` : ''}. Ask another Admin or Owner{seniors.length ? `: ${seniors.map(partyName).join(', ')}` : ''}.</span></span><Chip>unavailable</Chip></InsetRow><InsetRow label=""><span>Rename, close or delete {store.name}<span className="hint">Group names cannot be changed or deleted. To stop sharing, remove members and rotate the secrets they could read.</span></span><Chip>unavailable</Chip></InsetRow><InsetRow label=""><span>Reset this server’s local state<span className="hint">Open Servers &amp; devices for {server?.name} to reset this Mac’s server state.</span></span><Button size="sm" disabled>Servers &amp; devices →</Button></InsetRow></Inset><div className="dz"><div className="sec">Rekeys {store.name}</div><div className="rowend"><span className="t">Remove a party and rekey<span className="hint">Removing a party blocks future access but cannot erase copies it already downloaded. This Mac can remove {removable.length} of {partiesOf(world, store.id).length} parties.</span></span><span className="a"><Button variant="danger" disabled={!safest} onClick={() => onSheet('remove', safest)}>Remove {safest ? partyName(safest) : 'a party'}…</Button><span className="hint">Review affected items before confirming.</span></span></div></div></>;
}

function inspectResponse(world: World, store: Store, tab: Tab): unknown {
  if (tab === 'federation') {
    return world.federation.filter((entry) => entry.store === store.id).map((entry) => ({
      remote_profile: entry.remote_profile,
      remote_team_alias: entry.remote_team_alias,
      remote_host_id_hex: entry.remote_host_id_hex,
      remote_team_id_hex: entry.remote_team_id_hex,
      destination: entry.destination,
      ...(entry.operation_id_hex ? { operation_id_hex: entry.operation_id_hex } : {}),
      active: entry.active,
    }));
  }
  if (tab === 'store') {
    return itemsOf(world, store.id).map((item) => ({
      path: item.path,
      kind: item.kind,
      size: item.size,
      version: item.version,
      read: item.read,
      write: item.write,
    }));
  }
  return partiesOf(world, store.id).map((party) => ({
    ...(party.username ? { username: party.username } : {}),
    party_kind: party.party_kind,
    generation: party.generation,
    locally_manageable: party.locally_manageable,
    party_id_hex: party.party_id_hex,
    ...(party.scoped_host_id_hex ? { scoped_host_id_hex: party.scoped_host_id_hex } : {}),
    source_role: party.source_role,
    destination_role: party.destination_role,
  }));
}

function GroupSheet({ world, bridge, store, sheet, target, onClose, onSwitch, onApplied, onError }: { world: World; bridge: Bridge; store: Store; sheet: Exclude<Sheet, null>; target: Party | null; onClose: () => void; onSwitch: (sheet: Sheet, target?: Party) => void; onApplied: (message: string) => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const reads = target ? readsOf(world, target) : [];
  const [username, setUsername] = useState('jules.park');
  const [visibility, setVisibility] = useState(0);
  const [role, setRole] = useState<RoleDto>({ role: 'Member', visibility: 0 });
  const [busy, setBusy] = useState(false);
  const [name, setName] = useState('Platform');
  const [createKind, setCreateKind] = useState<'named' | 'adhoc'>('named');
  const creationAccounts = world.stores.filter((candidate) =>
    candidate.kind === 'account' &&
    !leaseLapsed(world, candidate.id) &&
    !serverBlocked(world, candidate.id) &&
    !serverLeaseUnavailable(world, candidate.id) &&
    world.accounts.some((account) =>
      account.store === candidate.id ||
      (account.alias === candidate.account && account.server === candidate.server),
    ),
  );
  const [accountStoreId, setAccountStoreId] = useState(
    () => creationAccounts.find((candidate) => candidate.account === 'work')?.id
      ?? creationAccounts[0]?.id
      ?? '',
  );
  const creationAccount = creationAccounts.find((candidate) => candidate.id === accountStoreId);
  const creationIdentity = creationAccount
    ? world.accounts.find((account) =>
        account.store === creationAccount.id ||
        (account.alias === creationAccount.account && account.server === creationAccount.server),
      )
    : undefined;
  const server = serverOf(world, sheet === 'create' && creationAccount ? creationAccount.id : store.id);
  const ownerStore = store.kind === 'team'
    ? world.stores.find((candidate) => candidate.kind === 'account' && candidate.server === store.server && candidate.account === store.account)
    : undefined;
  const ownerAccount = ownerStore
    ? world.accounts.find((account) => account.store === ownerStore.id || (account.server === ownerStore.server && account.alias === ownerStore.account))
    : undefined;
  const inviter = ownerAccount?.username.split('.')[0] ?? ownerAccount?.username ?? 'the group Admin';
  const inviteMessage = `Hi Jules — I'd like to add you to ${store.name} on FOKS.\n1. Install FOKS: https://foks.app/download (placeholder)\n2. Add the server: ${server?.name ?? store.server}\n3. Create your account there with username firstname.lastname — for you: jules.park\n4. Then tell ${inviter} your username — there are no invite links; I add you by username.\nOnce I have, ${store.name} shows up under Groups in your app.`;
  const teamAlias = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
  const remote = world.stores.find((candidate) => candidate.kind === 'team' && candidate.active && candidate.team_kind === 'named' && candidate.server !== store.server);
  const title = sheet === 'manage' ? `Manage ${store.name}` : sheet === 'invite' ? `Invite someone to ${store.name}` : sheet === 'add' ? `Add someone to ${store.name}` : sheet === 'demote' ? `Change ${target ? partyName(target) : 'party'}’s role in ${store.name}` : sheet === 'remove' ? `Remove ${target ? partyName(target) : 'party'} from ${store.name}?` : sheet === 'admit' ? `Add a group to ${store.name}` : 'Create a group';
  const subtitle = sheet === 'create'
    ? `${server?.name ?? 'Selected server'} · you become its Owner with the account you hold there`
    : sheet === 'invite'
      ? 'There are no invite links — this writes the message that gets them an account'
      : sheet === 'add'
        ? 'By username — there are no invite links to accept'
        : sheet === 'demote'
          ? 'Only a lower role can be chosen — this demotes and nothing else'
          : sheet === 'remove' && target
            ? `${roleText(target)} · generation ${target.generation} · ${shortId(target.party_id_hex)}`
            : sheet === 'admit'
              ? 'Every member of it reads here as one party, at the role you give it'
              : sheet === 'manage' && store.kind === 'team'
                ? `${server?.name} · id ${shortId(store.team_id_hex)}`
                : server?.name;
  const [demotion, setDemotion] = useState<RoleDto | null>(() => target ? demotionFor(target) : null);
  const currentRole = target ? parseRole(target.destination_role) : null;
  const maxMemberVisibility = currentRole?.kind === 'member' ? (currentRole.visibility ?? 0) - 1 : 0;
  useEffect(() => {
    if (sheet === 'demote') setDemotion(target ? demotionFor(target) : null);
  }, [sheet, target]);
  const apply = async (): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      if (sheet === 'manage') return;
      if (sheet === 'invite') await bridge.copyText(inviteMessage);
      else if (sheet === 'add') await bridge.addGroupMember({ storeId: store.id, username: username.trim(), destination: role.role === 'Member' ? { ...role, visibility } : role });
      else if (sheet === 'demote' && target && canTarget(world, target) && demotion) await bridge.demoteGroupMember({ storeId: store.id, username: target.username!, destination: demotion });
      else if (sheet === 'remove' && target && canTarget(world, target)) await bridge.removeGroupMember({ storeId: store.id, username: target.username! });
      else if (sheet === 'admit' && remote) await bridge.admitGroup({ storeId: store.id, remoteStoreId: remote.id, visibility });
      else if (sheet === 'create') {
        const account = world.stores.find((candidate) => candidate.kind === 'account' && candidate.id === accountStoreId);
        if (!account) throw new Error('No account store is available for group creation.');
        await bridge.createGroup({ accountStoreId: account.id, teamAlias, name: createKind === 'named' ? name : '', kind: createKind });
      }
      if (sheet !== 'invite') await onApplied(`${title} completed`);
      onClose();
    } catch (error) {
      onError(error);
      const typed = normalizeCommandError(error);
      if (typed.code !== 'agent-lost') {
        try {
          await onApplied(`${typed.message} Roster refreshed — review the retained draft`);
        } catch (refreshError) {
          onError(refreshError);
        }
      }
    } finally {
      setBusy(false);
    }
  };
  return <Dialog className="backdrop" role={sheet === 'remove' ? 'alertdialog' : 'dialog'}><div className={`sheet${sheet === 'invite' || sheet === 'manage' ? ' wide' : ''}`}><div className="hd">{sheet === 'create' ? <span className="kico md group" style={{ background: hue(name || 'group') }}>G</span> : <GroupMark store={store} />}<span className="t"><h2>{title}</h2><small>{subtitle}</small></span></div><div className="sb">
    {sheet === 'manage' ? <><div className="sec">People and groups in {store.name}</div><p>{peopleGroups(partiesOf(world, store.id))} — roles and read counts come from the roster and item roles.</p><Inset className="rows manage-members">{partiesOf(world, store.id).map((party) => <InsetRow className="manage-member" key={party.party_id_hex} label={<span className="manage-member-name"><Stack parties={[party]} /><span>{partyName(party)}</span>{party.label ? <Chip>you</Chip> : null}</span>}><span className="manage-member-meta"><span className="dim">{server?.name} ·</span><code>{shortId(party.party_id_hex)}</code><Button size="sm" onClick={() => void bridge.copyText(party.party_id_hex).catch(onError)}>Copy</Button><span className="dim">· gen {party.generation}</span></span><span className="manage-member-access"><Chip>{roleText(party)}</Chip><span className="hint">Reads {readsOf(world, party).length} of {itemsOf(world, store.id).length}</span>{canTarget(world, party) ? <>{demotionFor(party) ? <Button size="sm" onClick={() => onSwitch('demote', party)}>Change role</Button> : null}<Button size="sm" variant="danger" onClick={() => onSwitch('remove', party)}>Remove…</Button></> : <span className="hint">{party.label ? 'your own membership' : 'not changeable here'}</span>}</span></InsetRow>)}</Inset><div className="sec">Add people</div><Inset><InsetRow label="Username"><input value={username} onChange={(event) => setUsername(event.target.value)} /></InsetRow>{(['Member', 'Admin', 'Owner'] as const).map((next) => <button type="button" className={`radio${role.role === next ? ' on' : ''}`} key={next} onClick={() => setRole(next === 'Member' ? { role: next, visibility } : { role: next })}><span className="rb" /><span className="t"><b>{next}</b><small>{next === 'Member' ? `Member · visibility ${visibility} — opens items whose read role admits this level.` : next === 'Admin' ? 'Change items, add or remove members.' : 'Full control of the group, including other Owners.'}</small></span></button>)}</Inset>{role.role === 'Member' ? <div className="vis"><Button size="sm" disabled={visibility <= VIS_MIN} onClick={() => setVisibility((value) => value - 1)}>−</Button><span>Visibility {visibility}</span><Button size="sm" disabled={visibility >= VIS_MAX} onClick={() => setVisibility((value) => value + 1)}>+</Button></div> : null}<p className="hint">They need an account on {server?.name} first. Invite someone creates a message you can send. If adding is interrupted, you can resume it.</p></> : null}
    {sheet === 'invite' ? <><p>They need an account on {server?.name} before you can add them. Send this message, then add their username.</p><Inset><InsetRow label="Message"><span className="msg">{inviteMessage}</span><Button size="sm" onClick={() => void bridge.copyText(inviteMessage).catch(onError)}>Copy</Button></InsetRow></Inset><div className="sec">Message contents</div><Inset className="rows"><InsetRow label="Install link"><code>https://foks.app/download</code> <Chip tone="warn">placeholder</Chip><span className="hint">Temporary download address.</span></InsetRow><InsetRow label="Server"><span>{server?.name}<span className="hint">They enter this address during setup.</span></span></InsetRow><InsetRow label="Signup invite"><span>Optional<span className="hint">Required only if the server requires an invite.</span></span></InsetRow><InsetRow label="When they reply"><span>Add their username as a Member, Admin, or Owner.</span><Button size="sm" onClick={() => onSwitch('add')}>Add someone</Button></InsetRow></Inset><p className="hint">Copying this message does not change the server.</p></> : null}
    {sheet === 'add' ? <><div className="sec">Username on {server?.name}</div><Inset><InsetRow label="Username"><input value={username} onChange={(event) => setUsername(event.target.value)} /></InsetRow></Inset><p className="hint">This person needs an account on {server?.name}. Use Invite someone if they have not signed up.</p><div className="sec">Role in {store.name}</div><Inset>{(['Member', 'Admin', 'Owner'] as const).map((next) => <button type="button" className={`radio${role.role === next ? ' on' : ''}`} key={next} onClick={() => setRole(next === 'Member' ? { role: next, visibility } : { role: next })}><span className="rb" /><span className="t"><b>{next}</b><small>{next === 'Member' ? `Opens items whose read role admits visibility ${visibility}; 0 is the default and lower levels see less.` : next === 'Admin' ? 'Manages the roster — adds, lowers and removes parties — and reads Admin or below.' : 'The highest group role. Opens everything, including Owner-only items.'}</small></span></button>)}</Inset>{role.role === 'Member' ? <div className="vis"><Button size="sm" disabled={visibility <= VIS_MIN} onClick={() => setVisibility((value) => value - 1)}>−</Button><span>Visibility {visibility}</span><Button size="sm" disabled={visibility >= VIS_MAX} onClick={() => setVisibility((value) => value + 1)}>+</Button></div> : null}<p className="hint">Roles can only be lowered after a person is added. FOKS checks the latest roster before applying the change.</p></> : null}
    {sheet === 'demote' ? <><p><Chip>{target ? roleText(target) : ''}</Chip> → <Chip>{demotion ? fmtRole(demotion) : 'No lower role'}</Chip></p><div className="sec">New role — strictly below {target ? roleText(target) : ''}</div><Inset>{currentRole?.kind === 'owner' ? <button type="button" className={`radio${demotion?.role === 'Admin' ? ' on' : ''}`} onClick={() => setDemotion({ role: 'Admin' })}><span className="rb" /><span className="t"><b>Admin</b><small>Below Owner. Keeps roster management and loses Owner-only reads.</small></span></button> : null}<button type="button" className={`radio${demotion?.role === 'Member' ? ' on' : ''}`} disabled={maxMemberVisibility < VIS_MIN} onClick={() => setDemotion({ role: 'Member', visibility: maxMemberVisibility })}><span className="rb" /><span className="t"><b>Member</b><small>{currentRole?.kind === 'member' ? `Same role, lower level; ${maxMemberVisibility} at most.` : 'Loses roster management and opens only what its visibility level admits.'}</small></span></button><button type="button" className="radio off" disabled><span className="rb" /><span className="t"><b>{target ? roleText(target) : 'Current role'} <Chip>current</Chip></b><small>Roles can only be lowered here. To raise a role, remove and re-add the person.</small></span></button></Inset>{demotion?.role === 'Member' ? <div className="vis"><Button size="sm" disabled={demotion.visibility <= VIS_MIN} onClick={() => setDemotion({ role: 'Member', visibility: demotion.visibility - 1 })}>−</Button><span>Visibility {demotion.visibility}</span><Button size="sm" disabled={demotion.visibility >= maxMemberVisibility} onClick={() => setDemotion({ role: 'Member', visibility: demotion.visibility + 1 })}>+</Button></div> : null}<p>Raising a role requires removing and re-adding the person, which rekeys the group.</p><Band title="Revokes future access">This change rekeys the group and revokes access at the old role. It cannot recall downloaded data.</Band><p className="hint">FOKS checks the latest roster before applying the change.</p></> : null}
    {sheet === 'remove' ? <><p>Removing this party rekeys the group and revokes future access. It cannot recall copied keys or files.</p>{target && !canTarget(world, target) ? <Notice title={`${partyName(target)} cannot be removed here`}>This party does not have a unique username managed by this account. Remove it from the account or server that manages it.</Notice> : null}<div className="sec">What {target ? partyName(target) : 'this party'} could read <span className="lnk">{reads.length} of {itemsOf(world, store.id).length}</span></div><Inset className="rows">{reads.length ? reads.map((item) => <InsetRow key={item.path} label="✓"><code>{item.path}</code><span className="tag">{kindOf(item)} · v{item.version}</span><span className="hint">{item.kind === 'Secret' ? 'Rotate this secret after removal.' : 'Replace sensitive content after removal; downloaded copies remain.'}</span><Chip>{fmtRole(item.read)}</Chip></InsetRow>) : <InsetRow><span className="dim">No item in this store is readable at that role.</span></InsetRow>}<p className="txt">Rotate these items after removal. FOKS records permissions, not whether an item was downloaded.</p></Inset><p className="hint">FOKS checks the latest roster before removing the party.</p></> : null}
    {sheet === 'admit' ? <><p>{store.name} treats the other group as one member and cannot see its roster.</p><div className="sec">Where it lives</div><Inset><InsetRow label="Server"><span>{remote ? `${serverOf(world, remote.id)?.name}${serverOf(world, remote.id)?.host_id ? ` — pinned ${serverOf(world, remote.id)?.host_id}` : ' — local pin unavailable'}` : 'No eligible remote group'}</span></InsetRow><InsetRow label="Group"><span>{remote?.kind === 'team' ? remote.alias : 'No active named group'}</span></InsetRow><p className="txt">You can add an active named group from another authenticated server. FOKS verifies its group ID and host before continuing.</p></Inset><div className="sec">Role in {store.name}</div><Inset><InsetRow label="Member"><Chip>Member · visibility {visibility}</Chip><span className="hint">Member groups can only be Members and cannot manage this roster.</span></InsetRow></Inset><div className="vis"><Button size="sm" disabled={visibility <= VIS_MIN} onClick={() => setVisibility((value) => value - 1)}>−</Button><span>Visibility {visibility}</span><Button size="sm" disabled={visibility >= VIS_MAX} onClick={() => setVisibility((value) => value + 1)}>+</Button></div><p className="hint">If adding the group is interrupted, use Restore access after both server check-ins are current. Group memberships cannot be removed from this Mac.</p></> : null}
    {sheet === 'create' ? <><p>A group shares items with people and other groups according to their roles.</p><Inset><InsetRow label="Name"><input value={name} onChange={(event) => setName(event.target.value)} /></InsetRow><InsetRow label="Server and account"><span>{creationAccount ? serverOf(world, creationAccount.id)?.name : 'No available account'}{creationIdentity ? ` — ${creationIdentity.username}` : ''}</span></InsetRow><p className="txt">Local alias: <code>{teamAlias || '—'}</code>. Group names cannot be changed after creation.</p></Inset><div className="sec">Server and account</div><Inset>{creationAccounts.map((account) => <button type="button" className={`radio${account.id === accountStoreId ? ' on' : ''}`} key={account.id} onClick={() => setAccountStoreId(account.id)}><span className="rb" /><span className="t"><b>{serverOf(world, account.id)?.name}</b><small>{world.accounts.find((candidate) => candidate.store === account.id || (candidate.alias === account.account && candidate.server === account.server))?.username}</small></span></button>)}</Inset><div className="sec">Kind</div><Inset>{(['named', 'adhoc'] as const).map((kind) => <button type="button" className={`radio${kind === createKind ? ' on' : ''}`} key={kind} onClick={() => setCreateKind(kind)}><span className="rb" /><span className="t"><b>{kind === 'named' ? 'Named' : 'Ad-hoc'}</b><small>{kind === 'named' ? 'Visible by name on the server and available to add to other groups.' : 'Identified by ID on the server and by the local alias shown above.'}</small></span></button>)}</Inset><p className="hint">If creation is interrupted, the group appears as Inactive. Select Resume creation to continue without repeating completed steps.</p></> : null}
  </div><div className="ft"><Button disabled={busy} onClick={onClose}>{sheet === 'manage' ? 'Done' : 'Cancel'}</Button>{sheet === 'manage' ? <Button variant="primary" disabled={!username.trim()} onClick={() => onSwitch('add')}>Add</Button> : sheet === 'remove' ? <Button variant="primary" className="danger" disabled={busy || !target || !canTarget(world, target)} onClick={() => void apply()}>Remove and rekey</Button> : <Button variant="primary" disabled={busy || (sheet === 'add' && !username.trim()) || (sheet === 'demote' && (!target || !canTarget(world, target) || !demotion)) || (sheet === 'admit' && !remote) || (sheet === 'create' && (!teamAlias || !accountStoreId))} onClick={() => void apply()}>{sheet === 'invite' ? 'Copy message' : sheet === 'add' ? `Add as ${fmtRole(role)}` : sheet === 'demote' ? `Lower to ${demotion ? fmtRole(demotion) : 'no lower role'}` : sheet === 'admit' ? `Add as Member · visibility ${visibility}` : `Create ${createKind === 'named' ? 'named' : 'ad-hoc'} group`}</Button>}</div></div></Dialog>;
}

function JoinPane({ world, bridge, onCreate, onError }: { world: World; bridge: Bridge; onCreate: () => void; onError: (error: unknown) => void }): ReactNode {
  // Every choice on this pane starts from an account store and resolves the
  // identity from it, never the other way round: an alias is profile-local, so
  // two profiles can each hold `personal` and "the account called personal" is
  // not an answer.
  const accounts = world.stores
    .filter((store): store is AccountStore => store.kind === 'account')
    .flatMap((store) => {
      const account = world.accounts.find((candidate) => candidate.store === store.id);
      return account
        ? [{ store, account, server: world.servers.find((server) => server.id === store.server) }]
        : [];
    });
  // Fixture-only: the named acceptance scene opens the sheet on the exact
  // fixture StoreRef it is a picture of, not on whichever account happens to
  // be aliased `work`. Nothing but this initialiser reads a scene name.
  const [inviteStore, setInviteStore] = useState<StoreRef | null>(
    () => stateName() === 'join-invite'
      ? accounts.find((candidate) => candidate.store.id === 'acct:work')?.store.id
        ?? accounts[0]?.store.id
        ?? null
      : null,
  );
  // Exact, and no fallback: if the chosen account leaves the catalog while the
  // sheet is open, the sheet reports that rather than composing a message that
  // names somebody else's server and username.
  const chosen = accounts.find((candidate) => candidate.store.id === inviteStore);
  const message = chosen
    ? `1. Install FOKS: https://foks.app/download\n2. When it asks for a server address, type ${chosen.server?.name ?? chosen.account.server}\n3. Create your account with username firstname.lastname\n4. Then tell ${chosen.account.username} your username — there are no invite links, so that is what I add.`
    : '';
  const copy = async (text: string): Promise<void> => {
    try { await bridge.copyText(text); } catch (error) { onError(error); }
  };
  return <><div className="path"><div className="loc"><h1>Join or create a group</h1><small>Join an existing group or create a new one</small></div></div><div className="body"><div className="plain"><h2>Join a group</h2><p>An Admin or Owner adds your username on the server. FOKS does not use invite links.</p>{accounts.map(({ store, account, server }) => <div className="copybox" key={store.id}><span className="v">“Add {account.username} on {server?.name ?? account.server} to your group”</span><Button onClick={() => void copy(`Add ${account.username} on ${server?.name ?? account.server} to your group`)}>Copy the sentence</Button></div>)}<h2>Invite someone</h2><p>Send them the download link and server address. Add the username they choose after setup.</p>{accounts.map(({ store, account, server }) => <div className="copybox" key={`invite|${store.id}`}><span className="v"><b>{server?.name ?? account.server}</b> — you are <code>{account.username}</code> there</span><Button onClick={() => setInviteStore(store.id)}>Invite someone…</Button></div>)}<h2>Create a group</h2><p>You become the Owner. If creation is interrupted, you can resume it.</p><Button variant="primary" onClick={onCreate}>Create a group…</Button></div></div>{inviteStore ? <Dialog className="backdrop" role="dialog"><div className="sheet"><div className="hd"><span className="kico md invite">I</span><span className="t">{chosen ? <><h2>Invite to {chosen.server?.name ?? chosen.account.server}</h2><small>Help set up a new user</small></> : <><h2>That account is no longer available</h2><small>Message unavailable</small></>}</span></div><div className="sb">{chosen ? <><div className="copybox"><span className="v">{message}</span></div><p className="fn">The download address is a placeholder. Add their username after they finish setup.</p></> : <p className="fn">This account is no longer in the catalog. Close this dialog and choose another account.</p>}</div><div className="ft"><Button onClick={() => setInviteStore(null)}>Done</Button>{chosen ? <Button variant="primary" onClick={() => void copy(message)}>Copy message</Button> : null}</div></div></Dialog> : null}</>;
}

export function GroupsScreen({ world, bridge, location, onNavigate, onApplied, onError, initialSheet, onIntentConsumed }: { world: World; bridge: Bridge; location: Extract<Location, { kind: 'join' | 'groups' | 'group-admin' }>; onNavigate: (location: Location) => void; onApplied: (message: string) => Promise<void>; onError: (error: unknown) => void; initialSheet?: 'manage'; onIntentConsumed?: () => void }): ReactNode {
  const initial = stateName();
  const [tab, setTab] = useState<Tab>(initial === 'federation' || initial === 'admit' ? 'federation' : initial === 'store' ? 'store' : initial === 'danger' ? 'danger' : 'people');
  const [sheet, setSheet] = useState<Sheet>(initialSheet ?? (['manage', 'invite', 'add', 'demote', 'remove', 'admit', 'create', 'party-remove'].includes(initial) ? initial === 'party-remove' ? 'remove' : initial as Sheet : null));
  useEffect(() => {
    if (initialSheet) onIntentConsumed?.();
  }, [initialSheet, onIntentConsumed]);
  const store = location.kind === 'group-admin' ? storeOf(world, location.ref) : undefined;
  const parties = useMemo(() => store ? partiesOf(world, store.id) : [], [store, world]);
  const [selected, setSelected] = useState<Party | null>(() => initial === 'party' ? parties.find((party) => party.username === 'deploy-bot') ?? null : null);
  const [target, setTarget] = useState<Party | null>(() => {
    if (initial === 'demote') return parties.find((party) => party.username === 'priya.n') ?? null;
    if (initial === 'remove') return parties.find((party) => party.username === 'dana.okafor') ?? null;
    if (initial === 'party-remove') {
      const party = parties.find((candidate) => candidate.username === 'deploy-bot');
      return party ? { ...party, locally_manageable: false } : null;
    }
    return null;
  });
  const openSheet = (next: Sheet, party?: Party): void => { setTarget(party ?? null); setSheet(next); };
  const counts = useMemo(() => store ? { people: parties.length, federation: world.federation.filter((entry) => entry.store === store.id).length, store: itemsOf(world, store.id).length } : null, [parties, store, world]);
  const mutate = async (action: () => Promise<unknown>, message: string): Promise<void> => {
    try {
      await action();
      await onApplied(message);
    } catch (error) {
      onError(error);
      const typed = normalizeCommandError(error);
      if (typed.code !== 'agent-lost') {
        try {
          await onApplied(`${typed.message} State refreshed — review before retrying`);
        } catch (refreshError) {
          onError(refreshError);
        }
      }
    }
  };
  const copy = async (text: string): Promise<void> => {
    try {
      await bridge.copyText(text);
    } catch (error) {
      onError(error);
    }
  };
  const creationContext = world.stores.find((candidate) => candidate.kind === 'account');
  if (location.kind === 'join') return <JoinPane world={world} bridge={bridge} onCreate={() => { openSheet('create'); onNavigate({ kind: 'groups' }); }} onError={onError} />;
  if (!store || location.kind === 'groups') return <><GroupsList world={world} onNavigate={onNavigate} onSheet={openSheet} onResume={(storeId) => void mutate(() => bridge.resumeGroupCreation(storeId), 'Group creation resumed')} onCopy={(text) => void copy(text)} />{sheet === 'create' && creationContext ? <GroupSheet world={world} bridge={bridge} store={creationContext} sheet="create" target={null} onClose={() => setSheet(null)} onSwitch={openSheet} onApplied={onApplied} onError={onError} /> : null}</>;
  const server = serverOf(world, store.id);
  if (storeDescriptionState(world, store) !== 'normal') return <StoreAccessTakeover world={world} store={store} lead={<><Button size="sm" onClick={() => onNavigate({ kind: 'groups' })}>‹ Groups</Button><GroupMark store={store} /></>} onOpenServer={(profile) => onNavigate({ kind: 'servers', profile })} onFinishSetup={() => void mutate(() => bridge.resumeGroupCreation(store.id), 'Group creation resumed')} />;
  if (store.kind !== 'team') return <GroupsList world={world} onNavigate={onNavigate} onSheet={openSheet} onResume={(storeId) => void mutate(() => bridge.resumeGroupCreation(storeId), 'Group creation resumed')} onCopy={(text) => void copy(text)} />;
  const manageable = store.team_kind === 'named';
  const mine = parties.find((party) => party.label === 'you');
  return <><div className="path"><div className="loc"><Button size="sm" onClick={() => onNavigate({ kind: 'groups' })}>‹ Groups</Button><Stack parties={parties} size="lg" /><h1>{store.name}</h1><small>{peopleGroups(parties)} · {store.team_kind === 'adhoc' ? 'ad-hoc' : 'named'} group on {server?.name}</small><Chip>{shortId(store.team_id_hex)}</Chip><Button size="sm" onClick={() => void copy(store.team_id_hex)}>Copy id</Button></div></div>
    <div className="toolbar"><span className="tabs">{(['people', 'federation', 'store', 'danger'] as const).map((next) => <Button key={next} className={next === 'danger' ? 'danger' : ''} on={tab === next} onClick={() => { setTab(next); setSelected(null); }}>{next === 'people' ? 'People & groups' : next[0].toUpperCase() + next.slice(1)} {counts && next !== 'danger' ? <span className="n">{counts[next]}</span> : null}</Button>)}</span><span className="spacer" />{mine ? <><span className="scope">Your role</span><Chip>{roleText(mine)}</Chip></> : null}{tab === 'people' ? <Button variant="primary" disabled={!manageable} onClick={() => openSheet('invite')}>Invite someone</Button> : tab === 'federation' ? <Button disabled={!manageable} onClick={() => openSheet('admit')}>Add a group</Button> : tab === 'store' ? <Button onClick={() => onNavigate({ kind: 'store', ref: store.id })}>Open in vault →</Button> : null}</div>
    <div className={`body${selected ? ' group-panel-open' : ''}`}><div className="groups-wrap">{!manageable ? <Inset className="group-limit"><p className="txt">Roster and federation changes are unavailable for an ad-hoc group; read-only roster facts remain visible.</p></Inset> : null}{tab === 'people' ? <PeopleTab world={world} store={store} selected={selected} onSelect={setSelected} onSheet={openSheet} onRefresh={() => void mutate(() => Promise.resolve(), 'Roster re-read')} onCopy={(text) => void copy(text)} manageable={manageable} /> : tab === 'federation' ? <FederationTab world={world} store={store} onSheet={openSheet} onRerun={(operationId) => void mutate(() => bridge.rerunGroupAdmission(store.id, operationId), 'Group access restored')} onCopy={(text) => void copy(text)} manageable={manageable} /> : tab === 'store' ? <StoreTab world={world} store={store} /> : <DangerTab world={world} store={store} onSheet={openSheet} manageable={manageable} />}<details className="insp"><summary>Inspect response</summary><pre>{JSON.stringify(inspectResponse(world, store, tab), null, 1)}</pre></details></div></div>{selected ? <PartyPanel world={world} party={selected} onClose={() => setSelected(null)} onSheet={openSheet} onFederation={() => { setTab('federation'); setSelected(null); }} onCopy={(text) => void copy(text)} manageable={manageable} /> : null}{sheet ? <GroupSheet world={world} bridge={bridge} store={store} sheet={sheet} target={target} onClose={() => setSheet(null)} onSwitch={openSheet} onApplied={onApplied} onError={onError} /> : null}</>;
}

export function VaultGroupOverlay({ world, bridge, scene, onApplied, onError }: { world: World; bridge: Bridge; scene: 'manage' | 'party-remove'; onApplied: (message: string) => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const store = storeOf(world, scene === 'manage' ? 'team:household' : 'team:eng');
  const initialTarget = scene === 'party-remove'
    ? partiesOf(world, 'team:eng').find((party) => party.username === 'deploy-bot')
    : undefined;
  const [sheet, setSheet] = useState<Sheet>(scene === 'manage' ? 'manage' : 'remove');
  const [target, setTarget] = useState<Party | null>(
    initialTarget ? { ...initialTarget, locally_manageable: false } : null,
  );
  if (!store || !sheet) return null;
  const switchSheet = (next: Sheet, party?: Party): void => {
    setTarget(party ?? null);
    setSheet(next);
  };
  return <GroupSheet world={world} bridge={bridge} store={store} sheet={sheet} target={target} onClose={() => setSheet(null)} onSwitch={switchSheet} onApplied={onApplied} onError={onError} />;
}
