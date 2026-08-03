use super::*;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TerminalKey {
    pub tab_id: String,
    pub pane_id: String,
}

impl TerminalKey {
    pub fn new(tab_id: impl Into<String>, pane_id: impl Into<String>) -> Self {
        Self {
            tab_id: tab_id.into(),
            pane_id: pane_id.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputRead {
    pub data: Vec<u8>,
    pub cursor: u64,
    pub truncated: bool,
    pub stream_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextOutputRead {
    pub data: String,
    pub cursor: u64,
    pub truncated: bool,
    pub stream_id: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AnsiState {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

impl AnsiState {
    fn feed(&mut self, byte: u8) -> bool {
        match *self {
            Self::Ground if byte == 0x1b => {
                *self = Self::Escape;
                false
            }
            Self::Ground => true,
            Self::Escape => {
                *self = match byte {
                    b'[' => Self::Csi,
                    b']' => Self::Osc,
                    _ => Self::Ground,
                };
                false
            }
            Self::Csi => {
                if (0x40..=0x7e).contains(&byte) {
                    *self = Self::Ground;
                }
                false
            }
            Self::Osc if byte == 0x07 => {
                *self = Self::Ground;
                false
            }
            Self::Osc if byte == 0x1b => {
                *self = Self::OscEscape;
                false
            }
            Self::Osc => false,
            Self::OscEscape if byte == b'\\' => {
                *self = Self::Ground;
                false
            }
            Self::OscEscape if byte == 0x1b => false,
            Self::OscEscape => {
                *self = Self::Osc;
                false
            }
        }
    }
}

#[derive(Debug)]
pub struct OutputRing {
    bytes: VecDeque<u8>,
    capacity: usize,
    start_cursor: u64,
    end_cursor: u64,
    stream_id: u64,
    ansi_state_at_start: AnsiState,
}

impl OutputRing {
    pub fn new(stream_id: u64) -> Self {
        Self::with_capacity(stream_id, OUTPUT_CAPACITY)
    }

    pub fn with_capacity(stream_id: u64, capacity: usize) -> Self {
        let capacity = capacity.clamp(1, OUTPUT_CAPACITY);
        Self {
            bytes: VecDeque::with_capacity(capacity),
            capacity,
            start_cursor: 0,
            end_cursor: 0,
            stream_id,
            ansi_state_at_start: AnsiState::Ground,
        }
    }

    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn cursor_range(&self) -> (u64, u64) {
        (self.start_cursor, self.end_cursor)
    }

    pub fn append(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let added = u64::try_from(data.len()).unwrap_or(u64::MAX);
        self.end_cursor = self.end_cursor.saturating_add(added);
        if data.len() >= self.capacity {
            for byte in self.bytes.drain(..) {
                self.ansi_state_at_start.feed(byte);
            }
            let discarded = data.len().saturating_sub(self.capacity);
            for &byte in &data[..discarded] {
                self.ansi_state_at_start.feed(byte);
            }
            self.bytes.extend(data[discarded..].iter().copied());
            self.start_cursor = self
                .end_cursor
                .saturating_sub(u64::try_from(self.bytes.len()).unwrap_or(u64::MAX));
            return;
        }

        self.bytes.extend(data.iter().copied());
        let excess = self.bytes.len().saturating_sub(self.capacity);
        for byte in self.bytes.drain(..excess) {
            self.ansi_state_at_start.feed(byte);
        }
        self.start_cursor = self
            .end_cursor
            .saturating_sub(u64::try_from(self.bytes.len()).unwrap_or(u64::MAX));
    }

    pub fn read(
        &self,
        requested_stream: Option<u64>,
        requested_cursor: Option<u64>,
        limit: usize,
    ) -> OutputRead {
        let stream_changed = requested_stream.is_some_and(|id| id != self.stream_id);
        let requested_cursor = requested_cursor.unwrap_or(0);
        let invalid_cursor =
            requested_cursor < self.start_cursor || requested_cursor > self.end_cursor;
        let truncated = stream_changed || invalid_cursor;
        let cursor = if truncated {
            self.start_cursor
        } else {
            requested_cursor
        };
        let offset = usize::try_from(cursor.saturating_sub(self.start_cursor))
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
        let take = limit.min(MAX_READ_BYTES).min(self.bytes.len() - offset);
        let data = self
            .bytes
            .iter()
            .skip(offset)
            .take(take)
            .copied()
            .collect::<Vec<_>>();

        OutputRead {
            cursor: cursor.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX)),
            data,
            truncated,
            stream_id: self.stream_id,
        }
    }

    /// Returns valid UTF-8 while cursors continue to count raw bytes.
    pub fn read_text(
        &self,
        requested_stream: Option<u64>,
        requested_cursor: Option<u64>,
        limit: usize,
        strip_ansi: bool,
    ) -> TextOutputRead {
        let stream_changed = requested_stream.is_some_and(|id| id != self.stream_id);
        let requested_cursor = requested_cursor.unwrap_or(0);
        let invalid_cursor =
            requested_cursor < self.start_cursor || requested_cursor > self.end_cursor;
        let truncated = stream_changed || invalid_cursor;
        let raw_cursor = if truncated {
            self.start_cursor
        } else {
            requested_cursor
        };
        let retained = self.bytes.iter().copied().collect::<Vec<_>>();
        let mut start = usize::try_from(raw_cursor.saturating_sub(self.start_cursor))
            .unwrap_or(usize::MAX)
            .min(retained.len());
        while start < retained.len() && is_utf8_continuation(retained[start]) {
            start += 1;
        }

        let mut end = start
            .saturating_add(limit.min(MAX_READ_BYTES))
            .min(retained.len());
        if end < retained.len() && end > start && is_utf8_continuation(retained[end]) {
            while end < retained.len()
                && is_utf8_continuation(retained[end])
                && end - start < MAX_READ_BYTES
            {
                end += 1;
            }
            if end < retained.len() && is_utf8_continuation(retained[end]) {
                while end > start && is_utf8_continuation(retained[end]) {
                    end -= 1;
                }
            }
        }
        end = complete_utf8_prefix_end(&retained, start, end);

        let visible = if strip_ansi {
            let mut state = self.ansi_state_at_start;
            for &byte in &retained[..start] {
                state.feed(byte);
            }
            retained[start..end]
                .iter()
                .copied()
                .filter(|byte| state.feed(*byte))
                .collect::<Vec<_>>()
        } else {
            retained[start..end].to_vec()
        };
        TextOutputRead {
            data: valid_utf8_without_replacement(&visible),
            cursor: self
                .start_cursor
                .saturating_add(u64::try_from(end).unwrap_or(u64::MAX)),
            truncated,
            stream_id: self.stream_id,
        }
    }
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0xc0 == 0x80
}

/// Returns the end of the largest prefix that does not finish with an
/// incomplete UTF-8 sequence. Complete invalid bytes remain consumed so a
/// client cannot get stuck retrying the same cursor forever.
fn complete_utf8_prefix_end(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut offset = start;
    while offset < end {
        match std::str::from_utf8(&bytes[offset..end]) {
            Ok(_) => return end,
            Err(error) => {
                offset += error.valid_up_to();
                match error.error_len() {
                    Some(invalid_len) => offset = offset.saturating_add(invalid_len).min(end),
                    None => return offset,
                }
            }
        }
    }
    end
}

fn valid_utf8_without_replacement(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                output.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    output.push_str(
                        std::str::from_utf8(&remaining[..valid_up_to])
                            .expect("Utf8Error valid_up_to must be valid"),
                    );
                }
                let skip = error
                    .error_len()
                    .unwrap_or_else(|| remaining.len().saturating_sub(valid_up_to))
                    .max(1);
                remaining = &remaining[(valid_up_to + skip).min(remaining.len())..];
            }
        }
    }
    output
}

