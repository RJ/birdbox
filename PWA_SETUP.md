# Progressive Web App (PWA) Setup

Your DoorBird intercom is now configured as a Progressive Web App! This provides several benefits:

## Benefits

1. **Better Autoplay Support**: When installed as a PWA, audio autoplay is more likely to work without user interaction
2. **App-Like Experience**: Runs in standalone mode without browser chrome
3. **Home Screen Icon**: Quick access from your phone's home screen
4. **Offline Manifest**: Proper app metadata and theming

## How to Install

### On iOS (iPhone/iPad):
1. Open Safari and navigate to your intercom page (e.g., `http://your-server-ip:3000/intercom`)
2. Tap the **Share** button (square with arrow pointing up)
3. Scroll down and tap **"Add to Home Screen"**
4. Tap **"Add"** in the top right
5. The app icon will appear on your home screen

### On Android:
1. Open Chrome and navigate to your intercom page
2. Tap the **menu** (three dots in top right)
3. Tap **"Add to Home Screen"** or **"Install App"**
4. Tap **"Add"** or **"Install"**
5. The app icon will appear on your home screen

## Features When Installed

- **Standalone Mode**: No browser UI, just your app
- **Black Status Bar**: Matches the dark theme
- **Portrait Lock**: Optimized for vertical use
- **App Title**: Shows "Intercom" in app switcher
- **Theme Colors**: Black background matching the design

## Icon Customization

The app currently uses an SVG icon with a doorbell/intercom design. To customize:

1. Replace `/static/icon.svg` with your own design
2. Optionally generate PNG versions for better compatibility:
   ```bash
   cd static
   # If you have ImageMagick:
   convert -background none icon.svg -resize 192x192 icon-192.png
   convert -background none icon.svg -resize 512x512 icon-512.png
   ```

## Autoplay Behavior

- **In Browser**: May show "Touch to start audio" prompt on first connection
- **As PWA**: Audio should start automatically in most cases
- **iOS**: May still require initial tap due to strict policies, but generally better than browser

## Testing

After installing, launch the app from your home screen and connect. You should see:
- Full-screen experience
- Black status bar
- Better audio autoplay behavior
- No browser address bar or controls

Enjoy your dedicated intercom app! 🎉


