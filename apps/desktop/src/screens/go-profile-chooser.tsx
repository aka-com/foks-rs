import { useId } from 'react';
import type { ReactNode } from 'react';
import type { GoProfileCandidate } from '../bridge';

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
      role="radiogroup"
      aria-label="FOKS accounts"
    >
      {candidates.map((candidate) => {
        const unavailable = !candidate.pairable && !candidate.copyable;
        return (
          <label className="go-account-option" key={candidate.candidateId}>
            <input
              type="radio"
              name={group}
              aria-label={`Select account ${candidate.username ?? shortId(candidate.userId)} on device ${shortId(candidate.deviceId)}`}
              checked={selected === candidate.candidateId}
              disabled={unavailable}
              onChange={() => onSelect(candidate)}
            />
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
          </label>
        );
      })}
    </div>
  );
}
