import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Dialog } from '/kit/overlay-primitives';
import type { AccountDevice, AppInfo, BackupEnrollment, Bridge, PairingOffer, ServerStatusSnapshot, YubiCommand, YubiEnrollment } from '../bridge';
import { Button, Icon, Inset, InsetRow, Notice } from '../components';
import type { Location, SettingsSection } from '../location';
import { serverLeaseState } from '../model';
import type { AccountStore, World } from '../model';
import { PageHeader } from '../shell/page-header';

interface Props {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'settings' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
}

type Sheet = 'phrase' | 'pair' | 'recover' | 'enrol' | 'provision' | 'yubi' | 'revoke' | 'passphrase' | 'remove-device' | null;
const SECTIONS: readonly { id: SettingsSection; label: string; icon: 'file' | 'key' | 'vault' | 'gear' | 'info' }[] = [
  { id: 'macs', label: 'Your Macs & recovery', icon: 'file' },
  { id: 'keys', label: 'Security keys', icon: 'key' },
  { id: 'account', label: 'Account', icon: 'vault' },
  { id: 'agent', label: 'Agent', icon: 'gear' },
  { id: 'about', label: 'About', icon: 'info' },
];

function accountStores(world: World): AccountStore[] {
  return world.stores.filter((store): store is AccountStore => store.kind === 'account');
}

