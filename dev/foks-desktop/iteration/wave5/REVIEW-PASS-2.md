# Wave 5 — independent review, pass 2

Verified against `REVIEW-PASS-1.md`, `DESIGN.md` §1–§4 and the current shared
files. Thirty-three states rendered, no console errors, `scrollWidth` 1280.

## 1. Pass-1 findings

| # | Verdict | Evidence |
| --- | --- | --- |
| 1 deploy-bot removed vs. on the roster | **Resolved** | 06 `?state=all`: the Rotate card is now *Rotate 3 items dana.okafor could read*, item-for-item the list 03 `?state=remove` promises. deploy-bot appears nowhere in it. |
| 2 band runs two directions / band ≥ 1 dead end | **Resolved (other way round)** | Both steppers call `clampVis` (−16384…16383) over the same `FX.copy.visibility`: 02 `?state=new` "Members at band 1 · Members at this band or above"; 03 `?state=add` "Visibility band − 0 +". One axis, and a Member can now be raised, so band ≥ 1 is reachable. See N1 for the damage. |
| 3 keys row prints a fact the wire lacks | **Resolved** | 05 `?state=keys`: enrolled row is `primary key / foks.example.net · enrolled from this Mac`; serial only under *Inspect enrolled list* and on the Connected-now row; no Connected chip. |
| 4 "Join a team" over "no join button" | **Resolved** | 03 `?state=teams`: the button reads *How to get added* and is quiet. |
| 5 Home tiles disagree with the rail | **Resolved** | 01 `?state=home`: "2 accounts on 3 servers"; Attention tile "4 need you" with exactly the four rows the badge (4) counts; the lease is one of the four in 06 too. |
| 6 default views hide the safety state | **Resolved (by inverting it)** | `FX.servers[acme].lease` is now `fresh` and each file calls `setLease` for the world it draws, saying so in its note. No screen silently overrides the fixture. Cross-file cost in N3. |
| 7 selected rows lose secondary text | **Resolved** | `system.css:127` covers `.sub/.faint/.tiny/.muted` at 85 % white; 02 `?state=login` `/logins` and 03 `?state=people` deploy-bot's em-dash both legible. |
| 8 "profile" in user copy | **Resolved** | 04 `?state=add` "Adding writes this server's record on this Mac"; Danger "the durable directory for this server (its profile)"; toast "Server added". |
| 9 two Reset sheets | **Resolved** | `shared.js:openResetSheet`; 04 `?state=reset` and 06 `?state=rollback` → Reset render identical text, five rows, placeholder = the server name. 04 rollback now lands on the banner, sheet closed. |
| 10 empty state promises six sources | **Resolved** | 06 `?state=empty` lists seven, including "Steps you skipped in Get started". |
| 11 pairing drawn as a live primary | **Resolved** | 05 `?state=macs`: plain, dimmed *Start…* beside *not in the desktop yet*; account switcher added, 01's step 5 now reads "for rae". |
| 12 the same object drawn two ways | **Not** | 01 `?state=home` Homelab: amber-tinted card, avatar stack, role chip. 03 `?state=teams` Homelab: white tile, *Inactive* chip top-right. Still two `teamCard()`s, one per file. |
| 13 truncated New-sheet summary | **Resolved** | 02 `?state=new` footer wraps to two lines, "…created only if nothing is there yet" fully visible. |
| 14 two "N of M" chips | **Resolved** | 02 shows `Readable by 6 ▾`; 03 keeps `Reads · of 4 items` with M in the header. |
| Priya 3 (install link) | **Resolved** | 03 `?state=invite`: step 1 is the install URL, with an *Install link · placeholder* row saying it is set at release. |
| Marcus 1 (where to put things) | **Resolved** | 02 `?state=file`: a Deferred store-header line ("put new things in Personal for now") and a real drop zone in the New sheet's Document kind. |
| Marcus 2 (one box, "Groups") | **Partly, by DESIGN's choice** | 03 `?state=create` is still Name + Server + Kind, and the word is "team". Unchanged and defensible. |
| Sol 3 (colour of consequential actions) | **Partly** | Create team is now a plain button — but 04 `?state=list` still gives *Add server…* the same blue as *Check now*, two primaries on one surface (N8). |

## 2. New and newly noticed, ranked

**N1. 03 `?state=demote`, any Member target: the demote sheet promotes.**
Open *Change role* on `dana.okafor` (Member · 0) and press **+** three times: the
header still says "New role — strictly below Member · 0", the option still says
"Same role, lower band: reads less", and the sheet now reads
`Member · 0 → Member · 3` with the button *Demote to Member · 3*. Opening the
clamp to `VIS_MAX` for finding 2 removed the only thing that made the demote
stepper a demotion, on the same sheet that says twice there is no
promote-in-place. Fix: bound the demote stepper by the current role —
`max = (cur.role === "Member" ? cur.visibility - 1 : VIS_MAX)` — and disable
**+** at the bound. The add and admit steppers keep the full range.

