import { sameLocation } from './routes';
import type { LocationAction, LocationState } from './types';

/**
 * Pure transition function for location state actions.
 *
 * Navigating to a new location clears the current item selection while
 * preserving active search queries. Re-navigating to the current location
 * preserves the active selection.
 */
export function transition(
  state: LocationState,
  action: LocationAction,
): LocationState {
  switch (action.type) {
    case 'navigate':
      if (sameLocation(state.location, action.location)) return state;
      // A replacement moves within the place the reader is in, so the item
      // they had selected and the folder they had open are still theirs.
      if (action.replace) return { ...state, location: action.location };
      return {
        ...state,
        location: action.location,
        ...(state.sheet ? { sheet: undefined } : {}),
        selection: null,
        folder: '',
        closedFolders: [],
        query:
          action.location.kind === 'chat' || state.location.kind === 'chat'
            ? ''
            : state.query,
      };
    case 'select':
      // Selecting an item automatically opens the details panel; deselecting keeps the panel open.
      return {
        ...state,
        selection: action.selection,
        details: action.selection
          ? true
          : action.closeDetails
            ? false
            : state.details,
      };
    case 'search':
      return state.query === action.query
        ? state
        : { ...state, query: action.query };
    case 'view':
      return state.view === action.view
        ? state
        : { ...state, view: action.view };
    case 'details':
      return state.details === action.open
        ? state
        : { ...state, details: action.open };
    case 'kind':
      return state.kind === action.kind
        ? state
        : { ...state, kind: action.kind };
    case 'sort':
      return state.sort === action.sort &&
        state.sortDirection === (action.direction ?? 'asc')
        ? state
        : {
            ...state,
            sort: action.sort,
            sortDirection: action.direction ?? 'asc',
          };
    case 'folder':
      return state.folder === action.folder && state.selection === null
        ? state
        : { ...state, folder: action.folder, selection: null };
    case 'toggle-folder': {
      const closed = new Set(state.closedFolders);
      if (closed.has(action.folder)) closed.delete(action.folder);
      else closed.add(action.folder);
      return { ...state, closedFolders: [...closed].sort() };
    }
  }
}
