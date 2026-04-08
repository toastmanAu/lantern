//! Lantern Tauri shell library.
//!
//! Owns desktop lifecycle, window/webview orchestration, and the secure
//! bridge into the Rust wallet core. Business logic does NOT live here —
//! see crates/wallet-core. Real Tauri commands land in plan 1f.

use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Init structured logging — env var LANTERN_LOG controls filter
    let filter = EnvFilter::try_from_env("LANTERN_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,lantern=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    tracing::info!("Lantern v{} starting", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .setup(|_app| {
            tracing::info!("Tauri setup complete");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Lantern application");
}
