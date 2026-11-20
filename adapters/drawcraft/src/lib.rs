//! DrawCraft Android adapter library.

use android_activity::AndroidApp;
use std::path::{Path, PathBuf};

mod system_fonts;
mod mcp_http;

fn migrate_dir_contents(src: &Path, dst: &Path) {
    if !src.is_dir() {
        return;
    }
    let _ = std::fs::create_dir_all(dst);
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = dst.join(file_name);
            if src_path.is_dir() {
                migrate_dir_contents(&src_path, &dst_path);
                let _ = std::fs::remove_dir(&src_path);
            } else if src_path.is_file() {
                if std::fs::rename(&src_path, &dst_path).is_err() {
                    if std::fs::copy(&src_path, &dst_path).is_ok() {
                        let _ = std::fs::remove_file(&src_path);
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir(src);
}

fn resolve_media_dir(app: &AndroidApp) -> Option<PathBuf> {
    if let Some(path) = app.external_data_path() {
        let s = path.to_string_lossy();
        if s.contains("/Android/data/") {
            let replaced = s.replace("/Android/data/", "/Android/media/");
            let media_path = PathBuf::from(replaced);
            let _ = std::fs::create_dir_all(&media_path);
            if path.exists() && path != media_path {
                migrate_dir_contents(&path, &media_path);
            }
            return Some(media_path);
        }
        return Some(path);
    }
    app.internal_data_path()
}

#[no_mangle]
fn android_main(app: AndroidApp) {
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var("RUST_BACKTRACE", "1");
        if let Some(path) = app.internal_data_path().or_else(|| app.external_data_path()) {
            let tmp = path.join("tmp");
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("TMPDIR", &tmp);
        }
        if let Some(media) = resolve_media_dir(&app) {
            std::env::set_var("TRACEDRAW_DOCUMENTS_DIR", &media);
            std::env::set_var("DRAWCRAFT_DOCUMENTS_DIR", &media);
            std::env::set_var("XDG_DATA_HOME", &media);
        }
    }
    system_fonts::install_font_env(&app);
    let _mcp = mcp_http::start_mcp_service(
        "DrawCraft".to_string(),
        "0.41.0".to_string(),
        std::sync::Arc::new(|| true),
    );
    tracedraw::android::android_main(app);
}
