#!/usr/bin/env python3
"""Fetch the latest contributor snapshots from upstream craftapp Release workflow runs."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

APPS = [
    "cadcraft",
    "deckcraft",
    "designcraft",
    "effectcraft",
    "figcraft",
    "filmcraft",
    "gridcraft",
    "lightcraft",
    "pdfcraft",
    "photocraft",
    "soundcraft",
    "vectorcraft",
    "wordcraft",
]

ROOT = Path(__file__).resolve().parent.parent
DATA_DIR = ROOT / "data" / "contributors"
DATA_DIR.mkdir(parents=True, exist_ok=True)


def run_cmd(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def fetch_contributor_snapshot(app):
    repo = f"storytold/{app}"
    print(f"[{app}] Checking release runs for {repo}...")
    
    # 1. List recent release runs
    runs_raw = run_cmd([
        "gh", "run", "list",
        "--repo", repo,
        "--workflow", "release.yml",
        "--limit", "15",
        "--json", "databaseId,status,conclusion,createdAt"
    ])
    if not runs_raw:
        print(f"[{app}] No runs found or error querying {repo}")
        return False
        
    try:
        runs = json.loads(runs_raw)
    except Exception as e:
        print(f"[{app}] Error parsing runs JSON: {e}")
        return False

    # 2. Iterate through runs to find one with contributor-snapshot artifact
    for run in runs:
        run_id = run["databaseId"]
        artifacts_raw = run_cmd([
            "gh", "api", f"/repos/{repo}/actions/runs/{run_id}/artifacts",
            "--jq", '.artifacts[] | {name: .name, id: .id}'
        ])
        if not artifacts_raw:
            continue
            
        for line in artifacts_raw.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                art = json.loads(line)
                art_name = art.get("name", "")
                if art_name.startswith("contributor-snapshot-"):
                    print(f"[{app}] Found snapshot artifact '{art_name}' in run {run_id}")
                    with tempfile.TemporaryDirectory() as tmpdir:
                        dl = subprocess.run([
                            "gh", "run", "download", str(run_id),
                            "--repo", repo,
                            "--name", art_name,
                            "--dir", tmpdir
                        ], capture_output=True, text=True)
                        if dl.returncode == 0:
                            json_path = Path(tmpdir) / "contributors.json"
                            if json_path.exists():
                                # Validate JSON
                                with open(json_path) as f:
                                    data = json.load(f)
                                commits = data.get("commits", 0)
                                contribs = len(data.get("contributors", []))
                                target_path = DATA_DIR / f"{app}.json"
                                target_path.write_text(json.dumps(data, indent=2) + "\n")
                                print(f"[{app}] Successfully updated {target_path.name}: {contribs} contributors, {commits} commits")
                                return True
            except Exception as e:
                print(f"[{app}] Error reading artifact: {e}")

    if app == "gridcraft":
        print(f"[{app}] Note: storytold/gridcraft does not run credits: in release.yml; preserving data/contributors/gridcraft.json")
    else:
        print(f"[{app}] No valid contributor-snapshot artifact found in recent runs")
    return False


def main():
    updated = 0
    total = len(APPS)
    for app in APPS:
        if fetch_contributor_snapshot(app):
            updated += 1
            
    print(f"\nDone: {updated}/{total} apps updated.")


if __name__ == "__main__":
    main()
