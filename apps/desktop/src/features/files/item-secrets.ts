/**
 * The details panel's item plaintext: the value Show reveals and an open
 * editor's draft, with every transition that keeps or clears them in one
 * reducer. `PLAINTEXT_FIELDS` names each field that holds plaintext, so a
 * test can check every concealing transition against it.
 */

import { useCallback, useEffect, useReducer, useRef } from 'react';
import type { RefObject } from 'react';
import type { Item } from '../../model';

/** One item version's Show state, keyed by `store|path|version`. */
export type ReadState =
  | { key: string; state: 'loading' }
  | { key: string; state: 'shown'; value: string }
  | { key: string; state: 'error'; message: string };

/**
 * The item, scope identity and access generation an editor opened under. A
 * save writes against this item's version, and is refused once the scope or
 * the generation moves.
 */
export interface EditBinding {
  item: Item;
  scope: string;
  generation: number;
}

export interface Editor {
  /** The draft. */
  value: string;
  /**
   * The value the editor opened from, against which `value` is unsaved work.
   * Null for a draft restored from a conflict, whose stored counterpart is by
   * definition not what it holds.
   */
  baseline: string | null;
  binding: EditBinding;
  /** The lines the structured login editor lists, or null for the textarea. */
  loginLines: readonly number[] | null;
  /** The draft is masked behind one Contents row after focus loss. */
  concealed: boolean;
  passwordShown: boolean;
  /** A file dropped onto Replace, by its local path. */
  replacementPath: string | null;
}

export interface ItemSecrets {
  read: ReadState | null;
  /** The login rows shown while `read` holds the text; meaningless otherwise. */
  revealed: readonly string[];
  editor: Editor | null;
  /** The editor's error or notice; set by a refused write while closed. */
  editError: string | null;
  /** The selected Secret reads as bytes, so it is edited as a file. */
  binaryFile: boolean;
}

export const EMPTY: ItemSecrets = {
  read: null,
  revealed: [],
  editor: null,
  editError: null,
  binaryFile: false,
};

/**
 * Every field of `ItemSecrets` that holds item plaintext, as a dotted path.
 * The reducer test classifies each string the state can hold as plaintext,
 * from this list, or as a named non-plaintext field; a new string field fails
 * that test until it is listed in one of the two.
 */
export const PLAINTEXT_FIELDS = [
  'read.value',
  'editor.value',
  'editor.baseline',
] as const;

/** The non-empty plaintext strings `state` holds, from `PLAINTEXT_FIELDS`. */
export function plaintextOf(state: ItemSecrets): string[] {
  return PLAINTEXT_FIELDS.flatMap((path) => {
    const [owner, field] = path.split('.') as [keyof ItemSecrets, string];
    const value = (state[owner] as Record<string, unknown> | null)?.[field];
    return typeof value === 'string' && value !== '' ? [value] : [];
  });
}

export const CHANGED_ON_SERVER =
  'This item changed on the server. Saving will check the original version.';

export type SecretsAction =
  /** Show started reading `key`. */
  | { type: 'reveal-start'; key: string }
  | { type: 'reveal-done'; key: string; value: string }
  | { type: 'reveal-fail'; key: string; message: string }
  /** Login rows to show; `keep` adds them to the rows already shown. */
  | { type: 'show-rows'; ids: readonly string[]; keep: boolean }
  /** Masks one login row while another stays shown. */
  | { type: 'hide-row'; id: string }
  /** Hide: drops the revealed value. */
  | { type: 'hide-value' }
  /** A Secret read as bytes; it is edited as a file from now on. */
  | { type: 'not-text' }
  /** Read access is refused for the selected item. */
  | { type: 'read-blocked' }
  /** Access moved generation; a read still loading is dropped when it lands. */
  | { type: 'generation-changed' }
  /**
   * The selected item, its version, scope or the access generation changed.
   * An editor still bound to `scope` and `generation` is kept.
   */
  | { type: 'item-changed'; scope: string; generation: number }
  /** The window lost focus; `file` is true for a File node. */
  | { type: 'conceal-inactive'; file: boolean }
  /** The shell asked for every revealed value and draft to be cleared. */
  | { type: 'conceal-signal' }
  | {
      type: 'open-editor';
      value: string;
      loginLines: readonly number[] | null;
      binding: EditBinding;
    }
  /** Reopens the editor on a draft retained across a conflict. */
  | {
      type: 'restore-draft';
      value: string;
      loginLines: readonly number[] | null;
      binding: EditBinding;
    }
  | { type: 'edit'; value: string }
  | { type: 'show-password'; shown: boolean }
  | { type: 'unconceal-draft' }
  | { type: 'replacement'; path: string | null; error: string | null }
  | { type: 'edit-error'; message: string | null }
  /** Cancel, a confirmed discard, or a completed save. */
  | { type: 'close-editor' };

