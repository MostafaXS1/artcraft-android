# ArtCraft Android

**The full ArtCraft PC experience, now on Android.**

[**Download the apps**](https://github.com/MostafaXS1/artcraft-android/releases/) · [ArtCraft website](https://getartcraft.com/) · [Explore the apps](https://getartcraft.com/apps)

![ArtCraft Android](artcraft-android-1.png)

ArtCraft Android brings the complete functionality of the ArtCraft PC applications to Android devices. The apps retain their PC capabilities while gaining native Android integrations for file access, touch input, keyboards, mice, trackpads, and other system features.

Whether you're using a phone, tablet, or an Android device with a keyboard and mouse, ArtCraft gives you the same powerful creative tools available on PC, adapted to work naturally with Android.

## The Full PC Experience on Android

ArtCraft Android is not a stripped-down mobile edition. The applications are built with the exact source code used in their PC counterparts, with Android-specific adaptations that connect them to the platform's input, storage, and system APIs.

### Native Android File Integration

- **Storage Access Framework (SAF):** Open files and choose save destinations using Android's system document picker.
- **Direct document access:** Work with files selected from Android document providers, including supported cloud storage providers and external storage locations.
- **Save changes back:** SAF integration handles document staging and writes updated content back to the selected document.
- **Open with:** Open supported files directly from compatible file managers and other Android applications.
- **File sharing:** Receive files through Android's sharing and intent mechanisms where applicable.
- **Drag and drop:** Open files by dragging them from compatible applications, including file managers and browsers.
- **File associations:** A shared registry connects supported file types with the appropriate ArtCraft application.

Your existing creative workflow stays available, with Android providing the system-level file access needed to make it work.

### Touch, Mouse, Trackpad, and Keyboard

ArtCraft Android adapts the applications' input handling to Android while preserving their PC functionality.

- **Touch gestures:** Two-finger scrolling and momentum scrolling for navigating large documents and canvases.
- **Mouse support:** Pointer movement, hover events, mouse buttons, and wheel scrolling.
- **Trackpad support:** Desktop-style navigation and scrolling on compatible devices.
- **Physical keyboards:** Keyboard shortcuts and modifier keys, including Ctrl, Alt, and Shift.
- **Android soft keyboard:** Input Method Editor (IME) integration for text entry, composition, deletion, and editor actions.
- **International text input:** Support for composed text and non-English input through Android's input system.
- **System clipboard:** Copy, cut, and paste through Android's clipboard, including keyboard shortcuts.
- **Back-button integration:** Android Back is mapped to the appropriate application behavior where supported by the runtime.

Connect a keyboard and mouse, use a trackpad, or work directly on a touchscreen. The applications remain fully featured regardless of the input method.

### Android System Integration

- **System fonts:** Android system fonts are exposed to the application renderer. User font discovery is also supported where configured.
- **Android printing:** Printing integrates with Android's print framework in supported workflows, including WordCraft and GridCraft.
- **Persistent preferences:** Application settings and configuration can persist between launches.
- **Display adaptation:** Full-screen presentation and appropriate UI scaling help the applications fit Android displays.
- **System appearance:** Light and dark appearance support integrates with the Android environment where applicable.
- **Release notifications:** The application can check for newer ArtCraft Android releases and notify you when updates are available.

These capabilities are implemented through shared Android runtime components, platform compatibility patches, and app-specific adapters.


[**Download the latest APKs**](https://github.com/MostafaXS1/artcraft-android/releases/)

## Built for Android Without Removing PC Functionality

The ArtCraft applications originate from the official Storytold repositories. ArtCraft Android adds the Android launch environment, shared platform integrations, and app-specific adapters required to run them on Android.

The objective is to preserve the functionality of the PC applications while making them work properly with Android's storage, input, and system services.

The Android build process uses templates, adapters, vendored changes, and compatibility patches as needed. This keeps the Android integration organized and makes it easier to maintain across the app suite.

## Links

- **Downloads:** [GitHub Releases](https://github.com/MostafaXS1/artcraft-android/releases/)
- **ArtCraft website:** [getartcraft.com](https://getartcraft.com/)
- **Explore the apps:** [getartcraft.com/apps](https://getartcraft.com/apps)
- **Source code:** [ArtCraft Android repository](https://github.com/MostafaXS1/artcraft-android)
- **Adapter documentation:** [`adapters/README.md`](adapters/README.md)
- **Architecture documentation:** [`architecture.md`](architecture.md)
- **Changelog:** [`CHANGELOG.md`](CHANGELOG.md)

---

## Build from Source

Build locally signed, 64-bit ARM Android APKs from the official Storytold repositories.

### Requirements

- Git
- Rust and Cargo
- Python 3
- A Java Development Kit (JDK)
- Android SDK and NDK build components

The build scripts can discover or bootstrap supported Android build components when they are not already available.

### Quick Start

Run the scripts from any directory. Each script resolves its own location and switches to the project directory before building.

```sh
# Optimized release APK
bash build.sh -pdfcraft

# Debug APK for testing
bash build-debug.sh -pdfcraft
```

Replace `-pdfcraft` with any supported application name.

### Supported Build Targets

| App | Repo |
| --- | --- |
| photocraft, vectorcraft, filmcraft, lightcraft, pdfcraft, effectcraft, designcraft, figcraft, wordcraft, soundcraft, deckcraft, cadcraft, gridcraft | Storytold (official) |
| subcraft | [Aalzard/subcraft](https://github.com/Aalzard/subcraft) *(Unofficial)* |

App names are passed as command-line flags, such as `-photocraft` or `-vectorcraft`.

### Build Options

| Option | Description |
| --- | --- |
| `-stable` | Build against the latest stable upstream release tag. This is the default. |
| `-main` or `-latest` | Build against the rolling upstream `main` branch. |
| `-tag=<tag>` | Build against a specific Git tag, for example `-tag=v0.12.1`. |
| `-branch=<branch>` | Build against a specific Git branch. |
| `-appname=<name>` | Override the Android display name. |
| `-packagename=<pkg>` | Override the Android package name. The default is `com.artcraft.<app>`. |
| `-appicon=<path>` | Use a custom launcher icon PNG. |
| `-h` or `--help` | Show command help. |

### Build Examples

```sh
# Build PhotoCraft
bash build.sh -photocraft

# Build a debug APK for testing
bash build-debug.sh -photocraft

# Build from a specific upstream branch
bash build.sh -photocraft -branch=main

# Customize the Android application
bash build.sh -vectorcraft \
  -appname="My Vector App" \
  -packagename=com.example.myvectorapp \
  -appicon=/path/to/icon.png
```

### Build Output

- **APK files:** `output/<app>-<version>.apk`
- **Build logs:** `logs/`
- **Checked-out source:** `source/<app>/`
- **Build artifacts:** `build/<app>/target/`

Release APKs are optimized for installation and everyday use. Debug APKs are larger and slower, and are intended for testing and troubleshooting.

### Testing on a Device

Install an APK and inspect its logs using Android Debug Bridge (ADB).

```sh
# Install an APK
adb install -r output/photocraft-*.apk

# View application logs
adb logcat -s com.artcraft.photocraft

# Check requested permissions
adb shell dumpsys package com.artcraft.photocraft | grep -A 15 "requested permissions"
```

### Signing and Source Checkouts

Builds are signed with a local development keystore stored under `keys/`. Keep the keystore and its password safe if you want future builds to install as updates over earlier builds signed with the same key.

Do not use the local development key for public distribution. Use a separately managed release-signing key for public releases.

Existing source checkouts are reused to preserve local Android port work. The build does not automatically reset or overwrite local source changes. Update existing checkouts deliberately with Git and review the diff before switching versions or incorporating upstream changes.
