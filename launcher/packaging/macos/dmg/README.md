# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side, laid out like the other Craft apps' DMGs. `package.sh` copies
these files into the image; nothing here is generated at build time, so the DMG builds with a plain
`hdiutil create -srcfolder` (no Finder scripting on CI, nothing extra installed in the signing job).

| File | What |
|---|---|
| `background.svg` | Source of the background: the ArtCraft mark in black (`docs/brand/artcraft-mark-black.svg`, linked, not copied) whole on the mark's blue (`#4e7bfb`), Ink and Paper. The mark is a trademark: it stays exactly as supplied, never cropped or recoloured (`docs/brand/LICENSE-brand.txt`), unlike the cropped icons on the other apps' DMGs. Its text (Inter, Geist Mono from `assets/fonts`) is outlined, so rendering it needs no fonts. No paper grain: noise doesn't compress, and this keeps `background.tiff` small. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (16-colour palette, Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, ArtCraft Launcher.app at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |

## The volume name has no version

The mounted volume is called `ArtCraft Launcher`, not `ArtCraft Launcher <version>`. The layout
points at the background through an alias that includes the volume name, and Finder resolves it by
that name: with a versioned name the window keeps its size and icon positions but shows no
background. The DMG file name (`artcraft-launcher-<version>-macos-<arch>.dmg`) still carries the
version.

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the area under both icons stays light (Paper).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and of
  the label strip under it.

## Regenerate

Edit `background.svg` (or the layout constants in `generate.py`), then run, on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py   # needs resvg on PATH
```

Both outputs are byte-for-byte reproducible and carry nothing from the machine that ran it: the
SVG renders without any fonts, and `dmg-layout.DS_Store` is written from scratch, its background
alias holding only the volume name and `/.background/background.tiff`. If you add text to the SVG,
outline it (`usvg --skip-system-fonts --use-font-file assets/fonts/Inter-Variable.ttf
--use-font-file assets/fonts/GeistMono-Variable.ttf` converts text to paths). The window is
660 × 432 with Finder's 32 pt title bar; the content area is 660 × 400.

This follows the other Craft apps' `packaging/macos/dmg/` (storytold/wordcraft#21,
storytold/photocraft#1201, first in storytold/vectorcraft#493).
