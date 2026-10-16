/** Rename a server: the name this device shows for it. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../../../bridge';
import { Button, Field, Inset, SheetDialog } from '../../../components';
import type { Server } from '../../../model';

export function RenameServerSheet({
  server,
  bridge,
  onClose,
  onRenamed,
  onError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onRenamed: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [name, setName] = useState(server.displayLabel ?? server.profileName);
  const [busy, setBusy] = useState(false);
  const trimmed = name.trim();
  const valid =
    !trimmed.includes('\0') &&
    !trimmed.includes('\n') &&
    !trimmed.includes('\r') &&
    new TextEncoder().encode(trimmed).length <= 64;
  return (
    <SheetDialog
      width="wide"
      icon="server"
      title="Rename server"
      onClose={onClose}
      footer={
        <>
          <Button disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              const label =
                trimmed === '' || trimmed === server.profileName
                  ? null
                  : trimmed;
              setBusy(true);
              void bridge
                .setServerLabel(server.profileName, label)
                .then((response) => {
                  if (
                    response.profile !== server.profileName ||
                    response.label !== label
                  )
                    throw new Error(
                      'set_server_label returned a different server label.',
                    );
                  return onRenamed();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Save
          </Button>
        </>
      }
    >
      <Inset>
        <Field label="Display name" value={name} onChange={setName} />
      </Inset>
    </SheetDialog>
  );
}