export function SettingsScreen({ world, bridge, location, scene, onNavigate, onRefresh, onError }: Props): ReactNode {
  // The named fixture scene this screen was *entered* at, captured once.
  // `scene` is read live from the address bar, which the shell rewrites to the
  // canonical `settings` name on its first state change; without this, every
  // fixture branch below would silently turn itself off as soon as anything
  // re-rendered. Navigating away and back remounts and re-reads it, so leaving
  // Settings still leaves its scene behind.
  const [enteredScene] = useState(scene);
  const section = location.section === 'phrase' ? 'macs' : location.section ?? 'macs';
  const stores = accountStores(world);
  // Exact, by StoreRef. An alias is profile-local: two profiles may both hold
  // an account called `personal`, and picking the first one that matches would
  // point every device, recovery, pairing and passphrase action here at the
  // wrong account. Three states, no fallback between them: no StoreRef in the
  // address means this Mac's first account (and is rewritten to its exact
  // StoreRef below); a StoreRef that resolves is the selection; a StoreRef
  // that does not resolve is reported as unavailable.
  const requested = location.store;
  const selected = requested ? stores.find((store) => store.id === requested) : stores[0];
  const unavailable = requested !== undefined && selected === undefined;
  const [sheet, setSheet] = useState<Sheet>(() => enteredScene === 'settings-phrase' ? 'phrase' : enteredScene === 'settings-enrol' ? 'enrol' : null);
  const [devices, setDevices] = useState<AccountDevice[]>([]);
  const [backups, setBackups] = useState<BackupEnrollment[]>([]);
  const [yubi, setYubi] = useState<YubiEnrollment[]>([]);
  const [cards, setCards] = useState<{ serial: number }[]>([]);
  const [pendingYubi, setPendingYubi] = useState<SimpleYubiAction | null>(null);
  const [passphraseMode, setPassphraseMode] = useState<'set' | 'change' | 'verify'>('set');
  const [passphraseStore, setPassphraseStore] = useState<AccountStore | null>(null);
  const [pairMode, setPairMode] = useState<'offer' | 'accept'>('offer');
  const [removeDevice, setRemoveDevice] = useState<AccountDevice | null>(null);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [statuses, setStatuses] = useState<Map<string, ServerStatusSnapshot>>(new Map());
  const [loadedProfiles, setLoadedProfiles] = useState<Set<string>>(new Set());
  const [accountDeviceNames, setAccountDeviceNames] = useState<Map<string, string>>(new Map());
  const [flash, setFlash] = useState<string | null>(null);
  // The profile of the selected account, and nothing when there is none: a
  // YubiKey enrollment list belongs to a profile this Mac holds an account on,
  // not to whichever server happens to be listed first.
  const profile = selected?.server ?? '';

  useEffect(() => {
    const conceal = (): void => {
      setSheet(null);
      setPendingYubi(null);
      setPassphraseStore(null);
      setRemoveDevice(null);
    };
    const concealWhenHidden = (): void => { if (document.hidden) conceal(); };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', concealWhenHidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', concealWhenHidden);
    };
  }, []);

  // A Settings address with no store in it means "this Mac's first account".
  // Rewrite it to that account's exact StoreRef, so a reload, a copied link and
  // every action from here name one account rather than a position in a list
  // that a catalog refresh may reorder.
  useEffect(() => {
    if (location.store || !selected) return;
    onNavigate({
      kind: 'settings',
      ...(location.section ? { section: location.section } : {}),
      store: selected.id,
    });
  }, [location.section, location.store, onNavigate, selected]);

  // Everything below is about one account. When the route moves to another —
  // or to one this catalog no longer has — none of it carries over: an open
  // sheet would submit against the account now on screen, and a device or
  // enrollment list left in place would be read as that account's.
  const selectedId = selected?.id;
  const shown = useRef(selectedId);
  useEffect(() => {
    if (shown.current === selectedId) return;
    shown.current = selectedId;
    setSheet(null);
    setPendingYubi(null);
    setPassphraseStore(null);
    setRemoveDevice(null);
    setDevices([]);
    setBackups([]);
    setYubi([]);
    setCards([]);
    setFlash(null);
  }, [selectedId]);

  useEffect(() => {
    let alive = true;
    const profiles = [...new Set(accountStores(world).map((store) => store.server))];
    void (async () => {
      const next = new Map<string, ServerStatusSnapshot>();
      const loaded = new Set<string>();
      for (const candidate of profiles) {
        const server = world.servers.find((item) => item.id === candidate);
        if (server?.state === 'blocked') { loaded.add(candidate); continue; }
        try {
          const status = await bridge.describeServerStatus(candidate);
          if (status.profile !== candidate) throw new Error('describe_server_status returned a different profile.');
          next.set(candidate, status);
          loaded.add(candidate);
        } catch (error) { if (alive) onError(error); }
      }
      if (alive) { setStatuses(next); setLoadedProfiles(loaded); }
    })();
    return () => { alive = false; };
  }, [bridge, onError, world]);

  const accessStopped = (candidate: string): boolean => {
    const server = world.servers.find((item) => item.id === candidate);
    if (!bridge.native && bridge.fixtureWorld && enteredScene.startsWith('settings-') && enteredScene !== 'settings-account') {
      return server?.state === 'blocked';
    }
    if (server?.state === 'blocked' || server?.state === 'lease-lapsed' || server?.state === 'never-probed') return true;
    if (!loadedProfiles.has(candidate)) return true;
    const status = statuses.get(candidate);
    return !status?.host || serverLeaseState(status) !== 'fresh';
  };
  const selectedStopped = selected ? accessStopped(selected.server) : true;
  const selectedStatusLoaded = selected ? loadedProfiles.has(selected.server) : false;

  useEffect(() => {
    let alive = true;
    if (!selected || selectedStopped) { setDevices([]); setBackups([]); return; }
    // The StoreRef this request is for, captured before the await. `alive` is
    // cleared when the route leaves it, so a slow answer for one account can
    // never be drawn as another account's devices.
    const requestedStore = selected.id;
    void (async () => {
      try {
        const nextDevices = await bridge.listAccountDevices(requestedStore);
        const nextBackups = await bridge.listBackupEnrollments(requestedStore);
        if (alive) { setDevices(nextDevices); setBackups(nextBackups); }
      } catch (error) { if (alive) onError(error); }
    })();
    return () => { alive = false; };
  }, [bridge, onError, selected, selectedStopped]);

  useEffect(() => {
    let alive = true;
    if (!profile || selectedStopped) { setCards([]); setYubi([]); return; }
    // Same rule as the device list: a late answer for the profile the route
    // has left is dropped rather than shown under the one it moved to.
    void Promise.all([bridge.listYubiCards(profile), bridge.listYubiAccounts(profile)]).then(([nextCards, nextYubi]) => {
      if (alive) { setCards(nextCards); setYubi(nextYubi); }
    }).catch(onError);
    return () => { alive = false; };
  }, [bridge, onError, profile, selectedStopped]);

  useEffect(() => {
    let alive = true;
    if (section !== 'account') { setAccountDeviceNames(new Map()); return; }
    void (async () => {
      const next = new Map<string, string>();
      for (const store of accountStores(world)) {
        const server = world.servers.find((entry) => entry.id === store.server);
        const status = statuses.get(store.server);
        const fixtureStopped = Boolean(bridge.fixtureWorld && enteredScene === 'settings-account' && store.id === 'acct:work');
        const stopped = fixtureStopped || server?.state === 'blocked' || server?.state === 'lease-lapsed' || server?.state === 'never-probed' || !loadedProfiles.has(store.server) || !status?.host || serverLeaseState(status) !== 'fresh';
        if (stopped) continue;
        const current = (await bridge.listAccountDevices(store.id)).find((device) => device.current);
        if (current) next.set(store.id, current.name ?? (current.id.startsWith('08') ? 'Security key' : 'Device'));
      }
      if (alive) setAccountDeviceNames(next);
    })().catch((error: unknown) => { if (alive) onError(error); });
    return () => { alive = false; };
  }, [bridge, enteredScene, loadedProfiles, onError, section, statuses, world]);

  useEffect(() => {
    if (selectedStatusLoaded && selectedStopped) setSheet(null);
  }, [selectedStatusLoaded, selectedStopped]);

  useEffect(() => {
    let alive = true;
    void bridge.appInfo().then((info) => { if (alive) setAppInfo(info); }).catch(onError);
    return () => { alive = false; };
  }, [bridge, onError]);

  // Moving between sections keeps the exact account, so Macs -> Account ->
  // Security keys is three views of one StoreRef rather than three lookups.
  const go = (next: SettingsSection, store = selected?.id): void => onNavigate({ kind: 'settings', section: next, ...(store ? { store } : {}) });
  const applied = async (message: string): Promise<void> => { setSheet(null); await onRefresh(message); setFlash(message); };
  const account = selected ? world.accounts.find((entry) => entry.store === selected.id) : undefined;
  const server = selected ? world.servers.find((entry) => entry.id === selected.server || entry.name === selected.server) : undefined;
  const scope = section === 'macs' ? null : section === 'keys' ? 'Security keys · YubiKeys enrolled from this Mac' : section === 'account' ? 'Account · one per server, as of the signed check-in' : section === 'agent' ? 'Agent · the local agent this window talks to' : 'About · this build, and setting up again';

  return <>
    <PageHeader title="Settings" subtitle="Accounts, keys and this Mac" />
    {scope ? <div className="toolbar"><span className="scope">{scope}</span></div> : null}
    <div className="body"><div className="settings-cols">
      <nav className="settings-sections" aria-label="Settings sections">{SECTIONS.map((item) => <button type="button" key={item.id} className={item.id === section ? 'nav on' : 'nav'} onClick={() => go(item.id)}><Icon name={item.icon}/><span className="t">{item.label}</span></button>)}</nav>
      <div className="settings-main">
        {unavailable ? <UnavailableAccount stores={stores} world={world} onSelect={(store) => go(section, store.id)} onRefresh={() => void onRefresh('Refreshed the catalog')} /> : null}
        {section === 'macs' && !unavailable ? <MacsSection stores={stores} selected={selected} devices={devices} backups={backups} world={world} stopped={selectedStopped} onSwitch={(store) => onNavigate({ kind: 'settings', section: 'macs', store: store.id })} onSheet={setSheet} onPair={(mode) => { setPairMode(mode); setSheet('pair'); }} onRemove={(device) => { setRemoveDevice(device); setSheet('remove-device'); }} /> : null}
        {section === 'keys' && !unavailable ? <KeysSection yubi={yubi} cards={cards} stopped={selectedStopped} onSheet={setSheet} onAction={(command) => { setSheet('yubi'); setPendingYubi(command); }} /> : null}
        {section === 'account' ? <AccountSection world={world} statuses={statuses} loadedProfiles={loadedProfiles} deviceNames={accountDeviceNames} onPassphrase={(store, mode) => { setPassphraseStore(store); setPassphraseMode(mode); setSheet('passphrase'); }} fixtureLapsed={Boolean(bridge.fixtureWorld && enteredScene === 'settings-account')} /> : null}
        {section === 'agent' ? <AgentSection world={world} bridge={bridge} appInfo={appInfo} onRefresh={onRefresh} onError={onError} onMessage={setFlash} /> : null}
        {section === 'about' ? <AboutSection appInfo={appInfo} onSetup={() => onNavigate({ kind: 'first-run', step: 'who' })} /> : null}
      </div>
    </div></div>
    {flash ? <div className="flash" aria-live="polite">{flash}</div> : null}
    {sheet === 'phrase' && selected && account && server ? <PhraseSheet bridge={bridge} profile={selected.server} accountAlias={selected.account} username={account.username} server={server.name} seedPhrase={enteredScene === 'settings-phrase' ? bridge.firstRunFixture?.backupPhrase : undefined} onClose={() => setSheet(null)} onDone={async () => applied('Backup phrase enrolled; the secret was cleared from this window')} onError={onError} /> : null}
    {sheet === 'pair' && selected ? <PairSheet bridge={bridge} store={selected} initialMode={pairMode} onClose={() => setSheet(null)} onDone={async (message) => applied(message)} onError={onError} /> : null}
    {sheet === 'recover' && selected ? <RecoverSheet bridge={bridge} store={selected} onClose={() => setSheet(null)} onDone={async () => applied('Recovery submitted; refresh and review the authenticated device list')} onError={onError} /> : null}
    {sheet === 'enrol' && selected ? <EnrolSheet bridge={bridge} store={selected} card={cards[0]} onClose={() => setSheet(null)} onDone={async () => applied('YubiKey account created; refreshed authenticated key lists')} onError={onError} /> : null}
    {sheet === 'provision' && selected ? <ProvisionSheet bridge={bridge} store={selected} cards={cards} onClose={() => setSheet(null)} onDone={async () => applied('YubiKey device provisioned; refreshed authenticated lists')} onError={onError} /> : null}
    {sheet === 'yubi' && selected && pendingYubi ? <YubiActionSheet bridge={bridge} store={selected} action={pendingYubi} alias={(pendingYubi === 'resume-enrolment' ? yubi.find((entry) => entry.state === 'pending') : yubi.find((entry) => entry.state === 'complete'))?.alias ?? ''} onClose={() => { setSheet(null); setPendingYubi(null); }} onDone={async () => { setPendingYubi(null); await applied('Security-key action completed; refreshed authenticated lists'); }} onError={onError} /> : null}
    {sheet === 'revoke' && selected && yubi.some((entry) => entry.state === 'complete') ? <RevokeSheet bridge={bridge} store={selected} alias={yubi.find((entry) => entry.state === 'complete')?.alias ?? ''} onClose={() => setSheet(null)} onDone={async () => applied('YubiKey revoked and affected account keys rotated')} onError={onError} /> : null}
    {sheet === 'passphrase' && passphraseStore ? <PassphraseSheet bridge={bridge} store={passphraseStore} initialMode={passphraseMode} onClose={() => { setSheet(null); setPassphraseStore(null); }} onDone={(message) => { setSheet(null); setPassphraseStore(null); setFlash(message); }} onError={onError} /> : null}
    {sheet === 'remove-device' && selected && removeDevice ? <RemoveDeviceSheet bridge={bridge} store={selected} device={removeDevice} onClose={() => { setSheet(null); setRemoveDevice(null); }} onDone={async () => { const name = removeDevice.name ?? 'device'; setRemoveDevice(null); await applied(`Removed ${name} from this account`); }} onError={onError} /> : null}
  </>;
}

