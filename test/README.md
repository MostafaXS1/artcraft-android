# Test Tools & Utilities

This directory contains testing scripts and validation tools for the `artcraft-android` builder.

## Available Tools

### 1. `check_releases.py`
A high-performance validator that checks whether the builder's stable release resolution is in sync with official published releases across all 32 supported apps.

It verifies that the builder:
* Correctly ignores stranded/failed CI tags (such as NoteCraft's unreleased `v0.0.16` nightly tag) and resolves the latest published release (`v0.0.15`).
* Correctly handles release candidates and pre-releases (such as SolidCraft's `v0.45.0-rc.4` vs official `v0.45.0`).
* Correctly handles release tags without a leading `v` (such as CapsuleCraft's `2.12.5`).
* Accurately flags apps configured to build against upstream `main` (`earthcraft`, `prowave`, `transit`, `rustgis`, `mapcraft`, `anicraft`).
* Detects any stale checkouts in `source/<app>`.

#### Usage

```bash
# Check all 32 supported apps
python3 test/check_releases.py

# Check specific apps
python3 test/check_releases.py notecraft solidcraft capsulecraft

# Show detailed notes on how tags were resolved
python3 test/check_releases.py -v

# Output JSON for CI / automation
python3 test/check_releases.py --json

# Strict mode: fail if any raw tag discrepancies or desyncs exist
python3 test/check_releases.py --strict
```
