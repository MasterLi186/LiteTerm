use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const PLAYBACK_CONTROLS_HEIGHT: f32 = 40.0;

#[derive(Clone, Default)]
pub struct RecordingRegistry {
    inner: Arc<Mutex<HashMap<String, ActiveRecording>>>,
}

struct ActiveRecording {
    path: PathBuf,
    started: Instant,
    writer: BufWriter<File>,
}

impl RecordingRegistry {
    pub fn start(&self, pane_id: &str, path: &Path, cols: u16, rows: u16) -> Result<(), String> {
        if pane_id.is_empty() {
            return Err("录屏目标无效".into());
        }
        let mut recordings = self
            .inner
            .lock()
            .map_err(|_| "录屏状态锁已损坏".to_string())?;
        if recordings.contains_key(pane_id) {
            return Err("当前终端已在录屏".into());
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建录屏目录：{error}"))?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("无法创建录屏文件：{error}"))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let header = serde_json::json!({
            "version": 2,
            "width": cols.max(1),
            "height": rows.max(1),
            "timestamp": timestamp,
            "title": "LiteTerm Native Recording"
        });
        let mut writer = BufWriter::new(file);
        writeln!(writer, "{header}").map_err(|error| format!("无法写入录屏头：{error}"))?;
        writer
            .flush()
            .map_err(|error| format!("无法写入录屏头：{error}"))?;

        recordings.insert(
            pane_id.to_string(),
            ActiveRecording {
                path: path.to_path_buf(),
                started: Instant::now(),
                writer,
            },
        );
        Ok(())
    }

    pub fn record_output(&self, pane_id: &str, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let Ok(mut recordings) = self.inner.lock() else {
            return;
        };
        let Some(recording) = recordings.get_mut(pane_id) else {
            return;
        };
        let event = serde_json::json!([
            recording.started.elapsed().as_secs_f64(),
            "o",
            String::from_utf8_lossy(bytes)
        ]);
        let _ = writeln!(recording.writer, "{event}");
    }

    pub fn stop(&self, pane_id: &str) -> Result<PathBuf, String> {
        let mut recording = self
            .inner
            .lock()
            .map_err(|_| "录屏状态锁已损坏".to_string())?
            .remove(pane_id)
            .ok_or_else(|| "当前终端没有进行中的录屏".to_string())?;
        recording
            .writer
            .flush()
            .map_err(|error| format!("保存录屏失败：{error}"))?;
        Ok(recording.path)
    }

    pub fn is_recording(&self, pane_id: &str) -> bool {
        self.inner
            .lock()
            .is_ok_and(|recordings| recordings.contains_key(pane_id))
    }

    pub fn stop_all(&self) {
        if let Ok(mut recordings) = self.inner.lock() {
            for (_, mut recording) in recordings.drain() {
                let _ = recording.writer.flush();
            }
        }
    }
}

pub struct PlaybackState {
    cast: Asciicast,
    next_event: usize,
    current_time: f64,
    playing: bool,
    speed: f64,
    last_tick: Instant,
}

impl PlaybackState {
    pub fn new(cast: Asciicast) -> Self {
        Self {
            cast,
            next_event: 0,
            current_time: 0.0,
            playing: false,
            speed: 1.0,
            last_tick: Instant::now(),
        }
    }

    pub fn dimensions(&self) -> (u16, u16) {
        (self.cast.width, self.cast.height)
    }

    pub fn current_time(&self) -> f64 {
        self.current_time
    }

