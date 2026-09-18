import { useId } from 'react';
import type { ReactNode } from 'react';
import type { GoProfileCandidate } from '../bridge';
import { Button } from '../components';

interface Props {
  candidates: GoProfileCandidate[];
  selected: string | null;
  onSelect: (candidate: GoProfileCandidate) => void;
  /**
   * Whether the chooser is embedded within a radio card. When true, renders
   * without container borders or corner radii and with reduced row padding.
   */
  nested?: boolean;
}

const shortId = (value: string): string =>
  `${value.slice(0, 8)}…${value.slice(-8)}`;

export function GoProfileChooser({
  candidates,
  selected,
  onSelect,
  nested = false,
}: Props): ReactNode {
  const group = useId();
  return (
    <div
      className={nested ? 'go-account-options nested' : 'go-account-options'}
      role={nested ? 'group' : 'radiogroup'}
      aria-label="FOKS accounts"
    >
      {candidates.map((candidate) => {
        const unavailable = !candidate.pairable && !candidate.copyable;
        const chosen = selected === candidate.candidateId;
        const account = candidate.username ?? shortId(candidate.userId);
        const text = (
          <>
            {/* Two-line row layout matching radio card title and detail:
                account name followed by server, role, and storage type.
                Profiles without a username display the account ID in the
                primary line. */}
            <span className="go-account-text">
              <b>
                {candidate.username ?? (
                  <>
                    Account <code>{shortId(candidate.userId)}</code>
                  </>
                )}
              </b>
              <small>
                {candidate.serverHint ??
                  `Server (${shortId(candidate.hostId)})`}{' '}
                ·{' '}
                {candidate.role.toLowerCase() === 'owner'
                  ? 'Account owner'
                  : 'Member'}{' '}
                ·{' '}
                {candidate.storageKind.includes('keychain')
                  ? 'Keychain storage'
                  : 'Local storage'}
              </small>
            </span>
          </>
        );
        if (nested)
          return (
            <div className="go-account-option" key={candidate.candidateId}>
              {text}
              <Button
                size="sm"
                aria-label={`${chosen ? 'Selected' : 'Select'} account ${account} on device ${shortId(candidate.deviceId)}`}
                disabled={unavailable || chosen}
                onClick={() => onSelect(candidate)}
              >
                {chosen ? 'Selected' : 'Select'}
              </Button>
            </div>
          );
        return (
          <label className="go-account-option" key={candidate.candidateId}>
            <input
              type="radio"
              name={group}
              aria-label={`Select account ${account} on device ${shortId(candidate.deviceId)}`}
              checked={chosen}
              disabled={unavailable}
              onChange={() => onSelect(candidate)}
            />
            {text}
          </label>
        );
      })}
    </div>
  );
}
