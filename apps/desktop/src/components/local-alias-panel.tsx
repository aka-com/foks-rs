import { useState } from 'react';
import { useWorkflowAccess } from '../workflow-context';
import { normalizeCommandError, type Bridge } from '../bridge';
import { useSheetGuard } from '../navigation-guard';
import {
  Button,
  Field,
  Inset,
  PanelSheet,
  type PanelPresentation,
} from './index';

export function LocalAliasPanel({
  bridge,
  store,
  alias,
  presentation,
  onComplete,
}: {
  bridge: Bridge;
  store: string;
  alias: string;
  presentation: PanelPresentation;
  onComplete: () => Promise<void>;
}) {
  const access = useWorkflowAccess();
  const eligibility = access.props('local-alias', {});
  const [name, setName] = useState(alias);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const label = name.trim();
  const valid =
    !!label &&
    new TextEncoder().encode(label).length <= 64 &&
    !/[\p{Cc}]/u.test(name);
  const dirty = name !== alias;
  useSheetGuard(
    busy
      ? {
          verdict: 'refuse',
          reason: 'Wait for the local alias update to finish.',
        }
      : dirty
        ? {
            verdict: 'prompt',
            title: 'Discard local alias change?',
            body: 'The new local alias has not been saved.',
            confirm: 'Discard',
            onConfirm: presentation.onClose,
          }
        : null,
  );
  const save = async () => {
    if (!valid || busy || label === alias) return;
    setBusy(true);
    setError(null);
    try {
      access.require('local-alias', {});
      const reply = await bridge.setLocalAccountAlias(store, label);
      if (reply.store !== store || reply.alias !== label)
        throw new Error('Local alias response belongs to a different account.');
      await onComplete();
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <PanelSheet
      presentation={presentation}
      busy={busy}
      footer={
        <>
          <Button disabled={busy} onClick={presentation.onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            title={eligibility.title}
            disabled={busy || eligibility.disabled || !valid || label === alias}
            onClick={() => void save()}
          >
            Save
          </Button>
        </>
      }
    >
      <p>Changes the name shown in this app only.</p>
      <Inset>
        <Field
          disabled={busy}
          label="Local alias"
          value={name}
          onChange={setName}
        />
      </Inset>
      {!valid ? (
        <p role="alert">
          Enter a name of 1–64 UTF-8 bytes without control characters.
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="crit">
          {error}
        </p>
      ) : null}
    </PanelSheet>
  );
}
