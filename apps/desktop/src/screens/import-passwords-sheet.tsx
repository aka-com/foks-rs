import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import type { ToastOptions } from '/kit/toasts';
import {
  Band,
  Button,
  CardSelect,
  Field,
  Icon,
  Inset,
  SectionLabel,
  SheetDialog,
} from '../components';
import type {
  Bridge,
  KvRoleInput,
  PasswordImportOutcome,
  PasswordImportPreview,
  PasswordImportStatus,
} from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { FoksIconName } from '../icons';
import {
  defaultCreateStore,
  displayPath,
  isTrashPath,
  storeAvailability,
  storeNavigationOrder,
  storeOf,
} from '../model';
import type { AgentSnapshot, StoreRef } from '../model';
import type { MutationFailureHandler } from '../mutation-recovery';
import { useSheetGuard } from '../navigation-guard';
import { joinPath, validFolderPath } from './item-moves';
import { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE } from './item-roles';
import { AccessBlock, storeOption, writeBlockReason } from './store-choice';
import type { WriteWorkflow } from './write-workflows';

/** The folder imported logins go to unless another is chosen. */
export const IMPORT_FOLDER = '/logins';

type RowStatus = PasswordImportStatus | 'pending' | 'running';

interface Row {
  status: RowStatus;
  message: string;
}

const ROW_CLASS: Readonly<Record<RowStatus, string>> = {
  pending: 'pending',
  running: 'running',
  created: 'done',
  exists: 'skipped',
  failed: 'failed',
  uncertain: 'uncertain',
  'not-started': 'pending',
};

const ROW_ICON: Readonly<Record<Exclude<RowStatus, 'running'>, FoksIconName>> =
  {
    pending: 'ellipsis',
    created: 'circleCheck',
    exists: 'check',
    failed: 'close',
    uncertain: 'info',
    'not-started': 'ellipsis',
  };

/** What a row says once the host reports its outcome. */
function outcomeRow(outcome: PasswordImportOutcome): Row {
  return {
    status: outcome.status,
    message:
      outcome.status === 'created'
        ? 'Saved'
        : outcome.status === 'uncertain'
          ? `${outcome.message ?? 'The outcome is unknown.'} Refresh to check; trying again cannot save it twice.`
          : (outcome.message ?? ''),
  };
}

function loginCount(count: number): string {
  return `${count} ${count === 1 ? 'login' : 'logins'}`;
}

/** The path of a preview entry under `folder`, as stored. */
export function importedPath(
  folder: string,
  entry: { folder: string; name: string },
): string {
  return joinPath(
    entry.folder ? joinPath(folder, entry.folder) : folder,
    entry.name,
  );
}

/** Why `folder` cannot receive imported logins, or null. */
export function importFolderProblem(folder: string): string | null {
  if (!validFolderPath(folder))
    return 'Enter a folder path that starts with /, such as /logins.';
  if (isTrashPath(folder)) return 'Logins cannot be imported into Trash.';
  return null;
}

/**
 * Imports logins from a CSV file that another password manager exported.
 * The host opens and parses the file and keeps every field value; this
 * sheet receives the name and folder of each login, chooses the vault and
 * folder they are saved in, and lists each outcome. A login whose path is
 * taken is left as it is, so importing the same file again saves only what
 * is missing.
 */
