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

if uses_workspace:
    # Replace arboard in [workspace.dependencies] to point to vendored path
    def repl_ws(m):
        inner = m.group(0)
        features = re.search(r'features\s*=\s*\[([^\]]+)\]', inner)
        if features:
            return f'arboard = {{ path = "vendor/arboard", features = [{features.group(1)}] }}'
        return 'arboard = { path = "vendor/arboard" }'
    s2 = re.sub(r'^\s*arboard\s*=\s*\{[^}]*\}\s*(?:#.*)?$', repl_ws, s, flags=re.M)
    if s2 == s:
        s2 = re.sub(r'^\s*arboard\s*=.*$', 'arboard = { path = "vendor/arboard" }', s, flags=re.M)
    s = s2
else:
    s = re.sub(r'^\s*arboard\s*=\s*\{.*?\}\s*(?:#.*)?\n?', '', s, flags=re.M)
    s = re.sub(r'^\s*arboard\s*=.*\n?', '', s, flags=re.M)
    header = "[patch.crates-io]"
    if entry_path not in s:
        s = (s.replace(header, header + "\n" + entry_path, 1) if header in s
             else s.rstrip() + "\n\n" + header + "\n" + entry_path + "\n")

try:
    tomllib.loads(s)
except Exception as e:
    print(f"ERROR: {e}", file=sys.stderr)
    sys.exit(1)

root_toml.write_text(s)