    pub fn total_time(&self) -> f64 {
        self.cast
            .events
            .last()
            .map(|event| event.time)
            .unwrap_or(0.0)
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn toggle(&mut self, terminal: &mut crate::terminal::TerminalState) {
        if self.next_event >= self.cast.events.len() {
            self.restart(terminal);
        }
        self.playing = !self.playing;
        self.last_tick = Instant::now();
    }

    pub fn restart(&mut self, terminal: &mut crate::terminal::TerminalState) {
        terminal.reset_replay(self.cast.width, self.cast.height);
        self.next_event = 0;
        self.current_time = 0.0;
        self.playing = false;
        self.last_tick = Instant::now();
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(0.25, 10.0);
        self.last_tick = Instant::now();
    }

    pub fn seek_ratio(&mut self, ratio: f64, terminal: &mut crate::terminal::TerminalState) {
        let was_playing = self.playing;
        self.restart(terminal);
        self.current_time = self.total_time() * ratio.clamp(0.0, 1.0);
        self.feed_due_events(terminal);
        self.playing = was_playing && self.next_event < self.cast.events.len();
        self.last_tick = Instant::now();
    }

    pub fn tick(&mut self, terminal: &mut crate::terminal::TerminalState) -> bool {
        if !self.playing {
            return false;
        }
        let now = Instant::now();
        self.current_time = (self.current_time
            + now.duration_since(self.last_tick).as_secs_f64() * self.speed)
            .min(self.total_time());
        self.last_tick = now;
        self.feed_due_events(terminal);
        if self.next_event >= self.cast.events.len() {
            self.playing = false;
        }
        true
    }

    fn feed_due_events(&mut self, terminal: &mut crate::terminal::TerminalState) {
        while let Some(event) = self.cast.events.get(self.next_event) {
            if event.time > self.current_time {
                break;
            }
            terminal.feed_replay_output(&event.data);
            self.next_event += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackAction {
    None,
    Toggle,
    Restart,
    SetSpeed(f64),
    Seek(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaybackSnapshot {
    pub current_time: f64,
    pub total_time: f64,
    pub playing: bool,
    pub speed: f64,
}

impl From<&PlaybackState> for PlaybackSnapshot {
    fn from(state: &PlaybackState) -> Self {
        Self {
            current_time: state.current_time(),
            total_time: state.total_time(),
            playing: state.is_playing(),
            speed: state.speed(),
        }
    }
}

pub fn render_playback_controls(
    ctx: &egui::Context,
    sidebar_width: f32,
    bottom_offset: f32,
    state: PlaybackSnapshot,
) -> PlaybackAction {
    let mut action = PlaybackAction::None;
    let screen = ctx.screen_rect();
    let width = (screen.width() - sidebar_width).max(200.0);
    egui::Area::new(egui::Id::new("recording_playback_controls"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            sidebar_width,
            screen.bottom() - bottom_offset - PLAYBACK_CONTROLS_HEIGHT,
        ))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                .stroke(egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                ))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_width(width - 20.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(if state.playing { "暂停" } else { "播放" })
                            .clicked()
                        {
                            action = PlaybackAction::Toggle;
                        }
                        if ui.button("重播").clicked() {
                            action = PlaybackAction::Restart;
                        }
                        for speed in [1.0, 2.0, 5.0, 10.0] {
                            let selected = (state.speed - speed).abs() < f64::EPSILON;
                            if ui
                                .selectable_label(selected, format!("{speed:.0}x"))
                                .clicked()
                            {
                                action = PlaybackAction::SetSpeed(speed);
                            }
                        }

                        let current = state.current_time;
                        let total = state.total_time;
                        let available = (ui.available_width() - 105.0).max(80.0);
                        let (rect, response) = ui
                            .allocate_exact_size(egui::vec2(available, 14.0), egui::Sense::click());
                        let ratio = if total > 0.0 {
                            (current / total).clamp(0.0, 1.0) as f32
                        } else {
                            0.0
                        };
                        ui.painter().rect_filled(
                            egui::Rect::from_center_size(
                                rect.center(),
                                egui::vec2(rect.width(), 6.0),
                            ),
                            3.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        );
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(rect.left(), rect.center().y - 3.0),
                                egui::vec2(rect.width() * ratio, 6.0),
                            ),
                            3.0,
                            egui::Color32::from_rgb(0x00, 0xd4, 0xff),
                        );
                        if response.clicked() {
                            if let Some(pointer) = response.interact_pointer_pos() {
                                action = PlaybackAction::Seek(
                                    ((pointer.x - rect.left()) / rect.width()) as f64,
                                );
                            }
                        }
                        ui.monospace(format!("{} / {}", format_time(current), format_time(total)));
                    });
                });
        });
    action
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asciicast {
    pub width: u16,
    pub height: u16,
    pub events: Vec<AsciicastEvent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AsciicastEvent {
    pub time: f64,
    pub data: Vec<u8>,
}

#[derive(Deserialize)]
struct AsciicastHeader {
    version: u8,
    width: u16,
    height: u16,
}

pub fn load(path: &Path) -> Result<Asciicast, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|error| format!("无法读取录屏：{error}"))?;
    parse(&contents)
}

