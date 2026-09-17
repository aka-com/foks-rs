import { useDeviceCache } from '../device-cache';
import { useCallback, useEffect, useState } from 'react';
import type { Bridge, YubiEnrollment } from '../bridge';
import { enqueueProfileWork } from '../bridge';
import { Button, Inset, InsetRow, SectionLabel } from '../components';
import { serverAvailability } from '../model';
import type { AgentSnapshot, Server } from '../model';
import { YubiActionSheet } from './device-sheets';
import type { SimpleYubiAction } from './device-sheets';

export function ProfileKeys({
  snapshot,
  server,
  bridge,
  onError,
}: {
  snapshot: AgentSnapshot;
  server: Server;
  bridge: Bridge;
  onError: (error: unknown) => void;
}) {
  const deviceCache = useDeviceCache();
  const [entries, setEntries] = useState<YubiEnrollment[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [action, setAction] = useState<{
    alias: string;
    kind: SimpleYubiAction;
  } | null>(null);
  const available = serverAvailability(snapshot, server).available;
  const load = useCallback(async () => {
    return enqueueProfileWork(bridge, server.id, () =>
      bridge.listYubiAccounts(server.id),
    );
  }, [bridge, server.id]);
  useEffect(() => {
    let active = true;
    setEntries([]);
    setFailed(false);
    setLoading(available);
    if (available)
      void load()
        .then((rows) => {
          if (active) setEntries(rows);
        })
        .catch((error: unknown) => {
          if (active) {
            setFailed(true);
            onError(error);
          }
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    return () => {
      active = false;
    };
  }, [available, load, onError]);
  useEffect(() => {
    const conceal = () => setAction(null);
    const hidden = () => {
      if (document.hidden) conceal();
    };
    window.addEventListener('blur', conceal);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      window.removeEventListener('blur', conceal);
      document.removeEventListener('visibilitychange', hidden);
    };
  }, []);
  return (
    <section aria-label="Security key enrollments">
      <SectionLabel>Security key enrollments</SectionLabel>

      <Inset>
        {!available ? (
          <InsetRow label="Unavailable">
            Restore access to this server to manage security keys.
          </InsetRow>
        ) : loading ? (
          <InsetRow label="Loading">Reading enrollments…</InsetRow>
        ) : failed ? (
          <InsetRow label="Unavailable">
            Enrollments could not be read.
          </InsetRow>
        ) : !entries.length ? (
          <InsetRow label="None">
            No security key enrollments on this server.
          </InsetRow>
        ) : (
          entries.map((entry) => (
            <InsetRow
              key={`${entry.alias}:${entry.state}`}
              label={entry.alias}
              action={
                entry.state === 'pending' ? (
                  <Button
                    onClick={() =>
                      setAction({
                        alias: entry.alias,
                        kind: 'resume-enrollment',
                      })
                    }
                  >
                    Resume enrollment…
                  </Button>
                ) : (
                  <>
                    <Button
                      onClick={() =>
                        setAction({ alias: entry.alias, kind: 'change-pin' })
                      }
                    >
                      Change PIN…
                    </Button>
                    <Button
                      onClick={() =>
                        setAction({ alias: entry.alias, kind: 'unblock' })
                      }
                    >
                      Unblock PIN…
                    </Button>
                    <Button
                      onClick={() =>
                        setAction({ alias: entry.alias, kind: 'change-puk' })
                      }
                    >
                      Change unlock code…
                    </Button>
                  </>
                )
              }
            >
              {entry.state === 'complete' ? 'Enrolled' : 'Incomplete'}
            </InsetRow>
          ))
        )}
      </Inset>
      {action && (
        <YubiActionSheet
          bridge={bridge}
          profile={server.id}
          alias={action.alias}
          action={action.kind}
          onClose={() => setAction(null)}
          onError={onError}
          onDone={async () => {
            deviceCache?.clear();
            setAction(null);
            setEntries(await load());
          }}
        />
      )}
    </section>
  );
}
