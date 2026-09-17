import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Dialog } from '/kit/overlay-primitives';
import type { Bridge, CheckedServer, ResetPreview, ServerStatusSnapshot } from '../bridge';
import { Button, Icon, Inset, InsetRow, Notice } from '../components';
import type { Location } from '../location';
import { serverLeaseState } from '../model';
import type { Server, World } from '../model';
import { PageHeader } from '../shell/page-header';

interface Props {
  world: World;
  bridge: Bridge;
  location: Extract<Location, { kind: 'servers' }>;
  scene: string;
  onNavigate: (location: Location) => void;
  onRefresh: (message: string) => Promise<void>;
  onError: (error: unknown) => void;
}

type Sheet = 'add' | 'reset' | 'forget' | null;

const shortId = (value: string): string => `${value.slice(0, 10)}…${value.slice(-8)}`;
const expires = (value: number | null): string => {
  if (value === null) return 'No signed expiry is available';
  const date = new Date(value * 1000);
  return Number.isFinite(date.getTime())
    ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(date)
    : `Unix time ${value} seconds`;
};
const acceptanceText = (value: CheckedServer['acceptance']): string =>
  value === 'inserted'
    ? 'New pin'
    : value === 'advanced'
      ? 'Pinned history advanced safely'
      : 'Pinned history unchanged';

function serverFor(world: World, profile: string | undefined): Server | undefined {
  return profile ? world.servers.find((server) => server.id === profile) : undefined;
}

function StatusChip({ state }: { state: 'checked' | 'lapsed' | 'unavailable' | 'blocked' | 'unprobed' }): ReactNode {
  return <span className={state === 'checked' ? 'chip ok' : state === 'unprobed' ? 'chip' : 'chip bad'}>
    {state === 'checked' ? 'Checked' : state === 'lapsed' ? 'Server check-in lapsed' : state === 'unavailable' ? 'Signed check-in unavailable' : state === 'blocked' ? 'History mismatch' : 'Never checked'}
  </span>;
}

