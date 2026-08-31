# wave7-servers — four from-scratch Servers pages, and two simple ones

Read `DESIGN.md` for the theses, costs and the fold-in assessment against
`../wave6/04-servers.html`. Every file accepts `?state=` and renders inside the
wave 6 window frame with the review strip outside it.

- `01-ledger.html` — the server as this Mac's ordered record; the present is the newest rows. On the wave 6 shell. States: `list · server · lapsed · rollback · reset · add · unprobed · check`
- `02-trust.html` — "can this Mac trust what it reads here right now?", four conditions, failing one first. On the wave 6 shell. States: `list · server · lapsed · rollback · reset · add · unprobed · check`
- `03-identity.html` — one lane per server: your account → Macs → vault → groups; server facts in a drawer. Standalone chrome. States: `list · server · lapsed · rollback · reset · add · unprobed · check`
- `04-inspector.html` — one dense table, one inspector, keyboard-driven (↑ ↓ ⌘R ⌘C ⌘N). Standalone chrome. States: `list · server · lapsed · rollback · reset · add · unprobed · check`
- `05-status-list.html` — the simple one: rows with one status chip and its date, a second line only when a server is not fine; five plain rows on the page, the rest under Details. On the wave 6 shell. States: `list · server · lapsed · rollback · reset · add · unprobed · check`
- `06-verdict-cards.html` — the other simple one: one card per server, the verdict in plain words first, two sentences and one action; the page is the same card over four rows and Details. On the wave 6 shell. States: `list · server · lapsed · rollback · reset · add · unprobed · check`

Render check: `node <scratch>/shot.mjs <file> <out.png> "?state=<state>"` —
every state above renders with zero console errors and `scrollWidth === 1280`.
