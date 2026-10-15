/** Reset this device: each server's local state, after typed confirmation. */

import { localAliasOf } from '../../model';
import { useCallback, useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { enqueueProfileWork, normalizeCommandError } from '../../bridge';
import type { Bridge, ResetPreview } from '../../bridge';
import {
  Band,
  Button,
  Inset,
  InsetRow,
  SectionLabel,
  SheetDialog,
} from '../../components';
import { useSheetGuard } from '../../navigation-guard';
import { purgeProfiles } from '../../platform/device-storage';
import {
  accountStores,
  plural,
  serverDisplayLabel,
  usernameOf,
} from '../../model';
import type { AgentSnapshot, Server } from '../../model';

/**
 * Reset this device: `describe_reset` and `reset_server` once per server.
 *
 * There is no command that spans profiles, so each server answers with its own
 * one-use token and each profile name is typed. A run that fails part way says
 * how far it got rather than pretending the rest happened.
 *
 * A reset is the way out of local state that no longer works, so one server's
 * preview failing does not hold the others: the run covers every server whose
 * preview answered, says which it left out, and offers each of those its own
 * retry. A preview that could not read this Mac's credentials still answers,
 * saying so, and the reset it authorizes erases what is on disk.
 */
export function ResetMacSheet({
  snapshot,
  bridge,
  onClose,
  onDone,
  onError,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  onClose: () => void;
  onDone: (count: number) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const servers = snapshot.servers;
  const [previews, setPreviews] = useState<Map<string, ResetPreview>>(
    new Map(),
  );
  const [failures, setFailures] = useState<Map<string, string>>(new Map());
  const [typed, setTyped] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);

  const [retrying, setRetrying] = useState<Set<string>>(new Set());

  const preview = useCallback(
    async (server: Server): Promise<ResetPreview> => {
      const preview = await enqueueProfileWork(bridge, server.profileName, () =>
        bridge.describeReset(server.profileName),
      );
      if (preview.profile !== server.profileName)
        throw new Error('describe_reset returned a different profile.');
      return preview;
    },
    [bridge],
  );
  const load = useCallback((): void => {
    setLoading(true);
    setPreviews(new Map());
    setFailures(new Map());
    void (async () => {
      const found = new Map<string, ResetPreview>();
      const failed = new Map<string, string>();
      for (const server of servers) {
        try {
          found.set(server.profileName, await preview(server));
        } catch (error) {
          failed.set(server.profileName, normalizeCommandError(error).message);
        }
      }
      setPreviews(found);
      setFailures(failed);
      setLoading(false);
    })();
  }, [preview, servers]);
  useEffect(load, [load]);
  // One server's preview again, leaving the others' answers and typed names
  // where they are.
  const retry = (server: Server): void => {
    setRetrying((current) => new Set(current).add(server.profileName));
    void (async () => {
      try {
        const answer = await preview(server);
        setPreviews((current) =>
          new Map(current).set(server.profileName, answer),
        );
        setFailures((current) => {
          const next = new Map(current);
          next.delete(server.profileName);
          return next;
        });
      } catch (error) {
        setFailures((current) =>
          new Map(current).set(
            server.profileName,
            normalizeCommandError(error).message,
          ),
        );
      } finally {
        setRetrying((current) => {
          const next = new Set(current);
          next.delete(server.profileName);
          return next;
        });
      }
    })();
  };

  // Reset executes sequentially per server using single-use confirmation
  // tokens. Navigating away during execution would prevent status updates.
  // Typed server names confirm execution and are not persisted.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the reset to finish.' }
      : null,
  );

  // The servers whose preview answered are the run; a server whose preview
  // did not is left out and said so, not a reason to hold the rest.
  const included = servers.filter((server) => previews.has(server.profileName));
  const excluded = servers.filter(
    (server) => !previews.has(server.profileName),
  );
  const ready =
    !loading &&
    retrying.size === 0 &&
    included.length > 0 &&
    included.every(
      (server) => typed[server.profileName] === server.profileName,
    );
  const stores = accountStores(snapshot);

  // Each server may configure a different reset preview token TTL. Display the
  // expiration notice only after all server previews have responded, without
  // assuming a fallback duration.
  const lifetimes = included.map(
    (server) => previews.get(server.profileName)?.expiresInSeconds,
  );
  const answered =
    !loading &&
    included.length > 0 &&
    lifetimes.every((seconds) => seconds !== undefined);
  const agreed =
    answered && new Set(lifetimes).size === 1 ? lifetimes[0] : undefined;
  const expiry = !answered
    ? null
    : agreed !== undefined
      ? `Confirmations expire in ${agreed} seconds.`
      : `Confirmations expire per server: ${included
          .map(
            (server) =>
              `${serverDisplayLabel(server)} (${previews.get(server.profileName)?.expiresInSeconds} seconds)`,
          )
          .join(', ')}.`;

  return (
    <SheetDialog
      width="wide"
      danger
      dismissible={!busy}
      onClose={() => {
        if (busy) return;
        onClose();
      }}
      title="Reset this device?"
      icon="trash"
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            disabled={!ready || busy}
            onClick={() => {
              setBusy(true);
              void (async () => {
                let done = 0;
                try {
                  for (const server of included) {
                    const preview = previews.get(server.profileName);
                    if (!preview) continue;
                    await bridge.resetServer(
                      server.profileName,
                      server.profileName,
                      preview.token,
                    );
                    // The native reset forgets the profile's chat-local
                    // state; this removes the webview's records of it.
                    purgeProfiles([server.profileName]);
                    done += 1;
                  }
                  await onDone(done);
                } catch (error) {
                  onError(error);
                  // Every token is spent or stale once a run has started.
                  load();
                } finally {
                  setBusy(false);
                }
              })();
            }}
          >
            {excluded.length && included.length
              ? `Reset ${included.length} of ${servers.length} servers`
              : 'Reset this device'}
          </Button>
        </>
      }
    >
      <p>
        Deletes this Mac's account keys. Your data stays on the server, but
        without another device or a recovery phrase you cannot get back into the
        account. The passphrase alone is not enough.
      </p>
      <Inset>
        <InsetRow label="Unaffected">
          Remote accounts and all other enrolled devices remain active. Only
          data stored locally on this Mac is erased.
        </InsetRow>
        <InsetRow label="App records">
          For each server reset, this app also erases its setup progress,
          invitation labels, Recent and Favorites entries, and exposure reviews.
        </InsetRow>
        <InsetRow label="Accounts on this Mac">
          {stores.length
            ? stores
                .map(
                  (store) =>
                    `${usernameOf(snapshot, store) ?? store.account} (${localAliasOf(snapshot, store)})`,
                )
                .join(', ')
            : 'None'}
        </InsetRow>
      </Inset>
      {servers.map((server) => {
        const preview = previews.get(server.profileName);
        const failure = failures.get(server.profileName);
        return (
          <div key={server.profileName}>
            <SectionLabel>
              {serverDisplayLabel(server)} · {server.profileName}
            </SectionLabel>
            <Inset>
              {failure ? (
                <InsetRow
                  label="Preview"
                  action={
                    <Button
                      size="sm"
                      disabled={busy || retrying.has(server.profileName)}
                      busy={retrying.has(server.profileName)}
                      onClick={() => retry(server)}
                    >
                      Retry preview
                    </Button>
                  }
                >
                  <span className="danger-title">{failure}</span>{' '}
                  {loading ? null : 'Not included in this reset.'}
                </InsetRow>
              ) : preview ? (
                <>
                  {preview.credentialsUnavailable ? (
                    <InsetRow label="Credentials">
                      <span className="danger-title">
                        {preview.credentialsUnavailable}
                      </span>{' '}
                      The reset erases this server’s local data and leaves the
                      credential records it cannot read.
                    </InsetRow>
                  ) : null}
                  <InsetRow label="Discarded operations">
                    {preview.resumables.length
                      ? preview.resumables
                          .map(
                            (row) =>
                              `${row.kind} · ${row.alias}${row.target ? ` · ${row.target}` : ''}`,
                          )
                          .join(', ')
                      : 'No resumable operations found.'}
                  </InsetRow>
                  <InsetRow label="Data to erase">
                    {preview.artifacts.length
                      ? preview.artifacts
                          .map(
                            (row) =>
                              `${row.kind} · ${row.entries} entries · ${row.bytes.toLocaleString()} bytes`,
                          )
                          .join(', ')
                      : 'No local data to erase.'}
                  </InsetRow>
                </>
              ) : (
                <InsetRow label="Preview">Loading…</InsetRow>
              )}
              <InsetRow label="Confirm">
                <input
                  value={typed[server.profileName] ?? ''}
                  disabled={!preview}
                  onChange={(event) =>
                    setTyped((current) => ({
                      ...current,
                      [server.profileName]: event.target.value,
                    }))
                  }
                  placeholder="Type server profile name to confirm"
                />
              </InsetRow>
            </Inset>
          </div>
        );
      })}
      {servers.length ? null : (
        <Band label="No servers on this Mac">
          There is nothing for this reset to erase.
        </Band>
      )}
      {!loading && excluded.length ? (
        <Band label="Not every server answered">
          {included.length
            ? `${plural(excluded.length, 'server')} could not be previewed and ${excluded.length === 1 ? 'is' : 'are'} not included. Retry ${excluded.length === 1 ? 'its' : 'their'} preview, or reset the rest now and ${excluded.length === 1 ? 'that server' : 'those servers'} later.`
            : 'No server could be previewed. Retry each preview to reset it.'}
        </Band>
      ) : null}
      {expiry ? <p className="hint">{expiry}</p> : null}
    </SheetDialog>
  );
}
