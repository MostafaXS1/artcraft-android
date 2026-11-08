# FigCraft Android touch UI

The FigCraft Android launcher adds an overlay action strip after the editor draws. It is Android-launcher-only and does not change the desktop UI.

## Direct tools

- Move
- Hand
- Frame
- Rectangle
- Ellipse
- Text
- Pen
- Comment

These buttons call FigCraft's `tool.set` command with documented tool IDs. The strip scrolls horizontally when the viewport cannot fit every button.

## Editing actions

- Undo / Redo
- Copy / Cut / Paste
- Duplicate / Delete
- Group / Ungroup
- Actions palette

Selection-dependent commands are disabled when nothing is selected. The strip is placed above the existing bottom toolbar and remains visible in portrait and landscape.

## Responsive menus

The build-time Android patch constrains submenu panels to the viewport, makes long context/submenus vertically scrollable, and adapts the Actions palette dimensions to the available screen. The Gemini Copilot panel is also viewport-sized and vertically scrollable.

## Build verification

`lib/figcraft_mobile.py` is syntax-checked and applied only for the FigCraft Android build. It fails loudly if upstream source anchors change, rather than silently shipping a partially patched UI.

## Known limitations

- This is a touch action strip, not a full replacement for every desktop tool or property editor.
- Gemini remains text-only. It does not yet inspect the live canvas or apply a validated action plan.
- The latest source changes still require a successful CI build and device testing in portrait and landscape.
