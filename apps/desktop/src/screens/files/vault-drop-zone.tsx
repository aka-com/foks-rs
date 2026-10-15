/** The veil a file dragged over Files shows, and the upload a drop starts. */

import { useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useToast } from '../../../kit/toasts';
import { Icon } from '../../components';
import type { Store } from '../../model';
import { normalizeCommandError } from '../../bridge';
import type { Bridge } from '../../bridge';
import { useFileDrop } from '../../file-drop';
import { fileDropPath } from '../write-workflow-model';

/** Where a file dropped on the current view is saved. */
export interface DropTarget {
  store: Store;
  /** The selected folder, when one narrows the destination. */
  folder?: string;
  /** Why the vault cannot take an upload, or null when it can. */
  blocked: string | null;
}

/** The request a dropped file turns into. */
export interface DropUpload {
  storeId: string;
  path: string;
  sourcePath: string;
}

/**
 * A drop zone covering the vault's content area.
 *
 * The runtime reports drops for the whole window rather than for an element,
 * so this renders a veil naming the destination instead of relying on the
 * pointer position, and stays out of the pointer's way while it is shown.
 */
export function VaultDropZone({
  bridge,
  target,
  onUpload,
}: {
  bridge: Bridge;
  target: DropTarget;
  onUpload: (upload: DropUpload) => Promise<void>;
}): ReactNode {
  const toasts = useToast();
  const [hovering, setHovering] = useState(false);
  const [uploading, setUploading] = useState<string | null>(null);
  // Guards against a second drop landing while the first upload is in flight:
  // the agent serializes mutations and would refuse the overlapping write.
  const inFlight = useRef(false);
  const warn = (message: string): void => {
    toasts.show(message, { tone: 'warning' });
  };
  useFileDrop({
    bridge,
    active: true,
    priority: 'background',
    onHover: setHovering,
    onPaths: (paths) => {
      setHovering(false);
      if (target.blocked) {
        warn(`${target.store.name}: ${target.blocked}`);
        return;
      }
      if (paths.length !== 1) {
        warn('Drop one file at a time to upload it.');
        return;
      }
      const sourcePath = paths[0];
      const name = sourcePath?.split(/[\\/]/).at(-1);
      if (!sourcePath || !name || inFlight.current) return;
      inFlight.current = true;
      setUploading(name);
      void onUpload({
        storeId: target.store.id,
        path: fileDropPath(name, target.folder),
        sourcePath,
      })
        .catch((error: unknown) => {
          warn(normalizeCommandError(error).message);
        })
        .finally(() => {
          inFlight.current = false;
          setUploading(null);
        });
    },
    onError: (error) => {
      warn(normalizeCommandError(error).message);
    },
  });
  if (!uploading && !hovering) return null;
  const folder = target.folder && target.folder !== '/' ? target.folder : null;
  return (
    <div
      className={target.blocked && !uploading ? 'drop-veil off' : 'drop-veil'}
      role="status"
      aria-live="polite"
    >
      <div className="drop-card">
        <span className="glyph">
          <Icon name="file" />
        </span>
        <b>
          {uploading
            ? `Uploading ${uploading}…`
            : target.blocked
              ? `Cannot upload to ${target.store.name}`
              : `Drop to upload to ${target.store.name}`}
        </b>
        <small>
          {uploading
            ? 'Encrypting and saving this file'
            : (target.blocked ??
              (folder ? `Saved in ${folder}` : 'Saved at the top level'))}
        </small>
      </div>
    </div>
  );
}