export function ServersScreen({ world, bridge, location, scene, onNavigate, onRefresh, onError }: Props): ReactNode {
  // The named fixture scene this screen was *entered* at, captured once. See
  // the same capture in settings-screen.tsx: `scene` is read live from the
  // address bar, which the shell rewrites to the canonical `servers` name on
  // its first state change, so reading it during render makes `rollback` and
  // the fixture flags below switch themselves off as soon as anything
  // re-renders. Navigating away and back remounts and re-reads it.
  const [enteredScene] = useState(scene);
  const [statuses, setStatuses] = useState<Map<string, ServerStatusSnapshot>>(new Map());
  const [statusFailures, setStatusFailures] = useState<Set<string>>(new Set());
  const [checked, setChecked] = useState<Map<string, CheckedServer>>(new Map());
  const [sheet, setSheet] = useState<Sheet>(() => enteredScene === 'servers-add' ? 'add' : enteredScene === 'servers-reset' ? 'reset' : null);
  const [reset, setReset] = useState<ResetPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [full, setFull] = useState(false);
  const [oob, setOob] = useState('');
  const [flash, setFlash] = useState<string | null>(null);
  const selected = serverFor(world, location.profile);
  const seededCheck = useRef(false);
  const rollback = enteredScene === 'servers-rollback';
  const fixtureLapsed = Boolean(bridge.fixtureWorld && (enteredScene === 'servers-list' || enteredScene === 'servers-add' || enteredScene === 'servers-lapsed'));

  useEffect(() => {
    let alive = true;
    void (async () => {
      const rows = new Map<string, ServerStatusSnapshot>();
      const failures = new Set<string>();
      for (const server of world.servers) {
        if (server.state === 'blocked') continue;
        try {
          const status = await bridge.describeServerStatus(server.id);
          if (status.profile !== server.id) throw new Error('describe_server_status returned a different profile.');
          rows.set(server.id, status);
        } catch (error) {
          failures.add(server.id);
          if (alive) onError(error);
        }
      }
      if (alive) {
        setStatuses(rows);
        setStatusFailures(failures);
      }
    })();
    return () => { alive = false; };
  }, [bridge, onError, world.servers]);

  useEffect(() => {
    if (!bridge.fixtureWorld || enteredScene !== 'servers-check' || !selected || seededCheck.current) return;
    seededCheck.current = true;
    void bridge.checkServer(selected.id).then(async (report) => {
      if (report.profile !== selected.id) throw new Error('check_server returned a different profile.');
      setChecked((current) => new Map(current).set(selected.id, report));
      const passive = await bridge.describeServerStatus(selected.id);
      if (passive.profile !== selected.id) throw new Error('describe_server_status returned a different profile.');
      setStatuses((current) => new Map(current).set(selected.id, passive));
      setStatusFailures((current) => { const next = new Set(current); next.delete(selected.id); return next; });
    }).catch(onError);
  }, [bridge, enteredScene, onError, selected]);

  useEffect(() => {
    if (sheet !== 'reset' || !selected) return;
    let alive = true;
    setReset(null);
    void bridge.describeReset(selected.id).then((preview) => {
      if (preview.profile !== selected.id) throw new Error('describe_reset returned a different profile.');
      if (alive) setReset(preview);
    }).catch(onError);
    return () => { alive = false; };
  }, [bridge, onError, selected, sheet]);

  const check = async (server: Server): Promise<void> => {
    setBusy(true);
    try {
      const report = await bridge.checkServer(server.id);
      if (report.profile !== server.id) throw new Error('check_server returned a different profile.');
      setChecked((current) => new Map(current).set(server.id, report));
      setFlash(`Checked ${report.canonicalName} — ${acceptanceText(report.acceptance)}`);
      try {
        const passive = await bridge.describeServerStatus(server.id);
        if (passive.profile !== server.id) throw new Error('describe_server_status returned a different profile.');
        setStatuses((current) => new Map(current).set(server.id, passive));
        setStatusFailures((current) => { const next = new Set(current); next.delete(server.id); return next; });
        await onRefresh(`Checked ${report.canonicalName}; refreshed signed server status`);
      } catch (error) {
        setStatusFailures((current) => new Set(current).add(server.id));
        onError(error);
      }
    } catch (error) {
      onError(error);
    } finally {
      setBusy(false);
    }
  };

  const currentHost = selected
    ? checked.get(selected.id) ?? statuses.get(selected.id)?.host ?? null
    : null;

  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'n') {
        event.preventDefault(); setSheet('add'); return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'r' && selected) {
        event.preventDefault(); void check(selected); return;
      }
      if (!['ArrowDown', 'ArrowUp'].includes(event.key) || !world.servers.length) return;
      event.preventDefault();
      const at = selected ? world.servers.findIndex((server) => server.id === selected.id) : -1;
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      const next = world.servers[(at + delta + world.servers.length) % world.servers.length];
      onNavigate({ kind: 'servers', profile: next.id });
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

  const overlay = sheet === 'add' ? (
    <AddServerSheet bridge={bridge} onClose={() => setSheet(null)} onAdded={async (profile) => {
      setSheet(null);
      await onRefresh('Server added — check it before trusting anything on it');
      onNavigate({ kind: 'servers', profile });
    }} onError={onError} />
  ) : sheet === 'reset' && selected ? (
    <ResetSheet server={selected} preview={reset} bridge={bridge} onClose={() => { setSheet(null); setReset(null); }} onReset={async () => {
      setSheet(null); setReset(null); await onRefresh(`Reset ${selected.name} — check it again before using it`);
    }} onError={onError} />
  ) : sheet === 'forget' && selected ? (
    <ForgetSheet server={selected} bridge={bridge} onClose={() => setSheet(null)} onForgot={async () => {
      setSheet(null); onNavigate({ kind: 'servers' }); await onRefresh(`Forgot ${selected.name} on this Mac`);
    }} onError={onError} />
  ) : null;

  return <>
    {selected ? <ServerPage world={world} server={selected} status={statuses.get(selected.id)} statusFailed={statusFailures.has(selected.id)} host={currentHost} checked={checked.get(selected.id)} rollback={rollback} fixtureLapsed={fixtureLapsed && selected.id === 'acme'} busy={busy} full={full} oob={oob} setFull={setFull} setOob={setOob} onBack={() => onNavigate({ kind: 'servers' })} onCheck={() => void check(selected)} onReset={() => setSheet('reset')} onForget={() => setSheet('forget')} onCopy={(text) => void bridge.copyText(text).then(() => setFlash('Copied the full id')).catch(onError)} /> : <ServerList world={world} statuses={statuses} statusFailures={statusFailures} fixtureLapsed={fixtureLapsed} busy={busy} onOpen={(profile) => onNavigate({ kind: 'servers', profile })} onCheck={(server) => void check(server)} onAdd={() => setSheet('add')} />}
    {flash ? <div className="flash" aria-live="polite">{flash}</div> : null}
    {overlay}
  </>;
}