/**
 * The address names an account this catalog does not have.
 *
 * Nothing is selected in its place. Another account that happens to share the
 * alias is not this one: its devices, its recovery enrollments, its passphrase
 * and its security keys are all different, and quietly substituting it would
 * aim every button on this page at the wrong account. The way out is an exact
 * choice or a refresh, both taken deliberately.
 */
function UnavailableAccount({ stores, world, onSelect, onRefresh }: { stores: AccountStore[]; world: World; onSelect: (store: AccountStore) => void; onRefresh: () => void }): ReactNode {
  return <Notice severity="crit" title="This account is no longer available in the current catalog">
    <p>The address names an account this Mac does not list. No other account has been selected in its place.</p>
    <div className="sheet-actions">
      <Button variant="primary" onClick={onRefresh}>Refresh the catalog</Button>
      {stores.map((store) => <Button key={store.id} onClick={() => onSelect(store)}>{store.account} · {world.servers.find((server) => server.id === store.server)?.name ?? store.server}</Button>)}
    </div>
    {stores.length ? null : <p>This Mac has no available account at all. Add and check a server, then create or recover an account.</p>}
  </Notice>;
}

function MacsSection({ stores, selected, devices, backups, world, stopped, onSwitch, onSheet, onPair, onRemove }: { stores: AccountStore[]; selected?: AccountStore; devices: AccountDevice[]; backups: BackupEnrollment[]; world: World; stopped: boolean; onSwitch: (store: AccountStore) => void; onSheet: (sheet: Sheet) => void; onPair: (mode: 'offer' | 'accept') => void; onRemove: (device: AccountDevice) => void }): ReactNode {
  if (!selected) return <Notice title="No available account on this Mac"><p>Add and check a server, then create or recover an account.</p></Notice>;
  const account = world.accounts.find((entry) => entry.store === selected.id);
  return <>
    <div className="settings-label"><div className="sec">This account</div><div className="seg txt">{stores.map((store) => {
      // The alias is profile-local, so two profiles can both hold `personal`.
      // Where that happens the label carries the server too — otherwise the
      // switcher would offer two identical-looking buttons for two different
      // accounts.
      const ambiguous = stores.some((other) => other.id !== store.id && other.account === store.account);
      const server = world.servers.find((entry) => entry.id === store.server)?.name ?? store.server;
      return <button type="button" className={store.id === selected.id ? 'on' : ''} key={store.id} title={`${store.account} on ${server}`} onClick={() => onSwitch(store)}>{ambiguous ? `${store.account} · ${server}` : store.account}</button>;
    })}</div></div>
    <Inset className="settings-inset"><InsetRow label="Signed in as"><b>{account?.username ?? 'Identity unavailable'}</b></InsetRow><InsetRow label="Local alias">{selected.account}</InsetRow></Inset>
    {stopped ? <Notice severity="crit" title="Account access is stopped"><p>A usable signed server check-in is unavailable. No account, recovery, pairing or security-key read or write is offered here.</p></Notice> : null}
    <div className="sec">Your Macs</div><Inset className="settings-inset">{devices.map((device) => <InsetRow key={device.id} label={device.name ?? (device.id.startsWith('08') ? 'YubiKey' : 'Device')} action={<>{device.current ? <span className="chip you">{device.id.startsWith('08') ? 'current security key' : 'this Mac'}</span> : device.id.startsWith('04') ? <Button size="sm" variant="danger" disabled={stopped} onClick={() => onRemove(device)}>Remove…</Button> : <span className="chip">managed under Security keys</span>}</>}><b>{device.role}</b><small>{device.id}</small></InsetRow>)}<div className="txt">An owner device can do everything your account can. Names are authenticated when the agent reports them; otherwise only the device id is shown.</div></Inset>
    <div className="sec">Set up another Mac</div><Inset className="settings-inset"><InsetRow label="From this Mac" action={<Button variant="primary" disabled={stopped} onClick={() => onPair('offer')}>Start pairing…</Button>}>Publish an offer, show its short phrase, then Finish after the other Mac accepts.</InsetRow><InsetRow label="On this Mac" action={<Button disabled={stopped} onClick={() => onPair('accept')}>Accept or resume…</Button>}>Type the offer from a Mac you still use. Recovery remains available when no other Mac can vouch.</InsetRow></Inset>
    <div className="sec">Recovery</div><Inset className="settings-inset"><InsetRow label="Backup phrase" action={<Button disabled={stopped} onClick={() => onSheet('phrase')}>{backups.length ? 'Enrol another…' : 'Enrol…'}</Button>}>{backups.length ? `${backups.length} authenticated enrollment${backups.length === 1 ? '' : 's'} on this account` : 'No enrollment reported for this account.'}</InsetRow><InsetRow label="Recover here" action={<Button disabled={stopped} onClick={() => onSheet('recover')}>Recover…</Button>}>Use all 17 tokens to recover an owner device on this Mac.</InsetRow></Inset>
  </>;
}

