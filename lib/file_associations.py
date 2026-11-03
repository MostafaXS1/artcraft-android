"""Android file associations (``ACTION_VIEW`` / ``ACTION_SEND`` intent filters) for the ArtCraft suite.

A single source of truth shared by three callers:

* ``lib/build-common.sh`` (the launcher generator, for the 11 generated apps),
* ``adapters/photocraft/prepare.sh`` (the PhotoCraft adapter),
* the custom-adapter branch in ``lib/build-common.sh`` (the VectorCraft adapter).

For each app we list the *document* extensions it offers to open. Standard extensions are mapped to
the MIME type Android itself resolves for them (that is what actually drives the "Open with" chooser
for ``content://`` URIs); custom project formats have no standard MIME and are matched by path
pattern only. ``intent_filter_toml`` renders the whole ``intent_filter = [...]`` value so every
launcher advertises exactly the same associations.
"""

import re

# Extension (lower-case, no dot) -> MIME types to advertise for it.
#
# The *first* entry per extension is the type Android's own MIME database reports for that
# extension. The mapping is not guesswork: it was verified against AOSP's ``android.mime.types`` and
# ``libcore.net.MimeUtils`` and then checked on a live Android 16 device by writing one file per
# extension and reading back ``MediaStore``'s ``mime_type`` column. Android matches MIME types
# *literally and case-sensitively*, so a filter that spells a type even slightly differently (an
# upper-case ``macroEnabled``, an ``image/bmp`` where Android says ``image/x-ms-bmp`` or an
# ``image/vnd.adobe.photoshop`` where Android says ``image/x-photoshop``) silently never matches.
#
# Further entries are registered/legacy aliases that a provider, file manager or older Android may
# report instead; listing them costs nothing and lets the same APK answer to every spelling. When
# Android reports ``application/octet-stream`` (an extension it does not know, e.g. ``psb`` or
# ``cr3``) there is no type to declare and the filter falls back to the by-extension path pattern.
EXT_MIME = {
    # Word-processing
    "docx": ("application/vnd.openxmlformats-officedocument.wordprocessingml.document",),
    "docm": ("application/vnd.ms-word.document.macroenabled.12", "application/vnd.ms-word.document.macroEnabled.12"),
    "dotx": ("application/vnd.openxmlformats-officedocument.wordprocessingml.template",),
    "dotm": ("application/vnd.ms-word.template.macroenabled.12", "application/vnd.ms-word.template.macroEnabled.12"),
    "doc": ("application/msword",),
    "dot": ("application/msword",),
    "rtf": ("text/rtf", "application/rtf"),
    "odt": ("application/vnd.oasis.opendocument.text",),
    "txt": ("text/plain",),
    "md": ("text/markdown",),
    "markdown": ("text/markdown",),
    # Spreadsheets
    "xlsx": ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",),
    "xlsm": ("application/vnd.ms-excel.sheet.macroenabled.12", "application/vnd.ms-excel.sheet.macroEnabled.12"),
    "xltx": ("application/vnd.openxmlformats-officedocument.spreadsheetml.template",),
    "xltm": ("application/vnd.ms-excel.template.macroenabled.12", "application/vnd.ms-excel.template.macroEnabled.12"),
    "xls": ("application/vnd.ms-excel",),
    "xlt": ("application/vnd.ms-excel",),
    "xlsb": ("application/vnd.ms-excel.sheet.binary.macroenabled.12", "application/vnd.ms-excel.sheet.binary.macroEnabled.12"),
    "ods": ("application/vnd.oasis.opendocument.spreadsheet",),
    "csv": ("text/comma-separated-values", "text/csv"),
    "tsv": ("text/tab-separated-values",),
    "tab": ("text/tab-separated-values",),
    # Presentations
    "pptx": ("application/vnd.openxmlformats-officedocument.presentationml.presentation",),
    "potx": ("application/vnd.openxmlformats-officedocument.presentationml.template",),
    "ppsx": ("application/vnd.openxmlformats-officedocument.presentationml.slideshow",),
    # Page layout / print
    "pdf": ("application/pdf",),
    "idml": ("application/vnd.adobe.indesign-idml-package",),
    "indd": ("application/x-indesign",),
    "indt": ("application/x-indesign",),
    # Vector graphics
    "svg": ("image/svg+xml",),
    "svgz": ("image/svg+xml",),
    "ai": ("application/postscript",),
    "ait": ("application/postscript",),
    "eps": ("application/postscript",),
    "dxf": ("image/vnd.dxf",),
    "emf": ("image/emf",),
    "wmf": ("image/wmf",),
    # Raster / photographic
    "png": ("image/png",),
    "jpg": ("image/jpeg",),
    "jpeg": ("image/jpeg",),
    "gif": ("image/gif",),
    "webp": ("image/webp",),
    "tif": ("image/tiff",),
    "tiff": ("image/tiff",),
    "bmp": ("image/x-ms-bmp", "image/bmp"),
    "tga": ("image/x-tga",),
    "ico": ("image/x-icon",),
    "qoi": ("image/qoi",),
    "exr": ("image/x-exr",),
    "hdr": ("image/vnd.radiance",),
    "heic": ("image/heic",),
    "heif": ("image/heif",),
    "avif": ("image/avif",),
    "jxl": ("image/jxl",),
    "psd": ("image/x-photoshop", "image/vnd.adobe.photoshop"),
    "psb": ("image/x-photoshop", "image/vnd.adobe.photoshop"),
    "psdt": ("image/x-photoshop", "image/vnd.adobe.photoshop"),
    "pdn": ("image/x-paintnet",),
    "ora": ("image/openraster",),
    "kra": ("application/x-krita",),
    "krz": ("application/x-krita",),
    # Camera raw
    "dng": ("image/x-adobe-dng",),
    "cr2": ("image/x-canon-cr2",),
    "cr3": ("image/x-canon-cr3",),
    "nef": ("image/x-nikon-nef",),
    "nrw": ("image/x-nikon-nrw",),
    "arw": ("image/x-sony-arw",),
    "pef": ("image/x-pentax-pef",),
    "orf": ("image/x-olympus-orf",),
    "rw2": ("image/x-panasonic-rw2",),
    "raf": ("image/x-fuji-raf", "image/x-fujifilm-raf"),
    "rwl": ("image/x-leica-rwl",),
    "srw": ("image/x-samsung-srw",),
    "crw": ("image/x-canon-crw",),
    "mrw": ("image/x-minolta-mrw",),
    "x3f": ("image/x-sigma-x3f",),
    "kdc": ("image/x-kodak-kdc",),
    "erf": ("image/x-epson-erf",),
    "3fr": ("image/x-hasselblad-3fr",),
    # Video
    "mp4": ("video/mp4",),
    "m4v": ("video/mp4", "video/x-m4v"),
    "mov": ("video/quicktime",),
    "mkv": ("video/x-matroska",),
    "webm": ("video/webm",),
    "avi": ("video/avi", "video/x-msvideo"),
    "mxf": ("application/mxf",),
    "mts": ("video/mp2t",),
    "m2ts": ("video/mp2t",),
    "m2t": ("video/mpeg", "video/mp2t"),
    "ts": ("video/mp2ts", "video/mp2t"),
    "mpg": ("video/mpeg",),
    "mpeg": ("video/mpeg",),
    "vob": ("video/mpeg",),
    "mod": ("video/mpeg",),
    "tod": ("video/mp2t",),
    "m2v": ("video/mpeg",),
    "m1v": ("video/mpeg",),
    "mpv": ("video/x-matroska", "video/mpeg"),
    "3gp": ("video/3gpp",),
    "y4m": ("video/x-yuv4mpeg",),
    # Audio
    "wav": ("audio/x-wav",),
    "wave": ("audio/x-wav",),
    "bwf": ("audio/x-wav",),
    "aif": ("audio/x-aiff",),
    "aiff": ("audio/x-aiff",),
    "aifc": ("audio/x-aiff",),
    "mp3": ("audio/mpeg",),
    "mp2": ("audio/mpeg",),
    "m4a": ("audio/mpeg", "audio/mp4"),
    "m4b": ("audio/mp4",),
    "aac": ("audio/aac",),
    "flac": ("audio/flac",),
    "ogg": ("audio/ogg",),
    "oga": ("audio/ogg",),
    "opus": ("audio/ogg", "audio/opus"),
    "caf": ("audio/x-caf",),
    "mid": ("audio/midi",),
    "midi": ("audio/midi",),
    "smf": ("audio/sp-midi", "audio/midi"),
    # Captions
    "srt": ("application/x-subrip",),
    "vtt": ("text/vtt",),
    "scc": ("application/scc",),
    # 3D models (SubCraft's project and mesh formats)
    "subcraft": ("application/x-subcraft",),
    "obj": ("model/obj",),
    "gltf": ("model/gltf+json",),
    "glb": ("model/gltf-binary",),
    "fbx": ("application/octet-stream",),
}