function ServerList({ world, statuses, statusFailures, fixtureLapsed, busy, onOpen, onCheck, onAdd }: { world: World; statuses: Map<string, ServerStatusSnapshot>; statusFailures: Set<string>; fixtureLapsed: boolean; busy: boolean; onOpen: (profile: string) => void; onCheck: (server: Server) => void; onAdd: () => void }): ReactNode {
  return <>
    <PageHeader title="Servers & devices" subtitle={`${world.servers.length} servers on this Mac`} action={<Button icon="plus" onClick={onAdd}>Add a server…</Button>} />
    <div className="body"><div className="server-wrap"><div className="server-cards">
      {world.servers.map((server) => {
        const snapshot = statuses.get(server.id);
        const lease = serverLeaseState(snapshot);
        const lapsed = server.state === 'lease-lapsed' || lease === 'lapsed' || (fixtureLapsed && server.id === 'acme');
        const unavailable = statusFailures.has(server.id) || server.state === 'lease-unavailable' || Boolean(snapshot?.host && lease === 'unavailable');
        const state = server.state === 'blocked' ? 'blocked' : lapsed ? 'lapsed' : unavailable ? 'unavailable' : snapshot?.host && lease === 'fresh' ? 'checked' : 'unprobed';
        const availableCheckIn = snapshot?.leaseRequired === false ? 'Compatibility lease not required by this pinned protocol.' : `Signed lease expiry: ${expires(snapshot?.leaseExpiresAt ?? null)}.`;
        const account = world.accounts.find((item) => item.server === server.id || item.server === server.name);
        const groups = world.stores.filter((store) => store.kind === 'team' && store.server === server.id).length;
        return <div role="button" tabIndex={0} className={`server-card ${state === 'lapsed' || state === 'blocked' ? 'crit' : ''}`} key={server.id} onClick={() => onOpen(server.id)} onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onOpen(server.id); } }}>
          <span className="server-mark"><Icon name="server"/></span><span className="server-copy"><span className="server-line"><b>{server.name}</b><StatusChip state={state}/>{server.label ? <span className="hint">{server.label}</span> : null}</span>
          <span className="hint">{state === 'unprobed' ? 'Not checked. Check this server before using it.' : state === 'lapsed' ? `Access blocked. Signed lease expiry: ${expires(snapshot?.leaseExpiresAt ?? null)}.` : state === 'unavailable' ? 'Signed status unavailable. Reads and writes are blocked.' : state === 'blocked' ? 'The signed history does not match this Mac’s checkpoint.' : `Host verified. ${availableCheckIn}`}</span>
          {state !== 'lapsed' && state !== 'unavailable' && state !== 'blocked' && account ? <span className="hint">You: <b>{account.username}</b> · {account.alias} and {groups} {groups === 1 ? 'group' : 'groups'}</span> : null}</span>
          <span className="server-actions">{state === 'unprobed' ? <Button variant="primary" disabled={busy} onClick={(event) => { event.stopPropagation(); onCheck(server); }}>Check now</Button> : null}<Icon name="arrow"/></span>
        </div>;
      })}
    </div>
    </div></div>
  </>;
}