type SimpleYubiAction = 'sync' | 'pin-status' | 'change-pin' | 'set-passphrase' | 'change-passphrase' | 'verify-passphrase' | 'unblock' | 'change-puk' | 'recover-management' | 'recover-subkey' | 'resume-enrolment' | 'resume-rotation' | 'rotate';

function KeysSection({ yubi, cards, stopped, onSheet, onAction }: { yubi: YubiEnrollment[]; cards: { serial: number }[]; stopped: boolean; onSheet: (sheet: Sheet) => void; onAction: (action: SimpleYubiAction) => void }): ReactNode {
  const actions: readonly [SimpleYubiAction, string, string][] = [
    ['sync', 'Sync', 'Update the key account; optionally include federation.'], ['pin-status', 'PIN status', 'Read remaining PIN attempts from the connected card.'], ['change-pin', 'Change PIN', 'Enter the current PIN and a new PIN.'], ['set-passphrase', 'Set passphrase', 'Set a new account passphrase.'], ['change-passphrase', 'Change passphrase', 'Enter the card PIN and a confirmed new passphrase.'], ['verify-passphrase', 'Verify passphrase', 'Test the passphrase with the server login challenge.'], ['unblock', 'Unblock PIN', 'Use the PUK to set a new PIN.'], ['change-puk', 'Change PUK', 'Set a new unlock code.'], ['recover-management', 'Recover management key', 'Use a software account on this Mac without a card PIN.'], ['recover-subkey', 'Recover subkey', 'Re-derive the FOKS subkey with the card PIN.'], ['resume-enrolment', 'Resume enrolment', 'Continue an interrupted enrolment.'], ['resume-rotation', 'Resume management rotation', 'Continue an interrupted rotation.'], ['rotate', 'Rotate management key', 'Replace the card management key.'],
  ];
  return <>
    <div className="sec">Enrolled keys</div><Inset className="settings-inset">{yubi.length ? yubi.map((item) => <InsetRow key={item.alias} label={item.alias} action={<span className="chip">{item.state}</span>}>Which connected serial belongs to this alias is not reported.</InsetRow>) : <InsetRow label="None">No YubiKey enrollment was reported.</InsetRow>}</Inset>
    <div className="sec">Connected now</div><Inset className="settings-inset">{cards.length ? cards.map((card) => <InsetRow key={card.serial} label={`YubiKey ${card.serial}`}><span className="chip ok">Connected</span></InsetRow>) : <InsetRow label="None">No card is connected right now.</InsetRow>}</Inset>
    {stopped ? <Notice severity="crit" title="Security-key access is stopped"><p>A usable signed server check-in is unavailable. Controls stay inert until it is available.</p></Notice> : null}
    <div className="sec">Add</div><Inset className="settings-inset"><InsetRow label="New account" action={<Button variant="primary" disabled={stopped} onClick={() => onSheet('enrol')}>Create on YubiKey…</Button>}>Keys live on the card from the start.</InsetRow><InsetRow label="Existing account" action={<Button disabled={stopped} onClick={() => onSheet('provision')}>Provision…</Button>}>Make a currently connected card an owner device for a software account on this Mac.</InsetRow></Inset>
    <div className="sec">Everyday and recovery</div><Inset className="settings-inset">{actions.map(([id, label, copy]) => { const available = !stopped && (id === 'resume-enrolment' ? yubi.some((entry) => entry.state === 'pending') : yubi.some((entry) => entry.state === 'complete')); return <InsetRow key={id} label={label} action={<Button size="sm" disabled={!available} title={available ? undefined : stopped ? 'Server access is stopped' : id === 'resume-enrolment' ? 'No pending enrollment was reported' : 'No complete enrollment was reported'} onClick={() => onAction(id)}>{label.includes(' ') ? 'Open…' : label}</Button>}>{copy}</InsetRow>; })}</Inset>
    <div className="sec danger-title">Danger</div><Inset className="danger-box settings-inset"><InsetRow label="Revoke YubiKey" action={<Button variant="danger" disabled={stopped || !yubi.some((entry) => entry.state === 'complete')} onClick={() => onSheet('revoke')}>Revoke…</Button>}>Rotates affected account keys. Copies already read cannot be recalled.</InsetRow></Inset>
  </>;
}

