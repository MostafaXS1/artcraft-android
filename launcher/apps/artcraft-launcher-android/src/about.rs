//! The product pages for Android-catalog apps the core suite does not carry one for.
//!
//! Generated from about.json by generate-about.sh. Do not edit by hand.
//!
//! The core suite's pages live in artcraft-launcher-core::details; anything in the Android
//! catalog without one gets its page here: every community app, and AniCraft (an official
//! app this launcher ships but the desktop suite does not). The same `AppDetails` shape the
//! official Craft Apps use, so the details view treats a page here exactly like an official
//! one once `details::set_extra_details` has registered this slice at startup (done in
//! `app::new`). Text here is English; the page follows the official format.

use artcraft_launcher_core::details::{AppDetails, Feature};

/// The community apps' pages, in the order the catalog lists them.
pub static ABOUT: &[AppDetails] = &[
    AppDetails {
        id: "anicraft",
        headline: "2D vector animation studio, rebuilt in Rust.",
        about: "A native 2D vector animation editor in the spirit of Adobe Animate, written from scratch in pure Rust: timeline, layers, keyframes, classic and shape tweens, onion skinning and an egui stage. Open source, and built by the ArtCraft team.\n\nEarly alpha: not yet for production animation work. Audio layers, motion tweens and the macOS and Linux CI runners are still to come.",
        features: &[
            Feature { title: "Classic and shape tweens", text: "Tweens with 8 easing presets plus custom ease curves, stage-aware in-betweening, onion skinning and frame management across layers (F6 keyframes, F7 blanks, multi-frame selection)." },
            Feature { title: "Stage tools", text: "Pen with smooth handles and corner conversion (P), Pencil with Schneider curve fitting and smoothing (Shift+Y), Rectangle with corner radii, Oval, PolyStar, Line and Text (T), Free Transform (Q) and Subselection (A)." },
            Feature { title: "Layers & organization", text: "Normal, guide, motion-guide and mask layers, nestable layer folders, and cut/copy/paste of frames across layers." },
            Feature { title: "Symbols and scenes", text: "Graphic symbols with nested timelines and loop modes (Loop, Play Once, Single Frame), movie clips with filters and blend modes, and scenes for multi-scene animations." },
            Feature { title: "Knows your files", text: "Opens Adobe Animate documents (.fla/.xfl/XFL folders), imports SVG to the stage, and exports PNG frame sequences, GIF, SVG frames, sprite sheets, Lottie, and WebM/MP4 through your own ffmpeg." },
            Feature { title: "Agent-native", text: "Every action is an engine command: the same engine is drivable from the UI, the CLI and an MCP server, so agents can draw, animate and export." },
        ],
        shots: &[],
        platforms: "macOS, Windows, Linux, FreeBSD, web, Android",
    },
    AppDetails {
        id: "subcraft",
        headline: "A 3D texture painting app in the spirit of Substance Painter.",
        about: "Paint directly on 3D models with layered PBR materials, masks, generators and smart materials, bake mesh maps and export game-ready textures. Open source, and written in Rust.\n\nEarly but usable: painting, layers, baking, smart materials, save and load are covered by tests. The project file format may change between releases.",
        features: &[
            Feature { title: "Paint on the model", text: "Brush, eraser, clone, smudge, stencils, polygon fill and editable pen paths, with symmetry, lazy mouse, tips and saveable presets." },
            Feature { title: "A layer stack like Painter", text: "Paint and fill layers, folders, masks, 32 blend modes, generators and filters, smart materials and smart masks." },
            Feature { title: "Bake and export", text: "Nine mesh maps (ray-based AO, curvature and thickness can run on the GPU), with per-map export checkboxes and presets for Unreal, Unity, Godot 4 and glTF." },
            Feature { title: "Opens Painter projects", text: "Reads .spp files and rebuilds their layer stacks; imports OBJ, glTF/GLB, FBX, USD, Alembic, PLY and STL." },
            Feature { title: "Fast at 2048 and 4096", text: "Tiled texture storage, a GPU layer compositor and live-preview thinning for big brushes." },
        ],
        shots: &[],
        platforms: "Windows, Linux, Android",
    },
    AppDetails {
        id: "solidcraft",
        headline: "Parametric 3D CAD, with editable sketches and solids.",
        about: "An open-source parametric solid modeller by Tangency Labs. Sketch, constrain and build solids on an editable feature timeline: change a dimension and the model rebuilds. Early, but usable for simple parts. Open source, and it can be driven by AI agents through its MCP server.",
        features: &[
            Feature { title: "Sketches and constraints", text: "Lines, arcs, splines and text, solved by its own constraint solver, with parameter-driven dimensions." },
            Feature { title: "Features", text: "Extrude, revolve, sweep, loft, fillet, chamfer, shell, draft, holes, threads and patterns on a parametric timeline." },
            Feature { title: "Files", text: "STEP, IGES, 3MF, STL, OBJ and glTF in and out where supported, and .sc designs." },
            Feature { title: "Desktop, web and agents", text: "A desktop app, a browser app, a headless CLI and an MCP server, all running the same commands." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, web, Android",
    },
    AppDetails {
        id: "bimcraft",
        headline: "An open-source, clean-room take on the Revit workflow.",
        about: "Model a building the way Revit does: levels, walls, doors and windows, floors and ceilings, rooms, columns, stairs, ramps and railings, roofs and shafts that know about each other. Draw plans, sections, elevations and a 3D view from them, lay them out on sheets, and read and write IFC. Every action is a command, so agents can drive it as well as you can.\n\nBy IPedrax. Early development, and written in pure Rust.",
        features: &[
            Feature { title: "A real building model", text: "Doors and windows live in their walls, rooms find their boundaries from the walls around them, and views follow the model." },
            Feature { title: "Views and sheets", text: "Floor plans with Revit's view range, sections and elevations, a 3D view, ceiling plans, schedules and sheets printed to PDF." },
            Feature { title: "IFC in and out", text: "IFC2x3 and IFC4 files, as Revit and other BIM tools write them, and IFC4 goes back out." },
            Feature { title: "Command-first", text: "Every action is a command, so the desktop, the CLI and an MCP server all drive the same engine." },
            Feature { title: "Reads Revit files", text: "Opens .rvt files of release 2015 to 2027 (read only) into a new project." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "capsulecraft",
        headline: "Your knowledge base is a folder of emails.",
        about: "A local-first note editor and planner with no database, no cloud and no accounts: every note, task and dataset is a plain, RFC 5322 .eml file on your disk. Your knowledge base is a folder you can open in Thunderbird, search with grep, diff in git and back up with rsync.",
        features: &[
            Feature { title: "Zero database", text: "A folder of .eml files is the entire knowledge base; nothing is locked in a container." },
            Feature { title: "Block editor", text: "Headings, paragraphs, task lists, callouts, code blocks, CSV datagrids and wiki links with a live backlink index." },
            Feature { title: "Search that keeps up", text: "Okapi BM25 full-text search with relevance scores and highlighted snippets, plus \"related by meaning\" ranking." },
            Feature { title: "Planning", text: "Tasks aggregated into an agenda, Kanban, Gantt with dependency arrows and a calendar, with RFC 5545 ICS export." },
            Feature { title: "Offline everything", text: "An offline co-thinker, OCR, speech-to-text and text-to-speech run locally; no API keys and no cloud." },
        ],
        shots: &[],
        platforms: "Windows, Linux, Android",
    },
    AppDetails {
        id: "diagramcraft",
        headline: "Visio, rebuilt from scratch in Rust.",
        about: "A reimplementation of the Visio 2016 drawing experience in Rust: read and write .vsdx/.vstx/.vssx packages, render them to SVG/PNG, edit them through an undoable command layer, and work in a native desktop GUI with the Visio shell (ribbon, stencils, ShapeSheet).\n\nBuilt clean-room from file-format and behavior observations. No Microsoft code, content or assets.",
        features: &[
            Feature { title: "Real .vsdx files", text: "Reads and writes Visio packages (.vsdx, .vstx, .vssx) with lossless round-trip verification." },
            Feature { title: "A Visio shell", text: "A ribbon, QAT, status bar, canvas with rulers, grid and zoom, the Shapes pane with stencils, and a ShapeSheet inspector for every shape." },
            Feature { title: "The command layer", text: "Every edit is an undoable command with scope-addressed paths, so the same engine drives the GUI, the CLI and scripts." },
            Feature { title: "Libraries underneath", text: "A VSDX format library with masters, inheritance, ShapeSheet formulas and a verified writer." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "earthcraft",
        headline: "A 3D globe explorer in the spirit of Google Earth.",
        about: "A 3D globe explorer written in pure Rust with a CPU ray-casting renderer (and optional experimental GPU acceleration). Satellite imagery, streets and topography, day and night lighting, and your own placemarks and rulers. No account and no copy of your data.\n\nAn internet connection is required for map tiles. Tile and layer providers are credited in the app.",
        features: &[
            Feature { title: "A fast globe", text: "A high-performance optimized 3D globe renderer with multi-level cached tiles." },
            Feature { title: "Layers and overlays", text: "Satellite (Esri), streets (OSM), topography (OpenTopoMap), plus live AIS ship, aircraft, precipitation radar and traffic congestion layers." },
            Feature { title: "Your places", text: "Placemarks in an editable KML folder tree, animated fly-to, and a Tools panel with geodesic rulers in metric, imperial and novelty units." },
            Feature { title: "Real skies", text: "A NASA-source Milky Way background, real-time sunlight and daylight fading, plus a Moon view." },
            Feature { title: "Explore add-on", text: "Place coordinates and OSM details, Wikimedia Commons photos and narrated tours with system text-to-speech." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "humanmotioncraft",
        headline: "Humanoid keyframe animation, for games and film.",
        about: "A pure-Rust humanoid keyframe animation application with a native wgpu + egui UI: a 104-bone humanoid rig, two-bone and full-body IK, timeline layers and motion-matching travel, a physics-assisted solver and a pure-Rust learned in-betweener.\n\nThe releases are Windows builds for now; command-first design means it is also drivable headlessly and by agents over MCP.",
        features: &[
            Feature { title: "One rig, done right", text: "A calibrated 104-bone humanoid rig and mannequin, with Z-up metres and radians, and glTF export." },
            Feature { title: "IK and physics", text: "Analytical two-bone IK and full-body IK with pinned hands and flat feet, plus a physics solver for floor contact, balance and jump arcs." },
            Feature { title: "Motion-matching travel", text: "Generate foot-locked walking along paths, with timeline layers, six interpolation modes and additive/override stacking." },
            Feature { title: "AI in-betweening", text: "A trained transformer in-betweener that runs in plain Rust — no Python, PyTorch or ONNX at run time." },
            Feature { title: "Game-ready export", text: "Rigged .glb clips (full, game or body profiles) for your engine, plus a headless CLI and MCP server." },
        ],
        shots: &[],
        platforms: "Windows, Linux, Android",
    },
    AppDetails {
        id: "mapcraft",
        headline: "A level editor in the style of Hammer and TrenchBroom.",
        about: "A free, open-source level editor for brush-based maps, with a Godot-style scene tree, inspector and prefabs. Games are SDKs: entity definitions, compile profiles and sandboxed WebAssembly plugins, so anyone can teach it their game.\n\nEarly development; runs on Windows, macOS and Linux.",
        features: &[
            Feature { title: "Brush-based mapping", text: "Brush editing with a Godot-style scene tree, inspector and prefabs, written in pure Rust." },
            Feature { title: "SDKs and plugins", text: "Each game is an SDK: FGD entity definitions, material locations, compile profiles and Wasm plugins in any language that compiles to a component." },
            Feature { title: "SDK manager", text: "SDKs install from a registry, offered by the editor on first start; hosts of alternative registries are welcome." },
            Feature { title: "Supported so far", text: "SurfsUp through a Godot addon, Counter-Strike: Source (Quake-style maps) and Quake itself." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "notecraft",
        headline: "A clean-room OneNote for local files and your own sync.",
        about: "An open-source, clean-room recreation of the OneNote notebook experience, built around local files. Keep notebooks, sections, pages, pictures and attachments as ordinary folders and files on your device, and sync or back them up with the tools you already trust — no NoteCraft server and no account.",
        features: &[
            Feature { title: "Real notebooks", text: "Coloured notebooks, section groups and sections, pages with checklists, quotes, tables and equations, just like OneNote." },
            Feature { title: "Two page formats", text: "Pages as HTML or CommonMark, kept in their own format while the editor stays the same." },
            Feature { title: "Tags and equations", text: "To Do, Important, Question, Idea and more as plain text in the page, plus a native TeX-subset equation renderer." },
            Feature { title: "Both worlds of tooling", text: "An Office-style ribbon, themes with contrast-checked colours, drivable from a CLI, a control channel and an MCP server." },
        ],
        shots: &[],
        platforms: "Windows, Linux, Android",
    },
    AppDetails {
        id: "paintcraft",
        headline: "A familiar place to draw, paint and edit images.",
        about: "A painting and image editing app built in Rust, with a classic Windows 7/10/11 look, a Studio mode with tabs and floating panels, and a Win95-inspired Retro mode. Nine brushes with pen-pressure support, layers, filters, selections and MCP support so AI assistants can paint too.",
        features: &[
            Feature { title: "Three workspaces", text: "Classic (Windows 7, 10 or 11 skins), Studio (tabs, Save All, floating panels) and Retro (Win95-inspired)." },
            Feature { title: "Familiar tools", text: "Nine brushes, pencil, eraser, fill and colour picker; shapes, curves, text, selections, layers, undo/redo and flip/rotate/crop." },
            Feature { title: "Studio filters", text: "Thirteen editable filters with live previews and saved selection masks, including Curves, Vibrance and Hue/Saturation." },
            Feature { title: "Pen pressure everywhere", text: "Pressure controls for size and opacity, native on macOS and in the browser build." },
            Feature { title: "Agent-ready", text: "MCP support: let AI assistants create canvases, paint, edit layers, preview and save projects." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, web, Android",
    },
    AppDetails {
        id: "picassosuite",
        headline: "A cross-platform video editor in the shape of DaVinci Resolve.",
        about: "A cross-platform video editor written in Rust, laid out like DaVinci Resolve: Media, Photo, Cut, Edit, Fusion, Color, Fairlight and Deliver pages, with timelines that travel to and from other editors.\n\nBefore 1.0: about 95% of the menu entries work and the rest are shown disabled. Keep copies of projects you care about.",
        features: &[
            Feature { title: "Eight pages, one window", text: "Media, Photo, Cut, Edit, Fusion, Color, Fairlight and Deliver, in Resolve's order and proportions, with resizable panels." },
            Feature { title: "A professional timeline", text: "Blade, trim, ripple, roll, slip and slide, Dynamic Trim, compound clips, multicam, speed ramps, keyframes and transitions." },
            Feature { title: "Hardware accelerated", text: "Decoding on the GPU (D3D11, VideoToolbox, VA-API or CUDA) and export on NVENC, Quick Sync, AMF, VideoToolbox or VA-API." },
            Feature { title: "Color with the scopes", text: "Serial, parallel and layer nodes, lift/gamma/gain wheels, curves, qualifiers, power windows, LUTs, gallery stills, parade, waveform, vectorscope and histogram." },
            Feature { title: "Interoperability", text: "Opens DaVinci Resolve .drp projects and timelines, exports and imports Final Cut Pro XML, AAF, OpenTimelineIO and EDL." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "prowave",
        headline: "Music production with the look and workflow of Logic Pro.",
        about: "An open-source music production app in pure Rust, following Logic Pro's workflow: tracks, MIDI and audio, a Piano Roll, a Mixer with real effects and automation, and a lock-free real-time engine. No Electron and no JUCE.\n\nAlpha software: the project format may change, so keep backups.",
        features: &[
            Feature { title: "A real-time engine", text: "Sample-accurate transport with cycle, metronome and a tempo map, with no allocation, locks or disk I/O on the audio thread." },
            Feature { title: "Tracks and instruments", text: "Audio, Instrument, Drummer, Pattern, Aux, VCA and Folder tracks, SoundFonts, Hydrogen drum kits and a 48-voice subtractive synth." },
            Feature { title: "A full mixer", text: "Faders, pans, sends, buses, groups, VCA, automation with Read/Touch/Latch/Write, and plug-ins on any strip including the stereo out." },
            Feature { title: "Flex time and pitch", text: "Warp and re-pitch audio, detect tempo and follow a recording, non-destructively." },
            Feature { title: "Bounce and export", text: "Bounce to 32-bit WAV, export MIDI with SMPTE time and markers, and bounce-in-place for regions." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "rustgis",
        headline: "A fast, free GIS for mapping, analyzing and sharing.",
        about: "A fast, free and open-source GIS in pure Rust for mapping, analyzing and sharing geospatial data. Native on Windows, macOS and Linux, and in your browser. Your data stays on your machine unless you send it to an AI model you choose, and every action is a command you can script.",
        features: &[
            Feature { title: "Fast and native", text: "One native program drawn on the GPU, with no runtime and no server to run, plus a WebAssembly build at web.rustgis.app." },
            Feature { title: "1,000+ analysis tools", text: "GeoLibre and Whitebox tools for terrain, hydrology, LiDAR and remote sensing, in a tool gallery and the Tools pane." },
            Feature { title: "Everything is a command", text: "Every ribbon button has a command id, so it can be scripted from rustgis-cli, chained in notebooks or run by the AI Assistant." },
            Feature { title: "Symbology and tables", text: "Graduated colours and colour schemes, attribute tables with synced selection, and a layered Layers pane." },
            Feature { title: "A 3D globe", text: "A 3D globe view, Equal Earth, Robinson or any EPSG projection, plus web basemaps and raster support (GeoTIFF and COG)." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, web, Android",
    },
    AppDetails {
        id: "tracedraw",
        headline: "Vector illustration, page layout and print.",
        about: "An open-source, clean-room vector editor written from scratch in pure Rust: pages and layers, curves and nodes, text, fills and outlines, live effects and symbols, and real .cdr files. In one native program for Windows, macOS and Linux, and in the browser.\n\nAlpha: expect bugs.",
        features: &[
            Feature { title: "A full workspace", text: "Menu bar, context-sensitive property bar, toolbox with flyouts, 41 dockable panels, palette, rulers, guides and a page navigator." },
            Feature { title: "Your files", text: "A clean-room .cdr reader (versions 1–27) and writer (versions 7–27), plus SVG, PDF, AI, EPS, DXF, PSD, EMF, WMF, HPGL and text documents." },
            Feature { title: "Native and light", text: "One program per platform built with egui, a CPU renderer for exact output and no runtime to install; no Electron." },
            Feature { title: "Runs in the browser", text: "The whole editor compiles to WebAssembly and runs at tracedraw.github.io — nothing to install; on a tablet it installs as a full-screen, offline-capable app." },
            Feature { title: "Ready for scripts and agents", text: "Every change is a command with undo, so the same engine answers the window, the CLI, JavaScript and an MCP server." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, web, Android",
    },
    AppDetails {
        id: "transit",
        headline: "A DAW with native ASIO and measured latency.",
        about: "An open-source DAW in pure Rust with Ableton/Logic-style ergonomics and its own audio engine, device layer and egui UI. It has a native ASIO backend written from the protocol up, latency you can measure in the app, and plug-in loading that cannot freeze the window.\n\nAlpha software under active development; releases are frequent and list known issues.",
        features: &[
            Feature { title: "Native ASIO", text: "An ASIO backend written from the protocol up, without the Steinberg SDK, plus native WASAPI." },
            Feature { title: "Measured latency", text: "Latency you can measure in the app, not just read off a box, checked against reference hardware." },
            Feature { title: "Crash-free plug-in loading", text: "Plug-ins load without freezing the window, with delay compensation and multi-core rendering by default." },
            Feature { title: "Ableton and Logic ergonomics", text: "Clip and arrangement editing, a sampler (Pads, Slices and Instrument views with kit presets) and device rack design." },
            Feature { title: "Honest development", text: "Design rounds come first, every defect gets an escape record, and what is known broken is public." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "coimanide",
        headline: "A lightweight code editor and IDE in Rust.",
        about: "A lightweight code editor and IDE written in Rust with egui: language servers, debugging, Git, run configurations, Maven, and the VS Code themes, snippets and language servers you already have.\n\nNo telemetry, no network access, no account and no AI. The config folder lives next to Windows' %APPDATA% or the XDG config directory.",
        features: &[
            Feature { title: "Language servers", text: "Language servers for a dozen languages, with the VS Code extensions and snippets you already have." },
            Feature { title: "Debugging", text: "CodeLLDB, debugpy, Delve, js-debug and gdb integration, run configurations and tests for every language." },
            Feature { title: "Familiar environment", text: "Cargo and Maven support, Vim and Helix modes, local history and live previews." },
            Feature { title: "VS Code themes", text: "Any VS Code theme, chosen in Preferences: Color Theme — Dark Modern and your existing themes included." },
            Feature { title: "Private and offline", text: "No telemetry, no network access, no account and no AI; settings open in a window or as a TOML file." },
        ],
        shots: &[],
        platforms: "Windows, Linux, Android",
    },
    AppDetails {
        id: "keydeck",
        headline: "Open and edit Keynote presentations.",
        about: "Free desktop apps that open, edit and save Apple Keynote files — and work with PowerPoint too. Presentations on your computer with no account, no subscription and no tracking. Part of Bramble Office, alongside Values and Letters.\n\nOn Android 8+, beta.",
        features: &[
            Feature { title: "Real .key files", text: "Opens and saves Keynote presentations, and PowerPoint .pptx too." },
            Feature { title: "Present like Keynote", text: "Transitions and builds including Magic Move, dynamic backgrounds, a presenter display, recorded slideshows and 18 built-in themes." },
            Feature { title: "Keydeck Remote", text: "Run a slideshow on your computer from an Android phone on the same Wi-Fi, with your notes, the next slide, a timer and a laser pointer." },
            Feature { title: "Export anywhere", text: "Exports PDF, images, movies, animated GIFs and HTML, with charts linked to tables." },
            Feature { title: "Content Hub", text: "Openly licensed photos and illustrations from Wikimedia Commons, and over 1,200 shapes." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "values",
        headline: "Open and edit Numbers spreadsheets.",
        about: "Free desktop apps that open, edit and save Apple Numbers files — and work with Excel too. Spreadsheets on your computer with no account, no subscription and no tracking. Part of Bramble Office, alongside Keydeck and Letters.\n\nOn Android 8+, beta.",
        features: &[
            Feature { title: "Real .numbers files", text: "Opens and saves Numbers spreadsheets, and Microsoft Excel .xlsx too — formulas and formatting kept as Excel." },
            Feature { title: "Full spreadsheet power", text: "Formulas, charts, pivot tables, categories, filters and conditional highlighting." },
            Feature { title: "Export anywhere", text: "Exports PDF, CSV and TSV, with print settings that match the app you see on screen." },
            Feature { title: "Content Hub", text: "Openly licensed photos and illustrations from Wikimedia Commons, and over 1,200 shapes." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "letters",
        headline: "Open and edit Pages documents.",
        about: "Free desktop apps that open, edit and save Apple Pages files — and work with Word too. Word processing and page layout on your computer with no account, no subscription and no tracking. Part of Bramble Office, alongside Keydeck and Values.\n\nOn Android 8+, beta.",
        features: &[
            Feature { title: "Real .pages files", text: "Opens and saves Pages documents, and Word .docx, RTF, HTML, Markdown and text too." },
            Feature { title: "Word processing", text: "Styles, tables, lists, sections, live text wrap around pictures and shapes, and a page layout mode for designed documents." },
            Feature { title: "Export anywhere", text: "Exports PDF, Word, RTF, EPUB and HTML, with layout that survives the trip." },
            Feature { title: "Content Hub", text: "Openly licensed photos and illustrations from Wikimedia Commons, and over 1,200 shapes." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
    AppDetails {
        id: "procraft",
        headline: "Painting for tablets, with its own Rust engine.",
        about: "An open-source painting app for Android tablets (and Windows and Linux), with its own Rust engine and GPU renderer: a Kotlin/Jetpack Compose UI on Android, and the same interface in egui on the desktop. Pressure-sensitive brushes, layers with blend modes, adjustments, selection and transform, animation and 3D painting — written from scratch.",
        features: &[
            Feature { title: "Painting", text: "Eleven sets of brushes written from scratch, pen pressure for size and opacity, and a Brush Studio to shape tips, wet mix, grain and colour dynamics." },
            Feature { title: "Layers", text: "Blend modes, opacity, alpha lock, clipping masks, masks and groups; artworks with hundreds of layers stay fluid, kept compressed off the GPU." },
            Feature { title: "Animation", text: "Frame-by-frame animation with the Animation Assist, page assist and reference window." },
            Feature { title: "3D painting", text: "Paint on OBJ and USDZ models with texture sets, lights and environments. In progress." },
            Feature { title: "Plugins and agents", text: "Plugins (buttons, panels, colour pickers, new kinds of layer) and MCP support so AI assistants can paint, preview and save." },
        ],
        shots: &[],
        platforms: "Android, Windows, Linux",
    },
    AppDetails {
        id: "capcraft",
        headline: "Mobile video editing, open source.",
        about: "CapCraft is an open-source reimplementation of the CapCut experience for Android, written in Rust with a Kotlin/Jetpack Compose UI. Timeline editing with keyframes, 100+ filters, transitions, text templates, captions and sound — no account and no watermark.\n\nA beta, built by Bearinmind. Not affiliated with or endorsed by the makers of CapCut.",
        features: &[
            Feature { title: "Timeline editing", text: "Cut, split, trim, ripple, speed ramps, keyframes and a multi-track timeline with stickers, effects and sound rows." },
            Feature { title: "Filters and effects", text: "80+ filters previewed on the clip itself, video effects on shelves of motion, light, distort, retro, stylize and blur, and animated transitions." },
            Feature { title: "Text and captions", text: "Text templates, fonts and styles, captions made on the phone from speech, auto captions and lyrics, and import." },
            Feature { title: "Sound and export", text: "Built-in sound effects and voice handling, and export as video, GIF or photo at 480p to 4K." },
        ],
        shots: &[],
        platforms: "Android",
    },
    AppDetails {
        id: "concat",
        headline: "The free, open-source, local-first video editor.",
        about: "A free, open-source video editor for macOS, Windows, Linux and Android. It covers what most edits need: auto-captions, text-to-speech, background removal, keyframe animation, effects and titles, multi-track cutting, 4K export — with none of the catches: no watermark, no account, no subscription, no upload.\n\nEverything runs on your machine, and the AI models download once and work offline after that.",
        features: &[
            Feature { title: "Local AI", text: "Auto-captions, text-to-speech with voice cloning, and background removal — all on your machine, offline after the models download once." },
            Feature { title: "Cut fast", text: "Split, trim, ripple, merge, freeze frame and speed changes on a magnetic timeline, with multi-track overlays and picture-in-picture." },
            Feature { title: "Effects and titles", text: "100 effects, filters and transitions live in the preview; titles with fonts, stroke, shadow, plates and presets." },
            Feature { title: "Scriptable", text: "A built-in MCP server and a JSON-RPC API, so scripts and AI agents can edit video with it too." },
            Feature { title: "Nothing leaves your disk", text: "Your footage never leaves your machine; nothing is uploaded, no account and no watermark, ever." },
        ],
        shots: &[],
        platforms: "Windows, macOS, Linux, Android",
    },
];

/// The apps without a core page: anything in the Android catalog that artcraft-launcher-core's
/// suite does not carry one for.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_belongs_to_the_catalog() {
        let mut ids: Vec<_> = ABOUT.iter().map(|d| d.id).collect();
        for id in &ids {
            assert!(crate::catalog::app_by_id(id).is_some(), "{id} is not in the Android catalog");
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ABOUT.len());
    }

    #[test]
    fn every_android_app_without_a_core_page_has_one() {
        for app in &crate::catalog::SUITE_ANDROID {
            if artcraft_launcher_core::details::AppDetails::by_id(app.id).is_some() {
                continue;
            }
            assert!(ABOUT.iter().any(|d| d.id == app.id), "{} has no page in about.json", app.id);
        }
    }

    #[test]
    fn every_page_is_filled_in() {
        for page in ABOUT {
            assert!(!page.headline.is_empty(), "{}: headline", page.id);
            assert!(!page.about.is_empty() && page.paragraphs().next().is_some(), "{}: about", page.id);
            assert!(!page.features.is_empty(), "{}: features", page.id);
            for feature in page.features {
                assert!(!feature.title.is_empty() && !feature.text.is_empty(), "{}: feature", page.id);
            }
            assert!(!page.platforms.is_empty(), "{}: platforms", page.id);
        }
    }
}
