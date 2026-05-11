//! End-to-end fetcher smoke test (no network).

use rtunes::fetcher::{FetchEvent, FetchOpts, Fetcher, MockFetcher};
use url::Url;

#[test]
fn mock_fetcher_writes_file_and_sends_done() {
    let dir = std::env::temp_dir().join(format!("rtunes-int-fetch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, rx) = crossbeam_channel::unbounded();
    let f = MockFetcher;
    let url = Url::parse("https://example.com/audio/testclip").unwrap();
    let opts = FetchOpts {
        format: "mp3".into(),
        output_dir: dir.clone(),
    };
    f.fetch(&url, &opts, tx).unwrap();

    let mut done: Option<std::path::PathBuf> = None;
    while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
        if let FetchEvent::Done(p) = ev {
            done = Some(p);
            break;
        }
    }
    let path = done.expect("Done event");
    assert!(path.exists(), "expected {}", path.display());

    let roots = vec![dir.clone()];
    let tracks = rtunes::library::scan_paths(roots.as_slice());
    assert!(!tracks.is_empty(), "scanner should see the new file");

    let _ = std::fs::remove_dir_all(&dir);
}
