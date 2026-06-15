use guishell::core::transfer::{TransferItem, TransferQueue, TransferStatus, TransferDirection};

#[test]
fn test_queue_add_and_list() {
    let mut queue = TransferQueue::new(3);
    queue.add(TransferItem::new(
        "/remote/file.txt", "/local/file.txt", 1024, TransferDirection::Download,
    ));
    queue.add(TransferItem::new(
        "/remote/big.bin", "/local/big.bin", 1048576, TransferDirection::Download,
    ));
    let items = queue.items();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].status, TransferStatus::Pending));
}

#[test]
fn test_queue_concurrent_limit() {
    let mut queue = TransferQueue::new(2);
    for i in 0..5 {
        queue.add(TransferItem::new(
            &format!("/remote/{}.txt", i), &format!("/local/{}.txt", i),
            100, TransferDirection::Download,
        ));
    }
    let active = queue.activate_pending();
    assert_eq!(active.len(), 2);
    let pending_count = queue.items().iter()
        .filter(|i| matches!(i.status, TransferStatus::Pending)).count();
    assert_eq!(pending_count, 3);
}

#[test]
fn test_queue_complete_activates_next() {
    let mut queue = TransferQueue::new(1);
    queue.add(TransferItem::new("/r/a", "/l/a", 10, TransferDirection::Download));
    queue.add(TransferItem::new("/r/b", "/l/b", 10, TransferDirection::Download));
    let active = queue.activate_pending();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0], 0);
    queue.complete(0);
    let next = queue.activate_pending();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0], 1);
}

#[test]
fn test_queue_update_progress() {
    let mut queue = TransferQueue::new(1);
    queue.add(TransferItem::new("/r/a", "/l/a", 1000, TransferDirection::Upload));
    queue.activate_pending();
    queue.update_progress(0, 500);
    let item = &queue.items()[0];
    assert_eq!(item.bytes_transferred, 500);
    assert!(matches!(item.status, TransferStatus::Active));
}
