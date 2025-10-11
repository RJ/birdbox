# Progressive Web App (PWA)

Birdbox is configured as a Progressive Web App, providing an app-like experience when installed on mobile devices.

## Benefits

- **Improved Autoplay**: Better audio autoplay support without user interaction
- **Full-Screen Mode**: Runs without browser UI chrome
- **Home Screen Access**: Quick launch from device home screen
- **App-Like Feel**: Standalone window with proper app theming
- **Offline Manifest**: Proper app metadata and icon

## Installation

### iOS (iPhone/iPad)

1. **Open in Safari**: Navigate to `http://your-server-ip:3000/intercom`
2. **Tap Share**: The square icon with arrow pointing up
3. **Scroll down**: Find "Add to Home Screen"
4. **Tap Add**: Confirm in top right
5. **Launch**: New app icon appears on your home screen

**Note**: iOS requires Safari for PWA installation. Chrome/Firefox on iOS won't show the install option.

### Android

1. **Open in Chrome**: Navigate to `http://your-server-ip:3000/intercom`
2. **Tap menu**: Three dots in top right corner
3. **Select**: "Add to Home Screen" or "Install App"
4. **Tap Add/Install**: Confirm the installation
5. **Launch**: New app icon appears on your home screen

### Desktop (Chrome/Edge)

1. **Open in browser**: Navigate to the intercom page
2. **Look for install icon**: In address bar (⊕ or install icon)
3. **Click install**: Follow the prompt
4. **Launch**: App appears in your app drawer/start menu

## PWA Features

When installed as a PWA, the app:

- **Standalone display**: No browser address bar or buttons
- **Black status bar**: Matches dark theme aesthetic
- **Portrait orientation**: Optimized for vertical mobile use
- **Custom app name**: Shows "Intercom" in app switcher
- **Theme colors**: Black background consistent with design
- **Proper icon**: Doorbell/intercom icon

## Manifest Configuration

The PWA behavior is controlled by `/static/manifest.json`:

```json
{
  "name": "Intercom",
  "short_name": "Intercom",
  "description": "DoorBird Intercom",
  "start_url": "/intercom",
  "display": "standalone",
  "background_color": "#000000",
  "theme_color": "#000000",
  "orientation": "portrait",
  "icons": [
    {
      "src": "/static/icon.svg",
      "sizes": "any",
      "type": "image/svg+xml"
    }
  ]
}
```

## Customizing the Icon

### Current Icon

The app uses `/static/icon.svg` - a simple doorbell/intercom design.

### Changing the Icon

**Option 1: Replace SVG**
```bash
# Replace with your own SVG
cp your-icon.svg static/icon.svg
```

**Option 2: Generate PNG Icons**

For better compatibility, generate PNG versions:

```bash
cd static

# Using ImageMagick
brew install imagemagick  # macOS
# or: apt-get install imagemagick  # Linux

# Generate different sizes
convert -background none icon.svg -resize 192x192 icon-192.png
convert -background none icon.svg -resize 512x512 icon-512.png
```

Then update `manifest.json`:
```json
"icons": [
  {
    "src": "/static/icon-192.png",
    "sizes": "192x192",
    "type": "image/png"
  },
  {
    "src": "/static/icon-512.png",
    "sizes": "512x512",
    "type": "image/png"
  }
]
```

**Option 3: Online Tools**

If you don't have ImageMagick:
1. Upload `icon.svg` to https://cloudconvert.com/svg-to-png
2. Convert to 192x192 and save as `icon-192.png`
3. Convert to 512x512 and save as `icon-512.png`
4. Update manifest as shown above

### Icon Design Guidelines

**Recommended specifications**:
- **Format**: SVG (scalable) or PNG (multiple sizes)
- **Sizes**: 192x192 (minimum), 512x512 (high-res)
- **Background**: Transparent or solid color
- **Design**: Simple, recognizable at small sizes
- **Colors**: Contrast well with home screen

**iOS** may add its own background and rounding. Design accordingly.

## Autoplay Behavior

### In Browser

- May show "Touch to start audio" prompt on first connection
- Browser policies restrict autoplay without user interaction
- Varies by browser and device

### As PWA

- Audio autoplay generally works better
- Still subject to platform policies
- iOS may still require initial tap (stricter than Android)
- Typically more permissive than in-browser experience

**Why PWA helps**:
- Installed apps are considered "trusted" by browsers
- User has explicitly installed the app (shows intent)
- Standalone mode often has relaxed autoplay policies

### Troubleshooting Autoplay

If audio doesn't start automatically:
1. **Ensure PWA is installed** (not just in browser)
2. **Launch from home screen** (not browser)
3. **Grant permissions** if prompted
4. **iOS users**: May need to tap once on first connection
5. **Check browser settings**: Audio permissions

## Testing Your PWA

### Installation Checklist

- [ ] Manifest served correctly (`/static/manifest.json`)
- [ ] Icon accessible (`/static/icon.svg` or PNGs)
- [ ] HTTPS or localhost (required for PWA)
- [ ] Manifest linked in HTML (`<link rel="manifest">`)
- [ ] All manifest fields valid

### Post-Installation Testing

After installing:
- [ ] Launch from home screen works
- [ ] Runs in standalone mode (no browser UI)
- [ ] Status bar has correct color
- [ ] Orientation is portrait
- [ ] Audio autoplay works (or requires minimal interaction)
- [ ] Icon displays correctly on home screen
- [ ] App name shows in app switcher

### Browser DevTools

**Chrome**: Open DevTools → Application → Manifest
- Check for errors
- Preview icon
- Verify all fields

