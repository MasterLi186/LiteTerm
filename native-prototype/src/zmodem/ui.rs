use std::path::PathBuf;

const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x11, 0x17);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x21, 0x26, 0x2d);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xc9, 0xd1, 0xd9);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const CYAN: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3f, 0xb9, 0x50);
const YELLOW: egui::Color32 = egui::Color32::from_rgb(0xd2, 0x99, 0x22);
const RED: egui::Color32 = egui::Color32::from_rgb(0xf8, 0x51, 0x49);
const PANE_OVERLAY_INSET: f32 = 12.0;
const TRANSFER_STRIP_MAX_WIDTH: f32 = 520.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZmodemCapability {
    Enabled,
    Disabled { reason: String },
}

impl ZmodemCapability {
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self::Disabled {
            reason: reason.into(),
        }
    }

    pub fn disabled_reason(&self) -> Option<&str> {
        match self {
            Self::Enabled => None,
            Self::Disabled { reason } => Some(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferDirection {
    Send,
    Receive,
}

impl TransferDirection {
    fn label(self) -> &'static str {
        match self {
            Self::Send => "发送",
            Self::Receive => "接收",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Send => "↑",
            Self::Receive => "↓",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    Preparing,
    Transferring,
    Cancelling,
    Completed,
    Cancelled,
    Failed(String),
}

impl TransferStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Transferring | Self::Cancelling
        )
    }

    fn label(&self) -> &str {
        match self {
            Self::Preparing => "准备中",
            Self::Transferring => "传输中",
            Self::Cancelling => "正在取消",
            Self::Completed => "已完成",
            Self::Cancelled => "已取消",
            Self::Failed(_) => "失败",
        }
    }

    fn color(&self) -> egui::Color32 {
        match self {
            Self::Preparing | Self::Transferring => CYAN,
            Self::Cancelling => YELLOW,
            Self::Completed => GREEN,
            Self::Cancelled => MUTED,
            Self::Failed(_) => RED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferView {
    pub transfer_id: u64,
    pub direction: TransferDirection,
    pub filename: String,
    pub transferred: u64,
    pub total: u64,
    pub status: TransferStatus,
}

impl TransferView {
    pub fn progress_fraction(&self) -> f32 {
        if self.total == 0 {
            if self.status == TransferStatus::Completed {
                1.0
            } else {
                0.0
            }
        } else {
            (self.transferred as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PaneZmodemView {
    pub transfer: Option<TransferView>,
    send_error: Option<String>,
    transfer_error: Option<String>,
}

impl PaneZmodemView {
    pub fn set_transfer(&mut self, transfer: TransferView) {
        self.transfer = Some(transfer);
        self.send_error = None;
        self.transfer_error = None;
    }

    pub fn show_send_error(&mut self, error: impl Into<String>) {
        self.send_error = Some(error.into());
    }

    pub fn show_transfer_error(&mut self, error: impl Into<String>) {
        self.transfer_error = Some(error.into());
    }

    pub fn has_overlay(&self) -> bool {
        self.transfer.is_some() || self.send_error.is_some() || self.transfer_error.is_some()
    }

    pub fn update_progress(&mut self, transfer_id: u64, transferred: u64, total: u64) -> bool {
        let Some(transfer) = self
            .transfer
            .as_mut()
            .filter(|transfer| transfer.transfer_id == transfer_id)
        else {
            return false;
        };
        transfer.transferred = transferred;
        transfer.total = total;
        transfer.status = TransferStatus::Transferring;
        self.transfer_error = None;
        true
    }

    pub fn set_status(&mut self, transfer_id: u64, status: TransferStatus) -> bool {
        let Some(transfer) = self
            .transfer
            .as_mut()
            .filter(|transfer| transfer.transfer_id == transfer_id)
        else {
            return false;
        };
        transfer.status = status;
        self.transfer_error = None;
        true
    }

    pub fn dismiss_transfer(&mut self, transfer_id: u64) -> bool {
        if !self
            .transfer
            .as_ref()
            .is_some_and(|transfer| transfer.transfer_id == transfer_id)
        {
            return false;
        }
        self.transfer = None;
        self.transfer_error = None;
        true
    }

    pub fn active_transfer_id(&self) -> Option<u64> {
        self.transfer
            .as_ref()
            .filter(|transfer| transfer.status.is_active())
            .map(|transfer| transfer.transfer_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZmodemUiAction {
    StartSend { paths: Vec<PathBuf> },
    Cancel { transfer_id: u64 },
    Dismiss { transfer_id: u64 },
}

fn format_bytes(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.1} GB", bytes as f64 / 1_073_741_824.0),
    }
}

fn pane_overlay_anchor(pane_rect: egui::Rect) -> egui::Pos2 {
    let inset_x = PANE_OVERLAY_INSET.min((pane_rect.width() / 2.0).max(0.0));
    let inset_y = PANE_OVERLAY_INSET.min((pane_rect.height() / 2.0).max(0.0));
    pane_rect.right_bottom() - egui::vec2(inset_x, inset_y)
}

fn transfer_strip_max_width(pane_rect: egui::Rect) -> f32 {
    (pane_rect.width() - 2.0 * PANE_OVERLAY_INSET)
        .max(1.0)
        .min(TRANSFER_STRIP_MAX_WIDTH)
}

/// Render the pane-scoped ZMODEM transfer strip.
///
/// `pane_rect` is the active pane's rectangle in egui logical points. The
/// transfer strip stays inside the bottom-right of this rectangle, independent
/// of the surrounding window layout. Idle panes render nothing: files are sent
/// from the file manager or drag-and-drop, matching the main client.
///
/// Returned actions contain only paths or transfer IDs. The caller attaches the
/// current tab/pane/session identity before forwarding them to the runtime.
pub fn render(
    ctx: &egui::Context,
    pane_id: &str,
    pane_rect: egui::Rect,
    state: &mut PaneZmodemView,
) -> Vec<ZmodemUiAction> {
    let mut actions = Vec::new();
    if !state.has_overlay() {
        return actions;
    }

    egui::Area::new(egui::Id::new(("zmodem_transfer_strip", pane_id)))
        .fixed_pos(pane_overlay_anchor(pane_rect))
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .constrain_to(pane_rect)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(PANEL_BG)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_max_width(transfer_strip_max_width(pane_rect));
                    let mut dismiss = None;
                    let mut cancel = None;
                    let mut dismiss_send_error = false;
                    if let Some(transfer) = &state.transfer {
                        ui.horizontal(|ui| {
                            ui.colored_label(CYAN, transfer.direction.icon());
                            ui.label(
                                egui::RichText::new(&transfer.filename)
                                    .color(TEXT)
                                    .size(11.0),
                            );
                            ui.label(
                                egui::RichText::new(transfer.direction.label())
                                    .color(MUTED)
                                    .size(10.0),
                            );
                            ui.add(
                                egui::ProgressBar::new(transfer.progress_fraction())
                                    .desired_width(150.0)
                                    .fill(CYAN),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} / {}",
                                    format_bytes(transfer.transferred),
                                    format_bytes(transfer.total)
                                ))
                                .color(MUTED)
                                .size(10.0),
                            );
                            ui.colored_label(transfer.status.color(), transfer.status.label());

                            if transfer.status.is_active()
                                && transfer.status != TransferStatus::Cancelling
                            {
                                if ui.small_button("取消").clicked() {
                                    cancel = Some(transfer.transfer_id);
                                }
                            } else if !transfer.status.is_active()
                                && ui.small_button("关闭").clicked()
                            {
                                dismiss = Some(transfer.transfer_id);
                            }
                        });
                        if let TransferStatus::Failed(error) = &transfer.status {
                            ui.colored_label(RED, error);
                        }
                        if let Some(error) = &state.transfer_error {
                            ui.colored_label(RED, error);
                        }
                    }
                    if let Some(error) = &state.send_error {
                        ui.horizontal(|ui| {
                            ui.colored_label(RED, error);
                            if ui.small_button("关闭").clicked() {
                                dismiss_send_error = true;
                            }
                        });
                    }
                    if let Some(transfer_id) = cancel {
                        actions.push(ZmodemUiAction::Cancel { transfer_id });
                    }
                    if let Some(transfer_id) = dismiss {
                        actions.push(ZmodemUiAction::Dismiss { transfer_id });
                    }
                    if dismiss_send_error {
                        state.send_error = None;
                    }
                });
        });

    actions
}

#[cfg(test)]
mod tests {
    use super::{
        pane_overlay_anchor, transfer_strip_max_width, PaneZmodemView, TransferDirection,
        TransferStatus, TransferView,
    };

    #[test]
    fn progress_and_terminal_status_are_transfer_id_scoped() {
        let mut state = PaneZmodemView::default();
        state.set_transfer(TransferView {
            transfer_id: 7,
            direction: TransferDirection::Receive,
            filename: "image.bin".into(),
            transferred: 0,
            total: 100,
            status: TransferStatus::Preparing,
        });

        assert!(!state.update_progress(6, 90, 100));
        assert!(state.update_progress(7, 25, 100));
        let transfer = state.transfer.as_ref().unwrap();
        assert_eq!(transfer.transferred, 25);
        assert_eq!(transfer.progress_fraction(), 0.25);
        assert_eq!(state.active_transfer_id(), Some(7));

        assert!(state.set_status(7, TransferStatus::Completed));
        assert_eq!(state.active_transfer_id(), None);
        assert_eq!(
            state.transfer.as_ref().unwrap().progress_fraction(),
            0.25,
            "完成状态不能伪造未收到字节的进度"
        );
    }

    #[test]
    fn only_errors_and_transfers_create_a_terminal_overlay() {
        let mut state = PaneZmodemView::default();
        assert!(!state.has_overlay());

        state.show_send_error("发送失败");
        assert!(state.has_overlay());

        state.set_transfer(TransferView {
            transfer_id: 8,
            direction: TransferDirection::Send,
            filename: "archive.tar".into(),
            transferred: 0,
            total: 10,
            status: TransferStatus::Preparing,
        });
        assert!(state.has_overlay());

        assert!(state.set_status(8, TransferStatus::Failed("失败".into())));
        assert!(state.has_overlay());
        assert!(state.dismiss_transfer(8));
        assert!(!state.has_overlay());
    }

    #[test]
    fn transfer_strip_uses_the_supplied_pane_bottom_right() {
        let pane_rect =
            egui::Rect::from_min_max(egui::pos2(100.0, 200.0), egui::pos2(500.0, 600.0));

        assert_eq!(pane_overlay_anchor(pane_rect), egui::pos2(488.0, 588.0));
        assert_eq!(transfer_strip_max_width(pane_rect), 376.0);
    }

    #[test]
    fn transfer_strip_anchor_stays_inside_a_tiny_pane() {
        let pane_rect = egui::Rect::from_min_max(egui::pos2(40.0, 60.0), egui::pos2(50.0, 66.0));
        let anchor = pane_overlay_anchor(pane_rect);

        assert!(pane_rect.contains(anchor));
        assert_eq!(anchor, pane_rect.center());
        assert_eq!(transfer_strip_max_width(pane_rect), 1.0);
    }
}
