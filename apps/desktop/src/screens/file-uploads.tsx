import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../bridge';
import { normalizeCommandError } from '../bridge';
import type { FileUploads } from '../bridge/uploads';
import { fmtSize } from '../model';

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

/** On-demand local inventory: opening the Files view does not fan out to servers. */
export function FileUploadsPanel({
  bridge,
  storeId,
}: {
  bridge: Bridge;
  storeId: string;
}): ReactNode {
  const [open, setOpen] = useState(false);
  const [inventory, setInventory] = useState<FileUploads | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function act(id?: string, resume = true): Promise<void> {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (id) {
        if (resume) await bridge.resumeFileUpload(storeId, id);
        else await bridge.cancelFileUpload(storeId, id);
      }
      setInventory(await bridge.listFileUploads(storeId));
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
          if (!open) void act();
        }}
      >
        Uploads
      </button>
      {open ? (
        <div>
          <p>
            Prepared uploads can resume without the source file. For uploads
            marked “Check destination,” inspect the destination to confirm the
            result. Incomplete preparation needs a new upload. Refresh the vault
            after completion.
          </p>
          <button type="button" disabled={busy} onClick={() => void act()}>
            Refresh uploads
          </button>
          {error ? <p role="alert">{error}</p> : null}
          {busy ? <p role="status">Checking upload…</p> : null}
          {inventory ? (
            <>
              <p>
                Local encrypted spool limit:{' '}
                {fmtSize(inventory.capacity.maxReservedBytes)} ·{' '}
                {inventory.capacity.maxPending} pending uploads
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
                      {![
                        'awaiting-source',
                        'needs-source',
                        'cancelled',
                        'inline-untracked',
                      ].includes(upload.state) ? (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => void act(upload.uploadId)}
                        >
                          {' '}
                          {upload.state === 'completed'
                            ? 'Verify receipt'
                            : 'Resume'}
                        </button>
                      ) : null}
                      {!['publishing', 'verified'].includes(upload.state) ? (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => void act(upload.uploadId, false)}
                        >
                          {[
                            'completed',
                            'cancelled',
                            'inline-untracked',
                          ].includes(upload.state)
                            ? 'Clean up local spool'
                            : 'Cancel upload'}
                        </button>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )}
            </>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
