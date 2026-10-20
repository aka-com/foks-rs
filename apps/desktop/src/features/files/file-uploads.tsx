import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../../bridge';
import { normalizeCommandError } from '../../bridge';
import type { FileUploads } from '../../bridge/uploads';
import { fmtSize } from '../../model';

const uploadLabels: Record<string, string> = {
  'awaiting-source': 'Waiting for source file',
  'inline-untracked': 'Check destination',
  'needs-source': 'Source file needed',
  ready: 'Ready to resume',
  uploading: 'Uploading',
  'content-complete': 'Ready to publish',
  publishing: 'Checking publication',
  verified: 'Uploaded',
  completed: 'Completed',
  blocked: 'Needs attention',
  cancelled: 'Cancelled',
};

/** States whose row offers Resume: work the agent can still do. */
const resumable = [
  'ready',
  'uploading',
  'content-complete',
  'publishing',
  'verified',
  'blocked',
];
/**
 * Refusals the host answers before it discards the store's listed items and
 * asks the agent: another change's reservation, a malformed handle, the
 * application lock and a command without a deadline policy.
 */
const refusedBeforeDispatch = [
  'mutation-in-flight',
  'invalid-request',
  'app-locked',
  'missing-deadline-policy',
];
/** States with nothing left to upload, whose cancel only frees the spool. */
const settled = ['completed', 'cancelled', 'inline-untracked'];

/**
 * On-demand local inventory: opening the Files view does not fan out to
 * servers. A resume or cancel can publish or remove the store's file, so
 * `onChanged` refreshes the store's server afterwards; the list stays on the
 * page it showed.
 */
export function FileUploadsPanel({
  bridge,
  storeId,
  onChanged,
}: {
  bridge: Bridge;
  storeId: string;
  onChanged?: (message: string) => Promise<void>;
}): ReactNode {
  const [open, setOpen] = useState(false);
  const [inventory, setInventory] = useState<FileUploads | null>(null);
  // The cursor of the page `inventory` shows; null is the first page.
  const [page, setPage] = useState<number[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function act(
    after: number[] | null,
    change?: { id: string; resume: boolean; done: string },
  ): Promise<void> {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (change) {
        try {
          if (change.resume) await bridge.resumeFileUpload(storeId, change.id);
          else await bridge.cancelFileUpload(storeId, change.id);
        } catch (cause) {
          // The host discards the store's listed items before it asks the
          // agent, so any attempt it dispatched leaves them to be read again.
          if (
            !refusedBeforeDispatch.includes(normalizeCommandError(cause).code)
          )
            await onChanged?.(
              'The upload change failed; the vault was refreshed.',
            );
          throw cause;
        }
        await onChanged?.(change.done);
      }
      setInventory(await bridge.listFileUploads(storeId, after));
      setPage(after);
    } catch (cause) {
      setError(normalizeCommandError(cause).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="file-uploads" aria-label="File uploads">
      <button
        type="button"
        aria-expanded={open}
        disabled={busy}
        onClick={() => {
          setOpen(!open);
          if (!open) void act(null);
        }}
      >
        Uploads
      </button>
      {open ? (
        <div>
          <p>
            Prepared uploads can resume without the source file. For uploads
            marked “Check destination,” inspect the destination to confirm the
            result. Incomplete preparation needs a new upload. Encrypted local
            files are removed in the background; reserved space remains charged
            until removal finishes.
          </p>
          <button type="button" disabled={busy} onClick={() => void act(page)}>
            Refresh uploads
          </button>
          {error ? <p role="alert">{error}</p> : null}
          {busy ? <p role="status">Checking upload…</p> : null}
          {inventory ? (
            <>
              <p>
                Local encrypted spool limit:{' '}
                {fmtSize(inventory.capacity.maxReservedBytes)} ·{' '}
                {inventory.capacity.maxPending} pending uploads ·{' '}
                {inventory.capacity.maxReceipts} retained receipts
              </p>
              {inventory.uploads.length === 0 ? (
                <p>No recorded uploads.</p>
              ) : (
                <ul>
                  {inventory.uploads.map((upload) => (
                    <li key={upload.uploadId}>
                      <code>{upload.uploadId}</code> ·{' '}
                      {uploadLabels[upload.state]} · {upload.acknowledgedChunks}
                      /{upload.preparedChunks} chunks acknowledged ·{' '}
                      {fmtSize(upload.reservedBytes)} reserved
                      {['verified', 'completed'].includes(upload.state) &&
                      upload.nodeId !== null &&
                      upload.version !== null ? (
                        <>
                          {' '}
                          · saved as node <code>{upload.nodeId}</code>, version{' '}
                          {upload.version}
                        </>
                      ) : null}
                      {resumable.includes(upload.state) ? (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() =>
                            void act(page, {
                              id: upload.uploadId,
                              resume: true,
                              done: 'Upload resumed',
                            })
                          }
                        >
                          Resume
                        </button>
                      ) : null}
                      {!['publishing', 'verified'].includes(upload.state) ? (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() =>
                            void act(page, {
                              id: upload.uploadId,
                              resume: false,
                              done: settled.includes(upload.state)
                                ? 'Local upload spool cleaned up'
                                : 'Upload cancelled',
                            })
                          }
                        >
                          {settled.includes(upload.state)
                            ? 'Clean up local spool'
                            : 'Cancel upload'}
                        </button>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )}
              {inventory.next ? (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void act(inventory.next ?? null)}
                >
                  Next uploads
                </button>
              ) : null}
            </>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
