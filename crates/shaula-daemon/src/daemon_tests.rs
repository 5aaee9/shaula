//! Ownership-lock tests: kernel-held exclusivity, crash-takeover semantics.

#![allow(clippy::unwrap_used)]

use super::acquire_ownership_lock;

#[test]
fn second_acquire_fails_while_held() {
    let dir = tempfile::tempdir().unwrap();
    let _lock = acquire_ownership_lock(dir.path()).unwrap();
    let second = acquire_ownership_lock(dir.path());
    assert!(
        second.is_err(),
        "a live owner must never be probed away or coexisted with"
    );
}

#[test]
fn release_allows_reacquire() {
    let dir = tempfile::tempdir().unwrap();
    {
        let _lock = acquire_ownership_lock(dir.path()).unwrap();
    }
    let reacquired = acquire_ownership_lock(dir.path());
    assert!(reacquired.is_ok(), "release must release the kernel lock");
}
