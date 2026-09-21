import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge, GoProfileCandidate, GoProfileDiscovery } from '../bridge';
import { normalizeCommandError } from '../bridge';
import {
  Button,
  CopyBox,
  Field,
  Inset,
  SectionLabel,
  SheetDialog,
} from '../components';
import { useSheetGuard } from '../navigation-guard';
import type { Server } from '../model';
import { GoProfileChooser } from './go-profile-chooser';

interface Props {
  bridge: Bridge;
  onClose: () => void;
  existingProfile?: Pick<Server, 'id' | 'host_id'>;
  onAdded?: (profile: string) => Promise<void>;
  onConnected: (profile: string, alias: string) => Promise<void>;
  onError: (error: unknown) => void;
}

const localName = (value: string, fallback: string): string => {
  const normalized = value
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return normalized || fallback;
};

export function GoProfileConnectSheet({
  bridge,
  onClose,
  onConnected,
  existingProfile,
  onAdded,
  onError,
}: Props): ReactNode {
  const [discovery, setDiscovery] = useState<GoProfileDiscovery | null>(null);
  const [selected, setSelected] = useState<GoProfileCandidate | null>(null);
  const [profileName, setProfileName] = useState('');
  const [server, setServer] = useState('');
  // Suggestions, never values. Picking a candidate fills the alias in from
  // what was discovered; until then the field is the reader's to write.
  const [alias, setAlias] = useState('');
  const [deviceName, setDeviceName] = useState('');
  const [phrase, setPhrase] = useState('');
  const [checked, setChecked] = useState<{ profile: string } | null>(null);
  const [method, setMethod] = useState<'pair' | 'copy'>('pair');
  // Differentiate local scanning from network onboarding operations so the UI
  // can report specific progress.
  const [work, setWork] = useState<'scan' | 'add' | 'pair' | 'copy' | null>(
    null,
  );
  const busy = work !== null;
  const [error, setError] = useState<string | null>(null);

  const scan = (): void => {
    setWork('scan');
    setError(null);
    void bridge
      .discoverGoProfiles()
      .then(setDiscovery)
      .catch((failure) => {
        setError(normalizeCommandError(failure).message);
        onError(failure);
      })
      .finally(() => setWork(null));
  };

  useEffect(() => {
    let alive = true;
    setWork('scan');
    setError(null);
    void bridge
      .discoverGoProfiles()
      .then((result) => {
        if (alive) setDiscovery(result);
      })
      .catch((failure) => {
        if (!alive) return;
        setError(normalizeCommandError(failure).message);
        onError(failure);
      })
      .finally(() => {
        if (alive) setWork(null);
      });
    return () => {
      alive = false;
    };
  }, [bridge, onError]);
  const choose = (candidate: GoProfileCandidate): void => {
    setSelected(candidate);
    setChecked(existingProfile ? { profile: existingProfile.id } : null);
    setMethod('pair');
    setPhrase('');
    setServer(candidate.serverHint ?? '');
    setProfileName(
      localName(
        candidate.username ?? `go-${candidate.hostId.slice(0, 8)}`,
        'go-foks',
      ),
    );
    setAlias(localName(candidate.username ?? 'personal', 'personal'));
  };

  const check = async (): Promise<void> => {
    if (!selected || !server.trim() || !profileName.trim()) return;
    setWork('add');
    setError(null);
    try {
      const result = await bridge.checkAndAddGoProfile(
        selected.candidateId,
        selected.hostId,
        profileName.trim(),
        server.trim(),
      );
      if (result.hostId !== selected.hostId)
        throw new Error(
          'Server verification failed: the server does not match the selected CLI account.',
        );
      setChecked(result);
      await onAdded?.(result.profile);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      onError(failure);
    } finally {
      setWork(null);
    }
  };

  const pair = async (resume: boolean): Promise<void> => {
    if (!selected || !checked || !alias.trim()) return;
    if (!resume && (!deviceName.trim() || !phrase.trim())) return;
    const submitted = phrase;
    setPhrase('');
    setWork('pair');
    setError(null);
    try {
      const result = resume
        ? await bridge.resumeGoProfilePairing(
            selected.candidateId,
            checked.profile,
            alias.trim(),
          )
        : await bridge.acceptGoProfilePairing(
            selected.candidateId,
            checked.profile,
            alias.trim(),
            deviceName.trim(),
            submitted,
          );
      if (result.alias !== alias.trim())
        throw new Error(
          'Pairing failed: the server returned an unexpected account alias.',
        );
      await onConnected(checked.profile, result.alias);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      onError(failure);
    } finally {
      setWork(null);
    }
  };

  const copy = async (): Promise<void> => {
    if (!selected?.copyable || !checked || !alias.trim()) return;
    setWork('copy');
    setError(null);
    try {
      const result = await bridge.copyGoProfileDevice(
        selected.candidateId,
        checked.profile,
        alias.trim(),
      );
      if (result.alias !== alias.trim())
        throw new Error(
          'Device copy returned a different local account alias.',
        );
      await onConnected(checked.profile, result.alias);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      onError(failure);
    } finally {
      setWork(null);
    }
  };

  // Server checks add a local profile; pairing and credential copying add an
  // account. If pairing has not started, prompt before dismissing typed pairing
  // input.
  useSheetGuard(
    work === 'copy'
      ? {
          verdict: 'refuse',
          reason: 'Wait for the credential import to finish.',
        }
      : phrase
        ? {
            verdict: 'prompt',
            title: 'Discard pairing code?',
            body: 'The pairing code typed here has not been submitted.',
            confirm: 'Discard',
            onConfirm: () => {
              setPhrase('');
              onClose();
            },
          }
        : null,
  );

  const candidates =
    discovery?.candidates.filter(
      (candidate) =>
        (candidate.pairable || candidate.copyable) &&
        (!existingProfile || candidate.hostId === existingProfile.host_id),
    ) ?? [];

  return (
    <SheetDialog
      onClose={onClose}
      dismissible={!busy}
      title="Import from FOKS CLI"
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          {selected && !checked ? (
            <Button
              variant="primary"
              disabled={busy || !server.trim() || !profileName.trim()}
              onClick={() => void check()}
            >
              Add server
            </Button>
          ) : checked && method === 'pair' ? (
            <>
              <Button
                disabled={busy || !alias.trim()}
                onClick={() => void pair(true)}
              >
                Resume pairing
              </Button>
              <Button
                variant="primary"
                disabled={
                  busy || !alias.trim() || !deviceName.trim() || !phrase.trim()
                }
                onClick={() => void pair(false)}
              >
                Pair this device
              </Button>
            </>
          ) : checked && method === 'copy' ? (
            <Button
              variant="primary"
              disabled={busy || !selected?.copyable || !alias.trim()}
              onClick={() => void copy()}
            >
              Import credentials
            </Button>
          ) : null}
        </>
      }
    >
      {!discovery && busy ? (
        <p>Searching for existing FOKS CLI accounts…</p>
      ) : null}
      {discovery && !candidates.length ? (
        <Inset>
          <p>
            {discovery.installed
              ? 'No usable CLI profiles were found.'
              : 'The official FOKS CLI was not found on this computer.'}
          </p>
          <Button disabled={busy} onClick={scan}>
            Scan again
          </Button>
        </Inset>
      ) : null}
      {candidates.length && !selected ? (
        <GoProfileChooser
          candidates={candidates}
          selected={null}
          onSelect={choose}
        />
      ) : null}
      {selected && !checked ? (
        <>
          <SectionLabel>Add the server before pairing</SectionLabel>
          <Inset className="cli-server-fields">
            <Field
              disabled={busy}
              label="Server address"
              value={server}
              onChange={setServer}
              placeholder="foks.app:4430"
            />
            <Field
              disabled={busy}
              label="Profile name"
              value={profileName}
              onChange={setProfileName}
            />
          </Inset>
          <div className="cli-server-actions">
            <Button
              size="sm"
              disabled={busy}
              onClick={() => setServer('foks.app:4430')}
            >
              Use official FOKS server
            </Button>
            <Button
              size="sm"
              disabled={busy}
              onClick={() => {
                setSelected(null);
                setChecked(null);
              }}
            >
              Choose another account
            </Button>
          </div>
        </>
      ) : null}
      {selected && checked ? (
        <>
          <SectionLabel>Connection method</SectionLabel>
          <Inset>
            <Button
              disabled={busy}
              variant={method === 'pair' ? 'primary' : undefined}
              onClick={() => setMethod('pair')}
            >
              Add as a new device
            </Button>
            <p>Connect as an authorized device</p>
            <Button
              variant={method === 'copy' ? 'primary' : undefined}
              disabled={busy || !selected.copyable}
              onClick={() => setMethod('copy')}
            >
              Import this device’s FOKS CLI credentials
            </Button>
            <p>Share existing credentials with the FOKS CLI</p>
          </Inset>
          {method === 'pair' ? (
            <>
              <SectionLabel>Pair through the official CLI</SectionLabel>
              <p>
                In Terminal, switch the FOKS CLI to the selected account and run
                this command. Confirm the account when prompted.
              </p>
              <CopyBox
                text="foks --simple-ui key assist"
                onCopy={(text) => void bridge.copyText(text).catch(onError)}
              >
                <code>foks --simple-ui key assist</code>
              </CopyBox>
              <Inset>
                <Field
                  disabled={busy}
                  label="Account name"
                  value={alias}
                  placeholder="personal"
                  onChange={setAlias}
                />
                <Field
                  disabled={busy}
                  label="Device name"
                  value={deviceName}
                  placeholder="FOKS Desktop"
                  onChange={setDeviceName}
                />
                <Field
                  disabled={busy}
                  label="Pairing code"
                  value={phrase}
                  onChange={setPhrase}
                  type="password"
                />
              </Inset>
              <p>
                Leave the command running until pairing completes. If the
                terminal asks for a confirmation code, press Enter.
              </p>
            </>
          ) : (
            <>
              <SectionLabel>Copy the existing device</SectionLabel>
              <Inset>
                <Field
                  disabled={busy}
                  label="Account name"
                  value={alias}
                  onChange={setAlias}
                />
              </Inset>
              <p className="notice">
                This app shares the CLI's device. Deleting the account in the
                CLI disconnects it here too.
              </p>
            </>
          )}
        </>
      ) : null}
      {error ? <p className="crit">{error}</p> : null}
    </SheetDialog>
  );
}
