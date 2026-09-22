/** Labels and presentation components for SSO progress states. */

import type { ReactNode } from 'react';
import type { SsoProgress } from '../../sso-contract';
import { Chip } from '../chips';
import type { ChipTone } from '../chips';
import { Icon } from '../icon';
import { Inset, InsetRow } from '../inset';

export const SSO_MESSAGES: Record<SsoProgress['state'], string> = {
  'device-only': 'This host has not enabled organization sign-in.',
  'link-needed':
    'Link your existing account to your organization. Device access remains available during migration.',
  'locked-out':
    'Organization sign-in is enforced. Link this account with its owner device to restore access.',
  linked: 'This account is linked to your organization.',
  'not-eligible': 'This account is not eligible for existing-account linkage.',
  prepared:
    'Authentication was interrupted before the browser was opened. Cancel and begin again.',
  waiting: 'Complete sign-in in your browser, then check for completion.',
  ready:
    'Browser sign-in is verified. Finish to complete this account operation.',
  submitting:
    'The signed operation is being reconciled. Refresh status to check its outcome.',
  complete: 'The signed account operation was accepted.',
  cancelled: 'This browser flow was cancelled.',
  expired: 'This browser flow expired. Begin again.',
  'submission-unknown':
    'The signed request may have been accepted. Its outcome is still unknown. A new sign-in does not retry the interrupted operation.',
  rejected: 'Authentication could not be verified.',
  denied: 'Your identity provider declined sign-in.',
  'provider-unavailable':
    'Your identity provider is unavailable. Check again when it recovers.',
  'reauthentication-required': 'Service access is unavailable. Sign in again.',
  'service-unavailable':
    'The signed operation was accepted, but service access could not be verified. Check your connection and refresh status.',
  'hardware-verification-required':
    'Unlock the enrolled security key to verify service access.',
};

const CHIPS: Record<SsoProgress['state'], [ChipTone, string]> = {
  'device-only': ['default', 'Not enabled'],
  'link-needed': ['warn', 'Not linked'],
  'locked-out': ['bad', 'Locked out'],
  linked: ['ok', 'Linked'],
  'not-eligible': ['default', 'Not eligible'],
  prepared: ['warn', 'Not started'],
  waiting: ['warn', 'Waiting'],
  ready: ['ok', 'Verified'],
  submitting: ['warn', 'Submitting'],
  complete: ['ok', 'Accepted'],
  cancelled: ['default', 'Cancelled'],
  expired: ['default', 'Expired'],
  'submission-unknown': ['bad', 'Unknown'],
  rejected: ['bad', 'Rejected'],
  denied: ['bad', 'Declined'],
  'provider-unavailable': ['bad', 'Provider unavailable'],
  'reauthentication-required': ['warn', 'Sign in again'],
  'service-unavailable': ['warn', 'Accepted'],
  'hardware-verification-required': ['warn', 'Unlock key'],
};

export function ssoMessage(progress: SsoProgress): string {
  return progress.serviceAccess && !progress.accountStatus
    ? 'Account authentication and service access verified.'
    : SSO_MESSAGES[progress.state];
}

export function SsoStateChip({
  progress,
}: {
  progress: SsoProgress;
}): ReactNode {
  const [tone, label] = progress.serviceAccess
    ? (['ok', 'Verified'] as const)
    : CHIPS[progress.state];
  return <Chip tone={tone}>{label}</Chip>;
}

/** Displays an SSO status message with its state chip. */
export function SsoStatus({ progress }: { progress: SsoProgress }): ReactNode {
  const linkage = Boolean(progress.accountStatus);
  return (
    <Inset className="form sso-status">
      <InsetRow
        label={linkage ? 'Linkage' : 'Sign-in'}
        action={<SsoStateChip progress={progress} />}
      >
        <span role="status">{ssoMessage(progress)}</span>
      </InsetRow>
    </Inset>
  );
}

/** Displays the final result of an SSO operation. */
export function SsoOutcome({ progress }: { progress: SsoProgress }): ReactNode {
  const verified = progress.state === 'complete' && progress.serviceAccess;
  return (
    <div className={verified ? 'band ok' : 'band'} role="status">
      <Icon name={verified ? 'circleCheck' : 'alert'} />
      <span className="t">{ssoMessage(progress)}</span>
    </div>
  );
}
