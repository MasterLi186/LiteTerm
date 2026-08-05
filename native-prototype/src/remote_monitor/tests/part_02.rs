    #[test]
    fn refresh_and_detail_share_one_source_and_detail_is_not_swallowed() {
        let connects = Arc::new(AtomicUsize::new(0));
        let collects = Arc::new(AtomicUsize::new(0));
        let detail_requests = Arc::new(Mutex::new(Vec::new()));
        let network_requests = Arc::new(AtomicUsize::new(0));
        let source = DetailSource {
            snapshots: VecDeque::from([Ok(SAMPLE.to_string()), Ok(SAMPLE.to_string())]),
            collects: Arc::clone(&collects),
            detail_requests: Arc::clone(&detail_requests),
            network_requests: Arc::clone(&network_requests),
        };
        let mut source = Some(source);
        let factory_connects = Arc::clone(&connects);
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            31,
            move || {
                factory_connects.fetch_add(1, Ordering::SeqCst);
                source.take().ok_or_else(|| "no source".to_string())
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.fetch_process_detail("tab-1".to_string(), 9, 42);
        let event = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let RemoteMonitorEvent::ProcessDetail {
            generation,
            requester,
            request_id,
            result,
            ..
        } = event
        else {
            panic!("expected process detail")
        };
        assert_eq!(generation, 31);
        assert_eq!(requester, "tab-1");
        assert_eq!(request_id, 9);
        assert_eq!(result.unwrap().identity.pid, 42);

        handle.fetch_network_detail("network-tab-1".to_string(), 10);
        let event = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let RemoteMonitorEvent::NetworkDetail {
            generation,
            requester,
            request_id,
            result,
            ..
        } = event
        else {
            panic!("expected network detail")
        };
        assert_eq!(generation, 31);
        assert_eq!(requester, "network-tab-1");
        assert_eq!(request_id, 10);
        assert_eq!(result.unwrap().connections[0].pid, Some(42));

        handle.refresh();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        assert_eq!(connects.load(Ordering::SeqCst), 1);
        assert_eq!(collects.load(Ordering::SeqCst), 2);
        assert_eq!(*detail_requests.lock().unwrap(), vec![42]);
        assert_eq!(network_requests.load(Ordering::SeqCst), 1);
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn adjacent_refresh_requests_are_coalesced() {
        struct BlockingRefreshSource {
            collects: Arc<AtomicUsize>,
            second_collect_started: mpsc::Sender<()>,
            release_second_collect: mpsc::Receiver<()>,
        }

        impl SnapshotSource for BlockingRefreshSource {
            fn collect(&mut self) -> Result<String, String> {
                let collect_number = self.collects.fetch_add(1, Ordering::SeqCst) + 1;
                if collect_number == 2 {
                    self.second_collect_started
                        .send(())
                        .map_err(|_| "二次刷新通知接收端已关闭".to_string())?;
                    self.release_second_collect
                        .recv()
                        .map_err(|_| "二次刷新释放通知发送端已关闭".to_string())?;
                }
                Ok(SAMPLE.to_string())
            }
        }

        let collects = Arc::new(AtomicUsize::new(0));
        let (second_collect_started, second_collect_started_rx) = mpsc::channel();
        let (release_second_collect, release_second_collect_rx) = mpsc::channel();
        let mut source = Some(BlockingRefreshSource {
            collects: Arc::clone(&collects),
            second_collect_started,
            release_second_collect: release_second_collect_rx,
        });
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            32,
            move || source.take().ok_or_else(|| "no source".to_string()),
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.refresh();
        second_collect_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("第二次刷新应进入受控 collect");
        handle.refresh();
        handle.refresh();
        release_second_collect
            .send(())
            .expect("应能释放第二次刷新");
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        assert!(events_rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(collects.load(Ordering::SeqCst), 2);
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn detail_during_retry_returns_an_event_and_refresh_reconnects() {
        let collects = Arc::new(AtomicUsize::new(0));
        let detail_requests = Arc::new(Mutex::new(Vec::new()));
        let source = DetailSource {
            snapshots: VecDeque::from([Ok(SAMPLE.to_string())]),
            collects,
            detail_requests,
            network_requests: Arc::new(AtomicUsize::new(0)),
        };
        let mut sources = VecDeque::from([Err("SSH 连接失败".to_string()), Ok(source)]);
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            33,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Failed { .. }
        ));
        handle.fetch_network_detail("network-tab-offline".to_string(), 4);
        let event = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            RemoteMonitorEvent::NetworkDetail {
                requester,
                request_id: 4,
                result: Err(_),
                ..
            } if requester == "network-tab-offline"
        ));
        handle.fetch_process_detail("tab-offline".to_string(), 5, 42);
        let event = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            RemoteMonitorEvent::ProcessDetail {
                requester,
                request_id: 5,
                result: Err(_),
                ..
            } if requester == "tab-offline"
        ));
        handle.refresh();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn shutdown_does_not_block_and_prevents_a_second_collect() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            1,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(collects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_is_nonblocking_idempotent_and_reports_worker_done() {
        struct BlockingSource {
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        }

        impl SnapshotSource for BlockingSource {
            fn collect(&mut self) -> Result<String, String> {
                self.entered
                    .send(())
                    .map_err(|_| "进入通知接收端已关闭".to_string())?;
                self.release
                    .recv()
                    .map_err(|_| "释放通知发送端已关闭".to_string())?;
                Ok(SAMPLE.to_string())
            }
        }

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (events_tx, _events_rx) = mpsc::channel();
        let mut source = Some(BlockingSource {
            entered: entered_tx,
            release: release_rx,
        });
        let mut handle = super::start_worker_with_sink(
            MonitorKey::remote("user", "host", 22),
            9,
            move || source.take().ok_or_else(|| "no source".to_string()),
            events_tx,
            timing(),
        )
        .unwrap();
        let done = handle
            .take_done_receiver_for_test()
            .expect("生产 handle 应提供独立完成通知");

        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker 应进入阻塞 collect");
        let (returned_tx, returned_rx) = mpsc::channel();
        std::thread::spawn(move || {
            handle.shutdown();
            handle.shutdown();
            drop(handle);
            let _ = returned_tx.send(());
        });
        returned_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown 和 drop 不应等待阻塞中的 collect");
        assert!(
            done.try_recv().is_err(),
            "释放数据源前 worker 不应提前报告完成"
        );
        release_tx.send(()).expect("应能释放阻塞的数据源");
        done.recv_timeout(Duration::from_secs(1))
            .expect("collect 返回后 worker 应观察 shutdown 并结束");
    }

    #[test]
    fn dropping_handle_signals_shutdown_and_reports_worker_done() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Ok(source)]);
        let mut handle = super::start_worker_with_sink(
            MonitorKey::remote("user", "host", 22),
            10,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
        )
        .unwrap();
        let done = handle
            .take_done_receiver_for_test()
            .expect("生产 handle 应提供独立完成通知");

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        drop(handle);

        done.recv_timeout(Duration::from_secs(1))
            .expect("drop 不得遗留等待中的监控 worker");
    }

    #[test]
    fn connect_failure_reports_then_retries_to_update() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let source = source([Ok(SAMPLE.to_string())], &collects, &drops);
        let mut sources = VecDeque::from([Err("SSH 连接失败".to_string()), Ok(source)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            2,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Failed { .. }
        ));
        handle.refresh();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn collect_failure_drops_source_and_reconnect_resets_parser() {
        let collects = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let (events_tx, events_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let first = source(
            [
                Ok(SAMPLE.to_string()),
                Err("channel read failed".to_string()),
            ],
            &collects,
            &drops,
        );
        let second_snapshot = SAMPLE
            .replace(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "cpu  120 0 60 870 50 0 0 0 0 0",
            )
            .replace("eth0: 1000", "eth0: 5000")
            .replace("0 0 0 0 2000", "0 0 0 0 10000");
        let second = source([Ok(second_snapshot)], &collects, &drops);
        let mut sources = VecDeque::from([Ok(first), Ok(second)]);
        let handle = start_worker_with_sink_for_test(
            MonitorKey::remote("user", "host", 22),
            3,
            move || {
                sources
                    .pop_front()
                    .unwrap_or_else(|| Err("no source".to_string()))
            },
            events_tx,
            timing(),
            done_tx,
        )
        .unwrap();

        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Update { .. }
        ));
        handle.refresh();
        assert!(matches!(
            events_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            RemoteMonitorEvent::Failed { .. }
        ));
        handle.refresh();
        let update = events_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let RemoteMonitorEvent::Update { data, .. } = update else {
            panic!("expected update")
        };
        assert_eq!(data.cpu_percent, 0.0);
        assert!(data
            .net_interfaces
            .iter()
            .all(|iface| iface.rx_rate == 0 && iface.tx_rate == 0));
        assert!(drops.load(Ordering::SeqCst) >= 1);
        handle.shutdown();
        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }
