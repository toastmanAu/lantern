//! Lantern wallet core.
//!
//! This crate orchestrates the vault, account registry, signing coordinator,
//! chain backend manager, and extension host. Implementation lands in plans 1b–1f.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {
        // Placeholder so `cargo test -p lantern-wallet-core` runs.
        // Real tests land in subsequent plans.
    }
}
