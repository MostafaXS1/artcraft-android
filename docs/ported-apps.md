# Ported Android Applications

This document provides a comprehensive index of all applications ported to Android within the **ArtCraft Android** ecosystem. It lists each application's name, purpose, porting status, and direct links to its upstream and source GitHub repositories.

> [!TIP]
> For in-depth technical documentation detailing how each application is ported, its runtime bridges (SAF, IME, modifier chords, Termux IPC), and step-by-step adaptation processes, see [**`porting-guide.md`**](porting-guide.md).

---

## Overview

ArtCraft Android brings the complete desktop creative experience to Android devices (`aarch64-linux-android`). The suite comprises:

* **Official Storytold Suite (16 apps):** The core creative and office applications from the Storytold project.
* **Community Applications with In-Tree Adapters & CI Targets (24 apps):** Community-developed creative software with native Android build targets and custom port adapters.
* **Community Applications Tracked via Launcher Catalog (7 apps):** External community applications indexed and distributed directly through the ArtCraft Android Launcher.
* **ArtCraft Launcher (1 app):** The centralized Android launcher for discovering, managing, launching, and updating ArtCraft apps.

**Total Applications:** 48

---

## 1. Official Storytold Suite Applications

These applications originate from the official Storytold repositories and are compiled natively for Android with platform integration hooks, Storage Access Framework (SAF) document pickers, and IME soft keyboard input handling.