/** True when `editor` may still save under this scope and generation. */
export function editorBound(
  editor: Editor | null,
  scope: string,
  generation: number,
): boolean {
  return (
    editor !== null &&
    editor.binding.scope === scope &&
    editor.binding.generation === generation
  );
}

const UNREAD = { read: null, revealed: [] } as const;

function withEditor(
  state: ItemSecrets,
  change: (editor: Editor) => Partial<Editor>,
): ItemSecrets {
  return state.editor
    ? { ...state, editor: { ...state.editor, ...change(state.editor) } }
    : state;
}

export function itemSecrets(
  state: ItemSecrets,
  action: SecretsAction,
): ItemSecrets {
  switch (action.type) {
    case 'reveal-start':
      return { ...state, read: { key: action.key, state: 'loading' } };
    case 'reveal-done':
      return {
        ...state,
        read: { key: action.key, state: 'shown', value: action.value },
      };
    case 'reveal-fail':
      return {
        ...state,
        read: { key: action.key, state: 'error', message: action.message },
      };
    case 'show-rows':
      return {
        ...state,
        revealed: action.keep
          ? [...new Set([...state.revealed, ...action.ids])]
          : [...action.ids],
      };
    case 'hide-row':
      return {
        ...state,
        revealed: state.revealed.filter((id) => id !== action.id),
      };
    case 'hide-value':
      return { ...state, ...UNREAD };
    case 'not-text':
      return { ...state, ...UNREAD, binaryFile: true };
    case 'read-blocked':
      return withEditor({ ...state, ...UNREAD }, () => ({
        passwordShown: false,
      }));
    case 'generation-changed':
      return state.read?.state === 'loading' ? { ...state, ...UNREAD } : state;
    case 'item-changed': {
      const unread = { ...state, ...UNREAD };
      if (editorBound(state.editor, action.scope, action.generation))
        return withEditor({ ...unread, editError: CHANGED_ON_SERVER }, () => ({
          passwordShown: false,
        }));
      return { ...unread, editor: null, editError: null, binaryFile: false };
    }
    case 'conceal-inactive': {
      const coverDraft = !action.file && !state.binaryFile;
      return withEditor({ ...state, ...UNREAD }, (editor) => ({
        passwordShown: false,
        concealed: editor.concealed || coverDraft,
      }));
    }
    case 'conceal-signal':
      return { ...state, ...UNREAD, editor: null };
    case 'open-editor':
      return {
        ...state,
        ...UNREAD,
        editor: {
          value: action.value,
          baseline: action.value,
          binding: action.binding,
          loginLines: action.loginLines,
          concealed: false,
          passwordShown: false,
          replacementPath: null,
        },
      };
    case 'restore-draft':
      return {
        ...state,
        editor: {
          value: action.value,
          baseline: null,
          binding: action.binding,
          loginLines: action.loginLines,
          concealed: false,
          passwordShown: state.editor?.passwordShown ?? false,
          replacementPath: state.editor?.replacementPath ?? null,
        },
      };
    case 'edit':
      return withEditor(state, () => ({ value: action.value }));
    case 'show-password':
      return withEditor(state, () => ({ passwordShown: action.shown }));
    case 'unconceal-draft':
      return withEditor(state, () => ({ concealed: false }));
    case 'replacement':
      return withEditor({ ...state, editError: action.error }, () => ({
        replacementPath: action.path,
      }));
    case 'edit-error':
      return { ...state, editError: action.message };
    case 'close-editor':
      return { ...state, editor: null, editError: null };
  }
}

/**
 * Actions after which a read or edit-open still in flight must be dropped.
 * The reducer test checks that every other action which clears `read` is one
 * of the transitions the panel guards by other means.
 */
export const CONCEALING: ReadonlySet<SecretsAction['type']> = new Set([
  'hide-value',
  'item-changed',
  'conceal-inactive',
  'conceal-signal',
]);

/**
 * The details panel's plaintext state. `epoch` advances on every concealing
 * action and on unmount; a read records it when it starts and is dropped if
 * it has moved by the time the response lands.
 */
export function useItemSecrets(): {
  secrets: ItemSecrets;
  dispatch: (action: SecretsAction) => void;
  epoch: RefObject<number>;
} {
  const [secrets, send] = useReducer(itemSecrets, EMPTY);
  const epoch = useRef(0);
  const dispatch = useCallback((action: SecretsAction): void => {
    if (CONCEALING.has(action.type)) epoch.current += 1;
    send(action);
  }, []);
  useEffect(
    () => () => {
      epoch.current += 1;
    },
    [],
  );
  return { secrets, dispatch, epoch };
}
