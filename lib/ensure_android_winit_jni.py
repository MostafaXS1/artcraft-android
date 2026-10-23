"""Keep Android backend dependencies in a retained upstream winit manifest."""

from pathlib import Path
import sys
import tomllib


ANDROID = 'cfg(target_os = "android")'
JNI_DEPENDENCY = """[target.'cfg(target_os = "android")'.dependencies.jni]
version = "0.22"
"""


def ensure_android_jni(path: Path) -> None:
    original = path.read_text()
    manifest = tomllib.loads(original)
    dependencies = manifest.get("target", {}).get(ANDROID, {}).get("dependencies", {})
    existing = dependencies.get("jni")

    if existing is not None:
        version = existing.get("version") if isinstance(existing, dict) else existing
        if version != "0.22":
            raise ValueError(f"{path}: incompatible Android JNI dependency: {existing!r}")
        return

    updated = original.rstrip() + "\n\n" + JNI_DEPENDENCY
    tomllib.loads(updated)
    path.write_text(updated)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: ensure_android_winit_jni.py Cargo.toml")
    ensure_android_jni(Path(sys.argv[1]))