export function ImportPasswordsSheet({
  snapshot,
  bridge,
  workflow,
  setWorkflow,
  onApplied,
  onMutationError,
  onReload,
  accessNow,
}: {
  snapshot: AgentSnapshot;
  bridge: Bridge;
  workflow: { storeId: StoreRef; initialFolder?: string };
  setWorkflow: (workflow: WriteWorkflow) => void;
  onApplied: (
    message: string,
    profile?: string,
    options?: ToastOptions,
  ) => Promise<void>;
  onMutationError: MutationFailureHandler;
  /** Reads the catalog of `profile` again and resolves with it. */
  onReload: (profile?: string) => Promise<AgentSnapshot>;
  accessNow: () => number;
}): ReactNode {
  const [preview, setPreview] = useState<PasswordImportPreview | null>(null);
  const [picking, setPicking] = useState(false);
  const [storeId, setStoreId] = useState(() =>
    storeOf(snapshot, workflow.storeId)
      ? workflow.storeId
      : (defaultCreateStore(snapshot) ?? workflow.storeId),
  );
  const [folder, setFolder] = useState(
    workflow.initialFolder && workflow.initialFolder !== '/'
      ? workflow.initialFolder
      : IMPORT_FOLDER,
  );
  const [readRole, setReadRole] = useState<KvRoleInput>(DEFAULT_READ_ROLE);
  const [writeRole, setWriteRole] = useState<KvRoleInput>(DEFAULT_WRITE_ROLE);
  const [rows, setRows] = useState<readonly Row[] | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The held import, for the discard when the sheet goes away.
  const held = useRef<string | null>(null);
  useEffect(
    () => () => {
      if (held.current)
        void bridge.discardPasswordImport(held.current).catch(() => undefined);
    },
    // The held file is dropped once, when the sheet closes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  useSheetGuard(
    running
      ? { verdict: 'refuse', reason: 'Wait for the import to finish.' }
      : null,
    !running,
  );

  const store = storeOf(snapshot, storeId);
  const started = rows !== null;
  const close = (): void => setWorkflow(null);

  const choose = async (): Promise<void> => {
    if (picking || running) return;
    setPicking(true);
    setError(null);
    try {
      const chosen = await bridge.beginPasswordImport();
      if (!chosen) return;
      // A new file replaces the one held before, in the host as well.
      held.current = chosen.id;
      setPreview(chosen);
      setRows(null);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
    } finally {
      setPicking(false);
    }
  };

  const folderProblem = importFolderProblem(folder);
  const problem = (): string | null => {
    if (!preview) return null;
    if (!preview.entries.length)
      return 'This file holds no logins that can be imported.';
    if (!store) return 'Choose a vault to import into.';
    return writeBlockReason(snapshot, store) ?? folderProblem;
  };
  const blocked = problem();

  /** Sends the entries at `indexes` and records each outcome on its row. */
  const run = async (
    base: readonly Row[],
    indexes: readonly number[],
  ): Promise<void> => {
    if (!preview || !store) return;
    const profile = store.server;
    let current = base.map((row, index) =>
      indexes.includes(index)
        ? { status: 'running' as const, message: '' }
        : row,
    );
    setRows(current);
    setRunning(true);
    setError(null);
    const record = (outcome: PasswordImportOutcome): void => {
      current = current.map((row, index) =>
        index === outcome.index ? outcomeRow(outcome) : row,
      );
      setRows(current);
    };
    let thrown: unknown = null;
    try {
      const outcomes = await bridge.commitPasswordImport(
        {
          importId: preview.id,
          entries: indexes,
          storeId: store.id,
          folder,
          ...(store.kind === 'team' ? { readRole, writeRole } : {}),
        },
        record,
      );
      outcomes.forEach(record);
    } catch (failure) {
      thrown = failure;
      const typed = normalizeCommandError(failure);
      setError(typed.message);
      // Rows the host never reported on were not saved, unless the whole
      // request ended with an unknown outcome.
      current = current.map((row) =>
        row.status === 'running'
          ? typed.ambiguous
            ? {
                status: 'uncertain',
                message: 'The outcome is unknown. Refresh to check.',
              }
            : { status: 'failed', message: typed.message }
          : row,
      );
      setRows(current);
    }
    setRunning(false);
    if (thrown) await onMutationError(thrown, { report: false });
    const created = current.filter((row) => row.status === 'created').length;
    const existing = current.filter((row) => row.status === 'exists').length;
    if (created + existing === current.length) {
      setWorkflow(null);
      const message = created
        ? `Imported ${loginCount(created)} to ${store.name}${existing ? `; ${existing} already there` : ''}`
        : `All ${loginCount(existing)} are already in ${store.name}`;
      try {
        await onApplied(message, profile);
      } catch (failure) {
        await onMutationError(failure, { report: false });
      }
      return;
    }
    // The list behind the sheet shows what was saved. After an unknown
    // outcome, this read is also the refresh the next write waits for.
    if (!thrown) void onReload(profile).catch(() => undefined);
  };

  const start = (): void => {
    if (!preview || !store || blocked || running || started) return;
    if (
      !storeAvailability(snapshot, store, { nowSeconds: accessNow() }).available
    ) {
      setError(writeBlockReason(snapshot, store));
      return;
    }
    void run(
      preview.entries.map(() => ({ status: 'pending', message: 'Waiting' })),
      preview.entries.map((_, index) => index),
    );
  };

  // A login that failed, was not reached or has an unknown outcome is sent
  // again; its create-only write cannot replace one that was saved.
  const retryable = (rows ?? []).flatMap((row, index) =>
    ['failed', 'uncertain', 'not-started'].includes(row.status) ? [index] : [],
  );
  const retry = async (): Promise<void> => {
    if (!rows || running || !retryable.length || !store) return;
    try {
      await onReload(store.server);
    } catch (failure) {
      setError(normalizeCommandError(failure).message);
      return;
    }
    void run(rows, retryable);
  };

  const counts = (rows ?? []).reduce<Record<RowStatus, number>>(
    (total, row) => ({ ...total, [row.status]: total[row.status] + 1 }),
    {
      pending: 0,
      running: 0,
      created: 0,
      exists: 0,
      failed: 0,
      uncertain: 0,
      'not-started': 0,
    },
  );
  const status = running
    ? `${(rows?.length ?? 0) - counts.running - counts.pending} of ${rows?.length ?? 0} done`
    : [
        counts.created ? `${counts.created} saved` : '',
        counts.exists ? `${counts.exists} already there` : '',
        counts.failed + counts['not-started']
          ? `${counts.failed + counts['not-started']} not saved`
          : '',
        counts.uncertain ? `${counts.uncertain} unknown` : '',
      ]
        .filter(Boolean)
        .join(' · ');
  const total = preview ? loginCount(preview.entries.length) : '';
  const title = !preview
    ? 'Import passwords'
    : started
      ? running
        ? `Importing ${total}`
        : `Imported ${counts.created} of ${total}`
      : `Import ${total}`;

  return (
    <SheetDialog
      title={title}
      width="mid"
      glyph={
        <span className="kic Password" aria-hidden="true">
          <Icon name="key" />
        </span>
      }
      onClose={close}
      dismissible={!running}
      footer={
        <>
          {blocked && !started ? (
            <span className="move-preview bad" aria-live="polite">
              {blocked}
            </span>
          ) : null}
          {started ? (
            <>
              <Button
                icon="refresh"
                disabled={running || !retryable.length}
                onClick={() => void retry()}
              >
                Retry
              </Button>
              <Button variant="primary" disabled={running} onClick={close}>
                {running ? 'Importing…' : 'Done'}
              </Button>
            </>
          ) : (
            <>
              <Button disabled={picking} onClick={close}>
                Cancel
              </Button>
              {preview ? (
                <Button
                  variant="primary"
                  disabled={Boolean(blocked) || picking}
                  onClick={start}
                >
                  {`Import ${total}`}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  icon="upload"
                  disabled={picking}
                  data-dialog-autofocus="true"
                  onClick={() => void choose()}
                >
                  {picking ? 'Reading the file…' : 'Choose CSV file…'}
                </Button>
              )}
            </>
          )}
        </>
      }
    >
      {!preview ? (
        <>
          <p>
            Import logins from a CSV file exported by 1Password, Bitwarden,
            KeePass, KeePassXC, LastPass, or the password manager of Chrome,
            Edge, Firefox or Safari. Each login is saved as a password with its
            user name, website and notes.
          </p>
          <Band>
            The export holds every password in plain text. Delete it once the
            import is done.
          </Band>
          <p>
            To use the items of an existing FOKS account, connect that account
            instead: its items are stored on its server and appear once it is
            connected (Settings › Account › Import from FOKS CLI).
          </p>
        </>
      ) : (
        <>
          {started ? (
            <p role="status">{status}</p>
          ) : (
            <>
              <p>
                {`${preview.fileName || 'The file'} holds ${total}. Each is saved as a new password; one whose name is already taken in the folder is left out.`}
              </p>
              <SectionLabel>Import into</SectionLabel>
              <Inset>
                <CardSelect
                  label="Import into vault"
                  options={storeNavigationOrder(snapshot).map((candidate) =>
                    storeOption(snapshot, candidate),
                  )}
                  value={storeId}
                  onChange={setStoreId}
                  placeholder="Choose a vault"
                />
                <Field
                  label="Folder"
                  value={folder}
                  verbatim
                  mono
                  onChange={setFolder}
                  hint={folderProblem ?? undefined}
                />
              </Inset>
              {store ? (
                <AccessBlock
                  snapshot={snapshot}
                  store={store}
                  readRole={readRole}
                  writeRole={writeRole}
                  onReadRole={setReadRole}
                  onWriteRole={setWriteRole}
                />
              ) : null}
            </>
          )}
          <SectionLabel>Logins</SectionLabel>
          <ul className="bulk-rows" aria-label="Logins">
            {preview.entries.map((entry, index) => {
              const row = rows?.[index];
              const rowStatus = row?.status ?? 'pending';
              return (
                <li
                  key={`${entry.folder}/${entry.name}`}
                  className={`bulk-row ${ROW_CLASS[rowStatus]}`}
                >
                  {rowStatus === 'running' ? (
                    <span className="spin" aria-hidden="true" />
                  ) : (
                    <Icon name={ROW_ICON[rowStatus]} />
                  )}
                  <span className="t">
                    <b>{displayPath(importedPath(folder, entry))}</b>
                    <small>
                      {row?.message ||
                        (entry.document
                          ? 'Longer than 2,040 bytes, so it is saved as a document.'
                          : '')}
                    </small>
                  </span>
                </li>
              );
            })}
          </ul>
          {preview.skipped.length ? (
            <>
              <SectionLabel>Left out</SectionLabel>
              <ul className="bulk-rows" aria-label="Rows left out">
                {preview.skipped.map((skip) => (
                  <li key={skip.row} className="bulk-row refused">
                    <Icon name="alert" />
                    <span className="t">
                      <b>
                        {skip.name
                          ? `Row ${skip.row}: ${skip.name}`
                          : `Row ${skip.row}`}
                      </b>
                      <small>{skip.reason}</small>
                    </span>
                  </li>
                ))}
              </ul>
            </>
          ) : null}
          {preview.ignoredColumns.length ? (
            <p>{`Columns not imported: ${preview.ignoredColumns.join(', ')}.`}</p>
          ) : null}
        </>
      )}
      {error ? <p role="alert">{error}</p> : null}
    </SheetDialog>
  );
}
