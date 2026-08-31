# Placeholder icons

Every file here is a copy of AKA's application icon
(`src-tauri/icons/icon.png`), resized by `tauri icon`. They exist so the bundle
targets resolve and the deb's hicolor theme entries are well-formed — **not**
because FOKS should ship AKA's mark.

Replace all of them with FOKS artwork before the first packaged build. The
regeneration step is:

    pnpm exec tauri icon path/to/foks-icon.png -o foks-tauri/icons

then keep only the files `../tauri.conf.json` lists (`32x32.png`,
`128x128.png`, `128x128@2x.png`, `icon.icns`) plus `icon.png`, and delete the
Android, iOS, Windows Store and `.ico` output this app has no target for.