function AccountSection({ world, statuses, loadedProfiles, deviceNames, onPassphrase, fixtureLapsed }: { world: World; statuses: Map<string, ServerStatusSnapshot>; loadedProfiles: Set<string>; deviceNames: Map<string, string>; onPassphrase: (store: AccountStore, mode: 'set' | 'change' | 'verify') => void; fixtureLapsed: boolean }): ReactNode {
  return <>{accountStores(world).map((store) => { const server = world.servers.find((entry) => entry.id === store.server || entry.name === store.server); const account = world.accounts.find((entry) => entry.store === store.id); const status = statuses.get(store.server); const lease = serverLeaseState(status); const unavailable = !loadedProfiles.has(store.server) || !status?.host || lease !== 'fresh'; const lapsed = server?.state === 'lease-lapsed' || lease === 'lapsed' || (fixtureLapsed && store.id === 'acct:work'); const stopped = lapsed || server?.state === 'blocked' || server?.state === 'never-probed' || unavailable; const statusLabel = lapsed ? 'server check-in lapsed' : stopped ? 'signed check-in unavailable' : status?.leaseRequired === false ? 'compatibility lease not required' : 'signed check-in available'; return <div key={store.id}><div className="sec">{server?.name ?? store.server}{server?.label ? ` · ${server.label}` : ''}</div><Inset className="settings-inset"><InsetRow label="Username"><b>{account?.username ?? 'Identity unavailable'}</b> <span className={stopped ? 'chip warn' : 'chip'}>{statusLabel}</span></InsetRow><InsetRow label="Account alias">{store.account}<small>The local name; the server never sees it.</small></InsetRow><InsetRow label="Device">{stopped ? 'Not listed while access is stopped' : deviceNames.get(store.id) ?? 'No current device was reported'}<small>This account’s current authenticated device. All devices are under Your Macs &amp; recovery.</small></InsetRow><InsetRow label="Passphrase" action={<><Button size="sm" disabled={stopped} onClick={() => onPassphrase(store, 'set')}>Set…</Button><Button size="sm" disabled={stopped} onClick={() => onPassphrase(store, 'change')}>Change…</Button><Button size="sm" disabled={stopped} onClick={() => onPassphrase(store, 'verify')}>Verify</Button></>}>Whether one is set is not reported. {stopped ? 'Nothing can be set, changed or verified until server access is available.' : 'Verify runs the server public login challenge.'}</InsetRow></Inset></div>; })}</>;
}

function AgentSection({ world, bridge, appInfo, onRefresh, onError, onMessage }: { world: World; bridge: Bridge; appInfo: AppInfo | null; onRefresh: (message: string) => Promise<void>; onError: (error: unknown) => void; onMessage: (message: string) => void }): ReactNode {
  const ready = world.agent.phase === 'Ready';
  return <><div className="sec">Agent</div><Inset className="settings-inset"><InsetRow label="Status"><span className={ready ? 'agent' : 'agent warn'}><i/>{world.agent.phase}</span><small>FOKS is unavailable until the agent is ready.</small></InsetRow><InsetRow label="Socket" valueClass="mono" action={appInfo ? <Button size="sm" onClick={() => void bridge.copyText(appInfo.agentSocket).then(() => onMessage('Socket path copied')).catch(onError)}>Copy</Button> : undefined}>{appInfo?.agentSocket ?? 'Reading app info…'}<small>Local connection used by this desktop app.</small></InsetRow><InsetRow label="Connection" action={ready ? undefined : <Button variant="primary" onClick={() => void bridge.retryAgentConnection().then(async (status) => { await onRefresh(`Connection retry: ${status.phase}`); onMessage(`Connection retry: ${status.phase}`); }).catch(onError)}>Retry connection</Button>}>{ready ? 'Connected.' : 'Reconnect to the local agent. Interrupted changes will not be repeated.'}</InsetRow></Inset><details className="insp"><summary>Inspect AgentStatus</summary><pre>{JSON.stringify(world.agent, null, 2)}</pre></details></>;
}

function AboutSection({ appInfo, onSetup }: { appInfo: AppInfo | null; onSetup: () => void }): ReactNode { return <><div className="sec">About</div><Inset className="settings-inset"><InsetRow label="Version">FOKS Desktop {appInfo?.version ?? '…'}<small>Reported by this installed application.</small></InsetRow></Inset><div className="sec">Start over</div><Inset className="settings-inset"><InsetRow label="Set up again" action={<Button icon="again" onClick={onSetup}>Set up again…</Button>}>Walk first run again. Nothing already set up is undone.</InsetRow></Inset></>; }

function SheetFrame({ title, subtitle, children, footer, onClose, danger = false }: { title: string; subtitle: string; children: ReactNode; footer: ReactNode; onClose: () => void; danger?: boolean }): ReactNode { return <div className="veil" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><Dialog className="sheet wide" role={danger ? 'alertdialog' : 'dialog'} titleId="settings-sheet-title" onKeyDown={(event) => { if (event.key === 'Escape') onClose(); }}><div className="hd"><span className={`server-mark ${danger ? 'danger' : ''}`}><Icon name={danger ? 'trash' : 'gear'}/></span><span className="t"><h2 id="settings-sheet-title">{title}</h2><small>{subtitle}</small></span></div><div className="sb">{children}</div><div className="ft">{footer}</div></Dialog></div>; }

function PhraseSheet({ bridge, profile, accountAlias, username, server, seedPhrase, onClose, onDone, onError }: { bridge: Bridge; profile: string; accountAlias: string; username: string; server: string; seedPhrase?: string; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [phrase, setPhrase] = useState<string | null>(() => seedPhrase ?? null); const [alias, setAlias] = useState('paper-backup'); const [written, setWritten] = useState(false); const [busy, setBusy] = useState(false);
  const words = phrase?.split(/\s+/) ?? [];
  return <SheetFrame title={phrase ? 'Write these 17 tokens down' : 'Enrol a backup phrase'} subtitle={`${username} on ${server} · shown once, no copy button`} onClose={() => { setPhrase(null); onClose(); }} footer={<>{phrase ? <Button variant="primary" disabled={!written || busy} onClick={() => { setBusy(true); const once = phrase; setPhrase(null); void bridge.commitOwnerBackup(profile, accountAlias, alias, once).then(onDone).catch(onError).finally(() => setBusy(false)); }}>Done</Button> : <><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!alias.trim() || busy} onClick={() => { setBusy(true); void bridge.prepareOwnerBackup(profile, accountAlias, alias.trim()).then((result) => setPhrase(result.phrase)).catch(onError).finally(() => setBusy(false)); }}>Prepare phrase</Button></>}</>}>
    {phrase ? <><p>This phrase is shown once and cannot be copied. Write it down now. The agent stores only the public key.</p><div className="words">{words.map((word, index) => <span className="word" key={`${index}-${word}`}><i>{index + 1}</i>{word}</span>)}</div><label className="checkline"><input type="checkbox" checked={written} onChange={(event) => setWritten(event.target.checked)}/>I have written these down</label></> : <Inset><InsetRow label="Backup alias"><input value={alias} onChange={(event) => setAlias(event.target.value)}/></InsetRow></Inset>}
  </SheetFrame>;
}