# App -> the document extensions it offers to open. Kept to the app's own document formats (a
# spreadsheet does not claim every .txt, an image editor does not claim PDFs) so the chooser stays
# useful; the raw, order-sensitive extension lists live in the app sources this was derived from.
ASSOCIATIONS = {
    "wordcraft": ["docx", "docm", "dotx", "dotm", "doc", "dot", "rtf", "odt", "txt", "md", "markdown"],
    "gridcraft": ["xlsx", "xlsm", "xltx", "xltm", "xls", "xlt", "xlsb", "ods", "csv", "tsv"],
    "deckcraft": ["deckcraft", "pptx", "potx", "ppsx"],
    "designcraft": ["designcraft", "idml", "indd", "indt", "pdf"],
    "vectorcraft": ["vectorcraft", "drawcraft", "svg", "svgz", "ai", "ait", "eps", "pdf", "dxf", "emf", "wmf", "vctemplate"],
    "photocraft": [
        "pcraft", "psd", "psb", "psdt", "pdn", "ora", "kra", "krz",
        "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp", "tga", "ico", "qoi", "exr", "hdr",
        "heic", "heif", "avif", "svg", "svgz",
        "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf",
    ],
    "lightcraft": [
        "jpg", "jpeg", "png", "tif", "tiff", "webp", "gif", "bmp", "heic", "heif", "avif", "jxl",
        "dng", "cr2", "cr3", "nef", "nrw", "arw", "raf", "orf", "rw2", "rwl", "pef", "srw", "crw",
        "mrw", "x3f", "kdc", "erf", "3fr", "psd", "lrcat",
    ],
    "filmcraft": [
        "fcproj",
        "mp4", "m4v", "mov", "mkv", "webm", "avi", "mxf", "mts", "m2ts", "m2t", "ts", "mpg",
        "mpeg", "vob", "mod", "tod", "m2v", "m1v", "mpv", "3gp", "y4m",
        "wav", "wave", "bwf", "aif", "aiff", "mp3", "mp2", "m4a", "aac", "flac", "ogg", "opus",
        "srt", "vtt", "scc", "edl", "fcpxml", "otio", "aaf", "omf",
    ],
    "soundcraft": [
        "scraft", "mid", "midi", "smf",
        "wav", "wave", "bwf", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "m4a", "m4b",
        "aac", "caf", "mp4", "mkv", "webm",
    ],
    "pdfcraft": ["pdf"],
    "figcraft": ["figcraft", "fig"],
    "effectcraft": ["ecproj", "ecprojx", "ectemplate"],
    "cadcraft": ["dwg", "dwt", "dxf"],
    "subcraft": ["subcraft", "obj", "gltf", "glb", "fbx"],
}

