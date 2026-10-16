/** Remove a server and the credentials this device holds for it. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../../../bridge';
import { Button, Inset, InsetRow, SheetDialog } from '../../../components';
import { useSheetGuard } from '../../../navigation-guard';
import { purgeProfiles } from '../../../platform/device-storage';
import { serverDisplayLabelOrLoading } from '../../../model';
import type { Server } from '../../../model';

export function RemoveServerSheet({
  server,
  bridge,
  onClose,
  onRemoved,
  onError,
}: {
  server: Server;
  bridge: Bridge;
  onClose: () => void;
  onRemoved: () => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  const [confirmedProfileName, setConfirmedProfileName] = useState('');
  const [busy, setBusy] = useState(false);
  // Removing a server deletes what this device holds for it.
  useSheetGuard(
    busy
      ? { verdict: 'refuse', reason: 'Wait for the removal to finish.' }
      : null,
  );
  return (
    <SheetDialog
      width="wide"
      icon="trash"
      title={`Remove ${serverDisplayLabelOrLoading(server)} and its credentials?`}
      onClose={onClose}
      danger
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="danger"
            disabled={confirmedProfileName !== server.profileName || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .removeServerAndCredentials(
                  server.profileName,
                  confirmedProfileName,
                )
                .then((removed) => {
                  if (removed.profile !== server.profileName)
                    throw new Error(
                      'remove_server_and_credentials returned a different profile.',
                    );
                  purgeProfiles([server.profileName]);
                  return onRemoved();
                })
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Remove server and credentials
          </Button>
        </>
      }
    >
      <p>
        Your data on the server is not deleted. Type the server profile name to
        confirm.
      </p>
      <Inset>
        <InsetRow label="Removed">
          This server, all of its credentials, pinned host identity, connection
          history, cached data, and unfinished operations.
        </InsetRow>
        <InsetRow label="App records">
          This app also removes its setup progress, invitation labels, Recent
          and Favorites entries, and exposure reviews for this server.
        </InsetRow>
        <InsetRow label="Warning">
          Accounts without a recovery phrase or another paired device cannot be
          accessed again.
        </InsetRow>
        <InsetRow label="Unaffected">
          Other configured servers and their local data.
        </InsetRow>
      </Inset>
      <Inset className="confirm-inset">
        <InsetRow label="Confirm">
          <input
            value={confirmedProfileName}
            onChange={(event) => setConfirmedProfileName(event.target.value)}
            placeholder="Type server profile name to confirm"
          />
        </InsetRow>
      </Inset>
    </SheetDialog>
  );
}
