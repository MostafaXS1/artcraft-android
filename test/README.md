# Test Tools & Utilities

This directory contains testing scripts, validation tools, and unit tests for the `artcraft-android` builder.

## Available Tools & Test Suites

### 1. `check_releases.py`
A high-performance validator that checks whether the builder's stable release resolution is in sync with official published releases across all supported apps.

It verifies that the builder:
* Correctly ignores stranded/failed CI tags (such as NoteCraft's unreleased `v0.0.16` nightly tag) and resolves the latest published release (`v0.0.15`).
* Correctly handles release candidates and pre-releases (such as SolidCraft's `v0.45.0-rc.4` vs official `v0.45.0`).
* Correctly handles release tags without a leading `v` (such as CapsuleCraft's `2.12.5`).
* Accurately flags apps configured to build against upstream `main` (`earthcraft`, `prowave`, `transit`, `rustgis`, `mapcraft`, `anicraft`).
* Detects any stale checkouts in `source/<app>`.

#### Usage

```bash
# Check all supported apps
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

---

### 2. Unit Test Suites

The test suite validates the decentralized modular architecture of the build system (`lib/`).

#### Test Modules

- **`test_launcher_generator.py`**: Verifies `lib/launcher_generator.py`.
  - Tests CLI argument validation and missing argument handling.
  - Verifies AST-based workspace parsing and Cargo.toml inspection.
  - Tests GameActivity Rust shell generation, SAF recipe injection, and marker version checking (`GENERATOR_VERSION`).
  - Verifies idempotency and safe overwriting of existing launchers.

- **`test_upstream_patches.py`**: Verifies `lib/upstream_patches.py`.
  - Tests PhotoCraft export presets injection (`export_presets: take(&mut m, "export_presets")`).
  - Tests CADCraft egui 0.37 dependency downgrade logic.
  - Tests Coiman IDE upstream dependency and Termux shim fixes.
  - Verifies idempotency across all patch routines (applying twice produces identical source).

- **`test_cargo_patches.py`**: Verifies `lib/cargo_patches.py`.
  - Tests `[patch.crates-io]` workspace table injection.
  - Tests stripping Wayland and X11 features from vendored `winit`.
  - Tests egui-dock and egui-wgpu surface recovery overrides.
  - Tests `resolve-version` and `install-updater` CLI subcommands.

- **`test_toolchain.py`**: Verifies `lib/toolchain.sh`.
  - Tests Android SDK and NDK directory discovery across standard paths.
  - Tests JDK bootstrap and Temurin 21 detection.
  - Tests environment variable initialization (`ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME`).
  - Verifies tool validation (`cargo ndk`, `git`, `python3`).

- **`test_catalog.py`**: Verifies `lib/catalog.sh`.
  - Tests `resolve_app_catalog` mapping for official Storytold and community apps.
  - Tests custom origin overrides (e.g. SubCraft, SolidCraft, Coiman IDE, Transit).
  - Tests main-branch override detection.
  - Tests CLI `print_usage` flag listing and invalid app rejection.

#### Running Unit Tests

Run all unit tests via standard Python `unittest`:

```bash
# Run all unit tests
python3 -m unittest discover -s test -p 'test_*.py'

# Run individual test suites
python3 test/test_launcher_generator.py
python3 test/test_upstream_patches.py
python3 test/test_cargo_patches.py
python3 test/test_toolchain.py
python3 test/test_catalog.py
```