function PairSheet({ bridge, store, initialMode, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; initialMode: 'offer' | 'accept'; onClose: () => void; onDone: (message: string) => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [mode, setMode] = useState<'offer' | 'accept'>(initialMode); const [offer, setOffer] = useState<PairingOffer | null>(null); const [target, setTarget] = useState(store.account); const [device, setDevice] = useState('This Mac'); const [phrase, setPhrase] = useState(''); const [busy, setBusy] = useState(false);
  const act = (task: () => Promise<unknown>, message: string): void => { setOffer(null); setPhrase(''); setBusy(true); void task().then(() => onDone(message)).catch(onError).finally(() => setBusy(false)); };
  const revealOffer = (task: () => Promise<PairingOffer>): void => { setOffer(null); setBusy(true); void task().then((next) => { if (next.accountAlias !== store.account) throw new Error('pairing offer returned a different account.'); setOffer(next); }).catch(onError).finally(() => setBusy(false)); };
  return <SheetFrame title="Set up another Mac" subtitle="Start, Accept, Finish or Resume an authenticated pairing" onClose={() => { setOffer(null); setPhrase(''); onClose(); }} footer={<Button onClick={onClose}>Close</Button>}>
    <div className="seg txt"><button type="button" className={mode === 'offer' ? 'on' : ''} onClick={() => { setPhrase(''); setMode('offer'); }}>From this Mac</button><button type="button" className={mode === 'accept' ? 'on' : ''} onClick={() => { setOffer(null); setMode('accept'); }}>On this Mac</button></div>
    {mode === 'offer' ? <><p>Select Start, enter the pairing phrase on the other Mac, then select Finish here. Resume opens the pending offer.</p>{offer ? <Inset><InsetRow label="Pairing phrase" valueClass="mono">{offer.phrase}</InsetRow></Inset> : null}<div className="sheet-actions"><Button variant="primary" disabled={busy} onClick={() => revealOffer(() => bridge.startDevicePairing(store.id))}>Start</Button><Button disabled={busy} onClick={() => revealOffer(() => bridge.resumeDevicePairingOffer(store.id))}>Resume offer</Button><Button disabled={!offer || busy} onClick={() => act(async () => { const result = await bridge.finishDevicePairing(store.id); if (result.alias !== store.account) throw new Error('finish_device_pairing returned a different account.'); return result; }, 'Pairing finished; refreshed authenticated devices')}>Finish</Button></div></> : <><p>Enter the pairing phrase from the other Mac. Resume continues a pending acceptance.</p><Inset><InsetRow label="Account alias"><input value={target} onChange={(event) => setTarget(event.target.value)}/></InsetRow><InsetRow label="Device name"><input value={device} onChange={(event) => setDevice(event.target.value)}/></InsetRow><InsetRow label="Pairing phrase"><input type="password" value={phrase} onChange={(event) => setPhrase(event.target.value)}/></InsetRow></Inset><div className="sheet-actions"><Button variant="primary" disabled={busy || !target || !device || !phrase} onClick={() => act(async () => { const result = await bridge.acceptDevicePairing(store.server, target, device, phrase); if (result.alias !== target) throw new Error('accept_device_pairing returned a different account.'); return result; }, 'Pairing accepted; refreshed authenticated devices')}>Accept</Button><Button disabled={busy || !target} onClick={() => act(async () => { const result = await bridge.resumeDevicePairingAcceptance(store.server, target); if (result.alias !== target) throw new Error('resume_device_pairing_acceptance returned a different account.'); return result; }, 'Pairing acceptance resumed; refreshed authenticated devices')}>Resume acceptance</Button></div></>}
  </SheetFrame>;
}

function RecoverSheet({ bridge, store, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode { const [target, setTarget] = useState(store.account); const [device, setDevice] = useState('This Mac'); const [phrase, setPhrase] = useState(''); const [busy, setBusy] = useState(false); return <SheetFrame title="Recover on this Mac" subtitle="Use the 17-token backup phrase" onClose={() => { setPhrase(''); onClose(); }} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!target || !device || !phrase || busy} onClick={() => { const once = phrase; setPhrase(''); setBusy(true); void bridge.recoverOwnerAccount(store.server, target, once, device).then(onDone).catch(onError).finally(() => setBusy(false)); }}>Recover</Button></>}><p>Recovery adds this Mac as a new owner device. If interrupted, resume it from Issues with the same phrase.</p><Inset><InsetRow label="Local alias"><input value={target} onChange={(event) => setTarget(event.target.value)}/></InsetRow><InsetRow label="Device name"><input value={device} onChange={(event) => setDevice(event.target.value)}/></InsetRow><InsetRow label="17 tokens"><textarea value={phrase} onChange={(event) => setPhrase(event.target.value)}/></InsetRow></Inset></SheetFrame>; }

function EnrolSheet({ bridge, store, card, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; card?: { serial: number }; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [alias, setAlias] = useState('work-key'); const [username, setUsername] = useState(''); const [deviceName, setDeviceName] = useState(card ? `YubiKey ${card.serial}` : 'YubiKey'); const [invite, setInvite] = useState(''); const [pin, setPin] = useState(''); const [puk, setPuk] = useState(''); const [signingSlot, setSigningSlot] = useState('0x82'); const [pqSlot, setPqSlot] = useState('0x83'); const [pinAttempts, setPinAttempts] = useState(3); const [pukAttempts, setPukAttempts] = useState(3); const [busy, setBusy] = useState(false);
  const clear = (): void => { setInvite(''); setPin(''); setPuk(''); };
  const slot = (value: string): number | null => /^0x[0-9a-fA-F]{2}$/.test(value) ? Number.parseInt(value.slice(2), 16) : null;
  const signing = slot(signingSlot); const pq = slot(pqSlot);
  const validAttempts = Number.isInteger(pinAttempts) && pinAttempts > 0 && pinAttempts <= 255 && Number.isInteger(pukAttempts) && pukAttempts > 0 && pukAttempts <= 255;
  return <SheetFrame title="Create a YubiKey account" subtitle={`A new account on ${store.server}; keys live on the card from the start`} onClose={() => { clear(); onClose(); }} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!card || !alias.trim() || !username.trim() || !deviceName.trim() || !pin || !puk || signing === null || pq === null || signing === pq || !validAttempts || busy} onClick={() => { if (signing === null || pq === null) return; const command: YubiCommand = { command: 'create_yubi_account', args: { profile: store.server, alias: alias.trim(), username: username.trim(), deviceName: deviceName.trim(), email: '', invite, cardSerial: card?.serial ?? 0, signingSlot: signing, pqSlot: pq, pin, puk, pinAttempts, pukAttempts } }; clear(); setBusy(true); void bridge.runYubi(command).then(onDone).catch(onError).finally(() => setBusy(false)); }}>Prepare card and create account</Button></>}><p>Before you continue:</p><ol className="facts"><li><b>FOKS writes both keys to the card in one step.</b> If that step fails, resetting the PIV applet erases the card.</li><li><b>The card must use its factory management key.</b> A managed card cannot be used.</li><li><b>Save the unlock code you choose.</b> FOKS cannot recover it later.</li></ol>{card ? <p><b>YubiKey {card.serial}</b> is connected. FOKS cannot identify an existing alias from the card serial.</p> : <Notice title="Connect a YubiKey"><p>No card is connected.</p></Notice>}<Inset><InsetRow label="Alias"><input value={alias} onChange={(event) => setAlias(event.target.value)}/></InsetRow><InsetRow label="Username"><input value={username} onChange={(event) => setUsername(event.target.value)}/></InsetRow><InsetRow label="Device name"><input value={deviceName} onChange={(event) => setDeviceName(event.target.value)}/></InsetRow><InsetRow label="Card PIN"><input type="password" value={pin} onChange={(event) => setPin(event.target.value)}/></InsetRow><InsetRow label="Unlock code"><input type="password" value={puk} onChange={(event) => setPuk(event.target.value)}/></InsetRow><InsetRow label="Invite"><input type="password" value={invite} onChange={(event) => setInvite(event.target.value)}/><small>Optional; cleared on submission.</small></InsetRow></Inset><details className="adv"><summary>Advanced</summary><Inset><InsetRow label="Signing slot"><input className="mono" value={signingSlot} onChange={(event) => setSigningSlot(event.target.value)}/></InsetRow><InsetRow label="Second slot"><input className="mono" value={pqSlot} onChange={(event) => setPqSlot(event.target.value)}/></InsetRow><InsetRow label="PIN tries"><input type="number" min={1} max={255} value={pinAttempts} onChange={(event) => setPinAttempts(Number(event.target.value))}/></InsetRow><InsetRow label="PUK tries"><input type="number" min={1} max={255} value={pukAttempts} onChange={(event) => setPukAttempts(Number(event.target.value))}/></InsetRow></Inset></details><p className="hint">PIN, unlock code and invite go only to the local agent and are cleared when submitted.</p></SheetFrame>;
}

