#!/usr/bin/env python3
"""Drive the opt-in egui-winit clipboard image probe on a private headless Sway.

This controller never touches the user's Wayland/X11 session. It creates a fresh
XDG_RUNTIME_DIR, starts a private Sway headless compositor from the pinned
native-tools root, and starts `clipboard_image_probe` with a private WAYLAND_DISPLAY.
All `wl-copy`, `wl-paste`, `swaymsg`, and `wtype` children use that private
environment. Only child processes started by this script are terminated.

The parent supplies CARGO_TARGET_DIR and builds the example. This script does not
run Cargo, rustc, or any build/test command.

Usage:
    CARGO_TARGET_DIR=/managed/target \
    python3 crates/egui-winit/examples/clipboard_image_smoke.py \
        --evidence /absolute/durable/evidence/dir

Exit status 0 means every recorded check passed. Logs, offered bytes, the actually
copied PNG, verification output, source hashes, and result.json stay in a fresh
run directory under --evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

KNOWN_EVIDENCE_ROOT = Path("/home/codex/work/ssd2/task-evidence/egui037-migration-20261022")
NATIVE_TOOLS = KNOWN_EVIDENCE_ROOT / "native-tools"
NATIVE_ROOT = NATIVE_TOOLS / "root"

SEED_RGBA = bytes(
    [
        255, 0, 0, 255,     # opaque red
        0, 255, 0, 128,     # half-alpha green
        0, 0, 255, 64,      # quarter-alpha blue
        0, 0, 0, 0,         # fully transparent
    ]
)
SEED_ALPHA = bytes([255, 128, 64, 0])
SEED_PREMULTIPLIED = bytes(
    [
        255, 0, 0, 255,
        0, 128, 0, 128,
        0, 0, 64, 64,
        0, 0, 0, 0,
    ]
)
TEXT = "native clipboard text 🦀\nsecond line"
TEXT_BYTES = TEXT.encode("utf-8")
EXTERNAL_TEXT = "external regular العربية 🦀\r\nsecond line\r\n".encode("utf-8")
EXTERNAL_TEXT_NORMALIZED = EXTERNAL_TEXT.replace(b"\r\n", b"\n")
PRIMARY_TEXT = "primary sentinel עברית\n".encode("utf-8")
TEXT_MIME = "text/plain;charset=utf-8"
IMAGE_MIME = "image/png"
INVALID_PNG = b"\x89PNG\r\n\x1a\nthis-is-not-a-valid-png-payload"
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
WINDOW_TITLE = "egui-winit native clipboard image probe"


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fnv1a(data: bytes) -> int:
    value = 0xCBF29CE484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return value


def fnv_hex(data: bytes) -> str:
    return f"{fnv1a(data):016x}"


def png_chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def make_png(width: int, height: int, rgba: bytes) -> bytes:
    assert len(rgba) == width * height * 4
    scanlines = b"".join(
        b"\x00" + rgba[row * width * 4 : (row + 1) * width * 4]
        for row in range(height)
    )
    return (
        PNG_SIGNATURE
        + png_chunk(
            b"IHDR",
            struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0),
        )
        + png_chunk(b"IDAT", zlib.compress(scanlines, 9))
        + png_chunk(b"IEND", b"")
    )


EXTERNAL_PNG = make_png(2, 2, SEED_RGBA)


class Smoke:
    def __init__(self, evidence: Path) -> None:
        self.evidence = evidence
        stamp = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
        self.run_dir = evidence / f"clipboard-image-smoke-{stamp}-{os.getpid()}"
        self.run_dir.mkdir(parents=True, exist_ok=False)
        self.controller_log = (self.run_dir / "controller.log").open("w", encoding="utf-8")
        self.log_files = [self.controller_log]
        self.processes: list[subprocess.Popen[bytes]] = []
        self.sway: subprocess.Popen[bytes] | None = None
        self.app: subprocess.Popen[bytes] | None = None
        self.keyboard: subprocess.Popen[bytes] | None = None
        self.primary_owner: subprocess.Popen[bytes] | None = None
        self.regular_owner: subprocess.Popen[bytes] | None = None
        self.env: dict[str, str] | None = None
        self.runtime: Path | None = None
        self.checks: list[str] = []
        self.scenarios: list[dict[str, object]] = []
        self.error: str | None = None
        self.status = "not-started"

        target = os.environ.get("CARGO_TARGET_DIR")
        if not target:
            raise RuntimeError("CARGO_TARGET_DIR must be supplied by the parent build harness")
        self.cargo_target_dir = Path(target)
        self.probe = self.cargo_target_dir / "debug/examples/clipboard_image_probe"
        if not self.probe.is_file():
            raise RuntimeError(f"probe binary is missing: {self.probe}")

        self.sway_log_path = self.run_dir / "sway.log"
        self.probe_log_path = self.run_dir / "probe.log"
        self.probe_wayland_log_path = self.run_dir / "probe-wayland.log"

    def record(self, message: str) -> None:
        line = f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {message}"
        self.controller_log.write(line + "\n")
        self.controller_log.flush()
        print(message, flush=True)

    def check(self, message: str) -> None:
        self.checks.append(message)
        self.record(f"CHECK {message}")

    def run(
        self,
        args: list[str],
        *,
        timeout: float = 15.0,
        check: bool = True,
        stdout: object = subprocess.PIPE,
        stderr: object = subprocess.PIPE,
        env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[bytes]:
        assert self.env is not None
        result = subprocess.run(
            args,
            env=self.env if env is None else env,
            stdout=stdout,
            stderr=stderr,
            timeout=timeout,
            check=False,
        )
        if check and result.returncode != 0:
            raise RuntimeError(
                f"command failed ({result.returncode}): {args!r}\n"
                f"stdout={result.stdout!r}\nstderr={result.stderr!r}"
            )
        return result

    def spawn(
        self,
        args: list[str],
        *,
        stdout: object,
        stderr: object,
        env: dict[str, str] | None = None,
        stdin: object = subprocess.DEVNULL,
    ) -> subprocess.Popen[bytes]:
        assert self.env is not None
        process = subprocess.Popen(
            args,
            env=self.env if env is None else env,
            stdin=stdin,
            stdout=stdout,
            stderr=stderr,
        )
        self.processes.append(process)
        return process

    def stop_process(self, process: subprocess.Popen[bytes] | None, name: str) -> None:
        if process is None or process.poll() is not None:
            return
        self.record(f"cleanup: terminating child {name} pid={process.pid}")
        process.terminate()
        try:
            process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.record(f"cleanup: killing child {name} pid={process.pid}")
            process.kill()
            process.wait(timeout=8)

    def probe_text(self) -> str:
        try:
            return self.probe_log_path.read_text(encoding="utf-8", errors="replace")
        except FileNotFoundError:
            return ""

    @staticmethod
    def lines_starting(text: str, prefix: str) -> list[str]:
        # Origin lines begin with "FRAME "; callers also use this helper for substrings such as
        # "ORIGIN ctrl_v" and "RAW_PASTE frame=".
        return [line for line in text.splitlines() if prefix in line]

    def count(self, prefix: str, text: str | None = None) -> int:
        return len(self.lines_starting(self.probe_text() if text is None else text, prefix))

    def wait_until(self, predicate, description: str, timeout: float = 15.0) -> str:
        deadline = time.monotonic() + timeout
        while True:
            if self.app is not None and self.app.poll() is not None:
                raise RuntimeError(
                    f"probe exited while waiting for {description} (status {self.app.returncode})"
                )
            if self.sway is not None and self.sway.poll() is not None:
                raise RuntimeError(f"Sway exited while waiting for {description}")
            text = self.probe_text()
            if predicate(text):
                return text
            if time.monotonic() >= deadline:
                raise TimeoutError(
                    f"timed out waiting for {description}; see {self.probe_log_path}"
                )
            time.sleep(0.05)

    def wait_new(self, prefix: str, before: int, description: str, timeout: float = 15.0) -> str:
        return self.wait_until(
            lambda text: self.count(prefix, text) > before,
            description,
            timeout,
        )

    @staticmethod
    def last_failures(text: str) -> int:
        matches = re.findall(r"^FRAME (\d+) END failures=(\d+)$", text, re.MULTILINE)
        return int(matches[-1][1]) if matches else 0

    def snapshot(self) -> dict[str, int]:
        text = self.probe_text()
        return {
            "paste": self.count("RAW_PASTE frame=", text),
            "image": self.count("RAW_PASTE_IMAGE frame=", text),
            "failures": self.last_failures(text),
        }

    def wait_platform(self, frame: int, kind: str, timeout: float = 10.0) -> None:
        needle = f"FRAME {frame} PLATFORM_COMMAND "
        self.wait_until(
            lambda text: any(
                line.startswith(needle) and f"kind={kind}" in line
                for line in text.splitlines()
            ),
            f"frame {frame} platform command {kind}",
            timeout,
        )

    def wait_batch(self, frame: int, regular: str, timeout: float = 10.0) -> None:
        needle = f"FRAME {frame} BATCH regular={regular} primary=none"
        self.wait_until(
            lambda text: needle in text,
            f"frame {frame} batch winner {regular}",
            timeout,
        )

    def wait_failures_above(self, before: int, timeout: float = 10.0) -> None:
        self.wait_until(
            lambda text: self.last_failures(text) > before,
            f"clipboard failure count above {before}",
            timeout,
        )

    def assert_primary_unchanged(self, where: str) -> None:
        result = self.run(
            [str(NATIVE_ROOT / "usr/bin/wl-paste"), "--primary", "--no-newline"],
            timeout=4.0,
        )
        if result.stdout != PRIMARY_TEXT:
            raise AssertionError(
                f"PRIMARY changed at {where}: {result.stdout!r} != {PRIMARY_TEXT!r}"
            )
        self.check(f"PRIMARY sentinel unchanged ({where})")

    def setup_environment(self) -> None:
        self.runtime = Path(tempfile.mkdtemp(prefix="egui037-egui-clipboard-"))
        os.chmod(self.runtime, 0o700)
        for subdir in ("cache", "config", "data", "fontcache"):
            (self.runtime / subdir).mkdir(mode=0o700, exist_ok=True)

        environment = os.environ.copy()
        environment.update(json.loads((NATIVE_TOOLS / "runtime-environment.json").read_text()))
        environment["PATH"] = str(NATIVE_ROOT / "usr/bin") + ":" + environment.get("PATH", "")
        for name in (
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "WAYLAND_SOCKET",
            "SWAYSOCK",
            "DBUS_SESSION_BUS_ADDRESS",
        ):
            environment.pop(name, None)
        environment.update(
            {
                "XDG_RUNTIME_DIR": str(self.runtime),
                "XDG_CACHE_HOME": str(self.runtime / "cache"),
                "XDG_CONFIG_HOME": str(self.runtime / "config"),
                "XDG_DATA_HOME": str(self.runtime / "data"),
                "WLR_BACKENDS": "headless",
                "WLR_RENDERER": "pixman",
                "WLR_HEADLESS_OUTPUTS": "1",
                "WLR_LIBINPUT_NO_DEVICES": "1",
                "XKB_CONFIG_ROOT": (
                    str(NATIVE_ROOT / "usr/share/X11/xkb")
                    if (NATIVE_ROOT / "usr/share/X11/xkb").is_dir()
                    else "/usr/share/X11/xkb"
                ),
            }
        )

        fontconfig = self.run_dir / "fontconfig.xml"
        fontconfig.write_text(
            '<?xml version="1.0"?>'
            '<!DOCTYPE fontconfig SYSTEM "fonts.dtd">'
            "<fontconfig><dir>"
            + str(NATIVE_ROOT / "usr/share/fonts")
            + "</dir><cachedir>"
            + str(self.runtime / "fontcache")
            + "</cachedir></fontconfig>",
            encoding="utf-8",
        )
        environment["FONTCONFIG_FILE"] = str(fontconfig)
        self.env = environment

    def start_sway(self) -> None:
        assert self.env is not None and self.runtime is not None
        config = self.run_dir / "sway.conf"
        config.write_text(
            "xwayland disable\n"
            "output * resolution 1280x800\n"
            "seat seat0 fallback true\n"
            "focus_follows_mouse no\n"
            "default_border pixel 1\n"
            "font monospace 10\n",
            encoding="utf-8",
        )
        sway_log = self.sway_log_path.open("wb")
        self.log_files.append(sway_log)
        self.sway = self.spawn(
            [str(NATIVE_ROOT / "usr/bin/sway"), "--config", str(config), "--debug"],
            stdout=sway_log,
            stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 20.0
        while True:
            if self.sway.poll() is not None:
                raise RuntimeError("private Sway exited during startup")
            wayland = [p for p in self.runtime.glob("wayland-*") if p.is_socket()]
            ipc = [p for p in self.runtime.glob("sway-ipc.*.sock") if p.is_socket()]
            if wayland and ipc:
                self.env["WAYLAND_DISPLAY"] = wayland[0].name
                self.env["SWAYSOCK"] = str(ipc[0])
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("private Sway sockets did not appear")
            time.sleep(0.1)

        if not str(self.env["WAYLAND_DISPLAY"]).startswith("wayland-"):
            raise RuntimeError("private compositor did not use an isolated wayland socket")
        self.record(
            f"private compositor started: WAYLAND_DISPLAY={self.env['WAYLAND_DISPLAY']} "
            f"SWAYSOCK={self.env['SWAYSOCK']}"
        )

        wayland_info = self.run(
            [str(NATIVE_ROOT / "usr/bin/wayland-info")],
            timeout=15.0,
        )
        (self.run_dir / "wayland-globals.txt").write_bytes(wayland_info.stdout + wayland_info.stderr)
        if b"zwp_primary_selection_device_manager_v1" not in wayland_info.stdout:
            raise RuntimeError("private Sway did not advertise PRIMARY selection")
        self.check("private Sway advertised regular and PRIMARY clipboard protocols")

    def start_primary_sentinel(self) -> None:
        self.offer(
            PRIMARY_TEXT,
            mime=TEXT_MIME,
            primary=True,
            name="primary-sentinel.txt",
        )
        self.assert_primary_unchanged("initial offer")

    def start_probe(self) -> None:
        assert self.env is not None
        # Keep a keyboard on the private seat before mapping/focusing the window.
        self.keyboard = self.spawn([str(NATIVE_ROOT / "usr/bin/wtype"), "-s", "900000"], stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
        time.sleep(0.5)
        probe_log = self.probe_log_path.open("wb")
        wayland_log = self.probe_wayland_log_path.open("wb")
        self.log_files.extend([probe_log, wayland_log])
        probe_env = self.env.copy()
        probe_env["WAYLAND_DEBUG"] = "client"
        probe_env["RUST_BACKTRACE"] = "1"
        self.app = self.spawn(
            [str(self.probe)],
            stdout=probe_log,
            stderr=wayland_log,
            env=probe_env,
        )
        self.wait_until(
            lambda text: (
                "STATE_READY egui_winit_state=true wayland=true" in text
                and "USES_EGUI_WINIT_STATE=true" in text
                and "EGUI_STATE take_egui_input=1 "
                "handle_platform_output_with_event_loop=1" in text
                and "WINDOW_MAPPED" in text
            ),
            "softbuffer-mapped Wayland probe window",
            25.0,
        )
        self.check(
            "probe created a real egui_winit::State on Wayland and mapped a softbuffer window"
        )

        focus_deadline = time.monotonic() + 10.0
        while time.monotonic() < focus_deadline:
            self.run(
                [str(NATIVE_ROOT / "usr/bin/swaymsg"), f'[title="{WINDOW_TITLE}"] focus'],
                timeout=4.0,
                check=False,
            )
            if "FOCUSED true" in self.probe_text():
                break
            time.sleep(0.15)
        self.wait_until(
            lambda text: "FOCUSED true" in text,
            "probe window keyboard focus",
            10.0,
        )
        screenshot = self.run_dir / "private-desktop-after-present.png"
        # Focus/configure can precede the compositor's transaction commit. Verify painted
        # pixels, not merely a successful screencopy of an as-yet black output.
        present_deadline = time.monotonic() + 5.0
        while True:
            self.run([str(NATIVE_ROOT / "usr/bin/grim"), str(screenshot)], timeout=15.0)
            verification = self.run([str(self.probe), "--verify-window", str(screenshot)], check=False)
            (self.run_dir / "verify-presented-window.txt").write_bytes(verification.stdout + verification.stderr)
            if verification.returncode == 0:
                break
            if time.monotonic() >= present_deadline:
                raise RuntimeError("private compositor did not present the probe's painted swatches")
            time.sleep(0.1)
        tree = self.run(
            [str(NATIVE_ROOT / "usr/bin/swaymsg"), "-r", "-t", "get_tree"],
            timeout=10.0,
        )
        (self.run_dir / "sway-tree.json").write_bytes(tree.stdout)
        if WINDOW_TITLE.encode() not in tree.stdout:
            raise RuntimeError("private Sway tree does not contain the probe window title")
        self.check(
            "probe window received private-compositor keyboard focus; grim and the Sway tree "
            "show the presented window"
        )

    def offer(
        self,
        data: bytes,
        *,
        mime: str,
        primary: bool = False,
        name: str,
    ) -> None:
        if not primary:
            self.stop_process(self.regular_owner, "previous regular wl-copy")
            self.regular_owner = None

        payload = self.run_dir / name
        payload.write_bytes(data)
        source = payload.open("rb")
        owner_log = (self.run_dir / (name + ".wl-copy.log")).open("wb")
        self.log_files.append(owner_log)
        args = [
            str(NATIVE_ROOT / "usr/bin/wl-copy"),
            "--foreground",
            "--type",
            mime,
        ]
        if primary:
            args.append("--primary")
        process = self.spawn(args, stdin=source, stdout=subprocess.DEVNULL, stderr=owner_log)
        source.close()
        if primary:
            self.primary_owner = process
        else:
            self.regular_owner = process
        time.sleep(0.25)
        if process.poll() is not None:
            raise RuntimeError(f"wl-copy owner exited immediately for {name}")
        self.record(f"offered {name}: mime={mime} bytes={len(data)} primary={primary}")

    def wtype(self, arguments: list[str]) -> None:
        self.run(
            [str(NATIVE_ROOT / "usr/bin/wtype"), *arguments],
            timeout=10.0,
        )

    def send_probe_key(self, key: str) -> int:
        before = self.count("FRAME ", self.probe_text())
        origins = self.count(f"ORIGIN key={key} ")
        self.wtype(["-s", "150", "-k", key, "-s", "250"])
        self.wait_new(f"ORIGIN key={key} ", origins, f"key {key} origin", 10.0)
        # The newest origin line identifies the exact logical frame.
        matching = self.lines_starting(self.probe_text(), "FRAME ")
        matching = [line for line in matching if f" ORIGIN key={key} " in line]
        assert matching, "origin line disappeared"
        match = re.match(r"FRAME (\d+) ORIGIN ", matching[-1])
        assert match is not None
        frame = int(match.group(1))
        if self.count("FRAME ", self.probe_text()) <= before:
            raise RuntimeError("probe did not advance a frame after keyboard input")
        return frame

    def trigger_ctrl_v(self) -> int:
        before = self.count("ORIGIN ctrl_v", self.probe_text())
        self.wtype(["-M", "ctrl", "-s", "100", "-k", "v", "-s", "100", "-m", "ctrl"])
        self.wait_until(
            lambda text: self.count("ORIGIN ctrl_v", text) > before,
            "Ctrl+V origin frame",
            10.0,
        )
        matching = [line for line in self.probe_text().splitlines() if " ORIGIN ctrl_v" in line]
        assert matching
        match = re.match(r"FRAME (\d+) ORIGIN ", matching[-1])
        assert match is not None
        return int(match.group(1))

    def external_read(
        self,
        mime: str | None,
        *,
        primary: bool = False,
        timeout: float = 3.0,
    ) -> subprocess.CompletedProcess[bytes]:
        args = [str(NATIVE_ROOT / "usr/bin/wl-paste"), "--no-newline"]
        if mime is not None:
            args.extend(["--type", mime])
        if primary:
            args.append("--primary")
        try:
            return self.run(args, timeout=timeout, check=False)
        except subprocess.TimeoutExpired:
            return subprocess.CompletedProcess(args, 124, b"", b"timeout")

    def read_until(self, mime: str, predicate, description: str, timeout: float = 12.0) -> bytes:
        deadline = time.monotonic() + timeout
        last = b""
        while time.monotonic() < deadline:
            result = self.external_read(mime, timeout=2.0)
            last = result.stdout
            if result.returncode == 0 and predicate(result.stdout):
                return result.stdout
            time.sleep(0.15)
        raise TimeoutError(f"timed out reading {description} as {mime}; last={last!r}")

    def read_text_until(
        self,
        expected: bytes,
        description: str,
        timeout: float = 12.0,
    ) -> bytes:
        deadline = time.monotonic() + timeout
        last = b""
        while time.monotonic() < deadline:
            for mime in (None, TEXT_MIME, "text/plain"):
                result = self.external_read(mime, timeout=1.5)
                last = result.stdout
                if result.returncode == 0 and result.stdout == expected:
                    return result.stdout
            time.sleep(0.15)
        raise TimeoutError(f"timed out reading {description}; last={last!r}")

    def list_types(self, *, timeout: float = 4.0) -> list[str]:
        result = self.run(
            [str(NATIVE_ROOT / "usr/bin/wl-paste"), "--list-types"],
            timeout=timeout,
            check=False,
        )
        if result.returncode != 0:
            return []
        return [line for line in result.stdout.decode("utf-8", "replace").splitlines() if line]

    def wait_types(self, predicate, description: str, timeout: float = 10.0) -> list[str]:
        deadline = time.monotonic() + timeout
        last: list[str] = []
        while time.monotonic() < deadline:
            last = self.list_types()
            if predicate(last):
                return last
            time.sleep(0.15)
        raise TimeoutError(f"timed out waiting for {description}; types={last!r}")

    def verify_png(self, image_path: Path, label: str) -> dict[str, str]:
        result = self.run(
            [str(self.probe), "--verify-png", str(image_path)],
            timeout=15.0,
            check=False,
        )
        output = result.stdout + result.stderr
        (self.run_dir / f"verify-{label}.txt").write_bytes(output)
        if result.returncode != 0:
            raise AssertionError(f"PNG verification failed for {label}: {output!r}")
        text = output.decode("utf-8", "replace")
        match = re.search(
            r"PNG_VERIFY path=.* bytes=(\d+) dims=(\d+)x(\d+) rgba_hex=([0-9a-f]+) "
            r"rgba_fnv=([0-9a-f]{16}) alpha_hex=([0-9a-f]+) seed_rgba_match=(true|false)",
            text,
        )
        if match is None:
            raise AssertionError(f"unparseable PNG_VERIFY output: {text!r}")
        details = {
            "output": text.strip(),
            "bytes": match.group(1),
            "dims": f"{match.group(2)}x{match.group(3)}",
            "rgba_hex": match.group(4),
            "rgba_fnv": match.group(5),
            "alpha_hex": match.group(6),
            "seed_rgba_match": match.group(7),
        }
        if details["dims"] != "2x2":
            raise AssertionError(f"{label} copied PNG dimensions are {details['dims']}")
        if details["rgba_hex"] != SEED_RGBA.hex():
            raise AssertionError(f"{label} copied PNG RGBA differs: {details['rgba_hex']}")
        if details["rgba_fnv"] != fnv_hex(SEED_RGBA):
            raise AssertionError(f"{label} copied PNG RGBA hash differs")
        if details["alpha_hex"] != SEED_ALPHA.hex():
            raise AssertionError(f"{label} copied PNG alpha differs: {details['alpha_hex']}")
        if details["seed_rgba_match"] != "true":
            raise AssertionError(f"{label} copied PNG did not report seed_rgba_match=true")
        return details

    def assert_image_log(self, line_prefix: str) -> dict[str, str]:
        lines = self.lines_starting(self.probe_text(), line_prefix)
        assert lines
        line = lines[-1]
        match = re.search(
            r"^(RAW_PASTE_IMAGE|PLATFORM_IMAGE) frame=(\d+) index=(\d+) "
            r"dims=(\d+)x(\d+) pixels=(\d+) rgba_hex=([0-9a-f]+) "
            r"rgba_fnv=([0-9a-f]{16}) premult_hex=([0-9a-f]+) alpha_hex=([0-9a-f]+) "
            r"seed_match=(true|false)$",
            line,
        )
        if match is None:
            raise AssertionError(f"unparseable image log: {line}")
        expected = {
            "dims": "2x2",
            "pixels": 4,
            "rgba_hex": SEED_RGBA.hex(),
            "rgba_fnv": fnv_hex(SEED_RGBA),
            "premult_hex": SEED_PREMULTIPLIED.hex(),
            "alpha_hex": SEED_ALPHA.hex(),
            "seed_match": "true",
        }
        actual = {
            "dims": f"{match.group(4)}x{match.group(5)}",
            "pixels": int(match.group(6)),
            "rgba_hex": match.group(7),
            "rgba_fnv": match.group(8),
            "premult_hex": match.group(9),
            "alpha_hex": match.group(10),
            "seed_match": match.group(11),
        }
        if actual != expected:
            raise AssertionError(f"image log mismatch: {actual!r} != {expected!r}")
        return {key: str(value) for key, value in actual.items()}

    def assert_text_log(self, expected: bytes) -> dict[str, str]:
        lines = self.lines_starting(self.probe_text(), "RAW_PASTE frame=")
        assert lines
        line = lines[-1]
        match = re.search(
            r"^RAW_PASTE frame=(\d+) index=(\d+) bytes=(\d+) fnv=([0-9a-f]{16}) hex=([0-9a-f]*)$",
            line,
        )
        if match is None:
            raise AssertionError(f"unparseable text log: {line}")
        details = {
            "frame": match.group(1),
            "bytes": match.group(3),
            "fnv": match.group(4),
            "hex": match.group(5),
        }
        if int(details["bytes"]) != len(expected):
            raise AssertionError(f"text bytes {details['bytes']} != {len(expected)}")
        if details["fnv"] != fnv_hex(expected):
            raise AssertionError(f"text hash {details['fnv']} != {fnv_hex(expected)}")
        if details["hex"] != expected.hex():
            raise AssertionError(f"text payload differs: {details['hex']}")
        return details

    def scenario_external_text(self) -> None:
        before = self.snapshot()
        self.offer(EXTERNAL_TEXT, mime=TEXT_MIME, name="external-regular-text.bin")
        frame = self.trigger_ctrl_v()
        self.wait_new("RAW_PASTE frame=", before["paste"], "external regular text paste", 15.0)
        time.sleep(0.75)
        after = self.snapshot()
        if after["paste"] != before["paste"] + 1 or after["image"] != before["image"]:
            raise AssertionError(f"external text event counts wrong: {before} -> {after}")
        details = self.assert_text_log(EXTERNAL_TEXT_NORMALIZED)
        self.check(
            f"external regular text decoded exactly once at frame {frame} "
            f"(bytes={details['bytes']}, normalized CRLF)"
        )
        self.assert_primary_unchanged("after external regular text")
        self.scenarios.append(
            {"name": "external-regular-text", "frame": frame, "details": details}
        )

    def scenario_external_image(self) -> None:
        before = self.snapshot()
        self.offer(EXTERNAL_PNG, mime=IMAGE_MIME, name="external-regular-image.png")
        frame = self.trigger_ctrl_v()
        self.wait_new("RAW_PASTE_IMAGE frame=", before["image"], "external regular PNG paste", 15.0)
        time.sleep(0.75)
        after = self.snapshot()
        if after["image"] != before["image"] + 1 or after["paste"] != before["paste"]:
            raise AssertionError(f"external image event counts wrong: {before} -> {after}")
        details = self.assert_image_log("RAW_PASTE_IMAGE frame=")
        self.check(
            f"external regular PNG decoded exactly once at frame {frame} "
            "with exact straight-alpha RGBA and premultiplied pixels"
        )
        self.assert_primary_unchanged("after external regular image")
        self.scenarios.append(
            {"name": "external-regular-image", "frame": frame, "details": details}
        )

    def scenario_malformed_png(self) -> None:
        before = self.snapshot()
        self.offer(INVALID_PNG, mime=IMAGE_MIME, name="invalid-regular-image.png")
        frame = self.trigger_ctrl_v()
        self.wait_failures_above(before["failures"], 15.0)
        time.sleep(0.8)
        after = self.snapshot()
        if after["paste"] != before["paste"] or after["image"] != before["image"]:
            raise AssertionError(
                f"malformed PNG produced a paste event: {before} -> {after}"
            )
        if after["failures"] <= before["failures"]:
            raise AssertionError("malformed PNG did not record a clipboard failure")
        self.check(
            f"malformed image/png yielded no Paste/PasteImage and no text fallback "
            f"(Ctrl+V frame {frame}, failures {before['failures']} -> {after['failures']})"
        )
        self.assert_primary_unchanged("after malformed PNG")
        self.scenarios.append(
            {
                "name": "malformed-regular-png",
                "frame": frame,
                "failures_before": before["failures"],
                "failures_after": after["failures"],
            }
        )

    def scenario_absent_regular(self) -> None:
        self.stop_process(self.regular_owner, "regular wl-copy before absent test")
        self.regular_owner = None
        time.sleep(0.75)
        if self.list_types():
            raise AssertionError("regular clipboard still advertises an offer after its owner exited")
        before = self.snapshot()
        frame = self.trigger_ctrl_v()
        # NotFound is deliberately a quiet no-op, not a transport failure. Observe beyond the
        # composed paste deadline so a delayed/stale completion cannot make this pass early.
        time.sleep(3.25)
        after = self.snapshot()
        if after["paste"] != before["paste"] or after["image"] != before["image"]:
            raise AssertionError(f"absent regular selection produced paste: {before} -> {after}")
        if after["failures"] != before["failures"]:
            raise AssertionError("ordinary absence was reported as a transport failure")
        self.check(
            f"absent regular selection yielded no Paste/PasteImage "
            f"(Ctrl+V frame {frame}, failures {before['failures']} -> {after['failures']})"
        )
        self.assert_primary_unchanged("after absent regular selection")
        self.scenarios.append(
            {
                "name": "absent-regular-selection",
                "frame": frame,
                "failures_before": before["failures"],
                "failures_after": after["failures"],
            }
        )

    def scenario_native_image(self) -> None:
        frame = self.send_probe_key("i")
        self.wait_platform(frame, "CopyImage")
        self.wait_batch(frame, "image")
        image_bytes = self.read_until(
            IMAGE_MIME,
            lambda data: data.startswith(PNG_SIGNATURE),
            "native copied PNG",
            15.0,
        )
        native_png = self.run_dir / "native-copied-image.png"
        native_png.write_bytes(image_bytes)
        details = self.verify_png(native_png, "native-copied-image")
        types = self.wait_types(
            lambda values: any(value == IMAGE_MIME for value in values)
            and not any(value.startswith("text/") for value in values),
            "native image-only selection",
            12.0,
        )
        (self.run_dir / "native-image-types.txt").write_text(
            "\n".join(types) + "\n", encoding="utf-8"
        )
        self.check(
            f"native CopyImage used the key origin frame {frame}; external wl-paste PNG "
            f"decodes to exact seed RGBA/alpha (bytes={details['bytes']})"
        )

        before = self.snapshot()
        paste_frame = self.trigger_ctrl_v()
        self.wait_new("RAW_PASTE_IMAGE frame=", before["image"], "native image self-paste", 15.0)
        time.sleep(0.75)
        after = self.snapshot()
        if after["image"] != before["image"] + 1 or after["paste"] != before["paste"]:
            raise AssertionError(f"self image paste counts wrong: {before} -> {after}")
        self.assert_image_log("RAW_PASTE_IMAGE frame=")
        self.check(f"native CopyImage self-paste via Ctrl+V was exact and delivered once (frame {paste_frame})")
        self.assert_primary_unchanged("after native CopyImage")
        self.scenarios.append(
            {
                "name": "native-copy-image",
                "frame": frame,
                "self_paste_frame": paste_frame,
                "verify": details,
                "advertised_types": types,
            }
        )

    def scenario_native_text(self) -> None:
        frame = self.send_probe_key("t")
        self.wait_platform(frame, "CopyText")
        self.wait_batch(frame, "text")
        self.read_text_until(TEXT_BYTES, "native copied text", 12.0)
        types = self.wait_types(
            lambda values: any(value.startswith("text/") for value in values)
            and not any(value == IMAGE_MIME for value in values),
            "native text-only selection",
            12.0,
        )
        (self.run_dir / "native-text-types.txt").write_text(
            "\n".join(types) + "\n", encoding="utf-8"
        )
        self.check(f"native CopyText exposed exact UTF-8 text externally (frame {frame})")

        before = self.snapshot()
        paste_frame = self.trigger_ctrl_v()
        self.wait_new("RAW_PASTE frame=", before["paste"], "native text self-paste", 15.0)
        time.sleep(0.75)
        after = self.snapshot()
        if after["paste"] != before["paste"] + 1 or after["image"] != before["image"]:
            raise AssertionError(f"self text paste counts wrong: {before} -> {after}")
        self.assert_text_log(TEXT_BYTES)
        self.check(f"native CopyText self-paste via Ctrl+V was exact and delivered once (frame {paste_frame})")
        self.assert_primary_unchanged("after native CopyText")
        self.scenarios.append(
            {
                "name": "native-copy-text",
                "frame": frame,
                "self_paste_frame": paste_frame,
                "advertised_types": types,
            }
        )

    def scenario_mixed_image_then_text(self) -> None:
        frame = self.send_probe_key("m")
        self.wait_platform(frame, "CopyText")
        self.wait_batch(frame, "text")
        self.read_text_until(TEXT_BYTES, "mixed image-then-text winner", 12.0)
        types = self.wait_types(
            lambda values: any(value.startswith("text/") for value in values)
            and not any(value == IMAGE_MIME for value in values),
            "mixed image-then-text text-only selection",
            12.0,
        )
        self.check(
            f"same-batch image-then-text selected the last regular CopyText (frame {frame})"
        )
        self.assert_primary_unchanged("after mixed image-then-text")
        self.scenarios.append(
            {
                "name": "mixed-image-then-text",
                "frame": frame,
                "winner": "text",
                "advertised_types": types,
            }
        )

    def scenario_mixed_text_then_image(self) -> None:
        frame = self.send_probe_key("n")
        self.wait_platform(frame, "CopyImage")
        self.wait_batch(frame, "image")
        image_bytes = self.read_until(
            IMAGE_MIME,
            lambda data: data.startswith(PNG_SIGNATURE),
            "mixed text-then-image winner",
            15.0,
        )
        mixed_png = self.run_dir / "mixed-text-then-image.png"
        mixed_png.write_bytes(image_bytes)
        details = self.verify_png(mixed_png, "mixed-text-then-image")
        types = self.wait_types(
            lambda values: any(value == IMAGE_MIME for value in values)
            and not any(value.startswith("text/") for value in values),
            "mixed text-then-image image-only selection",
            12.0,
        )
        self.check(
            f"same-batch text-then-image selected the last regular CopyImage (frame {frame}) "
            "and external PNG decodes to the exact seed"
        )
        self.assert_primary_unchanged("after mixed text-then-image")
        self.scenarios.append(
            {
                "name": "mixed-text-then-image",
                "frame": frame,
                "winner": "image",
                "verify": details,
                "advertised_types": types,
            }
        )

    def assert_frame_lifecycle(self) -> None:
        text = self.probe_text()
        begins = len(re.findall(r"^FRAME \d+ BEGIN$", text, re.MULTILINE))
        state_lines = self.count(
            "EGUI_STATE take_egui_input=1 "
            "handle_platform_output_with_event_loop=1",
            text,
        )
        if begins == 0 or begins != state_lines:
            raise AssertionError(
                f"egui frame lifecycle mismatch: {begins} BEGIN vs {state_lines} State frames"
            )
        events = self.count("WINDOW_EVENT delivered=", text)
        if events < begins:
            raise AssertionError(f"only {events} actual window events for {begins} frames")
        if "USER_EVENT WAKE" not in text or "REPAINT_CALLBACK" not in text:
            raise AssertionError("no repaint callback/EventLoopProxy wake observed")
        for worker in ("winit-clipboard-read", "egui-winit-png-decode"):
            if not re.search(r'^REPAINT_CALLBACK .*thread=Some\("' + re.escape(worker) + r'"\)', text, re.MULTILINE):
                raise AssertionError(f"no repaint callback from {worker}")
        self.check(
            f"all {begins} logical frames took input and applied platform output once; "
            f"{events} actual WindowEvents forwarded, repaint callback and proxy wake observed; "
            "no periodic redraw timer"
        )

    def stop_probe(self) -> None:
        assert self.app is not None
        self.wtype(["-s", "150", "-k", "q", "-s", "250"])
        deadline = time.monotonic() + 12.0
        while time.monotonic() < deadline:
            if self.app.poll() is not None:
                text = self.probe_text()
                if "PROBE_TIMEOUT" in text:
                    raise RuntimeError("probe hit its internal timeout instead of exiting after q")
                if self.app.returncode != 0:
                    raise RuntimeError(f"probe exited with status {self.app.returncode}")
                if "EXIT frame=" not in text:
                    raise RuntimeError("probe exited without its EXIT evidence line")
                self.check("probe exited cleanly after q")
                return
            time.sleep(0.1)
        raise TimeoutError("probe did not exit after q")

    def execute(self) -> None:
        self.setup_environment()
        (self.run_dir / "probe-seed-rgba.bin").write_bytes(SEED_RGBA)
        (self.run_dir / "probe-text.txt").write_bytes(TEXT_BYTES)
        self.start_sway()
        self.start_primary_sentinel()
        self.start_probe()
        self.scenario_external_text()
        self.scenario_external_image()
        self.scenario_malformed_png()
        self.scenario_absent_regular()
        self.scenario_native_image()
        self.scenario_native_text()
        self.scenario_mixed_image_then_text()
        self.scenario_mixed_text_then_image()
        self.assert_primary_unchanged("final")
        self.assert_frame_lifecycle()
        self.stop_probe()

    def source_hashes(self) -> dict[str, dict[str, str | None]]:
        here = Path(__file__).resolve()
        workspace = here.parents[3]
        candidates = {
            "probe_source": here.with_name("clipboard_image_probe.rs"),
            "smoke_controller": here,
            "egui_winit_lib": workspace / "crates/egui-winit/src/lib.rs",
            "egui_winit_clipboard": workspace / "crates/egui-winit/src/clipboard.rs",
            "egui_winit_clipboard_bridge": workspace / "crates/egui-winit/src/clipboard_bridge.rs",
            "egui_winit_clipboard_image": workspace / "crates/egui-winit/src/clipboard_image.rs",
            "egui_winit_output_selector": workspace / "crates/egui-winit/src/output_selector.rs",
        }
        for env_name, key in (
            ("WINIT_SOURCE", "winit_source"),
            ("WINIT_CLIPBOARD_SOURCE", "winit_clipboard_source"),
        ):
            if value := os.environ.get(env_name):
                candidates[key] = Path(value)
        return {
            name: (
                {"path": str(path), "sha256": sha256_file(path)}
                if path.is_file()
                else {"path": str(path), "sha256": None}
            )
            for name, path in candidates.items()
        }

    def write_result(self) -> None:
        result = {
            "status": self.status,
            "error": self.error,
            "run_dir": str(self.run_dir),
            "evidence_root": str(self.evidence),
            "cargo_target_dir": str(self.cargo_target_dir),
            "probe_binary": str(self.probe),
            "probe_binary_sha256": sha256_file(self.probe) if self.probe.is_file() else None,
            "screenshot": str(self.run_dir / "private-desktop-after-present.png"),
            "sway_tree": str(self.run_dir / "sway-tree.json"),
            "wayland_globals": str(self.run_dir / "wayland-globals.txt"),
            "source_identity": os.environ.get("SOURCE_IDENTITY")
            or os.environ.get("EXPECTED_SOURCE_SHA")
            or "not supplied by parent; exact file hashes below",
            "source_hashes": self.source_hashes(),
            "seed_rgba_hex": SEED_RGBA.hex(),
            "seed_alpha_hex": SEED_ALPHA.hex(),
            "seed_premultiplied_hex": SEED_PREMULTIPLIED.hex(),
            "text_utf8_sha256": hashlib.sha256(TEXT_BYTES).hexdigest(),
            "checks": self.checks,
            "scenarios": self.scenarios,
            "scope": (
                "Real egui_winit::State on a private headless Sway compositor (Linux Wayland): "
                "softbuffer-mapped window, per-frame on_window_event/take_egui_input/"
                "handle_platform_output_with_event_loop, EventLoopProxy repaint wake, external "
                "and self regular text and PNG paste, malformed/absent negatives, native "
                "CopyImage/CopyText, and same-batch last-regular-command coalescing. PRIMARY was "
                "held by an external sentinel and verified unchanged; the probe never wrote it."
            ),
            "limitations": [
                "No multi-seat, focus-transfer, cancellation, trickling/slow-peer, or compositor-crash scenario was exercised.",
                "No X11, Windows, macOS, or non-Wayland path was exercised.",
                "PRIMARY was verified externally as untouched; this probe does not perform a middle-click PRIMARY paste.",
                "The controller uses the pinned headless Sway/wayland-utils/wl-clipboard/wtype extraction; it does not test a real desktop compositor or user session.",
                "PNG verify checks exact 2x2 RGBA/alpha and dimensions, not PNG byte identity with an encoder-specific representation.",
            ],
            "runtime_cleanup": "private XDG_RUNTIME_DIR removed; only controller-owned child processes were terminated",
        }
        (self.run_dir / "result.json").write_text(
            json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )

    def cleanup(self) -> None:
        if self.app is not None:
            self.stop_process(self.app, "probe")
        self.stop_process(self.regular_owner, "regular wl-copy")
        self.stop_process(self.primary_owner, "primary wl-copy")
        self.stop_process(self.keyboard, "private-seat keyboard")
        if self.sway is not None and self.sway.poll() is None:
            if self.env is not None and self.env.get("SWAYSOCK"):
                self.run(
                    [str(NATIVE_ROOT / "usr/bin/swaymsg"), "exit"],
                    timeout=10.0,
                    check=False,
                )
            try:
                self.sway.wait(timeout=12)
            except subprocess.TimeoutExpired:
                self.stop_process(self.sway, "private Sway")
        if self.runtime is not None:
            shutil.rmtree(self.runtime, ignore_errors=True)

    def close_logs(self) -> None:
        for handle in self.log_files:
            try:
                handle.close()
            except Exception:
                pass

    def run_smoke(self) -> None:
        try:
            self.execute()
            self.status = "passed"
        except BaseException as error:
            self.status = "failed"
            self.error = f"{type(error).__name__}: {error}"
            self.record(f"FAILED {self.error}")
            raise
        finally:
            try:
                self.cleanup()
            except BaseException as cleanup_error:
                self.record(f"CLEANUP_ERROR {type(cleanup_error).__name__}: {cleanup_error}")
                if self.error is None:
                    self.status = "failed"
                    self.error = f"cleanup: {type(cleanup_error).__name__}: {cleanup_error}"
            self.write_result()
            self.record(
                f"RESULT status={self.status} run_dir={self.run_dir} checks={len(self.checks)}"
            )
            self.close_logs()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--evidence",
        default=str(KNOWN_EVIDENCE_ROOT),
        help="absolute durable evidence directory (resolved native-tools stay pinned under the known task evidence root)",
    )
    args = parser.parse_args()
    evidence = Path(args.evidence)
    if not evidence.is_absolute():
        parser.error("--evidence must be an absolute path")
    evidence.mkdir(parents=True, exist_ok=True)
    if not NATIVE_ROOT.is_dir():
        print(f"pinned native-tools root is missing: {NATIVE_ROOT}", file=sys.stderr)
        return 2

    try:
        smoke = Smoke(evidence.resolve())
    except BaseException as error:
        print(f"clipboard image smoke setup failed: {error}", file=sys.stderr)
        return 2
    try:
        smoke.run_smoke()
    except BaseException as error:
        print(f"clipboard image smoke failed: {error}", file=sys.stderr)
        print(f"evidence: {smoke.run_dir}", file=sys.stderr)
        return 1
    print(f"clipboard image smoke passed; evidence: {smoke.run_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
