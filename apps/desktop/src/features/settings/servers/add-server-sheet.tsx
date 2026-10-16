/** Add a server: checks an address and adds it to this device's servers. */

import { useState } from 'react';
import type { ReactNode } from 'react';
import type { Bridge } from '../../../bridge';
import { Button, Field, Inset, SheetDialog } from '../../../components';

export function AddServerSheet({
  bridge,
  onClose,
  onAdded,
  onError,
}: {
  bridge: Bridge;
  onClose: () => void;
  onAdded: (profile: string) => Promise<void>;
  onError: (error: unknown) => void;
}): ReactNode {
  // Suggestions, never values: an address that was not typed must not be
  // submittable, and a profile name is the reader's to choose.
  const [profile, setProfile] = useState('');
  const [probe, setProbe] = useState('');
  const [busy, setBusy] = useState(false);
  const cleanProbe = probe.trim();
  const valid =
    /^[A-Za-z0-9_-]{1,64}$/.test(profile) &&
    cleanProbe.length > 0 &&
    !cleanProbe.includes('\0') &&
    !cleanProbe.includes('\n') &&
    !cleanProbe.includes('\r') &&
    new TextEncoder().encode(cleanProbe).length <= 2048;
  return (
    <SheetDialog
      width="wide"
      icon="server"
      title="Add a server"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={!valid || busy}
            onClick={() => {
              setBusy(true);
              void bridge
                .checkAndAddProfile(profile, cleanProbe)
                .then((added) => onAdded(added.profile))
                .catch(onError)
                .finally(() => setBusy(false));
            }}
          >
            Add server
          </Button>
        </>
      }
    >
      <p>
        Add FOKS protocol v019 servers here. The server is saved only after
        online verification succeeds.
      </p>
      <Inset>
        <Field
          label="Server name"
          value={profile}
          placeholder="new-foks"
          onChange={setProfile}
        />
        <Field
          label="Address"
          value={probe}
          placeholder="foks.example.com"
          onChange={setProbe}
        />
      </Inset>
    </SheetDialog>
  );
}
