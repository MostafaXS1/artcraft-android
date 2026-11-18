#!/usr/bin/env python3
import sys, re, tomllib
from pathlib import Path

root_toml = Path(sys.argv[1])
src_root = Path(sys.argv[2])  # $SRC

s = root_toml.read_text()

# Check if any crate uses arboard with workspace = true (excluding vendor)
uses_workspace = False
for cargo in src_root.rglob('Cargo.toml'):
    # Skip vendored copies
    if 'vendor/' in str(cargo):
        continue
    try:
        txt = cargo.read_text()
        if re.search(r'arboard\s*=\s*\{[^}]*workspace\s*=\s*true', txt):
            uses_workspace = True
            break
    except Exception:
        pass

entry_path = 'arboard = { path = "vendor/arboard" }'

def repl_arboard(m):
    inner = m.group(0)
    features = re.search(r'features\s*=\s*\[([^\]]+)\]', inner)
    default_feats = re.search(r'default-features\s*=\s*(false|true)', inner)
    parts = ['path = "vendor/arboard"']
    if default_feats:
        parts.append(f'default-features = {default_feats.group(1)}')
    if features:
        parts.append(f'features = [{features.group(1)}]')
    joined = ", ".join(parts)
    return f'arboard = {{ {joined} }}'

# Replace any direct arboard dependency in the root Cargo.toml (in [dependencies], [target...dependencies], or [workspace.dependencies])
s = re.sub(r'^\s*arboard\s*=\s*\{[^}]*\}\s*(?:#.*)?$', repl_arboard, s, flags=re.M)
s = re.sub(r'^\s*arboard\s*=\s*"[^"]*"\s*(?:#.*)?$', 'arboard = { path = "vendor/arboard" }', s, flags=re.M)

# Also ensure [patch.crates-io] has arboard = { path = "vendor/arboard" }
header = "[patch.crates-io]"
if header in s:
    parts = s.split(header, 1)
    next_sec = re.search(r'\n\[', parts[1])
    patch_sec = parts[1][:next_sec.start()] if next_sec else parts[1]
    rest = parts[1][next_sec.start():] if next_sec else ""
    patch_sec = re.sub(r'^\s*arboard\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', patch_sec, flags=re.M)
    s = parts[0] + header + "\n" + entry_path + "\n" + patch_sec.lstrip('\n') + rest
else:
    s = s.rstrip() + "\n\n" + header + "\n" + entry_path + "\n"

try:
    tomllib.loads(s)
except Exception as e:
    print(f"ERROR: {e}", file=sys.stderr)
    sys.exit(1)

root_toml.write_text(s)