**N2. Refused pages keep server-touching controls live.** 04 `?state=lapsed` and
05 `?state=account` both say "Nothing on this server can be read or changed from
this Mac" and then offer *Set… / Change… / Verify…* on the passphrase row, where
Verify is described as running the server's login challenge; 02 `?state=lease`
keeps **New** as a live blue primary over a store that lists nothing. Rule 4 says
the page refuses rather than greys, and 04 `?state=rollback` already does it
properly. Fix: apply the same inert pass on lapsed as on rollback, and disable
New with "nothing can be written here until the check-in is fresh".

**N3. The rail badge is a constant while each file picks its own lease world.**
`shared.js:rail()` hard-codes `attention: 4`; only 06 computes it. So 02
`?state=login`, 03 `?state=teams` and 05 `?state=keys` — all fresh worlds, where
the needs-you set is three — carry a badge of 4 whose fourth member is the lapsed
check-in the screen says is fresh; and Home (lapsed: "Engineering can't be
synced") sits one click from Items' default (fresh: Engineering, 4 items). Fix:
have each file pass `badges:{attention: needs}` computed from its own lease state
(3 fresh / 4 lapsed), or drive the whole set from one `?lease=` switch.

**N4. Two counts for one roster inside one file.** 03 `?state=teams` Engineering
card: "6 people and teams"; 03 `?state=people` header: "4 people · 1 team ·
1 machine". 02 `?state=file`: hero "shared with 6 people and teams", store header
"shared with 5 people and 1 team". Fix: one `peopleText(ps)` helper in
`shared.js`, used by both cards and both headers.

**N5. Finding 12 stands** (see the table). Fix as pass 1 said: promote 03's tile
into `shared.js` as `teamCard()` and let Home call it.

**N6. One fact, two severities.** 04 draws a lapsed check-in amber — amber stripe
and `chip warn` "Server check-in lapsed" — while 01, 02, 03 and 06 draw it red
and crit. 04 `?state=lapsed` shows both at once: amber chip in the page head over
a red banner saying nothing can be read. Fix: make `health()` return the danger
stripe and a `chip bad` for a lapsed lease.

**N7. dana's two stories are reconciled only inside a collapsed detail.** 06
`?state=all` shows "Engineering's roster already lists dana.okafor" three cards
above "dana.okafor was removed from Engineering and the team rekeyed"; the
sentence that joins them ("the addition above is a new one, from after the
removal") is inside the Rotate card's *detail*, unopened by default. The fixture
does not back the order either: dana is `generation 5` while the parties that
survived the rekey are 6. Fix: put the clause on the face of the addition card,
and give dana `generation: 7`.

**N8. One primary per surface.** 04 `?state=list` has two blue buttons (*Add
server…*, *Check now*); 03 `?state=teams` now has none. Fix: demote *Check now*
to a plain button, or make Create team the Teams-list primary again, so the two
list screens read alike.

**N9. Bare wire names in body copy, only in 05 and 04.** 05 `?state=macs`:
"StartDevicePairing → AcceptDevicePairing there → FinishDevicePairing here" and
"ListDevices lists, ProvisionOwnerDevice and pairing add" sit in row subtext; 04
`?state=server` has "the signed compatibility lease ListProfiles carries" and
"ListAccounts returns aliases only". Elsewhere such names live in a `<code>`-set
*From* line, a band or parentheses. Fix: do the same here.

**N10. Small things.** The shared Reset sheet says "this **banner** is drawn as it
will read then" while being a sheet. 04's `wire()` adds a `$("win")` click
listener on every `go()`, so sheet toasts fire once per prior navigation. 05's
header comment still quotes Home as "…for rae.chen". 05's *Start…* carries
`.disabled` yet opens the sheet. `deploy-bot` is still `locally_manageable:true`,
so the case Jun's ask was written about is still absent from the fixture.

## 3. Verdict

**Not ready to sign off** — one contract-breaking defect and one rule-4 breach,
both cheap to fix. Must-fix before sign-off:

1. **N1** — bound the demote stepper below the current role; it currently promotes.
2. **N2** — make lapsed pages refuse like rollback does (04 lapsed, 05 account, 02's New).
3. **N3** — compute the Attention badge per file so it stops counting a check-in the screen calls fresh.
4. **N5** — one `teamCard()` in `shared.js`.

N4, N6 and N7 are the next three, a few lines each; the rest can ride.
