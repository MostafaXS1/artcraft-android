//! TraceDraw Android adapter library.

use android_activity::AndroidApp;

mod system_fonts;
mod mcp_http;

#[no_mangle]
fn android_main(app: AndroidApp) {
    system_fonts::install_font_env(&app);
    let _mcp = mcp_http::start_mcp_service(
        "TraceDraw".to_string(),
        "0.41.0".to_string(),
        std::sync::Arc::new(|| true),
    );
    tracedraw::android::android_main(app);
}
