//! Library scan performance regression guard (no network).

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use rtunes::library::scan_paths;

#[test]
fn scan_1000_stub_mp3_completes_within_budget() {
    let dir = std::env::temp_dir().join(format!("rtunes-scan1k-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    for i in 0..1000 {
        let p: PathBuf = dir.join(format!("track_{i:04}.mp3"));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"ID3\x03\x00\x00\x00\x00\x00\x00x").unwrap();
    }

    let root = dunce::canonicalize(&dir).unwrap();
    let t0 = Instant::now();
    let tracks = scan_paths(&[root]);
    let ms = t0.elapsed().as_millis() as u64;

    assert_eq!(tracks.len(), 1000, "expected every stub indexed");

    let budget_ms: u64 = if cfg!(debug_assertions) { 8_000 } else { 2_000 };
    assert!(
        ms < budget_ms,
        "scan took {ms}ms (budget {budget_ms}ms); regression in scanner hot path?"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