| Application Name | App ID | Category / Tagline | Upstream GitHub Repository | Port Integration Model |
| :--- | :--- | :--- | :--- | :--- |
| **PhotoCraft** | `photocraft` | Professional image editing and retouching | [storytold/photocraft](https://github.com/storytold/photocraft) | Custom adapter (`adapters/photocraft`) with SAF document bridge & IME |
| **VectorCraft** | `vectorcraft` | Vector illustration and path design | [storytold/vectorcraft](https://github.com/storytold/vectorcraft) | Custom adapter (`adapters/vectorcraft`) with SAF document bridge & modifier tracking |
| **FilmCraft** | `filmcraft` | Non-linear video editing, color grading & audio | [storytold/filmcraft](https://github.com/storytold/filmcraft) | Zero-copy NDK `AMediaCodec` hardware decoding & Vulkan surface pipeline |
| **LightCraft** | `lightcraft` | Photo library management and RAW photo development | [storytold/lightcraft](https://github.com/storytold/lightcraft) | Generated launcher shell with multithreaded raw image pipelines |
| **PdfCraft** | `pdfcraft` | PDF reading, organizing, annotations & security | [storytold/pdfcraft](https://github.com/storytold/pdfcraft) | Generated launcher with PDF rendering engine (formerly *PrintCraft*) |
| **EffectCraft** | `effectcraft` | Visual effects, image filters and motion graphics | [storytold/effectcraft](https://github.com/storytold/effectcraft) | Generated launcher with GPU shader pipeline |
| **DesignCraft** | `designcraft` | Desktop publishing, page layout and editorial design | [storytold/designcraft](https://github.com/storytold/designcraft) | Generated launcher with Hayro PDF engine & Android print framework |
| **FigCraft** | `figcraft` | Diagramming, whiteboarding and UI prototyping | [storytold/figcraft](https://github.com/storytold/figcraft) | Generated launcher with touch gesture navigation and stylus support |
| **WordCraft** | `wordcraft` | Document design, typography and word processing | [storytold/wordcraft](https://github.com/storytold/wordcraft) | Generated launcher with IME debounce optimization & Android print framework |
| **SoundCraft** | `soundcraft` | Multi-track audio recording, editing and mixing | [storytold/soundcraft](https://github.com/storytold/soundcraft) | Generated launcher with low-latency AAudio / Oboe / cpal audio backend |
| **DeckCraft** | `deckcraft` | Presentations, slide decks and speaker notes | [storytold/deckcraft](https://github.com/storytold/deckcraft) | Generated launcher with full-screen presentation mode |
| **CadCraft** | `cadcraft` | 2D and 3D computer-aided design and drafting (CAD) | [storytold/cadcraft](https://github.com/storytold/cadcraft) | Generated launcher with 3D viewport and spatial indexing |
| **GridCraft** | `gridcraft` | Spreadsheets, table formulas and data analysis | [storytold/gridcraft](https://github.com/storytold/gridcraft) | Generated launcher with Android print framework & XLSX export |
| **MaterialCraft** | `materialcraft` | Procedural 3D material design and PBR textures | [storytold/materialcraft](https://github.com/storytold/materialcraft) | Generated launcher with 3D material preview renderer |
| **TextureCraft** | `texturecraft` | 3D model texture painting and surface sculpting | [storytold/texturecraft](https://github.com/storytold/texturecraft) | Generated launcher with 3D brush projection & pressure-sensitive input |
| **AniCraft** | `anicraft` | Frame-by-frame 2D animation and onion skinning | [storytold/AniCraft](https://github.com/storytold/AniCraft) | Custom adapter (`adapters/anicraft`) with bundled FFmpeg export pipeline |

---

## 2. Community Applications with In-Tree Adapters & CI Targets

These community applications have custom port adapters under `adapters/<app>/` and dedicated targets in the ArtCraft Android build script (`lib/build-common.sh`) and continuous integration pipeline (`.github/workflows/build.yml`).

| Application Name | App ID | Category / Description | Upstream GitHub Repository | Publisher |
| :--- | :--- | :--- | :--- | :--- |
| **SubCraft** | `subcraft` | 3D texture painting directly on 3D models | [Aalzard/subcraft](https://github.com/Aalzard/subcraft) | Aalzard |
| **SolidCraft** | `solidcraft` | Parametric 3D CAD modeling | [tangency-labs/solidcraft](https://github.com/tangency-labs/solidcraft) | Tangency Labs |
| **BIMCraft** | `bimcraft` | Building Information Modeling (BIM) & architectural design | [IPedrax/BIMCraft](https://github.com/IPedrax/BIMCraft) | IPedrax |
| **CapsuleCraft** | `capsulecraft` | Note-taking and email message archives using `.eml` files | [kostyk348/CapsuleCraft](https://github.com/kostyk348/CapsuleCraft) | kostyk348 |
| **DiagramCraft** | `diagramcraft` | Visio-style technical diagrams, flowcharts and stencils | [baradhili/DiagramCraft](https://github.com/baradhili/DiagramCraft) | baradhili |
| **EarthCraft** | `earthcraft` | Interactive 3D globe explorer and geographic visualization | [therichronicles/EarthCraft](https://github.com/therichronicles/EarthCraft) | therichronicles |
| **HumanMotionCraft** | `humanmotioncraft` | Humanoid character posing, skeletal kinematics & animation | [YalKouassi/HumanMotionCraft](https://github.com/YalKouassi/HumanMotionCraft) | YalKouassi |
| **MapCraft** | `mapcraft` | Brush-based 3D level editor and geometric environment design | [bearlikelion/MapCraft](https://github.com/bearlikelion/MapCraft) | bearlikelion |
| **NoteCraft** | `notecraft` | Local-first markdown notebooks and knowledge management | [AlienTechSoftware/notecraft](https://github.com/AlienTechSoftware/notecraft) | AlienTechSoftware |
| **PaintCraft** | `paintcraft` | Painting, raster sketching and digital canvas illustration | [replicraft/PaintCraft](https://github.com/replicraft/PaintCraft) | replicraft |
| **PicassoSuite** | `picassosuite` | DaVinci Resolve-style video editing suite | [fabox-os/Picasso-Suite](https://github.com/fabox-os/Picasso-Suite) | fabox |
| **ProWave** | `prowave` | Digital audio workstation (DAW), synthesizer & music production | [fabox-os/Pro-Wave](https://github.com/fabox-os/Pro-Wave) | fabox |
| **RustGIS** | `rustgis` | Geospatial information systems (GIS) and map analysis | [opengeos/RustGIS](https://github.com/opengeos/RustGIS) | opengeos |
| **TraceDraw** | `tracedraw` | Vector illustration, bezier tracing and print design | [tracedraw/tracedraw](https://github.com/tracedraw/tracedraw) | TraceDraw |
| **Transit** | `transit` | Digital audio workstation for sequencing and arrangement | [transit-audio/transit](https://github.com/transit-audio/transit) | transit-audio |
| **Coiman IDE** | `coimanide` | Code editor with language servers (LSP), Git and Termux bridge | [ManuelCoimbra/coiman-ide](https://github.com/ManuelCoimbra/coiman-ide) | ManuelCoimbra |
| **PCBCraft** | `pcbcraft` | Printed circuit board design and layout | [tangency-labs/pcbcraft](https://github.com/tangency-labs/pcbcraft) | Tangency Labs |
| **FieldCraft** | `fieldcraft` | Finite element analysis and physics simulation | [tangency-labs/fieldcraft](https://github.com/tangency-labs/fieldcraft) | Tangency Labs |
| **PaperCraft** | `papercraft` | Local design canvas built in Rust and egui | [slyim/papercraft](https://github.com/slyim/papercraft) | slyim |
| **Ferrite** | `ferrite` | Fast, lightweight text editor for Markdown and JSON | [OlaProeis/Ferrite](https://github.com/OlaProeis/Ferrite) | OlaProeis |
| **PixelCraft** | `pixelcraft` | Fast, native pixel art editor and animator | [asad-albadi/PixelCraft](https://github.com/asad-albadi/PixelCraft) | Asad |
| **ClothCraft** | `clothcraft` | Cloth simulation, garment patterning and 3D draping | [erythrofr/ClothCraft](https://github.com/erythrofr/ClothCraft) | erythrofr |
| **Splice** | `splice` | Fast video editor with MediaCodec hardware acceleration | [GeneticxCln/Splice](https://github.com/GeneticxCln/Splice) | GeneticxCln |
| **VidCut** | `vidcut` | Video trimmer, cutter and timeline editor with MediaCodec acceleration | [goriant-studio/VidCut](https://github.com/goriant-studio/VidCut) | goriant-studio |

---

## 3. Community Applications Distributed via Launcher Catalog

These community applications are cataloged in the ArtCraft Android Launcher (`launcher/apps/artcraft-launcher-android/app-catalog.json`), with automated release indexing for on-device installation and updates.

| Application Name | App ID | Category / Description | Upstream / Distribution GitHub Repository | Publisher |
| :--- | :--- | :--- | :--- | :--- |
| **Keydeck** | `keydeck` | Slide presentations (Apple Keynote and PowerPoint files) | [bramble-apps/bramble-office](https://github.com/bramble-apps/bramble-office) | Bramble Apps |
| **Values** | `values` | Spreadsheets (Apple Numbers and Excel files) | [bramble-apps/bramble-office](https://github.com/bramble-apps/bramble-office) | Bramble Apps |
| **Letters** | `letters` | Word processing, typography and structured page layout | [bramble-apps/bramble-office](https://github.com/bramble-apps/bramble-office) | Bramble Apps |
| **ProCraft** | `procraft` | Digital painting with pressure-sensitive brushes & layers | [artcraft-android/procraft](https://github.com/artcraft-android/procraft) | Bearinmind |
| **CapCraft** | `capcraft` | Mobile video editor with filters, effects and captions | [artcraft-android/capcraft-android](https://github.com/artcraft-android/capcraft-android) | Bearinmind |
| **Concat** | `concat` | Local-first video editor scriptable through JSON-RPC | [jub0t/Concat](https://github.com/jub0t/Concat) | jub0t |
| **ShapeCraft** | `shapecraft` | Direct-modeling 3D CAD for STEP, IGES, STL, OBJ & Shapr | [jackharvest/shapecraft](https://github.com/jackharvest/shapecraft) | jackharvest |

---

## 4. Launcher & System Integration Harness

| Application Name | App ID | Description | Upstream GitHub Repository | Android Repository |
| :--- | :--- | :--- | :--- | :--- |
| **ArtCraft Launcher** | `launcher` | Android launcher dashboard for discovering, installing, launching, and updating the full ArtCraft application suite | [storytold/craft-launcher](https://github.com/storytold/craft-launcher) | [MostafaXS1/artcraft-android](https://github.com/MostafaXS1/artcraft-android) |

---

## Alphabetical Quick Index

Below is an alphabetical reference list of all 44 applications with direct links to their GitHub repositories:

1. [AniCraft](https://github.com/storytold/AniCraft) — Frame-by-frame 2D animation
2. [ArtCraft Launcher](https://github.com/storytold/craft-launcher) ([Android Port](https://github.com/MostafaXS1/artcraft-android)) — ArtCraft Android ecosystem launcher
3. [BIMCraft](https://github.com/IPedrax/BIMCraft) — Building Information Modeling (BIM)
4. [CadCraft](https://github.com/storytold/cadcraft) — 2D and 3D computer-aided design (CAD)
5. [CapCraft](https://github.com/artcraft-android/capcraft-android) — Mobile video editor with effects and captions
6. [CapsuleCraft](https://github.com/kostyk348/CapsuleCraft) — Email and `.eml` note archives
7. [Coiman IDE](https://github.com/ManuelCoimbra/coiman-ide) — Code editor with Termux bridge and Git
8. [Concat](https://github.com/jub0t/Concat) — Scriptable local-first video editor
9. [DeckCraft](https://github.com/storytold/deckcraft) — Presentations and slide decks
10. [DesignCraft](https://github.com/storytold/designcraft) — Page layout and graphic design
11. [DiagramCraft](https://github.com/baradhili/DiagramCraft) — Visio-style diagrams and stencils
12. [EarthCraft](https://github.com/therichronicles/EarthCraft) — 3D globe explorer
13. [EffectCraft](https://github.com/storytold/effectcraft) — Visual effects and image filters
14. [Ferrite](https://github.com/OlaProeis/Ferrite) — Fast, lightweight text editor for Markdown and JSON
15. [FieldCraft](https://github.com/tangency-labs/fieldcraft) — Finite element analysis and physics simulation
16. [FigCraft](https://github.com/storytold/figcraft) — Interface design and prototyping
17. [FilmCraft](https://github.com/storytold/filmcraft) — Video editing, color grading and audio
18. [GridCraft](https://github.com/storytold/gridcraft) — Spreadsheets and data tables
19. [HumanMotionCraft](https://github.com/YalKouassi/HumanMotionCraft) — Humanoid character pose and animation
20. [Keydeck](https://github.com/bramble-apps/bramble-office) — Presentation authoring (.key / .pptx)
21. [Letters](https://github.com/bramble-apps/bramble-office) — Word processing and page layout
22. [LightCraft](https://github.com/storytold/lightcraft) — Photo library and RAW image development
23. [MapCraft](https://github.com/bearlikelion/MapCraft) — Brush-based 3D level editor
24. [MaterialCraft](https://github.com/storytold/materialcraft) — Procedural 3D material design
25. [NoteCraft](https://github.com/AlienTechSoftware/notecraft) — Local-first markdown notebooks
26. [PaintCraft](https://github.com/replicraft/PaintCraft) — Digital painting and canvas illustration
27. [PaperCraft](https://github.com/slyim/papercraft) — Local design canvas built in Rust and egui
28. [PCBCraft](https://github.com/tangency-labs/pcbcraft) — Printed circuit board design and layout
29. [PdfCraft](https://github.com/storytold/pdfcraft) — PDF viewing, annotating and management
30. [PhotoCraft](https://github.com/storytold/photocraft) — Professional image editing and manipulation
31. [PicassoSuite](https://github.com/fabox-os/Picasso-Suite) — Multi-page video editing suite
32. [PixelCraft](https://github.com/asad-albadi/PixelCraft) — Fast, native pixel art editor and animator
32. [ProCraft](https://github.com/artcraft-android/procraft) — Digital painting with pressure-sensitive brushes
33. [ProWave](https://github.com/fabox-os/Pro-Wave) — Music production and digital audio workstation
34. [RustGIS](https://github.com/opengeos/RustGIS) — Geospatial analysis and GIS mapping
35. [ShapeCraft](https://github.com/jackharvest/shapecraft) — Direct-modeling 3D CAD (STEP, IGES, STL)
36. [SolidCraft](https://github.com/tangency-labs/solidcraft) — Parametric 3D CAD modeling
37. [SoundCraft](https://github.com/storytold/soundcraft) — Audio recording, editing and mixing
38. [SubCraft](https://github.com/Aalzard/subcraft) — 3D texture painting on models
39. [TextureCraft](https://github.com/storytold/texturecraft) — 3D model texture painting
40. [TraceDraw](https://github.com/tracedraw/tracedraw) — Vector illustration and bezier tracing
41. [Transit](https://github.com/transit-audio/transit) — Digital audio workstation
42. [Values](https://github.com/bramble-apps/bramble-office) — Spreadsheet editor (.numbers / .xlsx)
43. [VectorCraft](https://github.com/storytold/vectorcraft) — Vector illustration and path design
44. [WordCraft](https://github.com/storytold/wordcraft) — Writing and word processing
