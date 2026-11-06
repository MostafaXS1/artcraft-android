//! TraceDraw Android adapter library.

use android_activity::AndroidApp;

mod system_fonts;

#[no_mangle]
fn android_main(app: AndroidApp) {
    system_fonts::install_font_env(&app);
    tracedraw::android::android_main(app);
}
