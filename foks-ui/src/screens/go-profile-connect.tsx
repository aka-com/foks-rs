import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type {
  Bridge,
  CheckedProfileResponse,
  GoProfileCandidate,
  GoProfileDiscovery,
} from '../bridge';
import { normalizeCommandError } from '../bridge';
import {
  Button,
  CopyBox,
  Field,
  Inset,
  SectionLabel,
  SheetDialog,
} from '../components';
import { GoProfileChooser } from './go-profile-chooser';

interface Props {
  bridge: Bridge;
  onClose: () => void;
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
  onError,
}: Props): ReactNode {
  const [discovery, setDiscovery] = useState<GoProfileDiscovery | null>(null);
  const [selected, setSelected] = useState<GoProfileCandidate | null>(null);
  const [profileName, setProfileName] = useState('');
  const [server, setServer] = useState('');
  const [alias, setAlias] = useState('personal');
  const [deviceName, setDeviceName] = useState('FOKS Desktop');
  const [phrase, setPhrase] = useState('');
  const [checked, setChecked] = useState<CheckedProfileResponse | null>(null);
  const [method, setMethod] = useState<'pair' | 'copy'>('pair');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const scan = (): void => {
    setBusy(true);
    setError(null);
    void bridge
      .discoverGoProfiles()
      .then(setDiscovery)
      .catch((failure) => {
        setError(normalizeCommandError(failure).message);
        onError(failure);
      })
      .finally(() => setBusy(false));
  };

  useEffect(() => {
    let alive = true;
    setBusy(true);
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
        if (alive) setBusy(false);
      });
    return () => {
      alive = false;
    };
  }, [bridge, onError]);
  useEffect(() => {
    const clear = (): void => setPhrase('');
    const hidden = (): void => {
      if (document.hidden) clear();
    };
    window.addEventListener('blur', clear);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      window.removeEventListener('blur', clear);
      document.removeEventListener('visibilitychange', hidden);
    };
  }, []);

  const choose = (candidate: GoProfileCandidate): void => {
    setSelected(candidate);
    setChecked(null);
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
    setBusy(true);
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
          'The checked server does not match the selected CLI profile.',
        );
      setChecked(result);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      onError(failure);
    } finally {
      setBusy(false);
    }
  };

  const pair = async (resume: boolean): Promise<void> => {
    if (!selected || !checked || !alias.trim() || !deviceName.trim()) return;
    if (!resume && !phrase.trim()) return;
    const submitted = phrase;
    setPhrase('');
    setBusy(true);
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
        throw new Error('Pairing returned a different local account alias.');
      await onConnected(checked.profile, result.alias);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      onError(failure);
    } finally {
      setBusy(false);
    }
  };

  const copy = async (): Promise<void> => {
    if (!selected?.copyable || !checked || !alias.trim()) return;
    setBusy(true);
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
      setBusy(false);
    }
  };

  const candidates =
    discovery?.candidates.filter(
      (candidate) => candidate.pairable || candidate.copyable,
    ) ?? [];

  return (
    <SheetDialog
      onClose={onClose}
      title="Connect from FOKS CLI"
      subtitle="Use an account already configured by the official client"
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          {selected && !checked ? (
            <Button
              variant="primary"
              disabled={busy || !server.trim() || !profileName.trim()}
              onClick={() => void check()}
            >
              Check server
            </Button>
          ) : checked && method === 'pair' ? (
            <>
              <Button
                disabled={busy || !alias.trim() || !deviceName.trim()}
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
                Pair this Mac
              </Button>
            </>
          ) : checked && method === 'copy' ? (
            <Button
              variant="primary"
              disabled={busy || !selected?.copyable || !alias.trim()}
              onClick={() => void copy()}
            >
              Copy existing device
            </Button>
          ) : null}
        </>
      }
    >
      {!discovery && busy ? <p>Looking for the official FOKS client…</p> : null}
      {discovery && !candidates.length ? (
        <Inset>
          <p>
            {discovery.installed
              ? 'No usable CLI profiles were found.'
              : 'The official FOKS client was not found in its standard location.'}
          </p>
          <Button disabled={busy} onClick={scan}>
            Scan again
          </Button>
        </Inset>
      ) : null}
      {candidates.length && !selected ? (
        <>
          <SectionLabel>CLI accounts on this Mac</SectionLabel>
          <GoProfileChooser
            candidates={candidates}
            selected={null}
            onSelect={choose}
          />
        </>
      ) : null}
      {selected && !checked ? (
        <>
          <SectionLabel>Destination server</SectionLabel>
          <Inset>
            <Field label="Server address" value={server} onChange={setServer} />
            <Field
              label="Server name on this Mac"
              value={profileName}
              onChange={setProfileName}
            />
            <Button
              onClick={() => {
                setSelected(null);
                setChecked(null);
              }}
            >
              Choose another account
            </Button>
          </Inset>
        </>
      ) : null}
      {selected && checked ? (
        <>
          <SectionLabel>Connection method</SectionLabel>
          <Inset>
            <Button
              variant={method === 'pair' ? 'primary' : undefined}
              onClick={() => setMethod('pair')}
            >
              Add as a new device
            </Button>
            <p>Recommended. This desktop gets its own revocable device key.</p>
            <Button
              variant={method === 'copy' ? 'primary' : undefined}
              disabled={!selected.copyable}
              onClick={() => setMethod('copy')}
            >
              Copy this Mac’s CLI device
            </Button>
            <p>Advanced. Both applications will act as the same FOKS device.</p>
          </Inset>
          {method === 'pair' ? (
            <>
              <SectionLabel>Pair through the official CLI</SectionLabel>
              <p>
                In Terminal, switch the official client to the selected account
                and run this command. Confirm the account shown there.
              </p>
              <CopyBox
                text="foks --simple-ui key assist"
                onCopy={(text) => void bridge.copyText(text).catch(onError)}
              >
                <code>foks --simple-ui key assist</code>
              </CopyBox>
              <Inset>
                <Field
                  label="Account alias"
                  value={alias}
                  onChange={setAlias}
                />
                <Field
                  label="This Mac’s name"
                  value={deviceName}
                  onChange={setDeviceName}
                />
                <Field
                  label="Key-exchange code"
                  value={phrase}
                  onChange={setPhrase}
                  type="password"
                />
              </Inset>
              <p>
                Leave the CLI command running until pairing completes. If it
                asks for this Mac’s code after the desktop connects, submit an
                empty response so its concurrent pairing wait can finish.
              </p>
            </>
          ) : (
            <>
              <SectionLabel>Copy the existing device</SectionLabel>
              <Inset>
                <Field
                  label="Account alias"
                  value={alias}
                  onChange={setAlias}
                />
              </Inset>
              <p>
                macOS may ask for permission to read the official client’s
                Keychain item. Removing the CLI profile will not remove this
                copy; revoking the device disables both clients, and later CLI
                passphrase changes do not change this desktop copy.
              </p>
            </>
          )}
        </>
      ) : null}
      {error ? <p className="crit">{error}</p> : null}
    </SheetDialog>
  );
}