MAIN_FILTER = '{ actions = ["android.intent.action.MAIN"], categories = ["android.intent.category.LAUNCHER"] }'


def _mime_types(app):
    seen = []
    for ext in ASSOCIATIONS.get(app, []):
        for mime in EXT_MIME.get(ext, ()):
            if mime not in seen:
                seen.append(mime)
    return seen


def _path_pattern(ext):
    """Return the TOML spelling of the glob that matches ``.<ext>`` at the end of a path.

    The dot has to be a literal, because ``.`` is itself a wildcard in an Android path pattern
    (``.*`` means "any sequence"). A literal dot is written ``\\.`` in the glob, and that spelling
    travels through two unescaping steps on its way to the device: TOML in ``Cargo.toml``
    (``\\\\`` -> ``\\``) and then aapt2 compiling the manifest attribute (``\\.`` -> ``.``). So the
    manifest text needs a doubled backslash and the TOML text a doubled one again, which is what
    ``manifest_pattern`` and the ``replace`` below build.
    """
    manifest_pattern = ".*\\\\." + ext
    return manifest_pattern.replace("\\", "\\\\")


def _by_extension_data(exts, typed):
    """Render the ``<data>`` elements that match ``exts`` by file name.

    ``host = "*"`` is not optional. Android only evaluates a path pattern when the ``<data>``
    element also declares an authority: AOSP ``IntentFilter.matchData`` performs the path check
    inside its ``if (mDataAuthorities != null)`` branch. A scheme-plus-path element with no host
    therefore degenerates into a scheme-only match, i.e. it claims *every* ``file://`` and
    ``content://`` URI -- which is what made the apps offer themselves for APKs and unrelated
    files. The wildcard authority matches any host and, on devices that enable wildcard filtering,
    an absent one too, so ``file://`` URIs still match; the path pattern keeps the match to the
    app's own documents either way.

    With ``typed`` the element also declares ``mime_type = "*/*"`` so that it matches intents that
    carry a type; without it, no type is declared and the element matches only typeless intents.
    """
    extra = ', mime_type = "*/*"' if typed else ""
    return ", ".join(
        '{{ scheme = "{0}", host = "*", path_pattern = "{1}"{2} }}'.format(
            scheme, _path_pattern(ext), extra
        )
        for ext in exts
        for scheme in ("content", "file")
    )


