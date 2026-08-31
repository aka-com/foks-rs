/**
 * Role arithmetic — ported from `wave6/shell.js:203-215`.
 *
 * Member(-0x4000) < Member(0) < Admin < Owner. Visibility is an ordered band
 * inside Member: 0 is the default; lower bands see less.
 */

import type { Role, RoleWire } from './types';

const MEMBER_ROLE = /^Member(?:\s*·\s*visibility\s*(-?\d+))?$/;

const KINDS: Readonly<Record<string, Role['kind']>> = {
  Member: 'member',
  Admin: 'admin',
  Owner: 'owner',
};

/**
 * Normalise a role off the wire.
 *
 * Accepts the display string (`"Owner"`, `"Member · visibility -16384"`,
 * `"Member"`) and the structured `{ role, visibility }`. Returns `null` for
 * anything that is not one of the three protocol roles — the mock let an
 * unknown string through with rank 0; refusing it is the same outcome without
 * inventing a fourth role.
 */
export function parseRole(wire: RoleWire): Role | null {
  if (typeof wire !== 'string') {
    const kind = KINDS[wire.role];
    if (!kind) return null;
    return kind === 'member'
      ? { kind, visibility: wire.visibility ?? 0 }
      : { kind };
  }
  const member = MEMBER_ROLE.exec(wire);
  if (member) {
    return { kind: 'member', visibility: member[1] != null ? +member[1] : 0 };
  }
  const kind = KINDS[wire];
  return kind ? { kind } : null;
}

/** Owner 3, Admin 2, Member 1, anything else 0. Takes a role either way up. */
export function roleRank(role: Role | RoleWire | null): number {
  if (role == null) return 0;
  const parsed =
    typeof role !== 'string' && 'kind' in role ? role : parseRole(role);
  if (!parsed) return 0;
  return { owner: 3, admin: 2, member: 1 }[parsed.kind];
}

/** 0 is the default band; a role outside Member has no band of its own. */
export function visibilityOf(role: Role): number {
  return role.visibility ?? 0;
}

/**
 * Does `held` — a party's destination role — admit reading something whose
 * read role is `need`?
 *
 * A higher rank always admits a lower one. At equal rank only Member has a
 * band to compare, and a party admits a band at or below its own.
 */
export function admits(held: RoleWire, need: RoleWire): boolean {
  const h = parseRole(held);
  const n = parseRole(need);
  const hr = roleRank(h);
  const nr = roleRank(n);
  if (hr !== nr) return hr > nr;
  if (!h || !n) return false;
  return h.kind === 'member' ? visibilityOf(h) >= visibilityOf(n) : true;
}

/** "Owner" / "Member · visibility 0" — the wording the design uses. */
export function formatRole(role: Role): string {
  switch (role.kind) {
    case 'owner':
      return 'Owner';
    case 'admin':
      return 'Admin';
    case 'member':
      return `Member · visibility ${visibilityOf(role)}`;
  }
}
