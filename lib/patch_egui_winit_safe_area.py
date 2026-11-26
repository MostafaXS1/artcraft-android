#!/usr/bin/env python3
"""Patch a vendored upstream egui-winit crate for 4-side Android safe/non-immersive insets."""
from pathlib import Path
import shutil
import sys

def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit("Usage: patch_egui_winit_safe_area.py <egui-winit-dir> <builder-root>")
    crate_dir = Path(sys.argv[1])
    root = Path(sys.argv[2])
    src_safe = root / "vendor" / "egui-winit" / "src" / "safe_area.rs"
    dst_safe = crate_dir / "src" / "safe_area.rs"
    if src_safe.is_file() and (crate_dir / "src").is_dir():
        shutil.copyfile(src_safe, dst_safe)

    lib_rs = crate_dir / "src" / "lib.rs"
    if not lib_rs.is_file():
        return
    txt = lib_rs.read_text(encoding="utf-8")
    if "crate::safe_area::inset_pixels()" in txt:
        return

    old_block = """        let screen_size_in_pixels = screen_size_in_pixels(window);
        let screen_size_in_points =
            screen_size_in_pixels / pixels_per_point(&self.egui_ctx, window);

        self.egui_input.screen_rect = (screen_size_in_points.x > 0.0
            && screen_size_in_points.y > 0.0)
            .then(|| Rect::from_min_size(Pos2::ZERO, screen_size_in_points));"""

    new_block = """        let screen_size_in_pixels = screen_size_in_pixels(window);
        let pixels_per_point = pixels_per_point(&self.egui_ctx, window);
        let screen_size_in_points =
            screen_size_in_pixels / pixels_per_point;

        #[cfg(target_os = "android")]
        let safe_insets_points = if pixels_per_point > 0.0 {
            let px = crate::safe_area::inset_pixels();
            egui::epaint::MarginF32 {
                left: px.left.max(0.0) / pixels_per_point,
                top: px.top.max(0.0) / pixels_per_point,
                right: px.right.max(0.0) / pixels_per_point,
                bottom: px.bottom.max(0.0) / pixels_per_point,
            }
        } else {
            egui::epaint::MarginF32::default()
        };
        #[cfg(not(target_os = "android"))]
        let safe_insets_points = egui::epaint::MarginF32::default();

        let max_x = screen_size_in_points.x - safe_insets_points.right;
        let max_y = screen_size_in_points.y - safe_insets_points.bottom;
        self.egui_input.screen_rect = (max_x > safe_insets_points.left
            && max_y > safe_insets_points.top)
            .then(|| {
                Rect::from_min_max(
                    Pos2::new(safe_insets_points.left, safe_insets_points.top),
                    Pos2::new(max_x, max_y),
                )
            });

        #[cfg(target_os = "android")]
        {
            self.egui_input.safe_area_insets = Some(egui::SafeAreaInsets(safe_insets_points));
        }"""

    if old_block in txt:
        txt = txt.replace(old_block, new_block, 1)
        lib_rs.write_text(txt, encoding="utf-8")
        print(f"Patched 4-side Android safe insets in {lib_rs}")

if __name__ == "__main__":
    main()
