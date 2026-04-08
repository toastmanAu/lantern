//! Lantern chain backend abstraction.
//!
//! Defines the `ChainBackend` trait and provides the four backend kinds:
//! `embedded-light` (subprocess), `remote-light`, `local-full`, `remote-full`.
//! Implementation lands in plan 1d.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
