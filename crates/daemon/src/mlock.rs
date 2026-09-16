//! Swap protection subsystem and kernel memory page locking.
//!
//! Provides primitives to lock sensitive process memory pages and cryptographic/biometric
//! buffers into RAM via `mlock(2)` / `mlockall(2)`, preventing plaintext key material
//! or biometric vectors from being paged to unencrypted swap space on disk.

use std::ops::{Deref, DerefMut};
use zeroize::Zeroize;

/// Attempts to lock the virtual memory pages backing the specified byte slice into RAM.
///
/// Returns `true` if `mlock` succeeded, or `false` if the process lacks `CAP_IPC_LOCK`
/// or exceeded resource limits (e.g. running in an unprivileged test container).
pub fn mlock_slice(slice: &[u8]) -> bool {
    if slice.is_empty() {
        return true;
    }
    // SAFETY:
    // `slice.as_ptr()` is a valid, readable pointer to `slice.len()` bytes allocated in the
    // current process's address space. `libc::mlock` does not mutate the buffer contents or
    // access beyond the page range containing `[ptr, ptr + len)`.
    let res = unsafe { libc::mlock(slice.as_ptr().cast(), slice.len()) };
    res == 0
}

/// Unlocks virtual memory pages previously pinned into RAM via `mlock`.
pub fn munlock_slice(slice: &[u8]) {
    if slice.is_empty() {
        return;
    }
    // SAFETY:
    // `slice.as_ptr()` points to `slice.len()` bytes allocated in the process's address space.
    // `libc::munlock` simply instructs the kernel to unpin the pages.
    unsafe {
        libc::munlock(slice.as_ptr().cast(), slice.len());
    }
}

/// Attempts to lock the calling process's entire address space into RAM via `mlockall`.
///
/// Uses `MCL_CURRENT | MCL_FUTURE` to ensure both current allocations and any future
/// heap/stack allocations made during authentication remain in physical RAM.
/// Returns `true` if granted (typically requires root / `CAP_IPC_LOCK`), or `false` if unprivileged.
pub fn mlock_process_address_space() -> bool {
    let flags = libc::MCL_CURRENT | libc::MCL_FUTURE;
    // SAFETY:
    // `libc::mlockall` is a standard POSIX system call operating on the calling process.
    // Flags `MCL_CURRENT | MCL_FUTURE` are valid kernel flags.
    let res = unsafe { libc::mlockall(flags) };
    res == 0
}

/// Unlocks all process virtual memory pages previously locked via `mlockall`.
pub fn munlock_process_address_space() {
    // SAFETY:
    // `libc::munlockall` is a standard POSIX system call unpinning pages for the calling process.
    unsafe {
        libc::munlockall();
    }
}

/// RAII memory-locked container that pins buffer pages into RAM and zeroizes on drop.
pub struct LockedBuffer<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> {
    inner: T,
    is_locked: bool,
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> LockedBuffer<T> {
    /// Creates a new `LockedBuffer`, attempting to lock backing pages into physical RAM.
    pub fn new(mut inner: T) -> Self {
        let is_locked = mlock_slice(inner.as_mut());
        Self { inner, is_locked }
    }

    /// Returns `true` if the underlying pages were successfully locked by the kernel.
    pub fn is_locked(&self) -> bool {
        self.is_locked
    }

    /// Borrows the inner data as an immutable slice.
    pub fn as_slice(&self) -> &[u8] {
        self.inner.as_ref()
    }

    /// Borrows the inner data as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.inner.as_mut()
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> AsRef<[u8]> for LockedBuffer<T> {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> AsMut<[u8]> for LockedBuffer<T> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.inner.as_mut()
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> Deref for LockedBuffer<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> DerefMut for LockedBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: AsRef<[u8]> + AsMut<[u8]> + Zeroize> Drop for LockedBuffer<T> {
    fn drop(&mut self) {
        if self.is_locked {
            munlock_slice(self.inner.as_ref());
        }
        self.inner.zeroize();
    }
}