#[derive(Default)]
struct RegistryState {
    rings: HashMap<TerminalKey, Arc<Mutex<OutputRing>>>,
    default_panes: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct OutputRegistry {
    state: Arc<RwLock<RegistryState>>,
    next_stream_id: Arc<AtomicU64>,
}

impl OutputRegistry {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryState::default())),
            next_stream_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Begins a fresh terminal byte stream and atomically replaces any old
    /// ring for the same pane. Existing sinks keep the detached old ring.
    pub fn begin_stream(
        &self,
        tab_id: impl Into<String>,
        pane_id: impl Into<String>,
    ) -> OutputSink {
        self.begin_stream_with_capacity(tab_id, pane_id, OUTPUT_CAPACITY)
    }

    pub fn begin_stream_with_capacity(
        &self,
        tab_id: impl Into<String>,
        pane_id: impl Into<String>,
        capacity: usize,
    ) -> OutputSink {
        let key = TerminalKey::new(tab_id, pane_id);
        let stream_id = self.next_stream_id.fetch_add(1, Ordering::Relaxed);
        let ring = Arc::new(Mutex::new(OutputRing::with_capacity(stream_id, capacity)));
        let mut state = write_lock(&self.state);
        state
            .default_panes
            .insert(key.tab_id.clone(), key.pane_id.clone());
        state.rings.insert(key.clone(), Arc::clone(&ring));
        OutputSink {
            key,
            stream_id,
            ring,
        }
    }

    pub fn read(
        &self,
        tab_id: &str,
        pane_id: &str,
        stream_id: Option<u64>,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<OutputRead, ApiError> {
        let key = TerminalKey::new(tab_id, pane_id);
        let ring = read_lock(&self.state)
            .rings
            .get(&key)
            .cloned()
            .ok_or_else(|| ApiError::not_found("terminal pane not found"))?;
        let result = mutex_lock(&ring).read(stream_id, cursor, limit);
        Ok(result)
    }

    pub fn read_text(
        &self,
        tab_id: &str,
        pane_id: &str,
        stream_id: Option<u64>,
        cursor: Option<u64>,
        limit: usize,
        strip_ansi: bool,
    ) -> Result<TextOutputRead, ApiError> {
        let key = TerminalKey::new(tab_id, pane_id);
        let ring = read_lock(&self.state)
            .rings
            .get(&key)
            .cloned()
            .ok_or_else(|| ApiError::not_found("terminal pane not found"))?;
        let result = mutex_lock(&ring).read_text(stream_id, cursor, limit, strip_ansi);
        Ok(result)
    }

    pub fn contains(&self, tab_id: &str, pane_id: &str) -> bool {
        read_lock(&self.state)
            .rings
            .contains_key(&TerminalKey::new(tab_id, pane_id))
    }

    pub fn default_pane(&self, tab_id: &str) -> Option<String> {
        read_lock(&self.state).default_panes.get(tab_id).cloned()
    }

    pub fn end_stream(&self, tab_id: &str, pane_id: &str, stream_id: u64) -> bool {
        let key = TerminalKey::new(tab_id, pane_id);
        let mut state = write_lock(&self.state);
        let matches = state
            .rings
            .get(&key)
            .is_some_and(|ring| mutex_lock(ring).stream_id() == stream_id);
        if !matches {
            return false;
        }
        state.rings.remove(&key);
        if state.default_panes.get(tab_id).map(String::as_str) == Some(pane_id) {
            let replacement = state
                .rings
                .keys()
                .find(|candidate| candidate.tab_id == tab_id)
                .map(|candidate| candidate.pane_id.clone());
            match replacement {
                Some(replacement) => {
                    state.default_panes.insert(tab_id.to_owned(), replacement);
                }
                None => {
                    state.default_panes.remove(tab_id);
                }
            }
        }
        true
    }
}

impl fmt::Debug for OutputRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputRegistry")
            .field("pane_count", &read_lock(&self.state).rings.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct OutputSink {
    key: TerminalKey,
    stream_id: u64,
    ring: Arc<Mutex<OutputRing>>,
}

impl OutputSink {
    pub fn key(&self) -> &TerminalKey {
        &self.key
    }

    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }

    pub fn append(&self, data: &[u8]) {
        let mut ring = mutex_lock(&self.ring);
        if ring.stream_id() == self.stream_id {
            ring.append(data);
        }
    }
}

impl fmt::Debug for OutputSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutputSink")
            .field("key", &self.key)
            .field("stream_id", &self.stream_id)
            .finish_non_exhaustive()
    }
}

pub(super) fn mutex_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------
