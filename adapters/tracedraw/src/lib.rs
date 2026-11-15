//! TraceDraw Android adapter library.

use android_activity::AndroidApp;

mod system_fonts;
mod mcp_http;

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
    }
    system_fonts::install_font_env(&app);
    let _mcp = mcp_http::start_mcp_service(
        "TraceDraw".to_string(),
        "0.41.0".to_string(),
        std::sync::Arc::new(|| true),
    );
    tracedraw::android::android_main(app);
}
