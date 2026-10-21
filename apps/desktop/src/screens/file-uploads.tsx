import { useState } from 'react';
import type { ReactNode } from 'react';
import { Button, Inset, InsetRow, SheetDialog } from '../components';
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

/**
 * The store's prepared and recorded uploads, behind the list foot's
 * Uploads… button. On-demand local inventory: opening the Files view does not
 * fan out to servers, and the sheet reads the inventory only when it opens.
 */
export function FileUploadsPanel({
  bridge,
  storeId,
  storeName,
}: {
  bridge: Bridge;
  storeId: string;
  storeName: string;
}): ReactNode {
  const [open, setOpen] = useState(false);
  const [inventory, setInventory] = useState<FileUploads | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function act(
    id?: string,
    resume = true,
    after?: number[] | null,
  ): Promise<void> {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (id) {
        if (resume) await bridge.resumeFileUpload(storeId, id);
        else await bridge.cancelFileUpload(storeId, id);
      }
      setInventory(await bridge.listFileUploads(storeId, after));
    } catch (cause) {
      setError(normalizeCommandError(cause).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <button
        type="button"
        className="lnk uploads-open"
        aria-haspopup="dialog"
        onClick={() => {
          setOpen(true);
          void act();
        }}
      >
        Uploads…
      </button>
      {open ? (
        <SheetDialog
          title="Uploads"
          subtitle={storeName}
          onClose={() => setOpen(false)}
          footer={
            <>
              <Button disabled={busy} onClick={() => void act()}>
                Refresh
              </Button>
              <span className="spacer" />
              <Button variant="primary" onClick={() => setOpen(false)}>
                Done
              </Button>
            </>
          }
        >
          <p>
            Prepared uploads can resume without the source file. For uploads
            marked “Check destination,” inspect the destination to confirm the
            result. Incomplete preparation needs a new upload. Refresh the vault
            after completion. Encrypted local files are removed in the
            background; reserved space remains charged until removal finishes.
          </p>
          {error ? (
            <p className="action-error" role="alert">
              {error}
            </p>
          ) : null}
          {busy ? (
            <p className="hint" role="status">
              Checking uploads…
            </p>
          ) : null}
          {inventory ? (
            <>
              <Inset className="file-uploads">
                {inventory.uploads.length === 0 ? (
                  <InsetRow>
                    <span className="dim">No recorded uploads.</span>
                  </InsetRow>
                ) : (
                  inventory.uploads.map((upload) => (
                    <InsetRow
                      key={upload.uploadId}
                      label={uploadLabels[upload.state]}
                      action={
                        <>
                          {![
                            'awaiting-source',
                            'needs-source',
                            'cancelled',
                            'inline-untracked',
                          ].includes(upload.state) ? (
                            <Button
                              disabled={busy}
                              onClick={() => void act(upload.uploadId)}
                            >
                              {upload.state === 'completed'
                                ? 'Show Saved Receipt'
                                : 'Resume'}
                            </Button>
                          ) : null}
                          {!['publishing', 'verified'].includes(
                            upload.state,
                          ) ? (
                            <Button
                              disabled={busy}
                              onClick={() => void act(upload.uploadId, false)}
                            >
                              {[
                                'completed',
                                'cancelled',
                                'inline-untracked',
                              ].includes(upload.state)
                                ? 'Clean Up Local Spool'
                                : 'Cancel Upload'}
                            </Button>
                          ) : null}
                        </>
                      }
                    >
                      <code>{upload.uploadId}</code>
                      <small>
                        {upload.acknowledgedChunks}/{upload.preparedChunks}{' '}
                        chunks acknowledged · {fmtSize(upload.reservedBytes)}{' '}
                        reserved
                      </small>
                    </InsetRow>
                  ))
                )}
              </Inset>
              <p className="hint">
                Local encrypted spool limit:{' '}
                {fmtSize(inventory.capacity.maxReservedBytes)} ·{' '}
                {inventory.capacity.maxPending} pending uploads ·{' '}
                {inventory.capacity.maxReceipts} retained receipts
              </p>
              {inventory.next ? (
                <Button
                  disabled={busy}
                  onClick={() => void act(undefined, true, inventory.next)}
                >
                  Next Uploads
                </Button>
              ) : null}
            </>
          ) : null}
        </SheetDialog>
      ) : null}
    </>
  );
}
