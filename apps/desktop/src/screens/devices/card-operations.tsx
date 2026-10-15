/** The card operations on a security key's own page, grouped by purpose. */

import { useWorkflowAccess } from '../../workflow-context';
import type { ReactNode } from 'react';
import type { YubiEnrollment } from '../../bridge';
import { Button, Inset, InsetRow, SectionLabel } from '../../components';
import type { AccountStore } from '../../model';
import type { Location } from '../../location';
import { YUBI_WORKFLOWS } from './credential-actions';
import type { SimpleYubiAction } from './credential-actions';

/**
 * The nine card operations that are recovery paths for a key already in
 * trouble — what used to be the "More…" menu's `RECOVERY_ACTIONS`, one menu
 * entry per action. Each is now a button on the key's own page instead, and
 * the sheet it opens reads the same label either way.
 */
type RecoveryAction =
  | 'sync'
  | 'set-passphrase'
  | 'change-passphrase'
  | 'verify-passphrase'
  | 'recover-management'
  | 'recover-subkey'
  | 'resume-enrollment'
  | 'resume-rotation'
  | 'rotate';

/**
 * The rows "Card operations" groups the nine `RecoveryAction`s into, on a
 * security key's own page: Sync (1) + Passphrase (3) + Recovery (5) = 9,
 * plus the Connected row's "Provision…" and "PIN status" — the eleven
 * operations the old "More…" menu and Connected row together held.
 */
const CARD_OP_GROUPS: ReadonlyArray<{
  label: string;
  hint: string;
  actions: readonly RecoveryAction[];
}> = [
  {
    label: 'Sync',
    hint: 'Refresh this account’s state on the card.',
    actions: ['sync'],
  },
  {
    label: 'Passphrase',
    hint: 'Set, change or verify the key passphrase.',
    actions: ['set-passphrase', 'change-passphrase', 'verify-passphrase'],
  },
  {
    label: 'Recovery',
    hint: 'Restore access if the card, its management key or its signing key is stuck.',
    actions: [
      'recover-management',
      'recover-subkey',
      'resume-enrollment',
      'resume-rotation',
      'rotate',
    ],
  },
];

/** The short label a `CARD_OP_GROUPS` button reads; the row it sits under
 * already carries the rest of the sentence. */
const CARD_OP_LABELS: Readonly<Record<RecoveryAction, string>> = {
  sync: 'Sync…',
  'set-passphrase': 'Set…',
  'change-passphrase': 'Change…',
  'verify-passphrase': 'Verify…',
  'recover-management': 'Restore management…',
  'recover-subkey': 'Restore signing key…',
  'resume-enrollment': 'Resume enrollment…',
  'resume-rotation': 'Resume rotation…',
  rotate: 'Rotate…',
};

/**
 * The maintenance operations a security key's card supports, grouped under
 * one heading on the enrollment's own page: what used to be the "More…" menu
 * on the list, plus "Provision…" and "PIN status", which used to be their own
 * buttons on a connected card's row. Every button here acts on this page's
 * own enrollment — there is no other one to guess at.
 */
export function CardOperations({
  enrollment,
  cards,
  store,
  onNavigate,
  onProvision,
  onYubiAction,
}: {
  enrollment: YubiEnrollment;
  /** The cards in this Mac's ports right now. */
  cards: { serial: number }[];
  stopped: { stopped: boolean; reason: string };
  store: AccountStore;
  onNavigate: (location: Location) => void;
  onProvision: () => void;
  onYubiAction: (action: SimpleYubiAction, entry: YubiEnrollment) => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const target = { profile: store.server, account: enrollment.alias };
  const why = access.props('yubi-provision', {
    profile: store.server,
    account: store.account,
  }).title;
  const cardConnected =
    enrollment.cardSerial != null &&
    cards.some((card) => card.serial === enrollment.cardSerial);
  const pinReason =
    access.props('yubi-pin', target).title ??
    (cards.length === 0
      ? 'No security key connected.'
      : !cardConnected
        ? 'This key’s card is not connected.'
        : undefined);
  // Every recovery action but "Resume enrollment" acts on a card whose
  // account already exists; that one acts on a card whose account does not,
  // so the two conditions are exact opposites rather than shades of one.
  const opReason =
    enrollment.state === 'complete'
      ? undefined
      : 'This enrollment is not complete.';
  const resumeReason =
    enrollment.state === 'pending'
      ? undefined
      : 'This enrollment is already complete.';
  return (
    <div
      className="settings-section"
      role="region"
      aria-labelledby="device-card-label"
    >
      <SectionLabel id="device-card-label">Card operations</SectionLabel>
      <Inset className="settings-inset middle wide">
        <InsetRow
          label="Connected"
          action={
            <>
              <Button
                size="sm"
                disabled={why !== undefined}
                title={why}
                onClick={onProvision}
              >
                Provision…
              </Button>
              <Button
                size="sm"
                disabled={pinReason !== undefined}
                title={pinReason}
                onClick={() => onYubiAction('pin-status', enrollment)}
              >
                PIN status
              </Button>
            </>
          }
        >
          Read from the card in the port, not from the account.
        </InsetRow>
        {CARD_OP_GROUPS.map((group) => (
          <InsetRow
            key={group.label}
            label={group.label}
            action={
              <>
                {group.actions.map((action) => {
                  const reason =
                    access.props(YUBI_WORKFLOWS[action], target).title ??
                    (action === 'resume-enrollment' ? resumeReason : opReason);
                  return (
                    <Button
                      key={action}
                      size="sm"
                      disabled={reason !== undefined}
                      title={reason}
                      onClick={() => onYubiAction(action, enrollment)}
                    >
                      {CARD_OP_LABELS[action]}
                    </Button>
                  );
                })}
              </>
            }
          >
            {group.hint}
          </InsetRow>
        ))}
        <InsetRow
          label="PIN"
          action={
            <Button
              size="sm"
              onClick={() =>
                onNavigate({
                  kind: 'settings',
                  section: 'account',
                  profile: store.server,
                })
              }
            >
              Settings › Server
            </Button>
          }
        >
          Set in Settings › Server, along with the key’s other credentials.
        </InsetRow>
      </Inset>
    </div>
  );
}