**Firefox**: about:debugging → This Firefox → Manifest
- View parsed manifest
- Check for warnings

## PWA and HTTPS

### Development (HTTP)

PWAs work on `localhost` and `127.0.0.1` without HTTPS:
- http://localhost:3000/intercom ✅
- http://127.0.0.1:3000/intercom ✅

### Production (HTTPS Required)

For LAN or internet access, HTTPS is required:
- http://192.168.1.100:3000/intercom ❌ (won't install as PWA)
- https://192.168.1.100:3000/intercom ✅ (works)

**Solution**: Use the included Caddy reverse proxy:

```yaml
# docker-compose.yml already includes Caddy
caddy:
  build:
    context: .
    dockerfile: Dockerfile.caddy
  ports:
    - "8443:443"
```

Access at: https://your-server:8443/intercom

### Self-Signed Certificates

If using self-signed certs:
1. Browser will show security warning
2. Click "Advanced" → "Proceed anyway"
3. Now PWA installation will work

For production, use Let's Encrypt (configured in Caddyfile).

## Platform-Specific Notes

### iOS

- **Safari only**: Must use Safari for installation
- **Private Browsing**: PWA won't install
- **Storage**: PWAs have limited storage vs native apps
- **Background**: Can't run in background like native apps
- **Autoplay**: Strictest autoplay policies

**iOS Quirks**:
- May add background color behind transparent icons
- Rounded corners applied automatically
- Some Web APIs unavailable compared to native

### Android

- **Chrome recommended**: Best PWA support
- **Multiple browsers**: Firefox, Edge also support PWAs
- **More permissive**: Better autoplay, more Web APIs
- **Background**: Better background support than iOS

### Desktop

- **Chrome/Edge**: Full PWA support
- **Firefox**: Limited PWA support
- **Safari**: No PWA installation on macOS

## Updating the PWA

### After Code Changes

1. **Update the app**: Deploy new version
2. **Users see update**: On next launch or reload
3. **Service worker**: (If implemented) Handle updates gracefully

**Current implementation**: No service worker, so updates apply immediately on page reload.

### Forcing Update

Users can force update by:
1. **Close and reopen** the app
2. **Reload**: Pull down to refresh (mobile)
3. **Reinstall**: Delete and reinstall from home screen

## Advanced: Adding a Service Worker

Currently, Birdbox doesn't use a service worker (not needed for basic PWA). To add one:

1. **Create** `static/service-worker.js`:
```javascript
self.addEventListener('install', (event) => {
  console.log('Service worker installed');
});

self.addEventListener('fetch', (event) => {
  // Handle fetch events
});
```

2. **Register** in `templates/intercom.html`:
```javascript
if ('serviceWorker' in navigator) {
  navigator.serviceWorker.register('/static/service-worker.js');
}
```

3. **Update Axum** to serve service worker:
Already handled by `ServeDir` in `main.rs`.

**Benefits of service worker**:
- Offline support
- Background sync
- Push notifications
- Better caching control

**Trade-offs**:
- More complexity
- Update management
- Storage management

For a real-time streaming app like Birdbox, service workers provide limited benefit since the core functionality requires network connection anyway.

## Troubleshooting

### PWA Won't Install

**Symptoms**: No "Add to Home Screen" option

**Common causes**:
1. **Not using HTTPS** (except localhost)
   - Solution: Use Caddy proxy or add HTTPS
2. **Manifest errors**
   - Solution: Check DevTools → Application → Manifest
3. **Wrong browser** (iOS requires Safari)
   - Solution: Switch to Safari
4. **Private browsing mode**
   - Solution: Use normal browsing mode

### Icon Not Showing

**Symptoms**: Generic placeholder icon on home screen

**Common causes**:
1. **Icon path wrong** in manifest
   - Solution: Verify `/static/icon.svg` is accessible
2. **Icon format not supported**
   - Solution: Generate PNG versions
3. **Cache issue**
   - Solution: Clear browser cache, reinstall

### App Opens in Browser Instead of Standalone

**Symptoms**: PWA launches in browser with address bar

**Common causes**:
1. **Manifest display not "standalone"**
   - Solution: Check manifest.json
2. **Opened from browser, not home screen**
   - Solution: Use the home screen icon
3. **Platform doesn't support standalone**
   - Solution: Some platforms always show minimal UI

### Autoplay Still Doesn't Work

**Symptoms**: Audio requires manual start even after PWA install

**Solutions**:
1. **Check device audio settings**: Ensure not muted
2. **Grant audio permissions**: May need browser permission
3. **Try different device**: iOS is stricter than Android
4. **Accept limitations**: Some platforms always require interaction

## Summary

### Quick Start

1. Open intercom page in Safari (iOS) or Chrome (Android)
2. Tap "Add to Home Screen" or "Install"
3. Launch from home screen icon
4. Enjoy improved autoplay and full-screen experience

### Key Points

- **HTTPS required** for non-localhost installs
- **Safari only** on iOS
- **Better autoplay** than in-browser
- **No offline support** (not needed for streaming)
- **Easy customization** via manifest.json and icon

### Resources

- [PWA on MDN](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps)
- [Web App Manifest](https://developer.mozilla.org/en-US/docs/Web/Manifest)
- [iOS Web App Meta Tags](https://developer.apple.com/library/archive/documentation/AppleApplications/Reference/SafariWebContent/ConfiguringWebApplications/ConfiguringWebApplications.html)

The PWA setup enhances Birdbox's usability on mobile devices, making it feel more like a native intercom app while maintaining the simplicity of web deployment.

