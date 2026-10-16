/**
 * The New item sheet: a login, or a document typed or imported from a file,
 * in a chosen vault and folder. Also holds the save refusal and the discard
 * confirmation that the new-folder and already-exists sheets share.
 */

import { useCallback, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { DismissibleDialog } from '../../../../kit/overlay-primitives';
import {
  Band,
  Button,
  CardSelect,
  DocumentSourceRow,
  DocumentSourceSwitch,
  Field,
  Inset,
  InsetRow,
  KindIcon,
  PasswordInput,
  SectionLabel,
  Sheet,
} from '../../../components';
import type { DocumentSourceKind } from '../../../components';
import { useConcealOnInactive } from '../../../use-conceal-on-inactive';
import {
  kindLabel,
  canCreateInStore,
  defaultCreateStore,
  storeNavigationOrder,
  storeAvailability,
  storeOf,
} from '../../../model';
import { normalizeCommandError } from '../../../bridge';
import type { KvRoleInput } from '../../../bridge';
import type { GuardVerdict } from '../../../location';
import {
  NavigationPrompt,
  useSheetGuard,
  useTabSheetState,
} from '../../../navigation-guard';
import { useFileDrop } from '../file-drop';
import { useToast } from '../../../../kit/toasts';
import { DEFAULT_READ_ROLE, DEFAULT_WRITE_ROLE } from '../item-roles';
import { AccessBlock, storeOption, writeBlockReason } from '../store-choice';
import { useShellServices, useShellSnapshot } from '../../../shell-services';
import { PATH_HINT, fileDropPath, namedPath } from '../write-workflow-model';
import type { NewKind, WriteWorkflow } from '../write-workflow-model';

/** Refusal shown when navigating while a save request is in flight. */
export const SAVING_REFUSAL = 'Wait for the item to finish saving.';

/**
 * Returns confirmation options for discarding an unsaved draft, shared by
 * navigation guards, Escape handling, and backdrop dismissal.
 */
export function discardNewItem(
  itemKind: NewKind,
  onConfirm?: () => void,
): Extract<GuardVerdict, { verdict: 'prompt' }> {
  return {
    verdict: 'prompt',
    title: 'Discard new item?',
    body: `Your new ${kindLabel(itemKind).toLowerCase()} has not been saved.`,
    confirm: 'Discard',
    ...(onConfirm ? { onConfirm } : {}),
  };
}

export interface NewSheetProps {
  workflow: Extract<NonNullable<WriteWorkflow>, { kind: 'new' }>;
  setWorkflow: (workflow: WriteWorkflow) => void;
}

export function NewSheet({ workflow, setWorkflow }: NewSheetProps): ReactNode {
  const snapshot = useShellSnapshot();
  const {
    bridge,
    refresh: onApplied,
    commandError: onError,
    mutationError: onMutationError,
    accessNow,
  } = useShellServices();
  // If the specified store is not available locally, fall back to the default vault.
  const [storeId, setStoreId] = useTabSheetState('item.storeId', () =>
    storeOf(snapshot, workflow.storeId)
      ? workflow.storeId
      : (defaultCreateStore(snapshot) ?? workflow.storeId),
  );
  const [site, setSite] = useTabSheetState(
    'item.site',
    workflow.draft?.site ?? '',
  );
  const [path, setPath] = useTabSheetState(
    'item.path',
    workflow.draft?.path ??
      (workflow.itemKind === 'Password'
        ? namedPath(
            workflow.draft?.site ?? '',
            '/logins',
            workflow.initialFolder,
          )
        : namedPath('', '/', workflow.initialFolder)),
  );
  const [pathShown, setPathShown] = useState(false);
  const [username, setUsername] = useTabSheetState(
    'item.username',
    workflow.draft?.username ?? '',
  );
  const [password, setPassword] = useTabSheetState(
    'item.password',
    workflow.draft?.password ?? '',
  );
  const [passwordShown, setPasswordShown] = useState(false);
  const [website, setWebsite] = useTabSheetState(
    'item.website',
    workflow.draft?.website ?? '',
  );
  const [value, setValue] = useTabSheetState(
    'item.value',
    workflow.draft?.value ?? '',
  );
  const [valueConcealed, setValueConcealed] = useState(false);
  const [resourceName, setResourceName] = useTabSheetState(
    'item.resourceName',
    workflow.draft?.resourceName ?? '',
  );
  const [sourcePath, setSourcePath] = useTabSheetState<string | null>(
    'item.sourcePath',
    workflow.draft?.sourcePath ?? null,
  ); // A document is typed text or an imported file; the switch picks which, and
  // dropping a file selects the File option automatically.
  const [source, setSource] = useTabSheetState<DocumentSourceKind>(
    'item.source',
    workflow.draft?.sourcePath ? 'file' : 'text',
  );
  const [readRole, setReadRole] = useTabSheetState<KvRoleInput>(
    'item.readRole',
    workflow.draft?.readRole ?? DEFAULT_READ_ROLE,
  );
  const [writeRole, setWriteRole] = useTabSheetState<KvRoleInput>(
    'item.writeRole',
    workflow.draft?.writeRole ?? DEFAULT_WRITE_ROLE,
  );
  const [hovering, setHovering] = useState(false);
  const [saving, setSaving] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);
  const store = storeOf(snapshot, storeId);
  const group = store?.kind === 'team';
  const canWrite = Boolean(store && canCreateInStore(snapshot, store.id));
  const blocked = store ? writeBlockReason(snapshot, store) : null;
  const itemKind = workflow.itemKind;
  useConcealOnInactive(
    () => setValueConcealed(true),
    itemKind === 'Document' && source === 'text' && Boolean(value),
  );
  useConcealOnInactive(
    () => setPasswordShown(false),
    itemKind === 'Password' && passwordShown,
  );
  const roleArgs = group ? { readRole, writeRole } : {};

  useFileDrop({
    bridge,
    active: itemKind === 'Document',
    onHover: setHovering,
    onPaths: (paths) => {
      setHovering(false);
      if (paths.length !== 1) {
        if (sourcePath)
          void bridge.releaseImportFile(sourcePath).catch(onError);
        setSourcePath(null);
        setFileError('Drop exactly one file.');
        return;
      }
      const first = paths[0];
      if (!first) return;
      if (sourcePath && sourcePath !== first)
        void bridge.releaseImportFile(sourcePath).catch(onError);
      setSourcePath(first);
      setSource('file');
      setFileError(null);
      const name = first.split(/[\\/]/).at(-1);
      if (name) setPath(fileDropPath(name, workflow.initialFolder));
    },
    onError: (error) => setFileError(normalizeCommandError(error).message),
  });

  /* --------------------------------------------------- unsaved new item -- */

  // Track the initial path to detect unsaved changes. Site and Name fields
  // account for path changes, and changing the item kind alone does not create
  // unsaved content.
  const openedPath = useRef(path).current;
  const hasContent =
    Boolean(
      site ||
      username ||
      password ||
      website ||
      value ||
      resourceName ||
      sourcePath,
    ) || path !== openedPath;
  const close = useCallback(() => {
    if (sourcePath) void bridge.releaseImportFile(sourcePath).catch(onError);
    setWorkflow(null);
  }, [bridge, onError, setWorkflow, sourcePath]);
  // Save requests in flight cannot be aborted. Pending drafts require user
  // confirmation before any action that unmounts the sheet.
  useSheetGuard(
    saving
      ? { verdict: 'refuse', reason: SAVING_REFUSAL }
      : hasContent
        ? discardNewItem(itemKind, close)
        : null,
    !saving,
  );
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);
  const toasts = useToast();
  // Escape and the backdrop are answered the same way a navigation is.
  const dismiss = (): void => {
    if (saving) {
      toasts.show(SAVING_REFUSAL, { tone: 'warning' });
      return;
    }
    if (hasContent) {
      setConfirmingDiscard(true);
      return;
    }
    setWorkflow(null);
  };

  const pathName = path.split('/').at(-1) ?? '';
  const submit = async (): Promise<void> => {
    if (
      !store ||
      !canWrite ||
      !storeAvailability(snapshot, store, { nowSeconds: accessNow() })
        .available ||
      !canCreateInStore(snapshot, store.id) ||
      saving ||
      !path.startsWith('/') ||
      !pathName
    )
      return;
    setSaving(true);
    let uploadSourcePath = sourcePath;
    let created = false;
    try {
      if (itemKind === 'Password') {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value: `user: ${username}\npassword: ${password}\nurl: ${website}`,
          ...roleArgs,
        });
      } else if (source === 'file') {
        if (!uploadSourcePath) {
          uploadSourcePath = await bridge.pickImportFile();
          if (!uploadSourcePath) return;
          setSourcePath(uploadSourcePath);
          setFileError(null);
        }
        await bridge.importDroppedFile({
          storeId: store.id,
          path,
          sourcePath: uploadSourcePath,
          ...roleArgs,
        });
      } else {
        await bridge.createTextItem({
          storeId: store.id,
          path,
          value,
          ...roleArgs,
        });
      }
      created = true;
    } catch (error) {
      const typed = normalizeCommandError(error);
      if (typed.code === 'already-exists') {
        setWorkflow({
          kind: 'exists',
          itemKind,
          storeId: store.id,
          path,
          initialFolder: path.slice(0, path.lastIndexOf('/')) || '/',
          draft: {
            path,
            site,
            username,
            password,
            website,
            value,
            resourceName,
            sourcePath: uploadSourcePath,
            readRole,
            writeRole,
          },
        });
        await onMutationError(error, { report: false });
      } else if (typed.code === 'inactive-group') {
        try {
          await onApplied(
            `Cannot create item: ${store.name} is inactive`,
            store.server,
          );
          setWorkflow(null);
        } catch (refreshError) {
          onError(refreshError);
        }
      } else await onMutationError(error);
    } finally {
      setSaving(false);
    }
    if (!created) return;
    setWorkflow(null);
    try {
      await onApplied(
        `${kindLabel(itemKind)} created in ${store.name}`,
        store.server,
      );
    } catch (error) {
      onError(error);
    }
  };

  const field = (
    label: string,
    current: string,
    set: (next: string) => void,
    placeholder: string,
    mono = false,
  ) => (
    <Field
      label={label}
      value={current}
      onChange={set}
      placeholder={placeholder}
      mono={mono}
    />
  );
  const sheet = (
    <Sheet
      width="mid"
      glyph={<KindIcon kind={itemKind} />}
      title={`New ${kindLabel(itemKind).toLowerCase()}`}
      footer={
        <>
          <Button onClick={dismiss}>Cancel</Button>
          <Button
            variant="primary"
            disabled={
              !canWrite ||
              saving ||
              !pathName ||
              (itemKind === 'Document' && source === 'text' && !value.trim())
            }
            onClick={() => void submit()}
          >
            {saving
              ? 'Creating…'
              : itemKind === 'Document' && source === 'file' && !sourcePath
                ? 'Choose file and create'
                : 'Create item'}
          </Button>
        </>
      }
    >
      <>
        <SectionLabel
          action={
            snapshot.stores.length ? (
              <button
                type="button"
                className="path-toggle"
                aria-expanded={pathShown}
                onClick={() => setPathShown((shown) => !shown)}
              >
                {pathShown ? 'Hide Path' : 'Path'}
              </button>
            ) : undefined
          }
        >
          Save in vault
        </SectionLabel>
        <Inset>
          {snapshot.stores.length ? (
            <CardSelect
              label="Save in vault"
              options={storeNavigationOrder(snapshot).map((candidate) =>
                storeOption(snapshot, candidate),
              )}
              value={storeId}
              onChange={setStoreId}
              placeholder="Choose a destination"
            />
          ) : (
            <InsetRow label="Vault">
              <span className="dim">No vaults available to store items.</span>
            </InsetRow>
          )}
          {snapshot.stores.length && pathShown
            ? field('Path', path, setPath, PATH_HINT[itemKind])
            : null}
        </Inset>
        {store && blocked ? (
          <Band live>{`${store.name}: ${blocked}`}</Band>
        ) : null}
        <SectionLabel
          action={
            itemKind === 'Document' ? (
              <DocumentSourceSwitch value={source} onChange={setSource} />
            ) : undefined
          }
        >
          {kindLabel(itemKind)}
        </SectionLabel>
        <Inset
          className={
            itemKind === 'Document' && hovering ? 'drop-hover' : undefined
          }
        >
          {itemKind === 'Password' ? (
            <>
              {field(
                'Site',
                site,
                (next) => {
                  setSite(next);
                  setPath(namedPath(next, '/logins', workflow.initialFolder));
                },
                'e.g. github.com',
              )}
              {field('User name', username, setUsername, 'username')}
              <PasswordInput
                value={password}
                onChange={setPassword}
                shown={passwordShown}
                onShownChange={setPasswordShown}
                context={[site, username]}
              />
              {field(
                'Website',
                website,
                setWebsite,
                'https://github.com/login',
              )}
            </>
          ) : null}
          {itemKind === 'Document' ? (
            <>
              {field(
                'Name',
                resourceName,
                (next) => {
                  setResourceName(next);
                  if (next)
                    setPath(
                      namedPath(
                        next.toLowerCase().replace(/[^a-z0-9._-]+/g, '-'),
                        '/',
                        workflow.initialFolder,
                      ),
                    );
                },
                'e.g. api-key',
              )}
              <DocumentSourceRow
                source={source}
                value={value}
                onValue={setValue}
                sourcePath={sourcePath}
                hovering={hovering}
                concealed={valueConcealed}
                onReveal={() => setValueConcealed(false)}
              />
            </>
          ) : null}
        </Inset>
        {fileError ? (
          <p className="action-error" role="alert">
            {fileError}
          </p>
        ) : null}
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
    </Sheet>
  );
  return (
    <>
      <DismissibleDialog
        className="backdrop"
        aria-label={`New ${kindLabel(itemKind).toLowerCase()}`}
        onDismiss={dismiss}
      >
        {sheet}
      </DismissibleDialog>
      {confirmingDiscard ? (
        <NavigationPrompt
          verdict={discardNewItem(itemKind)}
          onConfirm={close}
          onCancel={() => setConfirmingDiscard(false)}
        />
      ) : null}
    </>
  );
}
