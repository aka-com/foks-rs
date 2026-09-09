# Placeholder icons

These placeholder files let the bundle targets resolve and keep the Debian
hicolor theme entries well-formed. Replace them with final FOKS artwork before
the first packaged build. The regeneration step is:

    pnpm exec tauri icon path/to/foks-icon.png -o foks-tauri/icons

then keep only the files `../tauri.conf.json` lists (`32x32.png`,
`128x128.png`, `128x128@2x.png`, `icon.icns`) plus `icon.png`, and delete the
Android, iOS, Windows Store and `.ico` output this app has no target for.
