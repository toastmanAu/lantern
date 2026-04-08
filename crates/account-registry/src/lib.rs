//! Lantern account registry.
//!
//! Stores `AccountRecord` (public projections of accounts — never holds secrets)
//! and dispatches signing requests to the appropriate signer crate via the
//! signing coordinator. Implementation lands in plan 1c.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
