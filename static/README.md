# Static Assets

## PWA Icons

To generate the required PNG icons from the SVG:

### Using ImageMagick (recommended):
```bash
# Install ImageMagick if not already installed
# macOS: brew install imagemagick
# Linux: apt-get install imagemagick

# Generate icons
convert -background none icon.svg -resize 192x192 icon-192.png
convert -background none icon.svg -resize 512x512 icon-512.png
```

### Using online tools:
1. Upload `icon.svg` to https://cloudconvert.com/svg-to-png
2. Convert to 192x192 and save as `icon-192.png`
3. Convert to 512x512 and save as `icon-512.png`

### Custom icons:
You can replace `icon.svg` with your own design and regenerate the PNG files.

## Installing the PWA

### iOS (Safari):
1. Open the intercom page in Safari
2. Tap the Share button
3. Scroll down and tap "Add to Home Screen"
4. Tap "Add"

### Android (Chrome):
1. Open the intercom page in Chrome
2. Tap the menu (three dots)
3. Tap "Add to Home Screen"
4. Tap "Add"

Once installed, launching from the home screen should:
- Run in standalone mode (no browser chrome)
- Have better autoplay permissions
- Feel like a native app


