/** The standing condition the group page's tabs state above their content. */

import type { ReactNode } from 'react';
import { Band } from '../../components';
import type { Store } from '../../model';
import type { GroupSettingsTab } from '../../location';

/**
 * The one standing condition a tab states before its own content. A group
 * whose setup never finished never reaches a tab at all, so the only band left
 * here is the ad-hoc share's fixed membership.
 */
export function SituationBand({
  store,
  tab,
}: {
  store: Store;
  tab: GroupSettingsTab;
}): ReactNode {
  if (
    store.kind === 'team' &&
    store.team_kind === 'adhoc' &&
    tab !== 'settings'
  ) {
    return (
      <Band severity="info" label="Ad-hoc team">
        Memberships can’t be changed.
      </Band>
    );
  }
  return null;
}
