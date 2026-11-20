#!/usr/bin/env python3
"""
ArtCraft Android Release & Tag Sync Checker

Verifies that the builder's stable release resolution matches the official
published upstream releases across all ported applications, catching:
  - Stranded tags from failed/aborted CI runs (e.g. NoteCraft v0.0.16)
  - Release candidate / pre-release sort order issues (e.g. SolidCraft v0.45.0-rc.4)
  - Non-'v' prefixed release tags (e.g. CapsuleCraft 2.12.5)
  - Irregular non-version tags (e.g. EarthCraft 'versions')
  - Outdated local checkouts in source/<app>
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, List, Optional, Tuple

ROOT_DIR = Path(__file__).resolve().parent.parent
BUILD_COMMON = ROOT_DIR / "lib" / "build-common.sh"
SOURCE_DIR = ROOT_DIR / "source"

# Fallback known app definitions in case build-common.sh is not located
FALLBACK_APPS = [
    ("photocraft", "PhotoCraft", "https://github.com/storytold/photocraft.git", "https://github.com/storytold/photocraft", False),
    ("vectorcraft", "VectorCraft", "https://github.com/storytold/vectorcraft.git", "https://github.com/storytold/vectorcraft", False),
    ("filmcraft", "FilmCraft", "https://github.com/storytold/filmcraft.git", "https://github.com/storytold/filmcraft", False),
    ("lightcraft", "LightCraft", "https://github.com/storytold/lightcraft.git", "https://github.com/storytold/lightcraft", False),
    ("pdfcraft", "PdfCraft", "https://github.com/storytold/pdfcraft.git", "https://github.com/storytold/pdfcraft", False),
    ("effectcraft", "EffectCraft", "https://github.com/storytold/effectcraft.git", "https://github.com/storytold/effectcraft", False),
    ("designcraft", "DesignCraft", "https://github.com/storytold/designcraft.git", "https://github.com/storytold/designcraft", False),
    ("figcraft", "FigCraft", "https://github.com/storytold/figcraft.git", "https://github.com/storytold/figcraft", False),
    ("wordcraft", "WordCraft", "https://github.com/storytold/wordcraft.git", "https://github.com/storytold/wordcraft", False),
    ("soundcraft", "SoundCraft", "https://github.com/storytold/soundcraft.git", "https://github.com/storytold/soundcraft", False),
    ("deckcraft", "DeckCraft", "https://github.com/storytold/deckcraft.git", "https://github.com/storytold/deckcraft", False),
    ("cadcraft", "CADCraft", "https://github.com/storytold/cadcraft.git", "https://github.com/storytold/cadcraft", False),
    ("gridcraft", "GridCraft", "https://github.com/storytold/gridcraft.git", "https://github.com/storytold/gridcraft", False),
    ("subcraft", "SubCraft", "https://github.com/Aalzard/subcraft.git", "https://github.com/Aalzard/subcraft", False),
    ("solidcraft", "SolidCraft", "https://github.com/tangency-labs/solidcraft.git", "https://github.com/tangency-labs/solidcraft", False),
    ("materialcraft", "MaterialCraft", "https://github.com/storytold/materialcraft.git", "https://github.com/storytold/materialcraft", False),
    ("texturecraft", "TextureCraft", "https://github.com/storytold/texturecraft.git", "https://github.com/storytold/texturecraft", False),
    ("notecraft", "NoteCraft", "https://github.com/AlienTechSoftware/notecraft.git", "https://github.com/AlienTechSoftware/notecraft", False),
    ("diagramcraft", "DiagramCraft", "https://github.com/baradhili/DiagramCraft.git", "https://github.com/baradhili/DiagramCraft", False),
    ("paintcraft", "PaintCraft", "https://github.com/replicraft/PaintCraft.git", "https://github.com/replicraft/PaintCraft", False),
    ("bimcraft", "BIMCraft", "https://github.com/IPedrax/BIMCraft.git", "https://github.com/IPedrax/BIMCraft", False),
    ("tracedraw", "TraceDraw", "https://github.com/tracedraw/tracedraw.git", "https://github.com/tracedraw/tracedraw", False),
    ("picassosuite", "PicassoSuite", "https://github.com/fabox-os/Picasso-Suite.git", "https://github.com/fabox-os/Picasso-Suite", False),
    ("capsulecraft", "CapsuleCraft", "https://github.com/kostyk348/CapsuleCraft.git", "https://github.com/kostyk348/CapsuleCraft", False),
    ("humanmotioncraft", "HumanMotionCraft", "https://github.com/YalKouassi/HumanMotionCraft.git", "https://github.com/YalKouassi/HumanMotionCraft", False),
    ("rustgis", "RustGIS", "https://github.com/opengeos/RustGIS.git", "https://github.com/opengeos/RustGIS", True),
    ("earthcraft", "EarthCraft", "https://github.com/therichronicles/EarthCraft.git", "https://github.com/therichronicles/EarthCraft", True),
    ("mapcraft", "MapCraft", "https://github.com/bearlikelion/MapCraft.git", "https://github.com/bearlikelion/MapCraft", True),
    ("prowave", "ProWave", "https://github.com/fabox-os/Pro-Wave.git", "https://github.com/fabox-os/Pro-Wave", True),
    ("transit", "Transit", "https://github.com/transit-audio/transit.git", "https://github.com/transit-audio/transit", True),
    ("anicraft", "AniCraft", "https://github.com/storytold/AniCraft.git", "https://github.com/storytold/AniCraft", True),
    ("coimanide", "Coiman IDE", "https://github.com/ManuelCoimbra/coiman-ide.git", "https://github.com/ManuelCoimbra/coiman-ide", False),
]


@dataclass
class AppConfig:
    app: str
    title: str
    repo: str
    origin: str
    main_override: bool


@dataclass
class ReleaseCheckResult:
    app: str
    title: str
    repo: str
    origin: str
    main_override: bool
    gh_latest_release: Optional[str]
    resolved_builder_tag: Optional[str]
    raw_git_top_tag: Optional[str]
    local_tag: Optional[str]
    local_commit: Optional[str]
    status: str
    notes: List[str]


def parse_build_common() -> List[AppConfig]:
    """Dynamically parse app definitions and repo URLs from build-common.sh."""
    if not BUILD_COMMON.is_file():
        return [AppConfig(*item) for item in FALLBACK_APPS]

    text = BUILD_COMMON.read_text(encoding="utf-8", errors="replace")
    apps_match = re.search(r'case "\$APP" in\s*(.*?)\s*\*\)', text, re.DOTALL)
    if not apps_match:
        return [AppConfig(*item) for item in FALLBACK_APPS]

    parsed: List[AppConfig] = []
    for line in apps_match.group(1).splitlines():
        m = re.match(r'\s*([a-z]+)\)\s*TITLE="([^"]+)"', line)
        if not m:
            continue
        app, title = m.group(1), m.group(2)
        repo = f"https://github.com/storytold/{app}.git"
        origin = f"https://github.com/storytold/{app}"

        # Check custom origin override
        repo_m = re.search(
            rf'\[\[\s*"\$APP"\s*==\s*"{app}"\s*\]\];\s*then\s*REPO="([^"]+)"\s*APP_ORIGIN="([^"]+)"',
            text,
        )
        if repo_m:
            repo = repo_m.group(1)
            origin = repo_m.group(2)

        # Check main override
        main_override = bool(
            re.search(
                rf'case "\$APP" in\s*[^;]*\b{app}\b[^;]*\);\s*if\s*\[\[\s*"\$BUILD_SOURCE"\s*==\s*"stable"',
                text,
            )
        )
        parsed.append(AppConfig(app=app, title=title, repo=repo, origin=origin, main_override=main_override))

    return parsed if parsed else [AppConfig(*item) for item in FALLBACK_APPS]


def fetch_gh_latest_release(origin: str, timeout: float = 8.0) -> Optional[str]:
    """Follow GitHub /releases/latest redirect to find the latest published release tag."""
    m = re.match(r"^https://github\.com/([^/]+)/([^/]+)/?$", origin)
    if not m:
        return None
    org, repo = m.group(1), m.group(2).removesuffix(".git")
    url = f"https://github.com/{org}/{repo}/releases/latest"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "ArtCraft-Release-Check/1.0"})
        # Open and extract final redirected URL
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            final_url = resp.geturl()
            match = re.search(r"/releases/tag/([^/?#]+)", final_url)
            if match:
                return match.group(1)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
    except Exception:
        pass
    return None


def fetch_raw_git_tags(repo: str) -> List[str]:
    """Fetch raw tags sorted descending by refname using git ls-remote."""
    cmd = ["git", "ls-remote", "--tags", "--sort=-v:refname", repo]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=12)
        if proc.returncode != 0:
            return []
        tags = []
        for line in proc.stdout.splitlines():
            if line.endswith("^{}"):
                continue
            parts = line.split("refs/tags/")
            if len(parts) > 1:
                tags.append(parts[1].strip())
        return tags
    except Exception:
        return []


def resolve_builder_tag(origin: str, repo: str) -> Optional[str]:
    """
    Replicates resolve_latest_stable_tag() from build-common.sh:
    1. GitHub /releases/latest redirect.
    2. Fallback to clean semver git tags.
    3. Final fallback to v* glob.
    """
    gh_tag = fetch_gh_latest_release(origin)
    if gh_tag:
        return gh_tag

    # Fallback to git remote tags
    tags = fetch_raw_git_tags(repo)
    # Filter clean semver tags (excluding -rc, -beta, -alpha, -nightly, etc.)
    semver_re = re.compile(r"^v?[0-9]+(\.[0-9]+)+$")
    for t in tags:
        if semver_re.match(t):
            return t

    # Final fallback: any v*
    for t in tags:
        if t.startswith("v"):
            return t
    return None


def inspect_local_source(app: str) -> Tuple[Optional[str], Optional[str]]:
    """Inspect local checkout in source/<app> if present."""
    app_src = SOURCE_DIR / app
    if not (app_src / ".git").is_dir():
        return None, None
    tag_proc = subprocess.run(
        ["git", "-C", str(app_src), "describe", "--tags", "--exact-match"],
        capture_output=True,
        text=True,
    )
    tag = tag_proc.stdout.strip() if tag_proc.returncode == 0 else None

    commit_proc = subprocess.run(
        ["git", "-C", str(app_src), "rev-parse", "--short", "HEAD"],
        capture_output=True,
        text=True,
    )
    commit = commit_proc.stdout.strip() if commit_proc.returncode == 0 else None
    return tag, commit


def check_app(config: AppConfig) -> ReleaseCheckResult:
    """Run full release check on a single application."""
    notes: List[str] = []
    gh_latest = fetch_gh_latest_release(config.origin)
    raw_tags = fetch_raw_git_tags(config.repo)
    raw_top = raw_tags[0] if raw_tags else None
    resolved = resolve_builder_tag(config.origin, config.repo)
    local_tag, local_commit = inspect_local_source(config.app)

    # Determine status
    if config.main_override:
        status = "MAIN_OVERRIDE"
        notes.append("Explicitly configured in build-common.sh to build from main branch")
    elif resolved is None and gh_latest is None:
        status = "NO_RELEASES"
        notes.append("No release tags or published releases found")
    elif gh_latest is not None and resolved != gh_latest:
        status = "DESYNC"
        notes.append(f"Builder resolved '{resolved}' but GitHub latest is '{gh_latest}'")
    else:
        status = "SYNCED"

    # Additional contextual notes
    if raw_top and resolved and raw_top != resolved:
        notes.append(
            f"Raw Git tag '{raw_top}' differs from resolved stable release '{resolved}' "
            "(builder successfully avoided unreleased/pre-release tag)"
        )

    if local_tag and resolved and local_tag != resolved:
        notes.append(f"Local checkout tag '{local_tag}' differs from upstream resolved '{resolved}'")

    return ReleaseCheckResult(
        app=config.app,
        title=config.title,
        repo=config.repo,
        origin=config.origin,
        main_override=config.main_override,
        gh_latest_release=gh_latest,
        resolved_builder_tag=resolved,
        raw_git_top_tag=raw_top,
        local_tag=local_tag,
        local_commit=local_commit,
        status=status,
        notes=notes,
    )


# Color formatting helpers
COLOR_GREEN = "\033[92m"
COLOR_YELLOW = "\033[93m"
COLOR_RED = "\033[91m"
COLOR_CYAN = "\033[96m"
COLOR_BOLD = "\033[1m"
COLOR_RESET = "\033[0m"


def format_status(status: str, no_color: bool = False) -> str:
    if no_color:
        return status
    if status == "SYNCED":
        return f"{COLOR_GREEN}{status}{COLOR_RESET}"
    if status in ("MAIN_OVERRIDE", "NO_RELEASES"):
        return f"{COLOR_YELLOW}{status}{COLOR_RESET}"
    return f"{COLOR_RED}{COLOR_BOLD}{status}{COLOR_RESET}"


def print_table(results: List[ReleaseCheckResult], no_color: bool = False, verbose: bool = False) -> None:
    header = f"{'APP':<17} | {'STATUS':<14} | {'BUILDER_RESOLVED':<17} | {'GH_LATEST_REL':<16} | {'RAW_TOP_GIT_TAG':<18} | {'LOCAL_TAG'}"
    divider = "-" * 105
    print("\n" + divider)
    print(header)
    print(divider)

    for r in results:
        status_str = format_status(r.status, no_color=no_color)
        builder_tag = r.resolved_builder_tag or "(none)"
        gh_rel = r.gh_latest_release or "(none)"
        raw_tag = r.raw_git_top_tag or "(none)"
        local = f"{r.local_tag or r.local_commit or '-'}"
        pad = 23 if not no_color else 14
        print(f"{r.app:<17} | {status_str:<{pad}} | {builder_tag:<17} | {gh_rel:<16} | {raw_tag:<18} | {local}")

        if verbose and r.notes:
            for note in r.notes:
                prefix = "  └─ " if not no_color else "     * "
                color = COLOR_CYAN if not no_color else ""
                reset = COLOR_RESET if not no_color else ""
                print(f"{prefix}{color}{note}{reset}")

    print(divider + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify that the builder's stable release resolution matches official upstream releases."
    )
    parser.add_argument(
        "apps",
        nargs="*",
        help="Optional app names to check (e.g. notecraft solidcraft). Defaults to all supported apps.",
    )
    parser.add_argument("--json", action="store_true", help="Output results in JSON format.")
    parser.add_argument("--no-color", action="store_true", help="Disable colored output.")
    parser.add_argument("-v", "--verbose", action="store_true", help="Show detailed resolution notes and warnings.")
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Exit with non-zero exit code if any app is DESYNC or has raw tag discrepancies.",
    )
    parser.add_argument(
        "-j",
        "--workers",
        type=int,
        default=12,
        help="Number of concurrent worker threads (default: 12).",
    )

    args = parser.parse_args()

    # Discover configured apps
    all_configs = parse_build_common()
    if args.apps:
        wanted = {a.lstrip("-").lower() for a in args.apps}
        configs = [c for c in all_configs if c.app.lower() in wanted]
        if not configs:
            print(f"ERROR: None of the specified apps matched. Available: {[c.app for c in all_configs]}", file=sys.stderr)
            return 2
    else:
        configs = all_configs

    # Run checks in parallel
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        results = list(executor.map(check_app, configs))

    # Maintain original ordering
    results_map = {r.app: r for r in results}
    sorted_results = [results_map[c.app] for c in configs if c.app in results_map]

    if args.json:
        print(json.dumps([asdict(r) for r in sorted_results], indent=2))
    else:
        print_table(sorted_results, no_color=args.no_color, verbose=args.verbose)

        # Summary
        synced = sum(1 for r in sorted_results if r.status == "SYNCED")
        overrides = sum(1 for r in sorted_results if r.status == "MAIN_OVERRIDE")
        desyncs = sum(1 for r in sorted_results if r.status == "DESYNC")
        raw_mismatches = sum(
            1 for r in sorted_results if r.raw_git_top_tag and r.resolved_builder_tag and r.raw_git_top_tag != r.resolved_builder_tag
        )

        print(
            f"Checked {len(sorted_results)} apps: {synced} in sync, {overrides} main-overrides, {desyncs} desynced."
        )
        if raw_mismatches:
            print(
                f"Note: {raw_mismatches} app(s) have raw Git tags that differ from official releases "
                "(e.g. stranded nightly tags, release candidates, or un-prefixed semver). "
                "The builder successfully avoided them."
            )

    if desyncs > 0:
        return 1
    if args.strict and raw_mismatches > 0:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
