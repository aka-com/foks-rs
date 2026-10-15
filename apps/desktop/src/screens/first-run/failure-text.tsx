import type { ReactNode } from 'react';
import { Toggle } from '../../components';

/**
 * A reported failure: the sentence the agent produced, and behind a
 * disclosure, the raw error chain it kept out of that sentence. Every first-run
 * failure reports through here, so the detail a server check offers is the
 * detail account creation, recovery, pairing and the rest offer too.
 */
export function FailureText({
  text,
  reason,
}: {
  text: string;
  reason?: string;
}): ReactNode {
  return (
    <>
      {text}
      {reason ? (
        <Toggle label="Details">
          <pre>{reason}</pre>
        </Toggle>
      ) : null}
    </>
  );
}

const MISSING_SERVER_EXPLANATION =
  'The account you were creating could not be found on the server. This may happen because of a restart, server reset, or other error.';

export function MissingServerWarning({
  action,
}: {
  action?: ReactNode;
}): ReactNode {
  return (
    <div className="alt-path warning" role="alert">
      <div className="t">
        <b>The server saved for this account is missing</b>
        <span>{MISSING_SERVER_EXPLANATION}</span>
      </div>
      {action}
    </div>
  );
}