def intent_filter_toml(app):
    """Render the complete inline ``intent_filter = [...]`` *value* for ``app``.

    Starts with the MAIN/LAUNCHER filter the launcher needs and appends, when the app has file
    associations, the VIEW/SEND filters. Android's data matching applies every declared MIME type
    and every declared URI component in a single filter, so the MIME-only lookup (what a MediaStore
    ``content://`` URI carries), the browser handoff, the share targets and the by-extension path
    patterns are each their own element rather than one combined filter.
    """
    filters = [MAIN_FILTER]
    exts = ASSOCIATIONS.get(app, [])
    mimes = _mime_types(app)

    if mimes:
        data = ", ".join('{{ mime_type = "{0}" }}'.format(m) for m in mimes)
        filters.append(
            '{{ actions = ["android.intent.action.VIEW"], categories = ["android.intent.category.DEFAULT"], data = [{0}] }}'.format(data)
        )
        # Let a browser hand a remote document straight to the app too (content:// links already
        # match above through their MIME type).
        web = ", ".join(
            '{{ scheme = "{0}", mime_type = "{1}" }}'.format(scheme, mime)
            for mime in mimes
            for scheme in ("http", "https")
        )
        filters.append(
            '{{ actions = ["android.intent.action.VIEW"], categories = ["android.intent.category.DEFAULT", "android.intent.category.BROWSABLE"], data = [{0}] }}'.format(web)
        )
        share = ", ".join('{{ mime_type = "{0}" }}'.format(m) for m in mimes)
        filters.append(
            '{{ actions = ["android.intent.action.SEND", "android.intent.action.SEND_MULTIPLE"], categories = ["android.intent.category.DEFAULT"], data = [{0}] }}'.format(share)
        )

    if exts:
        # By-extension match, for a provider that reports a generic type (application/octet-stream)
        # for an app's own project format, or for a document whose type has no MIME entry. No MIME
        # type is declared, so a *typeless* ACTION_VIEW from a file manager matches here.
        filters.append(
            '{{ actions = ["android.intent.action.VIEW"], categories = ["android.intent.category.DEFAULT"], data = [{0}] }}'.format(
                _by_extension_data(exts, typed=False)
            )
        )
        # Typed variant of the same match: a filter that declares a type only matches intents that
        # carry one, and many providers and file managers stamp `application/octet-stream` (or
        # another guess) on an extension Android does not know. `*/*` accepts whatever type was
        # guessed while the path pattern still limits the match to this app's own extensions.
        #
        # ACTION_SEND is deliberately not offered here: a share intent carries its file in
        # EXTRA_STREAM and has no data URI, and a filter that declares a scheme only matches intents
        # that have one, so a by-name share target cannot be expressed without claiming every shared
        # type. Known types are still shareable through the type-only SEND filter above.
        filters.append(
            '{{ actions = ["android.intent.action.VIEW"], categories = ["android.intent.category.DEFAULT"], data = [{0}] }}'.format(
                _by_extension_data(exts, typed=True)
            )
        )

    return "[" + ", ".join(filters) + "]"


def patch_cargo_toml(path, app):
    """Replace the single ``intent_filter = [...]`` line in a copied launcher manifest.

    Returns True when the file was rewritten and False when the app has no associations (or the
    manifest has no intent_filter line to patch).
    """
    path = str(path)
    with open(path, "r", encoding="utf-8") as handle:
        text = handle.read()
    if "intent_filter" not in text:
        return False
    value = intent_filter_toml(app)
    new_text, count = re.subn(r"(?m)^intent_filter = .*$", lambda _m: "intent_filter = " + value, text, count=1)
    if count == 0:
        return False
    if new_text != text:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(new_text)
    return True
