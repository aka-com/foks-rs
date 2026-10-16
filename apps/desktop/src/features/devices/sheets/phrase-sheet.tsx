/**
 * Creates and saves a recovery phrase for an account, then checks the saved
 * copy.
 */
import { useDeviceOperation } from '../use-device-operation';
import { useWorkflowAccess } from '../../../workflow-context';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { recallPositions } from '../recovery-phrase';
import { RecallCheck } from '../recovery-phrase-recall';
import { useSheetGuard } from '../../../navigation-guard';
import { Button, Field, Inset, SheetDialog } from '../../../components';
import { useShellServices } from '../../../shell-services';

export function PhraseSheet({
  profile,
  accountAlias,
  seedPhrase,
  onClose,
  onDone,
  onError,
}: {
  profile: string;
  accountAlias: string;
  seedPhrase?: string;
  onClose: () => void;
  onDone: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const { bridge } = useShellServices();
  const access = useWorkflowAccess();
  const target = { profile, account: accountAlias };
  const eligibility = access.props('backup-create', target);
  const controller = useDeviceOperation(
    JSON.stringify([profile, accountAlias]),
  );
  const [phrase, setPhrase] = useState<string | null>(() => seedPhrase ?? null);
  const [alias, setAlias] = useState('paper-backup');
  // Whether the recall check matched the written copy; it gates the save.
  const [written, setWritten] = useState(false);
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState(false);
  const words = phrase?.split(/\s+/) ?? [];
  const positions = useMemo(() => (phrase ? recallPositions() : []), [phrase]);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const discard = (): void => {
    setPhrase(null);
    setWritten(false);
    setChecking(false);
    onClose();
  };
  // Navigation requires saving or explicitly dismissing the displayed key.
  useSheetGuard(
    phrase
      ? {
          verdict: 'refuse',
          reason: 'Save or dismiss the recovery phrase first.',
        }
      : busy
        ? {
            verdict: 'refuse',
            reason: 'Wait for the recovery phrase to finish.',
          }
        : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title={
        phrase && checking
          ? 'Check your copy'
          : phrase
            ? 'Save recovery phrase'
            : 'Create recovery phrase'
      }
      onClose={discard}
      dismissible={!busy}
      footer={
        <>
          {phrase && !checking ? (
            <>
              <Button disabled={busy} onClick={discard}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => setChecking(true)}>
                Next
              </Button>
            </>
          ) : phrase ? (
            <>
              <Button
                disabled={busy}
                onClick={() => {
                  setWritten(false);
                  setChecking(false);
                }}
              >
                Back
              </Button>
              <Button
                variant="primary"
                title={eligibility.title}
                disabled={!written || busy || eligibility.disabled}
                onClick={() => {
                  setBusy(true);
                  const once = phrase;
                  setPhrase(null);
                  setChecking(false);
                  setWritten(false);
                  void controller
                    .run(
                      () =>
                        access.run('backup-create', target, () =>
                          bridge.commitOwnerBackup(
                            profile,
                            accountAlias,
                            // The name the phrase was prepared under.
                            alias.trim(),
                            once,
                          ),
                        ),
                      onDone,
                      onError,
                    )
                    .finally(controller.settled(() => setBusy(false)));
                }}
              >
                Save recovery phrase
              </Button>
            </>
          ) : (
            <>
              <Button onClick={discard}>Cancel</Button>
              <Button
                variant="primary"
                title={eligibility.title}
                disabled={!alias.trim() || busy || eligibility.disabled}
                onClick={() => {
                  setBusy(true);
                  void access
                    .run('backup-create', target, () =>
                      bridge.prepareOwnerBackup(
                        profile,
                        accountAlias,
                        alias.trim(),
                      ),
                    )
                    .then((result) => {
                      if (mounted.current) setPhrase(result.phrase);
                    })
                    .catch(onError)
                    .finally(() => setBusy(false));
                }}
              >
                Generate phrase
              </Button>
            </>
          )}
        </>
      }
    >
      {phrase && checking ? (
        <RecallCheck
          phrase={phrase}
          positions={positions}
          unlocks="Save recovery phrase"
          reveal="Back"
          onMatched={setWritten}
        />
      ) : phrase ? (
        <>
          <p>
            Write these {words.length} words and numbers down now, in order. The
            phrase cannot be shown again after you close this.
          </p>
          <div className="words">
            {words.map((word, index) => (
              <span className="word" key={`${index}-${word}`}>
                <i>{index + 1}</i>
                {word}
              </span>
            ))}
          </div>
          <p className="recall-hint">
            Next, you will type three of these from your written copy.
          </p>
        </>
      ) : (
        <Inset>
          <Field
            label="Recovery phrase name"
            value={alias}
            onChange={setAlias}
          />
        </Inset>
      )}
    </SheetDialog>
  );
}
