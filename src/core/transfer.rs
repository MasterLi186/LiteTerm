/// Direction of a file transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferDirection {
    Download,
    Upload,
}

/// Status of a single transfer item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Active,
    Completed,
    Failed(String),
    Cancelled,
}

/// A single file transfer item in the queue.
#[derive(Debug, Clone)]
pub struct TransferItem {
    pub remote_path: String,
    pub local_path: String,
    pub total_size: u64,
    pub bytes_transferred: u64,
    pub direction: TransferDirection,
    pub status: TransferStatus,
}

impl TransferItem {
    /// Create a new transfer item with Pending status and zero bytes transferred.
    pub fn new(
        remote_path: &str,
        local_path: &str,
        total_size: u64,
        direction: TransferDirection,
    ) -> Self {
        Self {
            remote_path: remote_path.to_string(),
            local_path: local_path.to_string(),
            total_size,
            bytes_transferred: 0,
            direction,
            status: TransferStatus::Pending,
        }
    }

    /// Return the progress as a percentage (0.0 to 100.0).
    pub fn progress_percent(&self) -> f64 {
        if self.total_size == 0 {
            return 100.0;
        }
        (self.bytes_transferred as f64 / self.total_size as f64) * 100.0
    }
}

/// A queue of file transfers with a concurrency limit.
#[derive(Debug)]
pub struct TransferQueue {
    items: Vec<TransferItem>,
    max_concurrent: usize,
}

impl TransferQueue {
    /// Create a new transfer queue with the given maximum concurrent transfers.
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            items: Vec::new(),
            max_concurrent,
        }
    }

    /// Add a transfer item to the queue.
    pub fn add(&mut self, item: TransferItem) {
        self.items.push(item);
    }

    /// Return a reference to all items in the queue.
    pub fn items(&self) -> &[TransferItem] {
        &self.items
    }

    /// Activate pending items up to the concurrency limit.
    ///
    /// Returns the indices of newly activated items.
    pub fn activate_pending(&mut self) -> Vec<usize> {
        let active_count = self
            .items
            .iter()
            .filter(|i| matches!(i.status, TransferStatus::Active))
            .count();

        let slots = self.max_concurrent.saturating_sub(active_count);
        let mut activated = Vec::new();

        for (idx, item) in self.items.iter_mut().enumerate() {
            if activated.len() >= slots {
                break;
            }
            if matches!(item.status, TransferStatus::Pending) {
                item.status = TransferStatus::Active;
                activated.push(idx);
            }
        }

        activated
    }

    /// Update the bytes transferred for an item by index.
    pub fn update_progress(&mut self, index: usize, bytes_transferred: u64) {
        if let Some(item) = self.items.get_mut(index) {
            item.bytes_transferred = bytes_transferred;
        }
    }

    /// Mark an item as completed by index.
    pub fn complete(&mut self, index: usize) {
        if let Some(item) = self.items.get_mut(index) {
            item.bytes_transferred = item.total_size;
            item.status = TransferStatus::Completed;
        }
    }

    /// Mark an item as failed with an error message.
    pub fn fail(&mut self, index: usize, error: &str) {
        if let Some(item) = self.items.get_mut(index) {
            item.status = TransferStatus::Failed(error.to_string());
        }
    }

    /// Mark an item as cancelled.
    pub fn cancel(&mut self, index: usize) {
        if let Some(item) = self.items.get_mut(index) {
            item.status = TransferStatus::Cancelled;
        }
    }

    /// Remove all completed, failed, and cancelled items from the queue.
    ///
    /// Returns the number of items removed.
    pub fn remove_completed(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|item| {
            matches!(item.status, TransferStatus::Pending | TransferStatus::Active)
        });
        before - self.items.len()
    }
}
