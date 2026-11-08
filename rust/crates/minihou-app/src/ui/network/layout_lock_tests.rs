    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Pressing G or S over the network starts a layout transform. Asking egui for the pointer while its data
    /// lock is held used to wait on itself and freeze the whole window, so this runs on a thread with a deadline.
    #[test]
    fn starting_a_layout_transform_does_not_deadlock_on_the_context_lock() {
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let ctx = egui::Context::default();
            let doc = serde_json::json!({"nodes": {"a": {"type": "box"}, "out": {"type": "output", "inputs": {"geo": "a"}}}, "display": "out"});
            let selected: BTreeSet<String> = ["a".to_string()].into();
            let input = egui::RawInput { events: vec![egui::Event::PointerMoved(pos2(40., 40.))], ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                begin_layout(ctx, std::path::Path::new("lock-test.json"), &doc, &selected, LayoutKind::Move);
            });
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(Duration::from_secs(10)).is_ok(), "begin_layout hung on the egui context lock");
    }
