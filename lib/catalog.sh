#!/usr/bin/env bash
# App catalog metadata and repository origin mapping for ArtCraft Android.
# Extracted from lib/build-common.sh as part of modularization (issue #73).

resolve_app_catalog() {
    local app="$1"

    case "$app" in
        photocraft) TITLE="PhotoCraft" ;;
        vectorcraft) TITLE="VectorCraft" ;;
        filmcraft) TITLE="FilmCraft" ;;
        lightcraft) TITLE="LightCraft" ;;
        pdfcraft) TITLE="PdfCraft" ;;
        effectcraft) TITLE="EffectCraft" ;;
        designcraft) TITLE="DesignCraft" ;;
        figcraft) TITLE="FigCraft" ;;
        wordcraft) TITLE="WordCraft" ;;
        soundcraft) TITLE="SoundCraft" ;;
        deckcraft) TITLE="DeckCraft" ;;
        cadcraft) TITLE="CADCraft" ;;
        gridcraft) TITLE="GridCraft" ;;
        subcraft) TITLE="SubCraft" ;;
        solidcraft) TITLE="SolidCraft" ;;
        materialcraft) TITLE="MaterialCraft" ;;
        texturecraft) TITLE="TextureCraft" ;;
        notecraft) TITLE="NoteCraft" ;;
        diagramcraft) TITLE="DiagramCraft" ;;
        paintcraft) TITLE="PaintCraft" ;;
        bimcraft) TITLE="BIMCraft" ;;
        tracedraw) TITLE="TraceDraw" ;;
        picassosuite) TITLE="PicassoSuite" ;;
        capsulecraft) TITLE="CapsuleCraft" ;;
        humanmotioncraft) TITLE="HumanMotionCraft" ;;
        rustgis) TITLE="RustGIS" ;;
        earthcraft) TITLE="EarthCraft" ;;
        mapcraft) TITLE="MapCraft" ;;
        prowave) TITLE="ProWave" ;;
        transit) TITLE="Transit" ;;
        anicraft) TITLE="AniCraft" ;;
        coimanide) TITLE="Coiman IDE" ;;
        pcbcraft) TITLE="PCBCraft" ;;
        fieldcraft) TITLE="FieldCraft" ;;
        papercraft) TITLE="PaperCraft" ;;
        ferrite) TITLE="Ferrite" ;;
        pixelcraft) TITLE="PixelCraft" ;;
        *) echo "Unsupported app: $app" >&2; usage; exit 2 ;;
    esac

    SRC="source/$app"
    REPO="https://github.com/storytold/$app.git"
    APP_ORIGIN="https://github.com/storytold/$app"

    # Community apps with third-party origins
    if [[ "$app" == "subcraft" ]]; then
        REPO="https://github.com/Aalzard/subcraft.git"
        APP_ORIGIN="https://github.com/Aalzard/subcraft"
    elif [[ "$app" == "solidcraft" ]]; then
        REPO="https://github.com/tangency-labs/solidcraft.git"
        APP_ORIGIN="https://github.com/tangency-labs/solidcraft"
    elif [[ "$app" == "notecraft" ]]; then
        REPO="https://github.com/AlienTechSoftware/notecraft.git"
        APP_ORIGIN="https://github.com/AlienTechSoftware/notecraft"
    elif [[ "$app" == "diagramcraft" ]]; then
        REPO="https://github.com/baradhili/DiagramCraft.git"
        APP_ORIGIN="https://github.com/baradhili/DiagramCraft"
    elif [[ "$app" == "paintcraft" ]]; then
        REPO="https://github.com/replicraft/PaintCraft.git"
        APP_ORIGIN="https://github.com/replicraft/PaintCraft"
    elif [[ "$app" == "bimcraft" ]]; then
        REPO="https://github.com/IPedrax/BIMCraft.git"
        APP_ORIGIN="https://github.com/IPedrax/BIMCraft"
    elif [[ "$app" == "tracedraw" ]]; then
        REPO="https://github.com/tracedraw/tracedraw.git"
        APP_ORIGIN="https://github.com/tracedraw/tracedraw"
    elif [[ "$app" == "picassosuite" ]]; then
        REPO="https://github.com/fabox-os/Picasso-Suite.git"
        APP_ORIGIN="https://github.com/fabox-os/Picasso-Suite"
    elif [[ "$app" == "capsulecraft" ]]; then
        REPO="https://github.com/kostyk348/CapsuleCraft.git"
        APP_ORIGIN="https://github.com/kostyk348/CapsuleCraft"
    elif [[ "$app" == "humanmotioncraft" ]]; then
        REPO="https://github.com/YalKouassi/HumanMotionCraft.git"
        APP_ORIGIN="https://github.com/YalKouassi/HumanMotionCraft"
    elif [[ "$app" == "rustgis" ]]; then
        REPO="https://github.com/opengeos/RustGIS.git"
        APP_ORIGIN="https://github.com/opengeos/RustGIS"
    elif [[ "$app" == "earthcraft" ]]; then
        REPO="https://github.com/therichronicles/EarthCraft.git"
        APP_ORIGIN="https://github.com/therichronicles/EarthCraft"
    elif [[ "$app" == "mapcraft" ]]; then
        REPO="https://github.com/bearlikelion/MapCraft.git"
        APP_ORIGIN="https://github.com/bearlikelion/MapCraft"
    elif [[ "$app" == "prowave" ]]; then
        REPO="https://github.com/fabox-os/Pro-Wave.git"
        APP_ORIGIN="https://github.com/fabox-os/Pro-Wave"
    elif [[ "$app" == "transit" ]]; then
        REPO="https://github.com/transit-audio/transit.git"
        APP_ORIGIN="https://github.com/transit-audio/transit"
    elif [[ "$app" == "anicraft" ]]; then
        REPO="https://github.com/storytold/AniCraft.git"
        APP_ORIGIN="https://github.com/storytold/AniCraft"
    elif [[ "$app" == "coimanide" ]]; then
        REPO="https://github.com/ManuelCoimbra/coiman-ide.git"
        APP_ORIGIN="https://github.com/ManuelCoimbra/coiman-ide"
    elif [[ "$app" == "pcbcraft" ]]; then
        REPO="https://github.com/tangency-labs/pcbcraft.git"
        APP_ORIGIN="https://github.com/tangency-labs/pcbcraft"
    elif [[ "$app" == "fieldcraft" ]]; then
        REPO="https://github.com/tangency-labs/fieldcraft.git"
        APP_ORIGIN="https://github.com/tangency-labs/fieldcraft"
    elif [[ "$app" == "papercraft" ]]; then
        REPO="https://github.com/slyim/papercraft.git"
        APP_ORIGIN="https://github.com/slyim/papercraft"
    elif [[ "$app" == "ferrite" ]]; then
        REPO="https://github.com/OlaProeis/Ferrite.git"
        APP_ORIGIN="https://github.com/OlaProeis/Ferrite"
    elif [[ "$app" == "pixelcraft" ]]; then
        REPO="https://github.com/asad-albadi/PixelCraft.git"
        APP_ORIGIN="https://github.com/asad-albadi/PixelCraft"
    fi

    # Repositories whose releases either lack tags or require specific branches
    case "$app" in
        earthcraft|prowave|transit|rustgis|mapcraft|anicraft|solidcraft|pcbcraft|ferrite)
            if [[ "$BUILD_SOURCE" == "stable" ]]; then
                BUILD_SOURCE="main"
            fi
            ;;
        fieldcraft)
            if [[ "$BUILD_SOURCE" == "stable" ]]; then
                BUILD_SOURCE="v0.8.1"
            fi
            ;;
        papercraft)
            if [[ "$BUILD_SOURCE" == "stable" ]]; then
                BUILD_SOURCE="v0.4.0"
            fi
            ;;
        pixelcraft)
            if [[ "$BUILD_SOURCE" == "stable" ]]; then
                BUILD_SOURCE="v0.11.0"
            fi
            ;;
    esac
}
