import { useState } from 'react';
import type { ReactNode } from 'react';
import { Band, Button, Field, Icon, Inset, SheetDialog } from '../components';
import { normalizeCommandError } from '../bridge';
import { isTrashPath, kindOf, storeOf } from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import { useSheetGuard } from '../navigation-guard';
import type { WriteWorkflow } from './write-workflows';
import { useShellServices, useShellSnapshot } from '../shell-services';

/** The passwords of `store` an export writes: what Files lists as one. */
export function exportedPasswords(
  snapshot: AgentSnapshot,
  store: StoreRef,
): number {
  return snapshot.items.filter(
    (item) =>
      item.store === store &&
      !isTrashPath(item.path) &&
      kindOf(item) === 'Password',
  ).length;
}

function passwordCount(count: number): string {
  return `${count} ${count === 1 ? 'password' : 'passwords'}`;
}

/**
 * Exports the passwords of one vault to a CSV file in the column layout of
 * a Bitwarden export, which Import passwords and other password managers
 * read. The host reads the passwords and writes the file the save dialog
 * names; the file is plain text, so the export waits for the vault's name
 * to be typed.
 */
export function ExportPasswordsSheet({
  storeId,
  setWorkflow,
}: {
  storeId: StoreRef;
  setWorkflow: (workflow: WriteWorkflow) => void;
}): ReactNode {
  const snapshot = useShellSnapshot();
  const { bridge, refresh: onApplied } = useShellServices();
  const [typed, setTyped] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: 'Wait for the export to finish.' }
      : null,
    !saving,
  );
  const store = storeOf(snapshot, storeId);
  if (!store) return null;
  const count = exportedPasswords(snapshot, store.id);
  const confirmed = typed.trim() === store.name;
  const close = (): void => setWorkflow(null);

  const save = async (): Promise<void> => {
    if (saving || !confirmed || !count) return;
    setSaving(true);
    setError(null);
    try {
      const result = await bridge.exportLogins(store.id);
      if (!result.saved) return;
      setWorkflow(null);
      await onApplied(
        `Exported ${passwordCount(result.exported)} from ${store.name}${
          result.skipped ? `; ${result.skipped} not text, left out` : ''
        }. Delete the file once you have used it.`,
        undefined,
        { tone: 'warning' },
      );
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <SheetDialog
      title="Export passwords"
      width="mid"
      glyph={
        <span className="kic Password" aria-hidden="true">
          <Icon name="key" />
        </span>
      }
      onClose={close}
      dismissible={!saving}
      footer={
        <>
          <Button disabled={saving} onClick={close}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={saving || !confirmed || !count}
            onClick={() => void save()}
          >
            {saving ? 'Exporting…' : 'Export…'}
          </Button>
        </>
      }
    >
      <p>
        {count
          ? `Writes the ${passwordCount(count)} in ${store.name} to a CSV file, with each user name, website and note. Documents and files are not exported.`
          : `${store.name} has no passwords to export.`}
      </p>
      <Band severity="crit">
        The file holds every password in plain text, outside the vault&apos;s
        encryption. Anyone who can read it can use them. Delete it once you have
        imported it elsewhere.
      </Band>
      <p>
        The columns are those of a Bitwarden export, which Import passwords and
        most password managers read. Open it in a password manager, not a
        spreadsheet app: a spreadsheet can run a value that begins with =, +, -
        or @ as a formula, and anyone who can write to this vault can choose
        such a value.
      </p>
      <Inset>
        <Field
          label={`Type ${store.name} to confirm`}
          value={typed}
          verbatim
          disabled={saving}
          onChange={setTyped}
        />
      </Inset>
      {error ? <p role="alert">{error}</p> : null}
    </SheetDialog>
  );
}
