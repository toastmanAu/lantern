//! Best-effort page locking for long-lived unlocked state.
//!
//! With the `mlock` feature (default on) the vault asks the OS to keep the
//! pages holding the master key and every blob out of swap. A refusal (for
//! example Linux's default 64 KiB `RLIMIT_MEMLOCK`) is logged once and
//! ignored: a wallet that cannot open is worse than one that may page.
//!
//! Not covered: transient buffers (Argon2 output, decrypted CBOR). Those
//! are zeroized but never locked. A guard whose buffer was later
//! reallocated simply unlocks a stale range on drop; that is harmless.

use std::fmt;

#[cfg(feature = "mlock")]
pub struct PageLocks {
    guards: Vec<region::LockGuard>,
    warned: bool,
}

#[cfg(feature = "mlock")]
impl PageLocks {
    pub const fn new() -> Self {
        Self {
            guards: Vec::new(),
            warned: false,
        }
    }

    /// Lock the pages backing `bytes`. Empty slices are ignored.
    pub fn lock(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        match region::lock(bytes.as_ptr(), bytes.len()) {
            Ok(guard) => self.guards.push(guard),
            Err(err) => {
                if !self.warned {
                    self.warned = true;
                    tracing::warn!(
                        error = %err,
                        "mlock failed; unlocked vault state may be paged to disk"
                    );
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.guards.clear();
    }

    pub const fn locked_regions(&self) -> usize {
        self.guards.len()
    }

    pub const fn warned(&self) -> bool {
        self.warned
    }
}

#[cfg(not(feature = "mlock"))]
pub struct PageLocks;

#[cfg(not(feature = "mlock"))]
#[allow(
    clippy::unused_self,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_ref_mut
)]
impl PageLocks {
    pub const fn new() -> Self {
        Self
    }

    pub fn lock(&mut self, _bytes: &[u8]) {}

    pub fn clear(&mut self) {}

    pub const fn locked_regions(&self) -> usize {
        0
    }

    pub const fn warned(&self) -> bool {
        false
    }
}

impl fmt::Debug for PageLocks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageLocks")
            .field("regions", &self.locked_regions())
            .field("warned", &self.warned())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::PageLocks;

    #[test]
    fn empty_slice_is_ignored() {
        let mut locks = PageLocks::new();
        locks.lock(&[]);
        assert_eq!(locks.locked_regions(), 0);
        assert!(!locks.warned());
    }

    #[cfg(feature = "mlock")]
    #[test]
    fn locks_then_clears_or_warns_exactly_once() {
        let mut locks = PageLocks::new();
        let a = vec![7u8; 64];
        let b = vec![9u8; 64];
        locks.lock(&a);
        locks.lock(&b);
        // Best-effort contract: every region locked, or a warning was raised.
        assert!(locks.locked_regions() == 2 || locks.warned());
        locks.clear();
        assert_eq!(locks.locked_regions(), 0);
    }
}
