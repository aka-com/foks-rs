# FOKS app icon

`app-icon.svg` is the canonical source. It uses the sidebar's white Lucide paw
mark on alert orange (`#ff6f21`, from `apps/desktop/kit/tokens.css`), with a
macOS rounded-square silhouette, transparent margins, a fine highlight rim,
and a soft shadow. The glyph is outlined directly in the SVG, so generation
does not depend on installed fonts.

Regenerate from the repository root into a temporary directory, then copy only
the assets used by this app:

```sh
icon_output=$(mktemp -d)
npm exec -- tauri icon apps/desktop/src-tauri/icons/app-icon.svg -o "$icon_output"
for icon in 32x32.png 128x128.png 128x128@2x.png icon.icns icon.png; do
  cp "$icon_output/$icon" "apps/desktop/src-tauri/icons/$icon"
done
rm -r "$icon_output"
```

`../tauri.conf.json` lists the bundled PNG sizes and `icon.icns`; `icon.png` is
the 512px raster preview. Android, iOS, Windows Store, and `.ico` output
are not used by this app.
