//! Lantern extension host.
//!
//! Loads extension manifests, evaluates permissions, dispatches calls.
//! Treats first-party (statically linked) and third-party (sideloaded)
//! extensions identically. Implementation lands in plans 1f and 3.

#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    #[test]
    const fn placeholder() {}
}