function ProvisionSheet({ bridge, store, cards, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; cards: { serial: number }[]; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [targetAlias, setTargetAlias] = useState('new-key'); const [deviceName, setDeviceName] = useState(cards[0] ? `YubiKey ${cards[0].serial}` : 'YubiKey'); const [serial, setSerial] = useState(cards[0]?.serial ?? 0); const [pin, setPin] = useState(''); const [puk, setPuk] = useState(''); const [busy, setBusy] = useState(false);
  const clear = (): void => { setPin(''); setPuk(''); };
  const valid = Boolean(cards.some((card) => card.serial === serial) && targetAlias.trim() && deviceName.trim() && pin && puk);
  return <SheetFrame title="Provision a YubiKey device" subtitle={`A fresh local alias on ${store.account}`} onClose={() => { clear(); onClose(); }} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!valid || busy} onClick={() => { const command: YubiCommand = { command: 'provision_yubi_device', args: { accountStoreId: store.id, targetAlias: targetAlias.trim(), deviceName: deviceName.trim(), cardSerial: serial, signingSlot: 0x82, pqSlot: 0x83, pin, puk, pinAttempts: 3, pukAttempts: 3 } }; clear(); setBusy(true); void bridge.runYubi(command).then(onDone).catch(onError).finally(() => setBusy(false)); }}>Provision card</Button></>}><p>Enter a new alias and select a connected card.</p><Inset><InsetRow label="Target alias"><input value={targetAlias} onChange={(event) => setTargetAlias(event.target.value)}/></InsetRow><InsetRow label="Device name"><input value={deviceName} onChange={(event) => setDeviceName(event.target.value)}/></InsetRow><InsetRow label="Connected card"><select value={serial} onChange={(event) => setSerial(Number(event.target.value))}>{cards.map((card) => <option key={card.serial} value={card.serial}>YubiKey {card.serial}</option>)}</select></InsetRow><InsetRow label="Card PIN"><input type="password" value={pin} onChange={(event) => setPin(event.target.value)}/></InsetRow><InsetRow label="Unlock code"><input type="password" value={puk} onChange={(event) => setPuk(event.target.value)}/></InsetRow></Inset></SheetFrame>;
}

