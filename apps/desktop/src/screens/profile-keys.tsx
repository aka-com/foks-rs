import { useDeviceQueries, metadataFreshness } from '../device-cache';
import { FreshnessCaption } from '../components/metadata-status';
import { useMetadataQuery } from '../query-hooks';
import { useState } from 'react';
import type { Bridge } from '../bridge';
import { Button, Inset, InsetRow, SectionLabel } from '../components';
import { WorkflowProvider, useWorkflowAccess } from '../workflow-context';
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
  const deviceCache = useDeviceQueries(bridge, snapshot);
  const [action, setAction] = useState<{
    profile: string;
    alias: string;
    kind: SimpleYubiAction;
  } | null>(null);
  const access = useWorkflowAccess(snapshot);
  const available = access.availability('yubi-list', {
    profile: server.id,
  }).available;
  const query = available ? deviceCache.enrollments(server.id) : null;
  const state = useMetadataQuery(query, { onError });
  const entries = state.data ?? [];
  const failed =
    available && state.data === undefined && state.error !== undefined;
  const loading = available && state.data === undefined && !failed;
  const freshness = metadataFreshness([state]);
  return (
    <WorkflowProvider snapshot={snapshot}>
      <section aria-label="Security keys">
        <SectionLabel>Security keys</SectionLabel>
        <Inset className={freshness.stale ? 'stale' : undefined}>
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
                      {...access.props('yubi-resume', {
                        profile: server.id,
                        account: entry.alias,
                      })}
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
                        {...access.props('yubi-pin', {
                          profile: server.id,
                          account: entry.alias,
                        })}
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
                        {...access.props('yubi-pin', {
                          profile: server.id,
                          account: entry.alias,
                        })}
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
                        {...access.props('yubi-pin', {
                          profile: server.id,
                          account: entry.alias,
                        })}
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
        <FreshnessCaption
          label="security key metadata"
          freshness={freshness}
          onRetry={
            query ? () => void query.load().catch(() => undefined) : undefined
          }
        />
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
    </WorkflowProvider>
  );
}
