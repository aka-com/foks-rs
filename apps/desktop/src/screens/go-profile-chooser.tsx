import { useId } from 'react';
import type { ReactNode } from 'react';
import type { GoProfileCandidate } from '../bridge';

interface Props {
  candidates: GoProfileCandidate[];
  selected: string | null;
  onSelect: (candidate: GoProfileCandidate) => void;
}

const shortId = (value: string): string =>
  `${value.slice(0, 8)}…${value.slice(-8)}`;

export function GoProfileChooser({
  candidates,
  selected,
  onSelect,
}: Props): ReactNode {
  const group = useId();
  return (
    <div
      className="go-account-options"
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
            <span className="go-account-name">
              {candidate.username ?? 'Unknown account'}
            </span>
            <span className="go-account-detail">
              <span>
                {candidate.serverHint ??
                  `Server (${shortId(candidate.hostId)})`}
              </span>
              <small>
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
