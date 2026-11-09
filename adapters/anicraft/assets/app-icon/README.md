# AniCraft app icon

**Creature:** an engraved rabbit, AniCraft's mascot.

**Style:** an engraving-style portrait in the Crafting Apps template: a head-and-shoulders portrait on a
full-bleed field, no frame or roundel, with the body running off the tile.

**Palette** (exactly three colours):

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contour |
| Paper | `#f2f1ee` | the figure (a neutral white, so the rabbit doesn't read as cream on the yellow) |
| AniCraft colour | `#dac604` | the full-bleed field |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`, the same tile as every Crafting App.
The art is clipped 6 units inside it (a concentric rounded square, `x=y=6`, `500×500`, `rx=106`), so a
band of field colour runs round every edge where the rabbit runs off, as on the other icons. The padded files add Apple's transparent margin (tile = 824/1024 of the canvas); Windows
and Linux use the tile edge to edge.

**Provenance:** the owner's original ArtCraft drawing, vectorised with `trace.py`. `anicraft.svg` is the
canonical artwork. Licence: see `LICENSE.txt`.

## Files

| File | What | Used by |
|---|---|---|
| `anicraft.svg` | master vector (traced at 1600 px, about 275 KB) | source for everything below |
| `anicraft-small.svg` | lighter vector (traced at 1000 px, about 170 KB) | `hicolor/scalable` |
| `anicraft-1024.png` | full tile, 1024 px | docs, store listings |
| `anicraft-padded-1024.png` | tile with Apple's margin, 1024 px | landing pages, store listings |
| `anicraft-macos-512.png` | tile with Apple's margin, 512 px | runtime Dock icon on macOS (`src/main.rs`) |
| `anicraft.ico` | Windows icon, 16 to 256 px | `build.rs` (embedded in `anicraft.exe`) |
| `hicolor/<size>/apps/ai.storyteller.anicraft.png` | Linux theme icons, 16 to 512 px; the 256 one is also the runtime icon on Windows and Linux, the 128 one the mark in Help › About (`src/app.rs`) | Linux packaging, the Crafting Apps tables |
| `hicolor/scalable/apps/ai.storyteller.anicraft.svg` | Linux scalable icon | as above |
| `trace.py` | traces the 2000 px source drawing into the master SVG | regenerating the vectors |

## How it reaches each OS

- **macOS:** the app sets the Dock / app-switcher icon at runtime (`ViewportBuilder::with_icon`).
- **Windows:** `build.rs` embeds `anicraft.ico` with `winresource` (taskbar, Start menu, Explorer,
  Alt-Tab); the runtime icon covers the title bar.
- **Linux:** the runtime icon covers the window; install `hicolor/` to `share/icons/` for launchers.

## Regenerate

```sh
# From the 2000 px source drawing (needs numpy, Pillow, SciPy and vtracer; svgo 3.3.2 shrinks the paths):
python trace.py rabbit.webp master.svg 1600 4 1960 && npx svgo@3.3.2 -p 1 master.svg -o anicraft.svg
python trace.py rabbit.webp small.svg 1000 4 1960 && npx svgo@3.3.2 -p 1 small.svg -o anicraft-small.svg
# Every render (needs resvg on PATH and Python with Pillow):
scripts/icons.sh
```
