/** One key's page under Devices: its facts, its id and its actions. */

import { useWorkflowAccess } from '../../workflow-context';
import type { ReactNode } from 'react';
import type {
  AccountDevice,
  BackupEnrollment,
  YubiEnrollment,
} from '../../bridge';
import {
  Button,
  Chip,
  CopyBox,
  Inset,
  InsetRow,
  Notice,
  SectionLabel,
} from '../../components';
import type { AccountStore, AgentSnapshot } from '../../model';
import {
  accountSubtitle,
  serverDisplayLabelForStore,
  usernameOf,
} from '../../model';
import type { Location } from '../../location';
import { PageHeader } from '../../shell/page-header';
import type { SimpleYubiAction } from './credential-actions';
import type { DeviceEntry } from '../../data/device-model';
import type { RecoveryPhraseTestStatus } from '../../recovery-phrase-tests';
import { kindLabel, TestedChip } from './device-labels';
import { CardOperations } from './card-operations';

/**
 * One key's page: every fact the agent reports about it, the full id with a
 * way to copy it, and the one destructive action its kind has.
 */
export function DeviceDetail({
  snapshot,
  store,
  entry,
  deviceEnrollment,
  loading,
  failed,
  stopped,
  cards,
  metadataStatus,
  stale,
  refreshHardwareKeys,
  onBack,
  onNavigate,
  onCopy,
  onRemove,
  onRevokeBackup,
  testStatus,
  onTestBackup,
  onRevokeKey,
  onProvision,
  onYubiAction,
}: {
  snapshot: AgentSnapshot;
  store: AccountStore;
  /** The key the address names, once the lists have answered. */
  entry?: DeviceEntry;
  deviceEnrollment?: YubiEnrollment;
  loading: boolean;
  /** The lists never answered, so whether the key is listed is unknown. */
  failed: boolean;
  stopped: { stopped: boolean; reason: string };
  /** The cards in this Mac's ports right now. */
  cards: { serial: number }[];
  metadataStatus: ReactNode;
  /** The lists this page reads could not be refreshed. */
  stale: boolean;
  refreshHardwareKeys: ReactNode;
  onBack: () => void;
  onNavigate: (location: Location) => void;
  onCopy: (text: string) => void;
  onRemove: (device: AccountDevice) => void;
  onRevokeBackup: (backup: BackupEnrollment) => void;
  testStatus: (backup: BackupEnrollment) => RecoveryPhraseTestStatus;
  onTestBackup: () => void;
  onRevokeKey: (entry: YubiEnrollment) => void;
  onProvision: () => void;
  onYubiAction: (action: SimpleYubiAction, entry: YubiEnrollment) => void;
}): ReactNode {
  const access = useWorkflowAccess(snapshot);
  const target = { profile: store.server, account: store.account };
  if (!entry)
    return (
      <>
        <PageHeader
          ruled
          title="Devices"
          subtitle={accountSubtitle(snapshot, store)}
        />
        <div className="body">
          <div className="settings-main">
            <Notice
              severity={
                loading || stopped.stopped ? 'info' : failed ? 'warn' : 'crit'
              }
              title={
                stopped.stopped
                  ? 'Not listed while access is stopped'
                  : loading
                    ? 'Reading this account’s keys…'
                    : failed
                      ? 'Devices and keys could not be read'
                      : 'This key is not on this account'
              }
              actions={
                <>
                  <Button variant="primary" onClick={onBack}>
                    Back to Devices
                  </Button>
                  {refreshHardwareKeys}
                </>
              }
            >
              <p>
                {stopped.stopped
                  ? stopped.reason
                  : loading
                    ? 'Loading devices, recovery phrases, and security keys…'
                    : failed
                      ? 'Whether this key is still on this account is unknown until its devices, recovery phrases and security keys are read again.'
                      : 'This key is no longer associated with this account on this device.'}
              </p>
            </Notice>
            {metadataStatus}
          </div>
        </div>
      </>
    );

  const source = entry.source;
  const username = usernameOf(snapshot, store) ?? store.account;
  const enrollment = source.kind === 'yubi' ? source.entry : undefined;
  const revokeReason =
    access.props('yubi-revoke', target).title ??
    (enrollment && enrollment.state !== 'complete'
      ? 'This enrollment is not complete'
      : undefined);
  return (
    <>
      <PageHeader
        ruled
        title={entry.name}
        // An enrollment is the server's, not this account's, and the caption
        // says so rather than naming an account the agent did not answer for.
        subtitle={
          entry.scope === 'profile'
            ? `${kindLabel(entry.kind)} · on ${serverDisplayLabelForStore(snapshot, store)}`
            : `${kindLabel(entry.kind)} · ${accountSubtitle(snapshot, store)}`
        }
        // The chip states what this key is, so it reads beside the name
        // rather than among the header's actions.
        badge={
          entry.current ? (
            <Chip tone="you">
              {entry.kind === 'Key on a card'
                ? 'Current key on a card'
                : 'This device'}
            </Chip>
          ) : enrollment ? (
            <Chip tone={enrollment.state === 'complete' ? 'ok' : 'warn'}>
              {enrollment.state === 'complete' ? 'Enrolled' : 'Incomplete'}
            </Chip>
          ) : null
        }
        action={refreshHardwareKeys}
      />
      <div className="body">
        <div className="settings-main">
          <div
            className="settings-section"
            role="region"
            aria-labelledby="device-facts-label"
          >
            <SectionLabel id="device-facts-label">This key</SectionLabel>
            <Inset
              className={`settings-inset middle wide${stale ? ' stale' : ''}`}
            >
              {entry.scope === 'profile' ? (
                <InsetRow label="Server">
                  <b>{serverDisplayLabelForStore(snapshot, store)}</b>
                  <small>This enrollment applies across the server.</small>
                </InsetRow>
              ) : (
                <InsetRow label="Account">
                  <span className="namechip">
                    <b>{username}</b> <Chip>{store.account}</Chip>
                  </span>
                  <small>
                    on {serverDisplayLabelForStore(snapshot, store)}
                  </small>
                </InsetRow>
              )}
              {entry.role ? (
                <InsetRow label="Role on the account">{entry.role}</InsetRow>
              ) : null}
              <InsetRow label="Kind">{kindLabel(entry.kind)}</InsetRow>
              <InsetRow label={entry.keyLabel}>
                {entry.keyId ? (
                  <CopyBox text={entry.keyId} onCopy={onCopy}>
                    <span className="mono">{entry.keyId}</span>
                  </CopyBox>
                ) : (
                  <>
                    {entry.name}
                    <small>
                      {enrollment?.cardSerial
                        ? `Card serial ${enrollment.cardSerial}; no device key recorded.`
                        : 'This enrollment cannot be matched to a card on this device.'}
                    </small>
                  </>
                )}
              </InsetRow>
              {entry.current ? (
                <InsetRow label="Status">
                  Authenticated on this device now
                </InsetRow>
              ) : null}
              {source.kind === 'backup' ? (
                <InsetRow
                  label="Last tested"
                  action={
                    <Button
                      size="sm"
                      {...access.props('backup-list', target)}
                      onClick={onTestBackup}
                    >
                      Test…
                    </Button>
                  }
                >
                  <TestedChip status={testStatus(source.backup)} />
                  <small>
                    Type the written copy back in to check that it can recover
                    this account. The check runs on this device.
                  </small>
                </InsetRow>
              ) : null}
            </Inset>
            {metadataStatus}
          </div>
          {enrollment ? (
            <CardOperations
              enrollment={enrollment}
              cards={cards}
              stopped={stopped}
              store={store}
              onNavigate={onNavigate}
              onProvision={onProvision}
              onYubiAction={onYubiAction}
            />
          ) : null}
          <div
            className="settings-section"
            role="region"
            aria-labelledby="device-danger-label"
          >
            <SectionLabel className="danger-title" id="device-danger-label">
              Danger zone
            </SectionLabel>
            <Inset className="settings-inset middle wide danger-box">
              {/* A key on a card is the card's, whether or not this Mac is
                  authenticated with it now: it is not removed the way another
                  Mac is, so its kind decides this before its currency does. */}
              {source.kind === 'device' && entry.kind === 'Key on a card' ? (
                <InsetRow
                  className="dangerrow"
                  label="Revoked under its enrollment"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({
                          kind: 'devices',
                          section: 'keys',
                          store: store.id,
                          ...(deviceEnrollment
                            ? { device: `yubi:${deviceEnrollment.alias}` }
                            : {}),
                        })
                      }
                    >
                      {deviceEnrollment
                        ? `Open ${deviceEnrollment.alias}`
                        : 'Go to Devices'}
                    </Button>
                  }
                >
                  <small>
                    A key on a card is revoked under its enrollment, listed on
                    Devices.
                  </small>
                </InsetRow>
              ) : entry.current ? (
                <InsetRow
                  className="dangerrow"
                  label="This device cannot be removed here"
                  action={
                    <Button
                      size="sm"
                      onClick={() =>
                        onNavigate({ kind: 'settings', section: 'mac' })
                      }
                    >
                      Open Settings
                    </Button>
                  }
                >
                  <small>
                    To remove this device, use another enrolled device. Or, use
                    the Reset this device option in Settings to erase local data
                    and delete credentials across servers.
                  </small>
                </InsetRow>
              ) : source.kind === 'device' ? (
                <InsetRow
                  className="dangerrow"
                  label="Remove this device"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      {...access.props('device-remove', target)}
                      onClick={() => onRemove(source.device)}
                    >
                      Remove this device…
                    </Button>
                  }
                >
                  <small>
                    {entry.name} loses access to every vault, team and chat on
                    this account. Data already downloaded to it stays there.
                  </small>
                </InsetRow>
              ) : source.kind === 'backup' ? (
                <InsetRow
                  className="dangerrow"
                  label="Revoke this recovery phrase"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      {...access.props('backup-revoke', target)}
                      onClick={() => onRevokeBackup(source.backup)}
                    >
                      Revoke…
                    </Button>
                  }
                >
                  <small>
                    The written-down phrase stops working, and revocation
                    rotates every account key it could read.
                  </small>
                </InsetRow>
              ) : (
                <InsetRow
                  className="dangerrow"
                  label="Revoke this enrollment"
                  action={
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={revokeReason !== undefined}
                      title={revokeReason}
                      onClick={() => onRevokeKey(source.entry)}
                    >
                      Revoke…
                    </Button>
                  }
                >
                  <small>
                    Disconnects the key this enrollment was made on and updates
                    account security.
                  </small>
                </InsetRow>
              )}
            </Inset>
          </div>
        </div>
      </div>
    </>
  );
}
