/** A server's status chip, status mark and attention band. */

import type { ReactNode } from 'react';
import { Band, Button, Chip, Icon } from '../../../components';
import { expires, isLocked, markTone, STATE_LABEL } from './server-model';
import type { ServerUiState } from './server-model';

export function StatusChip({ state }: { state: ServerUiState }): ReactNode {
  // A server that answered is the ordinary case, and the row's mark already
  // reads as one: only a state worth acting on carries a chip.
  if (state === 'unprobed' || state === 'checked') return null;
  return (
    <Chip tone={state === 'pending' ? 'default' : 'bad'}>
      {STATE_LABEL[state]}
    </Chip>
  );
}

/** Status indicator icon for a server. */
export function ServerMark({ state }: { state: ServerUiState }): ReactNode {
  const locked = isLocked(state);
  return (
    <span className={['smark', markTone(state)].filter(Boolean).join(' ')}>
      <Icon name={locked ? 'alert' : 'server'} />
    </span>
  );
}

/**
 * Displays an alert banner and primary action for servers requiring attention.
 */
export function StatusBand({
  state,
  expiry,
  busy,
  onCheck,
  onRemove,
}: {
  state: ServerUiState;
  expiry: number | null;
  busy: boolean;
  onCheck: () => void;
  onRemove: () => void;
}): ReactNode {
  if (state === 'unprobed')
    return (
      <Band
        severity="info"
        label="Not checked yet"
        action={
          <Button
            size="sm"
            variant="primary"
            icon="refresh"
            disabled={busy}
            onClick={onCheck}
          >
            Check now
          </Button>
        }
      >
        The address is saved. Check the server to verify its certificate and
        connection.
      </Band>
    );
  if (state === 'lapsed')
    return (
      <Band
        severity="crit"
        label="Check-in expired"
        action={
          <Button
            variant="plain"
            size="sm"
            icon="refresh"
            disabled={busy}
            onClick={onCheck}
          >
            Check now
          </Button>
        }
      >
        Signed compatibility verification expired {expires(expiry)}. A newer
        signed result is required; checking the server identity does not renew
        it.
      </Band>
    );
  if (state === 'recovery-required')
    return (
      <Band severity="crit" label="Saved security state is missing">
        Restore or inspect the saved client security state before reconnecting.
        FOKS will not establish a replacement identity automatically.
      </Band>
    );
  if (state === 'unavailable')
    return (
      <Band severity="crit" label="Check-in status unknown">
        Cannot verify the status of this server. This server is locked until a
        valid status is confirmed. Use the Check button in the header to check
        status.
      </Band>
    );
  if (state === 'blocked')
    return (
      <Band
        severity="crit"
        label="Server identity mismatch"
        action={
          <Button size="sm" variant="danger" onClick={onRemove}>
            Remove…
          </Button>
        }
      >
        The server certificate or security history does not match the pinned
        identity on this device. Access has been blocked for your security.
      </Band>
    );
  if (state === 'incompatible')
    return (
      <Band severity="crit" label="Protocol incompatible">
        Compatibility verification does not permit operations with this server.
        This is not an identity mismatch; resetting trust will not resolve it.
      </Band>
    );
  if (state === 'schema')
    return (
      <Band severity="crit" label="Server schema incompatible">
        This client cannot safely read the server’s saved schema. Review the
        supported versions; FOKS will not reset this data automatically.
      </Band>
    );
  if (state === 'import-verification')
    return (
      <Band severity="crit" label="Imported profile needs verification">
        Verify this imported profile online before accessing its vaults. Use the
        Check button in the header to verify the profile.
      </Band>
    );
  return null;
}
