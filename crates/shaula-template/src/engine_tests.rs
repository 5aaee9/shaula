//! Engine unit tests that need NO process spawn. Every test that
//! executes the engine lives in `crates/shaula/tests/engine_fence.rs`:
//! since R7-06 the spawn runs behind the fence supervisor, whose hidden
//! marker only the real `shaula` binary routes — a unit-test binary
//! cannot.
#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn binary_hash_stable() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("engine.cmd");
    std::fs::write(&script, "@echo ok\r\n").unwrap();
    let h1 = hash_binary(&script).unwrap();
    let h2 = hash_binary(&script).unwrap();
    assert_eq!(h1, h2);
    assert!(h1.starts_with("sha256:"));
    std::fs::write(&script, "@echo changed\r\n").unwrap();
    let h3 = hash_binary(&script).unwrap();
    assert_ne!(
        h1, h3,
        "same-path binary replacement must change the digest"
    );
}
