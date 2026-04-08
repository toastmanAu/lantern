//! Lantern SDK schemas.
//!
//! Single source of truth for types that cross the IPC boundary into the
//! TypeScript SDK. All types here `#[derive(specta::Type)]` so tauri-specta
//! can export them as TS bindings. Real type defs land in plans 1c–1f.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
