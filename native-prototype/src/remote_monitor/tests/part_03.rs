    #[test]
    fn closed_sink_stops_worker() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            4,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            |_event| Err(()),
            timing(),
            done_tx,
        )
        .unwrap();

        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(collects.load(Ordering::SeqCst), 1);
        drop(handle);
    }
