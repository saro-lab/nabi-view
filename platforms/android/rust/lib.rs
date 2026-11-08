#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: nabi_view::android_activity::AndroidApp) {
    if let Err(error) = nabi_view::run_android(app) {
        eprintln!("NABI VIEW: {error}");
    }
}
