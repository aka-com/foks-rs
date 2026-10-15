import { useEffect, useState } from 'react';
import type { Bridge } from '../../bridge';
import { fixDeviceName } from '../../device-name';
import type { FirstRunCheckpoint } from '../../first-run-state';
import { normalizeUsername } from '../../name-normalization';

export function accountAliasFor(username: string): string {
  return (normalizeUsername(username.trim()) ?? username)
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+/, '')
    .replace(/-+$/, '')
    .slice(0, 64);
}

/** Suggest a fallback device name based on platform user agent. */
function suggestedDeviceName(): string {
  if (typeof navigator === 'undefined') return 'Mac';
  const source = `${navigator.platform} ${navigator.userAgent}`;
  if (/iPhone/i.test(source)) return 'iPhone';
  if (/iPad/i.test(source)) return 'iPad';
  if (/Mac/i.test(source)) return 'Mac';
  return 'This computer';
}

/**
 * The nonsecret account fields typed during setup: the username, this
 * device's name and the optional email, and the account alias derived from
 * them. Empty fields start from the checkpoint, the review fixture, or the
 * names the host reports for this computer and its user.
 */
export function useSetupIdentity({
  bridge,
  checkpoint,
}: {
  bridge: Bridge;
  checkpoint: FirstRunCheckpoint;
}) {
  const facts = bridge.firstRunFixture?.[checkpoint.path];
  const [username, setUsername] = useState(
    () => checkpoint.account?.username ?? facts?.username ?? '',
  );
  const [deviceName, setDeviceName] = useState(
    () =>
      checkpoint.account?.deviceName ??
      fixDeviceName(facts?.deviceName ?? suggestedDeviceName()),
  );
  const [email, setEmail] = useState('');

  useEffect(() => {
    const hasUserName = Boolean(
      checkpoint.account?.username || facts?.username,
    );
    const hasDeviceName = Boolean(
      checkpoint.account?.deviceName || facts?.deviceName,
    );
    if (hasUserName && hasDeviceName) return;
    const fallback = suggestedDeviceName();
    let alive = true;
    void bridge
      .appInfo()
      .then((info) => {
        if (!alive) return;
        const userName = info.userName?.trim();
        const computerName = info.computerName?.trim();
        if (!hasUserName && userName)
          setUsername((current) => current || userName);
        if (!hasDeviceName && computerName)
          setDeviceName((current) =>
            current === fallback ? fixDeviceName(computerName) : current,
          );
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [
    bridge,
    checkpoint.account?.deviceName,
    checkpoint.account?.username,
    facts?.deviceName,
    facts?.username,
  ]);

  const accountAlias =
    checkpoint.account?.alias ??
    checkpoint.sso?.alias ??
    facts?.accountAlias ??
    accountAliasFor(username);
  const aliasInvalid = username.trim().length > 0 && !accountAlias;

  return {
    username,
    setUsername,
    deviceName,
    setDeviceName,
    email,
    setEmail,
    accountAlias,
    aliasInvalid,
  };
}

export type SetupIdentity = ReturnType<typeof useSetupIdentity>;
