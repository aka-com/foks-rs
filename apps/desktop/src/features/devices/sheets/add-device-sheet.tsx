/**
 * The Devices page's "Add a device" chooser: pairing in either direction, a
 * recovery phrase or a hardware key.
 */
import { useWorkflowAccess } from '../../../workflow-context';
import type { WorkflowOperation } from '../../../model/workflow-availability';
import { useState } from 'react';
import type { ReactNode } from 'react';
import {
  Button,
  RadioCard,
  RadioGroup,
  SheetDialog,
} from '../../../components';
import type { AccountStore } from '../../../model';

/**
 * What "Add a device" leads to. Pairing is two directions, and
 * each is its own card: this Mac hands out a phrase, or types the one the
 * other Mac is showing.
 */
type AddChoice = 'pair' | 'pair-accept' | 'phrase' | 'provision';

/**
 * The chooser the Devices page's primary action opens: the ways this account
 * gains a key, each leading to the sheet that already did it.
 */
export function AddDeviceSheet({
  store,
  onChoose,
  onClose,
}: {
  store: AccountStore;
  onChoose: (choice: AddChoice) => void;
  onClose: () => void;
}): ReactNode {
  const access = useWorkflowAccess();
  const [choice, setChoice] = useState<AddChoice>('pair');
  const operation: WorkflowOperation =
    choice === 'phrase'
      ? 'backup-create'
      : choice === 'provision'
        ? 'yubi-provision'
        : choice === 'pair-accept'
          ? 'device-accept'
          : 'device-pair';
  const eligibility = access.props(operation, {
    profile: store.server,
    account: choice === 'pair-accept' ? undefined : store.account,
  });
  return (
    <SheetDialog
      width="wide"
      icon="settings"
      title="Add a device"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            {...eligibility}
            onClick={() => {
              if (
                !access.availability(operation, {
                  profile: store.server,
                  account: choice === 'pair-accept' ? undefined : store.account,
                }).available
              )
                return;
              onChoose(choice);
            }}
          >
            Continue
          </Button>
        </>
      }
    >
      <RadioGroup label="What to add">
        <RadioCard
          icon="laptop"
          title="Pair another device"
          detail="Get a pairing phrase here, to type on the device you are adding."
          selected={choice === 'pair'}
          onSelect={() => setChoice('pair')}
        />
        <RadioCard
          icon="laptop"
          title="Pair this device with another account"
          detail="Add this device to another account, by entering the pairing phrase displayed on your other device."
          selected={choice === 'pair-accept'}
          onSelect={() => setChoice('pair-accept')}
        />
        <RadioCard
          icon="file"
          title="Create a new recovery recovery phrase"
          detail="Add a phrase you can recover this account with."
          selected={choice === 'phrase'}
          onSelect={() => setChoice('phrase')}
        />
        <RadioCard
          icon="key"
          title="Connect a hardware key"
          detail="Add a connected hardware key as an authorized device for this account."
          selected={choice === 'provision'}
          onSelect={() => setChoice('provision')}
        />
      </RadioGroup>
    </SheetDialog>
  );
}