pub fn parse(contents: &str) -> Result<Asciicast, String> {
    let mut lines = contents.lines().filter(|line| !line.trim().is_empty());
    let header: AsciicastHeader =
        serde_json::from_str(lines.next().ok_or_else(|| "录屏文件为空".to_string())?)
            .map_err(|error| format!("录屏头格式无效：{error}"))?;
    if header.version != 2 || header.width == 0 || header.height == 0 {
        return Err("仅支持有效的 asciicast v2 录屏".into());
    }
    let mut events = Vec::new();
    let mut previous = 0.0_f64;
    for line in lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(items) = value.as_array() else {
            continue;
        };
        let (Some(time), Some(kind), Some(data)) = (
            items.first().and_then(serde_json::Value::as_f64),
            items.get(1).and_then(serde_json::Value::as_str),
            items.get(2).and_then(serde_json::Value::as_str),
        ) else {
            continue;
        };
        if kind != "o" || !time.is_finite() || time < 0.0 {
            continue;
        }
        let time = time.max(previous);
        previous = time;
        events.push(AsciicastEvent {
            time,
            data: data.as_bytes().to_vec(),
        });
    }
    Ok(Asciicast {
        width: header.width,
        height: header.height,
        events,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingDialogMode {
    Start,
    Playback,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecordingDialogAction {
    None,
    Cancel,
    Confirm {
        mode: RecordingDialogMode,
        path: PathBuf,
    },
}

#[derive(Default)]
pub struct RecordingDialog {
    mode: Option<RecordingDialogMode>,
    path: String,
    error: String,
    request_focus: bool,
}

impl RecordingDialog {
    pub fn open_start(&mut self) {
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let directory = dirs::video_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        self.open(
            RecordingDialogMode::Start,
            directory.join(format!("liteterm-{timestamp}.cast")),
        );
    }

    pub fn open_playback(&mut self) {
        self.open(RecordingDialogMode::Playback, PathBuf::new());
    }

    fn open(&mut self, mode: RecordingDialogMode, path: PathBuf) {
        self.mode = Some(mode);
        self.path = path.to_string_lossy().into_owned();
        self.error.clear();
        self.request_focus = true;
    }

    pub fn is_open(&self) -> bool {
        self.mode.is_some()
    }

    pub fn render(&mut self, ctx: &egui::Context) -> RecordingDialogAction {
        let Some(mode) = self.mode else {
            return RecordingDialogAction::None;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(match mode {
            RecordingDialogMode::Start => "开始录屏",
            RecordingDialogMode::Playback => "回放录屏",
        })
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("录屏文件路径");
            let response = ui
                .horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.path)
                            .desired_width(340.0)
                            .hint_text("/path/to/session.cast"),
                    );
                    if ui.button("浏览…").clicked() {
                        let current = PathBuf::from(self.path.trim());
                        let mut dialog =
                            rfd::FileDialog::new().add_filter("asciicast 录屏", &["cast"]);
                        if let Some(parent) = current.parent().filter(|path| path.is_dir()) {
                            dialog = dialog.set_directory(parent);
                        }
                        let selected = match mode {
                            RecordingDialogMode::Start => {
                                dialog = dialog.set_title("选择录屏保存位置");
                                if let Some(name) = current.file_name() {
                                    dialog = dialog.set_file_name(name.to_string_lossy());
                                }
                                dialog.save_file()
                            }
                            RecordingDialogMode::Playback => {
                                dialog.set_title("选择 asciicast 录屏文件").pick_file()
                            }
                        };
                        if let Some(path) = selected {
                            self.path = path.to_string_lossy().into_owned();
                            self.error.clear();
                        }
                    }
                    response
                })
                .inner;
            if self.request_focus {
                response.request_focus();
                self.request_focus = false;
            }
            if !self.error.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(0xf8, 0x51, 0x49), &self.error);
            }
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    cancel = true;
                }
                if ui.button("确定").clicked() {
                    confirm = true;
                }
            });
            cancel |= ui.input(|input| input.key_pressed(egui::Key::Escape));
            confirm |= ui.input(|input| input.key_pressed(egui::Key::Enter));
        });
        if cancel {
            self.mode = None;
            return RecordingDialogAction::Cancel;
        }
        if confirm {
            let path = PathBuf::from(self.path.trim());
            if self.path.trim().is_empty() {
                self.error = "请输入录屏文件路径".into();
            } else {
                self.mode = None;
                return RecordingDialogAction::Confirm { mode, path };
            }
        }
        RecordingDialogAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_v2_output_and_clamps_regressing_time() {
        let cast = parse(
            "{\"version\":2,\"width\":80,\"height\":24}\n[1.0,\"o\",\"a\"]\n[0.5,\"o\",\"b\"]\n",
        )
        .unwrap();
        assert_eq!((cast.width, cast.height), (80, 24));
        assert_eq!(cast.events[1].time, 1.0);
        assert_eq!(cast.events[1].data, b"b");
    }

    #[test]
    fn registry_writes_a_valid_cast_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.cast");
        let registry = RecordingRegistry::default();
        registry.start("pane", &path, 80, 24).unwrap();
        registry.record_output("pane", "中文".as_bytes());
        assert_eq!(registry.stop("pane").unwrap(), path);
        let cast = load(&path).unwrap();
        assert_eq!(cast.events[0].data, "中文".as_bytes());
        assert!(registry.start("pane", &path, 80, 24).is_err());
    }

    #[test]
    fn duplicate_active_recording_does_not_create_an_orphan_file() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first.cast");
        let second = temp.path().join("second.cast");
        let registry = RecordingRegistry::default();
        registry.start("pane", &first, 80, 24).unwrap();
        assert!(registry.start("pane", &second, 80, 24).is_err());
        assert!(!second.exists());
        registry.stop("pane").unwrap();
    }

    #[test]
    fn playback_seek_replays_events_up_to_the_target_time() {
        let cast = Asciicast {
            width: 80,
            height: 24,
            events: vec![
                AsciicastEvent {
                    time: 1.0,
                    data: b"first".to_vec(),
                },
                AsciicastEvent {
                    time: 2.0,
                    data: b"second".to_vec(),
                },
            ],
        };
        let mut playback = PlaybackState::new(cast);
        let mut terminal = crate::terminal::TerminalState::new_replay(80, 24);
        let revision = terminal.render_revision();
        playback.seek_ratio(0.5, &mut terminal);
        assert_eq!(playback.current_time(), 1.0);
        assert_eq!(playback.next_event, 1);
        assert!(!playback.is_playing());
        assert!(terminal.render_revision() > revision);
    }
}