function ServerPage({ world, server, status, statusFailed, host, checked, rollback, fixtureLapsed, busy, full, oob, setFull, setOob, onBack, onCheck, onReset, onForget, onCopy }: { world: World; server: Server; status?: ServerStatusSnapshot; statusFailed: boolean; host: ServerStatusSnapshot['host']; checked?: CheckedServer; rollback: boolean; fixtureLapsed: boolean; busy: boolean; full: boolean; oob: string; setFull: (value: boolean) => void; setOob: (value: string) => void; onBack: () => void; onCheck: () => void; onReset: () => void; onForget: () => void; onCopy: (text: string) => void }): ReactNode {
  const lease = serverLeaseState(status);
  const lapsed = fixtureLapsed || server.state === 'lease-lapsed' || lease === 'lapsed';
  const unavailable = statusFailed || server.state === 'lease-unavailable' || Boolean(host && lease === 'unavailable');
  const blocked = rollback || server.state === 'blocked';
  const factsUnavailable = unavailable || blocked;
  const checkIn = status?.leaseRequired === false ? 'Not required by this pinned protocol' : `Signed expiry ${expires(status?.leaseExpiresAt ?? null)}`;
  const account = world.accounts.find((item) => item.server === server.id || item.server === server.name);
  const groups = world.stores.filter((store) => store.kind === 'team' && store.server === server.id);
  const compare = !host || !oob.trim() ? null : host.hostId === oob.replace(/\s/g, '').toLowerCase() ? 'Match' : host.hostId.startsWith(oob.replace(/\s/g, '').toLowerCase()) ? 'Prefix only' : 'Mismatch';
  const state = blocked ? 'blocked' : lapsed ? 'lapsed' : unavailable ? 'unavailable' : host ? 'checked' : 'unprobed';
  const subtitle = !lapsed && !unavailable && !blocked && account
    ? `${server.label ?? 'Server'} · signed in as ${account.username}`
    : server.label ?? 'Server';
  return <>
    <PageHeader title={server.name} subtitle={subtitle} />
    <div className="toolbar"><StatusChip state={state}/><Button onClick={onBack}>‹ Servers</Button><span className="spacer"/>{!host ? <Button variant="primary" disabled={busy || blocked} onClick={onCheck}>Check now</Button> : null}</div>
    <div className="body"><div className="server-wrap">
      {blocked ? <Notice severity="crit" eyebrow={`${server.name} · history check failed`} title="This server’s history does not match this Mac’s checkpoint"><p>Access to the entire server is blocked.</p><p className="fn">Use Reset only after you confirm why the server history changed.</p></Notice> : lapsed ? <Notice severity="crit" eyebrow={`${server.name} · server check-in lapsed`} title={`Nothing on ${server.name} can be read or changed`}><p>The compatibility lease has expired. The agent renews it; Check does not.</p><p className="fn">Signed expiry: {expires(status?.leaseExpiresAt ?? null)}.</p></Notice> : unavailable ? <Notice severity="crit" eyebrow={`${server.name} · signed check-in unavailable`} title={`Nothing on ${server.name} can be read or changed`}><p>The signed compatibility-lease status is unavailable. Reads and writes are blocked until a valid status is available.</p></Notice> : null}
      {!host ? <p className="said">{factsUnavailable ? 'Host and pin status are unavailable.' : 'This server has not been checked. Check it to verify its signed chain and pin its host ID.'}</p> : checked ? <p className="said">Checked {server.name}. Canonical name: <b>{checked.canonicalName}</b>. <b>{acceptanceText(checked.acceptance)}</b>.</p> : null}
      <div className="status-list" aria-label="Server status">
        <StatusRow label="Status" value={blocked ? 'History mismatch — whole server blocked' : lapsed ? 'Check-in lapsed — reads and writes stopped' : unavailable ? 'Signed check-in unavailable — reads and writes stopped' : host ? 'Checked' : 'Never checked'} />
        <StatusRow label="Trusted since" value={host ? 'Pinned on first use; time is not reported' : factsUnavailable ? 'Pin fact unavailable' : 'Not pinned'} />
        <StatusRow label="Check-in" value={lapsed ? `Lapsed · signed expiry ${expires(status?.leaseExpiresAt ?? null)}` : unavailable ? 'No usable required expiry is available' : blocked ? 'Signed check-in unavailable while history is blocked' : checkIn} />
        <StatusRow label="You" value={lapsed || unavailable || blocked ? 'Not listed while server reads are stopped' : account ? `${account.username} · local alias ${account.alias}` : 'No account on this server'} />
        <StatusRow label="Groups" value={lapsed || unavailable || blocked ? 'Not listed while server reads are stopped' : account ? `${groups.length} ${groups.length === 1 ? 'group' : 'groups'} through this account` : `${groups.length ? groups.length : 'No'} ${groups.length === 1 ? 'group' : 'groups'} listed on this server`} />
      </div>
      <details className="server-details" open={rollback ? true : undefined} onClick={(event) => { if (rollback && (event.target as HTMLElement).closest('summary')) event.preventDefault(); }} onKeyDown={(event) => { if (rollback && (event.target as HTMLElement).closest('summary')) event.preventDefault(); }}><summary tabIndex={rollback ? -1 : undefined} aria-disabled={rollback || undefined}>Details</summary>
        <div className="sec">Host facts</div>
        <Inset>{host ? <>
          <InsetRow label="Lookup name">{host.lookupName}</InsetRow><InsetRow label="Canonical name">{host.canonicalName}</InsetRow>
          <InsetRow label="Host id" valueClass="mono" action={<><Button size="sm" disabled={blocked} onClick={() => setFull(!full)}>{full ? 'Show short' : 'Show full'}</Button><Button size="sm" disabled={blocked} onClick={() => onCopy(host.hostId)}>Copy</Button></>}>{full ? host.hostId : shortId(host.hostId)}</InsetRow>
          <InsetRow label="Signed history entries">{host.chain}</InsetRow><InsetRow label="Signed tree version">{host.epoch}</InsetRow>
          <InsetRow label="Server check-in">{lapsed ? 'Lapsed' : unavailable ? 'No usable required expiry is available' : checkIn}</InsetRow>
        </> : <InsetRow label={factsUnavailable ? 'Unavailable' : 'Nothing yet'} action={<Button variant="primary" disabled={busy || blocked} onClick={onCheck}>Check now</Button>}>{factsUnavailable ? 'Host, pin and prior-check facts are not available in this stopped view.' : 'Host id and signed history arrive only from Check.'}</InsetRow>}</Inset>
        {host ? <><div className="sec">Trust</div><Inset><InsetRow label="Pinned">On first use; no timestamp is reported.</InsetRow><InsetRow label="Check" action={<Button size="sm" disabled={busy || blocked} onClick={onCheck}>Check again</Button>}>Explicit network read; result: {checked ? acceptanceText(checked.acceptance) : 'not run in this view'}</InsetRow><InsetRow label="Compare out of band" className="server-compare"><input aria-label="They published" disabled={blocked} value={oob} onChange={(event) => setOob(event.target.value)} placeholder="Paste their full host id"/><span className={compare === 'Match' ? 'chip ok' : compare === 'Mismatch' ? 'chip bad' : 'chip'}>{compare ?? 'Waiting for an id'}</span><span className="hint">Compared only on this Mac. Nothing is stored or sent.</span></InsetRow><InsetRow label="History check">Runs before every operation against the pinned signed history.</InsetRow></Inset></> : null}
        <details className="insp" open={rollback ? true : undefined} onClick={(event) => { if (rollback) event.preventDefault(); }} onKeyDown={(event) => { if (rollback) event.preventDefault(); }}><summary tabIndex={rollback ? -1 : undefined} aria-disabled={rollback || undefined}>Inspect last check response</summary><pre>{JSON.stringify({ profile: status?.profile ?? server.id, configuredProbe: status?.configuredProbe ?? server.name, host, leaseRequired: status?.leaseRequired ?? null, leaseExpiresAt: status?.leaseExpiresAt ?? null }, null, 2)}</pre></details>
      </details>
      <div className="sec danger-title">Danger</div><Inset className="danger-box"><InsetRow label="Forget this server" action={<Button size="sm" disabled={blocked} onClick={onForget}>Forget…</Button>}>Takes it off this Mac. Nothing on the server changes.</InsetRow><InsetRow label="Reset local state" action={<Button variant="danger" disabled={false} onClick={onReset}>Reset…</Button>}>Discards the pinned checkpoint, cache and every resumable operation named in the preview.</InsetRow></Inset>
    </div></div>
  </>;
}

