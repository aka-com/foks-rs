import { useDeviceQueries, metadataFreshness } from '../device-cache';
import { MetadataStatus } from '../components/metadata-status';
import { useMetadataQuery } from '../query-hooks';
import { useEffect, useState } from 'react';
import type { Bridge } from '../bridge';
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
  const deviceCache = useDeviceQueries(bridge);
  const [action, setAction] = useState<{
    profile: string;
    alias: string;
    kind: SimpleYubiAction;
  } | null>(null);
  const available = serverAvailability(snapshot, server).available;
  const query = available ? deviceCache.enrollments(server.id) : null;
  const state = useMetadataQuery(query, { onError });
  const entries = state.data ?? [];
  const failed =
    available && state.data === undefined && state.error !== undefined;
  const loading = available && state.data === undefined && !failed;
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
    <section aria-label="Security keys">
      <SectionLabel>Security keys</SectionLabel>
      <MetadataStatus
        label="Security key metadata"
        freshness={metadataFreshness([state])}
      />

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
                        profile: server.id,
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
                        setAction({
                          profile: server.id,
                          alias: entry.alias,
                          kind: 'change-pin',
                        })
                      }
                    >
                      Change PIN…
                    </Button>
                    <Button
                      onClick={() =>
                        setAction({
                          profile: server.id,
                          alias: entry.alias,
                          kind: 'unblock',
                        })
                      }
                    >
                      Unblock PIN…
                    </Button>
                    <Button
                      onClick={() =>
                        setAction({
                          profile: server.id,
                          alias: entry.alias,
                          kind: 'change-puk',
                        })
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
      {action && available && action.profile === server.id && (
        <YubiActionSheet
          bridge={bridge}
          profile={server.id}
          alias={action.alias}
          action={action.kind}
          onClose={() => setAction(null)}
          onError={onError}
          onDone={async () => {
            setAction(null);
            deviceCache.invalidateEnrollments(server.id);
            deviceCache.repository.invalidate(['account-devices', server.id]);
            // Read errors are reported by the subscription after the action has
            // already succeeded, and never turn it into a failed write.
            await query?.load().catch(() => undefined);
          }}
        />
      )}
    </section>
  );
}
