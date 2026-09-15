/**
 * Role parsing, ranking, and admission rules.
 *
 * Role hierarchy: Member (ordered by visibility band) < Admin < Owner.
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
 * Accepts display strings (`"Owner"`, `"Member · visibility 0"`) or structured
 * `{ role, visibility }` objects. Returns `null` for unparseable or unknown roles.
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

/** Returns the numeric permission rank for a role (Owner: 3, Admin: 2, Member: 1, invalid: 0). Accepts either a Role or RoleWire representation. */
export function roleRank(role: Role | RoleWire | null): number {
  if (role == null) return 0;
  const parsed =
    typeof role !== 'string' && 'kind' in role ? role : parseRole(role);
  if (!parsed) return 0;
  return { owner: 3, admin: 2, member: 1 }[parsed.kind];
}

/** Returns the visibility level for a member role, defaulting to 0. Non-member roles return 0. */
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
  // Reject unparseable roles on either side before comparing ranks.
  if (!h || !n) return false;
  const hr = roleRank(h);
  const nr = roleRank(n);
  if (hr !== nr) return hr > nr;
  return h.kind === 'member' ? visibilityOf(h) >= visibilityOf(n) : true;
}

/** Formats a role into its canonical display string. */
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

/** The role's bare name, with no visibility band: what a row's chip says. */
export function roleName(role: Role): string {
  return role.kind === 'member' ? 'Member' : formatRole(role);
}

/**
 * The role as a member row's chip reads it. The visibility band rides inside
 * the chip — "Member (0)" — so a row stays one line of role.
 */
export function roleChipLabel(role: Role): string {
  return role.kind === 'member'
    ? `Member (${visibilityOf(role)})`
    : formatRole(role);
}