function YubiActionSheet({ bridge, store, action, alias, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; action: SimpleYubiAction; alias: string; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [pin, setPin] = useState(''); const [other, setOther] = useState(''); const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false);
  const needsPin = !['pin-status', 'recover-management'].includes(action);
  const needsOther = ['change-pin', 'set-passphrase', 'change-passphrase', 'verify-passphrase', 'unblock', 'change-puk'].includes(action);
  const needsConfirmation = action === 'set-passphrase' || action === 'change-passphrase';
  const valid = Boolean(alias && (!needsPin || action === 'resume-rotation' || pin) && (!needsOther || other) && (!needsConfirmation || other === confirmation));
  const firstLabel = action === 'unblock' || action === 'change-puk' ? 'Current PUK' : 'Card PIN';
  const otherLabel = action === 'change-pin' || action === 'unblock' ? 'New PIN' : action === 'change-puk' ? 'New PUK' : 'Passphrase';
  const submit = (): void => { let command: YubiCommand;
    switch (action) {
      case 'sync': command = { command: 'sync_yubi_account', args: { profile: store.server, alias, pin, withFederation: true } }; break;
      case 'pin-status': command = { command: 'yubi_pin_status', args: { profile: store.server, alias } }; break;
      case 'change-pin': command = { command: 'change_yubi_pin', args: { profile: store.server, alias, oldPin: pin, newPin: other } }; break;
      case 'set-passphrase': case 'change-passphrase': command = { command: action === 'set-passphrase' ? 'set_yubi_passphrase' : 'change_yubi_passphrase', args: { profile: store.server, alias, pin, passphrase: other, confirmation } }; break;
      case 'verify-passphrase': command = { command: 'verify_yubi_passphrase', args: { profile: store.server, alias, pin, passphrase: other } }; break;
      case 'unblock': command = { command: 'unblock_yubi_pin', args: { profile: store.server, alias, puk: pin, newPin: other } }; break;
      case 'change-puk': command = { command: 'change_yubi_puk', args: { profile: store.server, alias, oldPuk: pin, newPuk: other } }; break;
      case 'recover-management': command = { command: 'recover_yubi_management_key', args: { accountStoreId: store.id, yubiAlias: alias } }; break;
      case 'recover-subkey': command = { command: 'recover_yubi_subkey', args: { profile: store.server, alias, pin } }; break;
      case 'resume-enrolment': command = { command: 'resume_yubi_account', args: { profile: store.server, alias, pin } }; break;
      case 'resume-rotation': command = { command: 'resume_yubi_management_key', args: { profile: store.server, alias, ...(pin ? { pin } : {}) } }; break;
      case 'rotate': command = { command: 'rotate_yubi_management_key', args: { profile: store.server, alias, pin } }; break;
    }
    setPin(''); setOther(''); setConfirmation(''); setBusy(true); void bridge.runYubi(command).then(onDone).catch(onError).finally(() => setBusy(false));
  };
  return <SheetFrame title={action.replaceAll('-', ' ')} subtitle={alias || 'Choose an enrolled key alias'} onClose={() => { setPin(''); setOther(''); setConfirmation(''); onClose(); }} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!valid || busy} onClick={submit}>Continue</Button></>}><p>The values below go only to the local agent and are cleared when submitted.</p><Inset><InsetRow label="Key alias"><b>{alias || 'No key enrolled'}</b></InsetRow>{needsPin ? <InsetRow label={firstLabel}><input type="password" value={pin} onChange={(event) => setPin(event.target.value)}/></InsetRow> : null}{needsOther ? <InsetRow label={otherLabel}><input type="password" value={other} onChange={(event) => setOther(event.target.value)}/></InsetRow> : null}{needsConfirmation ? <InsetRow label="Confirm"><input type="password" value={confirmation} onChange={(event) => setConfirmation(event.target.value)}/></InsetRow> : null}</Inset></SheetFrame>;
}

function RevokeSheet({ bridge, store, alias, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; alias: string; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode { const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false); return <SheetFrame title={`Revoke ${alias}?`} subtitle="Rotates affected account keys" onClose={onClose} danger footer={<><Button onClick={onClose}>Cancel</Button><Button variant="danger" disabled={confirmation !== alias || busy} onClick={() => { setBusy(true); void bridge.runYubi({ command: 'revoke_yubi_device', args: { accountStoreId: store.id, yubiAlias: alias, confirmation } }).then(onDone).catch(onError).finally(() => setBusy(false)); }}>Revoke {alias}</Button></>}><p>The card will no longer open the account. Copies it already read cannot be recalled. The alias is used because the agent does not report which connected serial belongs to it.</p><Inset><InsetRow label="Confirm"><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} placeholder={`type ${alias}`}/></InsetRow></Inset></SheetFrame>; }

function RemoveDeviceSheet({ bridge, store, device, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; device: AccountDevice; onClose: () => void; onDone: () => Promise<void>; onError: (error: unknown) => void }): ReactNode { const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false); const expected = device.name ?? device.id; return <SheetFrame title={`Remove ${device.name ?? 'device'}?`} subtitle={`${store.account} · ${device.id}`} onClose={onClose} danger footer={<><Button onClick={onClose}>Cancel</Button><Button variant="danger" disabled={confirmation !== expected || busy} onClick={() => { setBusy(true); void bridge.removeAccountDevice(store.id, device.id).then((removed) => { if (removed.deviceId !== device.id) throw new Error('remove_account_device returned a different device.'); return onDone(); }).catch(onError).finally(() => setBusy(false)); }}>Remove device</Button></>}><p>This device loses future access to the account. Copies of values it already read cannot be recalled; rotate those secrets if the device is not under your control.</p><p className="fn">This action is available only for a non-current software device. YubiKeys are revoked under Security keys.</p><Inset><InsetRow label="Confirm"><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} placeholder={`type ${expected}`}/></InsetRow></Inset></SheetFrame>; }

function PassphraseSheet({ bridge, store, initialMode, onClose, onDone, onError }: { bridge: Bridge; store: AccountStore; initialMode: 'set' | 'change' | 'verify'; onClose: () => void; onDone: (message: string) => void; onError: (error: unknown) => void }): ReactNode { const [mode, setMode] = useState<'set' | 'change' | 'verify'>(initialMode); const [passphrase, setPassphrase] = useState(''); const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false); const submit = (): void => { const secret = passphrase; const repeated = confirmation; setPassphrase(''); setConfirmation(''); setBusy(true); const task = mode === 'set' ? bridge.setAccountPassphrase(store.id, secret, repeated) : mode === 'change' ? bridge.changeAccountPassphrase(store.id, secret, repeated) : bridge.verifyAccountPassphrase(store.id, secret); void task.then((report) => onDone(`Passphrase ${mode} succeeded · generation ${report.generation}`)).catch(onError).finally(() => setBusy(false)); }; return <SheetFrame title="Account passphrase" subtitle="Whether one is currently set is not reported" onClose={() => { setPassphrase(''); setConfirmation(''); onClose(); }} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!passphrase || (mode !== 'verify' && passphrase !== confirmation) || busy} onClick={submit}>{mode === 'verify' ? 'Verify' : mode === 'set' ? 'Set passphrase' : 'Change passphrase'}</Button></>}><div className="seg txt"><button type="button" className={mode === 'set' ? 'on' : ''} onClick={() => setMode('set')}>Set</button><button type="button" className={mode === 'change' ? 'on' : ''} onClick={() => setMode('change')}>Change</button><button type="button" className={mode === 'verify' ? 'on' : ''} onClick={() => setMode('verify')}>Verify</button></div><Inset><InsetRow label="Passphrase"><input type="password" value={passphrase} onChange={(event) => setPassphrase(event.target.value)}/></InsetRow>{mode === 'verify' ? null : <InsetRow label="Confirm"><input type="password" value={confirmation} onChange={(event) => setConfirmation(event.target.value)}/></InsetRow>}</Inset></SheetFrame>; }
