import { useCallback, useEffect, useState } from 'react';
import type { Bridge } from '../bridge';
import type { Location, LocationStore } from '../location';
import { applyRailColor, storedRailColor } from '../rail-theme';
import {
  rememberSideCollapsed,
  storedSideCollapsedPref,
} from '../sidebar-prefs';
import type { CommandErrorHandler } from './catalog-runtime';

export function useWindowRuntime({
  bridge,
  locations,
  here,
  commandError,
}: {
  bridge: Bridge;
  locations: LocationStore;
  here: Location;
  commandError: CommandErrorHandler;
}) {
  const [sideCollapsed, setSideCollapsed] = useState(storedSideCollapsedPref);
  // Initialize the rail color theme from local storage on initial render.
  // Settings applies preference changes directly upon user selection.
  useEffect(() => {
    applyRailColor(storedRailColor());
  }, []);
  // The explicit rail/topbar toggle is the only interaction that changes the
  // width, and the chosen width persists across navigation and item details.
  const toggleSidebar = useCallback(() => {
    const collapsed = !sideCollapsed;
    setSideCollapsed(collapsed);
    rememberSideCollapsed(collapsed);
  }, [sideCollapsed]);
  const [windowChromeHidden, setWindowChromeHidden] = useState(false);

  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void bridge
      .onOpenSettings(() => locations.navigate({ kind: 'settings' }))
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [bridge, locations]);

  useEffect(() => {
    if (!bridge.native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const apply = ({
      maximized,
      fullscreen,
    }: Awaited<ReturnType<Bridge['windowState']>>): void => {
      if (!disposed) setWindowChromeHidden(maximized || fullscreen);
    };
    void bridge
      .onWindowState(apply)
      .then(async (unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        stop = unlisten;
        apply(await bridge.windowState());
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      stop?.();
    };
  }, [bridge]);

  // The checklist screens draw the shell's own rail, collapse toggle and all.
  // The setup steps draw a rail with no toggle, which stays open.
  const firstRunRailCollapsible =
    here.kind === 'first-run' &&
    ['added', 'checklist-invited', 'checklist-own'].includes(here.step ?? '');
  const railCollapsed =
    sideCollapsed && (here.kind !== 'first-run' || firstRunRailCollapsible);
  const trafficLightsVisible = !railCollapsed;
  useEffect(() => {
    if (bridge.native)
      void bridge
        .setTrafficLightsVisible(trafficLightsVisible)
        .catch(commandError);
  }, [bridge, commandError, trafficLightsVisible]);
  // Scrollbars show while a pane is scrolling and for 700ms after. Scroll
  // events do not bubble, so the listener is capture-phase; that also covers
  // keyboard scrolling, which no pointer event would report.
  useEffect(() => {
    const timers = new WeakMap<Element, number>();
    const onScroll = (event: Event): void => {
      const pane = event.target;
      if (!(pane instanceof HTMLElement)) return;
      pane.classList.add('scrolling');
      window.clearTimeout(timers.get(pane));
      timers.set(
        pane,
        window.setTimeout(() => pane.classList.remove('scrolling'), 700),
      );
    };
    document.addEventListener('scroll', onScroll, true);
    return () => document.removeEventListener('scroll', onScroll, true);
  }, []);

  return { sideCollapsed, railCollapsed, toggleSidebar, windowChromeHidden };
}
