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

    // Save list with comments and whitespace
    let input = vec![
        "udp://tracker.opentrackr.org:1337/announce".to_string(),
        "   udp://open.demonii.com:1337/announce   ".to_string(),
        "".to_string(),
        "udp://explodie.org:6969/announce".to_string(),
    ];
    app.save_custom_trackers(&input).unwrap();

    // Verify read
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
