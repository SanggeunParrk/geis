# Geis app icon

`geis-icon.png` is the production master: cobalt ribbon, white tile, transparent
outside its rounded perimeter. The approved unmasked artwork is preserved at
`../../output/imagegen/mini-icon-01-cobalt.png`.

Recreate the alpha mask without regenerating the artwork:

```sh
swift -module-cache-path /tmp/geis-swift-module-cache assets/branding/prepare-icon.swift output/imagegen/mini-icon-01-cobalt.png assets/branding/geis-icon.png
npm run tauri -- icon assets/branding/geis-icon.png --output /tmp/geis-cobalt-app-icons
cp /tmp/geis-cobalt-app-icons/*.png /tmp/geis-cobalt-app-icons/icon.icns /tmp/geis-cobalt-app-icons/icon.ico src-tauri/icons/
```

The mask removes the original gray perimeter and outside background. Keep the
transparent margins when exporting platform icons; do not flatten onto white.