function StatusRow({ label, value }: { label: string; value: ReactNode }): ReactNode { return <div className="status-row"><span>{label}</span><b>{value}</b></div>; }

function SheetFrame({ title, subtitle, children, footer, onClose, danger = false }: { title: string; subtitle: string; children: ReactNode; footer: ReactNode; onClose: () => void; danger?: boolean }): ReactNode {
  return <div className="veil" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><Dialog className="sheet wide" role={danger ? 'alertdialog' : 'dialog'} titleId="phase6-sheet-title" onKeyDown={(event) => { if (event.key === 'Escape') onClose(); }}><div className="hd"><span className={`server-mark ${danger ? 'danger' : ''}`}><Icon name={danger ? 'trash' : 'server'}/></span><span className="t"><h2 id="phase6-sheet-title">{title}</h2><small>{subtitle}</small></span></div><div className="sb">{children}</div><div className="ft">{footer}</div></Dialog></div>;
}

function AddServerSheet({ bridge, onClose, onAdded, onError }: { bridge: Bridge; onClose: () => void; onAdded: (profile: string) => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [profile, setProfile] = useState('partner'); const [probe, setProbe] = useState('foks.partner.dev'); const [busy, setBusy] = useState(false);
  const cleanProbe = probe.trim();
  const valid = /^[A-Za-z0-9_-]{1,64}$/.test(profile) && cleanProbe.length > 0 && !cleanProbe.includes('\0') && !cleanProbe.includes('\n') && !cleanProbe.includes('\r') && new TextEncoder().encode(cleanProbe).length <= 2048;
  return <SheetFrame title="Add a server" subtitle="Save its address, then check its identity" onClose={onClose} footer={<><Button onClick={onClose}>Cancel</Button><Button variant="primary" disabled={!valid || busy} onClick={() => { setBusy(true); void bridge.addServer(profile, cleanProbe).then((added) => { if (added.profile !== profile) throw new Error('add_server returned a different profile.'); return onAdded(added.profile); }).catch(onError).finally(() => setBusy(false)); }}>Add server</Button></>}><p>Adding a server saves its profile and address on this Mac. Check it next to verify and pin its signed host identity.</p><Inset><InsetRow label="Profile"><input value={profile} onChange={(event) => setProfile(event.target.value)} /></InsetRow><InsetRow label="Address"><input value={probe} onChange={(event) => setProbe(event.target.value)} /></InsetRow></Inset></SheetFrame>;
}

function ResetSheet({ server, preview, bridge, onClose, onReset, onError }: { server: Server; preview: ResetPreview | null; bridge: Bridge; onClose: () => void; onReset: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false); const [available, setAvailable] = useState(false); const token = useRef<string | null>(null);
  useEffect(() => { token.current = preview?.token ?? null; setAvailable(Boolean(preview?.token)); }, [preview?.token]);
  return <SheetFrame title={`Reset ${server.name}?`} subtitle="Discard only this Mac’s local state for the whole server" onClose={onClose} danger footer={<><Button onClick={onClose}>Cancel</Button><Button variant="danger" disabled={confirmation !== server.id || !preview || !available || busy} onClick={() => { const once = token.current; token.current = null; setAvailable(false); if (!once) return; setBusy(true); void bridge.resetServer(server.id, confirmation, once).then(onReset).catch(onError).finally(() => setBusy(false)); }}>Reset local state</Button></>}><p>This does not delete server data. Check again before using this server.</p><Inset><InsetRow label="Discarded">Pinned host id, signed-history checkpoint and cached server artifacts.</InsetRow><InsetRow label="Lost">Local writes that were never accepted by the server cannot be recovered.</InsetRow><InsetRow label="Kept">Your keys and account remain on this Mac, but this reset does not sign you in.</InsetRow><InsetRow label="Untouched">Every other configured server and its local state.</InsetRow></Inset>{preview ? <><div className="sec">Also discarded · resumable operations</div><Inset>{preview.resumables.length ? preview.resumables.map((row, index) => <InsetRow key={`${row.kind}-${row.alias}-${index}`} label={row.kind}>{row.alias}{row.target ? ` · ${row.target}` : ''}</InsetRow>) : <InsetRow label="None">No resumable operation was reported.</InsetRow>}</Inset><div className="sec">Local artifacts discarded</div><Inset>{preview.artifacts.length ? preview.artifacts.map((row) => <InsetRow key={row.kind} label={row.kind}>{row.entries} entries · {row.bytes.toLocaleString()} bytes</InsetRow>) : <InsetRow label="None">No local artifacts were reported.</InsetRow>}</Inset><p className="hint">This preview token expires in {preview.expiresInSeconds} seconds and can be used once.{available ? '' : ' Preview again by closing and reopening Reset.'}</p></> : <p>Reading the exact reset preview…</p>}<Inset><InsetRow label="Confirm"><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} placeholder={`type ${server.id}`} /></InsetRow></Inset></SheetFrame>;
}

function ForgetSheet({ server, bridge, onClose, onForgot, onError }: { server: Server; bridge: Bridge; onClose: () => void; onForgot: () => Promise<void>; onError: (error: unknown) => void }): ReactNode {
  const [confirmation, setConfirmation] = useState(''); const [busy, setBusy] = useState(false);
  return <SheetFrame title={`Forget ${server.name}?`} subtitle="Removes the profile from this Mac" onClose={onClose} danger footer={<><Button onClick={onClose}>Cancel</Button><Button variant="danger" disabled={confirmation !== server.id || busy} onClick={() => { setBusy(true); void bridge.forgetServer(server.id, confirmation).then((forgotten) => { if (forgotten.profile !== server.id) throw new Error('forget_server returned a different profile.'); return onForgot(); }).catch(onError).finally(() => setBusy(false)); }}>Forget server</Button></>}><p>The server and its ciphertext are unchanged. Type the local profile name to confirm.</p><Inset><InsetRow label="Confirm"><input value={confirmation} onChange={(event) => setConfirmation(event.target.value)} placeholder={`type ${server.id}`} /></InsetRow></Inset></SheetFrame>;
}
