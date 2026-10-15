/**
 * The type word and the phrase-test chip that the Devices list and a key's
 * own page share.
 */

import type { ReactNode } from 'react';
import { Chip } from '../../components';
import type { DeviceEntry } from '../../data/device-model';
import { testedDate } from '../../recovery-phrase-tests';
import type { RecoveryPhraseTestStatus } from '../../recovery-phrase-tests';

/** The chip a recovery phrase's row and page carry: when it last matched here. */
export function TestedChip({
  status,
}: {
  status: RecoveryPhraseTestStatus;
}): ReactNode {
  if (status.state === 'never')
    return (
      <Chip
        tone="warn"
        title="No written copy of this phrase has been tested on this device."
      >
        Last tested: never
      </Chip>
    );
  return (
    <Chip tone={status.state === 'current' ? 'ok' : 'warn'}>
      {status.state === 'current' ? 'Tested' : 'Last tested'}{' '}
      {testedDate(status.testedAt)}
    </Chip>
  );
}

/**
 * The word a row's caption, and a key's own "Kind" fact, name its type with,
 * on this page only. The shared model still calls a card enrollment an
 * "Enrollment" object — People reads that literal value unchanged — but
 * Devices calls the same object a security key everywhere else in its copy.
 */
export function kindLabel(kind: DeviceEntry['kind']): string {
  return kind === 'Enrollment' ? 'Security key' : kind;
}
