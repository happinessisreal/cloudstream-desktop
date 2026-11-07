use cloudstream_core::App;

#[test]
fn custom_trackers_save_read_and_filter_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let res = dir.path().join("res");

    let app = App::open(data.clone(), res.clone()).unwrap();
    // Initially empty
    let initial = app.get_custom_trackers().unwrap();
    assert!(initial.is_empty());

    // Save list with comments, whitespace, and duplicates (including casing and trailing slashes)
    let input = vec![
        "udp://tracker.opentrackr.org:1337/announce".to_string(),
        "   udp://open.demonii.com:1337/announce   ".to_string(),
        "".to_string(),
        "UDP://tracker.opentrackr.org:1337/announce/".to_string(), // duplicate with uppercase and trailing slash
        "udp://explodie.org:6969/announce".to_string(),
        "udp://open.demonii.com:1337/announce/".to_string(), // duplicate with trailing slash
    ];
    app.save_custom_trackers(&input).unwrap();

    // Verify read: exactly 3 unique trackers, zero duplicates
    let read = app.get_custom_trackers().unwrap();
    assert_eq!(read.len(), 3);
    assert_eq!(read[0], "udp://tracker.opentrackr.org:1337/announce");
    assert_eq!(read[1], "udp://open.demonii.com:1337/announce");
    assert_eq!(read[2], "udp://explodie.org:6969/announce");

    // Reopen app and verify persistence
    let reopened = App::open(data, res).unwrap();
    let persisted = reopened.get_custom_trackers().unwrap();
    assert_eq!(persisted, read);
}
