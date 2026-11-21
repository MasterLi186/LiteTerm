use std::collections::HashSet;
use std::path::PathBuf;

const OVERLAY_INSET: f32 = 2.0;
const OVERLAY_CORNER_RADIUS: f32 = 4.0;
const DASH_LENGTH: f32 = 8.0;
const DASH_GAP: f32 = 6.0;
const OVERLAY_LABEL: &str = "释放文件上传到远程服务器";

/// Immutable identity captured when a native file drop starts.
///
/// The session token is intentionally part of equality: a reconnect can reuse
/// the same tab and pane IDs, but must not receive a batch dropped onto the old
/// connection. `remote_directory` is also captured so one OS drop batch cannot
/// be split across two browser destinations.
#[derive(Clone, PartialEq, Eq)]
pub struct DropTarget {
    pub tab_id: String,
    pub pane_id: String,
    pub session_generation: u64,
    pub session_token: String,
    pub remote_directory: Option<String>,
}

impl std::fmt::Debug for DropTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DropTarget")
            .field("tab_id", &self.tab_id)
            .field("pane_id", &self.pane_id)
            .field("session_generation", &self.session_generation)
            .field("session_token", &"<redacted>")
            .field(
                "remote_directory",
                &self.remote_directory.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropBatch {
    pub target: DropTarget,
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushDropOutcome {
    StartedBatch,
    AppendedToBatch,
    RejectedDifferentTarget,
}

/// Native drag/drop state owned by the event-loop thread.
///
/// Winit emits one `DroppedFile` per path. Callers append all events until the
/// next `about_to_wait`, then take and dispatch the batch exactly once.
#[derive(Clone, Default, Debug)]
pub struct DragUploadState {
    hovered_target: Option<DropTarget>,
    pending_batch: Option<DropBatch>,
}

impl DragUploadState {
    pub fn hover(&mut self, target: DropTarget) -> bool {
        if self.hovered_target.as_ref() == Some(&target) {
            return false;
        }
        self.hovered_target = Some(target);
        true
    }

    /// Clear hover presentation only. A cancellation event that follows one or
    /// more `DroppedFile` events must never discard the pending upload batch.
    pub fn cancel_hover(&mut self) -> bool {
        self.hovered_target.take().is_some()
    }

    pub fn hovered_target(&self) -> Option<&DropTarget> {
        self.hovered_target.as_ref()
    }

    /// The destination bound to the current OS drag gesture. Once the first
    /// path is dropped, subsequent `DroppedFile` events keep using that same
    /// destination even if the application switches tabs between events.
    pub fn drop_target(&self) -> Option<&DropTarget> {
        self.pending_batch
            .as_ref()
            .map(|batch| &batch.target)
            .or(self.hovered_target.as_ref())
    }

    pub fn push_drop(&mut self, target: DropTarget, path: PathBuf) -> PushDropOutcome {
        self.hovered_target = None;
        match &mut self.pending_batch {
            None => {
                self.pending_batch = Some(DropBatch {
                    target,
                    paths: vec![path],
                });
                PushDropOutcome::StartedBatch
            }
            Some(batch) if batch.target == target => {
                batch.paths.push(path);
                PushDropOutcome::AppendedToBatch
            }
            Some(_) => PushDropOutcome::RejectedDifferentTarget,
        }
    }

    pub fn pending_batch(&self) -> Option<&DropBatch> {
        self.pending_batch.as_ref()
    }

    pub fn take_batch(&mut self) -> Option<DropBatch> {
        self.pending_batch.take()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportUnavailable {
    SshNotConnected,
    NoReadyTransport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportChoice {
    Sftp,
    Zmodem,
    Unavailable(TransportUnavailable),
}

/// Choose the upload transport without consulting application state.
///
/// SFTP is usable only when the worker belongs to the captured pane/session and
/// that pane has observed `SftpEvent::Ready`. ZMODEM is the whole-batch fallback;
/// a later SFTP transfer failure must not be routed through this function again.
pub fn choose_transport(
    ssh_connected: bool,
    sftp_worker_matches: bool,
    sftp_ready: bool,
    zmodem_ready: bool,
) -> TransportChoice {
    if !ssh_connected {
        return TransportChoice::Unavailable(TransportUnavailable::SshNotConnected);
    }
    if sftp_worker_matches && sftp_ready {
        return TransportChoice::Sftp;
    }
    if zmodem_ready {
        return TransportChoice::Zmodem;
    }
    TransportChoice::Unavailable(TransportUnavailable::NoReadyTransport)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SftpUploadJob {
    pub local_path: PathBuf,
    pub original_name: String,
    pub remote_name: String,
    pub remote_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SftpPlanError {
    pub index: usize,
    pub path: PathBuf,
}

impl std::fmt::Display for SftpPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "第 {} 个拖入路径没有可用的文件名: {}",
            self.index + 1,
            self.path.display()
        )
    }
}

impl std::error::Error for SftpPlanError {}

/// Build one worker command per dropped path.
///
/// Name collisions are resolved within the OS drop batch using the same
/// `stem(1).ext` convention as the Tauri implementation. The local path remains
/// untouched; only the remote basename is changed.
pub fn plan_sftp_jobs(
    paths: &[PathBuf],
    remote_directory: &str,
) -> Result<Vec<SftpUploadJob>, SftpPlanError> {
    let mut used_names = HashSet::new();
    paths
        .iter()
        .enumerate()
        .map(|(index, local_path)| {
            let original_name = local_path
                .file_name()
                .filter(|name| !name.is_empty())
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| SftpPlanError {
                    index,
                    path: local_path.clone(),
                })?;
            let remote_name = deduplicate_name(&original_name, &mut used_names);
            let remote_path = join_remote_path(remote_directory, &remote_name);
            Ok(SftpUploadJob {
                local_path: local_path.clone(),
                original_name,
                remote_name,
                remote_path,
            })
        })
        .collect()
}

fn deduplicate_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    // Hidden files such as `.bashrc` have no extension for this purpose.
    let (stem, extension) = match base.rfind('.') {
        Some(index) if index > 0 => (&base[..index], &base[index..]),
        _ => (base, ""),
    };
    let mut suffix = 1_u64;
    loop {
        let candidate = format!("{stem}({suffix}){extension}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

/// Equivalent to `sftp::join_path`, kept local so this pure planning module can
/// be tested without constructing an SFTP worker.
fn join_remote_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", base.trim_end_matches('/'))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayGeometry {
    pub rect: egui::Rect,
    pub label_position: egui::Pos2,
}

/// Return overlay geometry only while the current active pane is exactly the
/// pane/session/destination that accepted the hover.
pub fn active_pane_overlay_geometry(
    state: &DragUploadState,
    current_target: &DropTarget,
    active_pane_rect: egui::Rect,
) -> Option<OverlayGeometry> {
    if state.hovered_target() != Some(current_target)
        || active_pane_rect.width() <= OVERLAY_INSET * 2.0
        || active_pane_rect.height() <= OVERLAY_INSET * 2.0
    {
        return None;
    }
    let rect = active_pane_rect.shrink(OVERLAY_INSET);
    Some(OverlayGeometry {
        rect,
        label_position: rect.center(),
    })
}

/// Paint the native equivalent of the Tauri drag-upload overlay.
///
/// Returns `true` when an overlay was painted, which also makes the helper easy
/// to exercise from integration tests without inspecting renderer internals.
pub fn render_active_pane_overlay(
    ctx: &egui::Context,
    state: &DragUploadState,
    current_target: &DropTarget,
    active_pane_rect: egui::Rect,
) -> bool {
    let Some(geometry) = active_pane_overlay_geometry(state, current_target, active_pane_rect)
    else {
        return false;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("drag_upload_overlay"),
    ));
    let cyan = egui::Color32::from_rgb(0x00, 0xd4, 0xff);
    painter.rect_filled(
        geometry.rect,
        OVERLAY_CORNER_RADIUS,
        egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 20),
    );
    paint_dashed_rect(&painter, geometry.rect, egui::Stroke::new(2.0, cyan));
    painter.text(
        geometry.label_position,
        egui::Align2::CENTER_CENTER,
        OVERLAY_LABEL,
        egui::FontId::proportional(14.0),
        cyan,
    );
    true
}

fn paint_dashed_rect(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    for (start, end) in [
        (rect.left_top(), rect.right_top()),
        (rect.right_top(), rect.right_bottom()),
        (rect.right_bottom(), rect.left_bottom()),
        (rect.left_bottom(), rect.left_top()),
    ] {
        paint_dashed_line(painter, start, end, stroke);
    }
}

fn paint_dashed_line(
    painter: &egui::Painter,
    start: egui::Pos2,
    end: egui::Pos2,
    stroke: egui::Stroke,
) {
    let vector = end - start;
    let length = vector.length();
    if length <= f32::EPSILON {
        return;
    }
    let direction = vector / length;
    let mut offset = 0.0;
    while offset < length {
        let dash_end = (offset + DASH_LENGTH).min(length);
        painter.line_segment(
            [start + direction * offset, start + direction * dash_end],
            stroke,
        );
        offset += DASH_LENGTH + DASH_GAP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn target(pane: &str, generation: u64, remote_directory: &str) -> DropTarget {
        DropTarget {
            tab_id: "tab-a".into(),
            pane_id: pane.into(),
            session_generation: generation,
            session_token: format!("token-{generation}"),
            remote_directory: Some(remote_directory.into()),
        }
    }

    #[test]
    fn hover_and_cancel_do_not_touch_a_pending_drop() {
        let mut state = DragUploadState::default();
        let expected_target = target("pane-a", 1, "/srv");
        assert!(state.hover(expected_target.clone()));
        assert!(!state.hover(expected_target.clone()));
        assert_eq!(state.hovered_target(), Some(&expected_target));
        assert_eq!(state.drop_target(), Some(&expected_target));

        assert_eq!(
            state.push_drop(expected_target.clone(), "/tmp/a".into()),
            PushDropOutcome::StartedBatch
        );
        assert_eq!(state.hovered_target(), None);
        assert_eq!(state.drop_target(), Some(&expected_target));
        assert!(!state.cancel_hover());
        assert_eq!(
            state.pending_batch().unwrap().paths,
            vec![PathBuf::from("/tmp/a")]
        );

        let later_hover = target("pane-b", 2, "/other");
        assert!(state.hover(later_hover));
        assert_eq!(state.drop_target(), Some(&expected_target));
    }

    #[test]
    fn dropped_files_coalesce_until_take_and_preserve_order_and_duplicates() {
        let mut state = DragUploadState::default();
        let target = target("pane-a", 1, "/srv");
        assert_eq!(
            state.push_drop(target.clone(), "/tmp/a".into()),
            PushDropOutcome::StartedBatch
        );
        assert_eq!(
            state.push_drop(target.clone(), "/tmp/b".into()),
            PushDropOutcome::AppendedToBatch
        );
        assert_eq!(
            state.push_drop(target.clone(), "/tmp/a".into()),
            PushDropOutcome::AppendedToBatch
        );

        let batch = state.take_batch().unwrap();
        assert_eq!(batch.target, target);
        assert_eq!(
            batch.paths,
            vec![
                PathBuf::from("/tmp/a"),
                PathBuf::from("/tmp/b"),
                PathBuf::from("/tmp/a")
            ]
        );
        assert!(state.take_batch().is_none());
    }

    #[test]
    fn a_different_target_is_rejected_instead_of_cross_routed() {
        let mut state = DragUploadState::default();
        let first = target("pane-a", 1, "/srv/a");
        let changed_pane = target("pane-b", 1, "/srv/a");
        let reconnected = target("pane-a", 2, "/srv/a");
        let changed_directory = target("pane-a", 1, "/srv/b");
        state.push_drop(first.clone(), "/tmp/a".into());

        for incoming in [changed_pane, reconnected, changed_directory] {
            assert_eq!(
                state.push_drop(incoming, "/tmp/rejected".into()),
                PushDropOutcome::RejectedDifferentTarget
            );
        }
        let batch = state.take_batch().unwrap();
        assert_eq!(batch.target, first);
        assert_eq!(batch.paths, vec![PathBuf::from("/tmp/a")]);
    }

    #[test]
    fn a_later_about_to_wait_batch_can_use_a_new_target() {
        let mut state = DragUploadState::default();
        let first = target("pane-a", 1, "/srv/a");
        let second = target("pane-b", 2, "/srv/b");
        state.push_drop(first.clone(), "/tmp/a".into());
        assert_eq!(state.take_batch().unwrap().target, first);
        assert_eq!(
            state.push_drop(second.clone(), "/tmp/b".into()),
            PushDropOutcome::StartedBatch
        );
        assert_eq!(state.take_batch().unwrap().target, second);
    }

    #[test]
    fn transport_prefers_ready_matching_sftp_then_falls_back_to_zmodem() {
        assert_eq!(
            choose_transport(true, true, true, true),
            TransportChoice::Sftp
        );
        assert_eq!(
            choose_transport(true, true, false, true),
            TransportChoice::Zmodem
        );
        assert_eq!(
            choose_transport(true, false, true, true),
            TransportChoice::Zmodem
        );
        assert_eq!(
            choose_transport(false, true, true, true),
            TransportChoice::Unavailable(TransportUnavailable::SshNotConnected)
        );
        assert_eq!(
            choose_transport(true, false, false, false),
            TransportChoice::Unavailable(TransportUnavailable::NoReadyTransport)
        );
    }

    #[test]
    fn sftp_plan_deduplicates_extensions_hidden_files_and_existing_candidates() {
        let jobs = plan_sftp_jobs(
            &[
                "/one/report.txt".into(),
                "/two/report.txt".into(),
                "/three/report(1).txt".into(),
                "/four/report.txt".into(),
                "/one/.bashrc".into(),
                "/two/.bashrc".into(),
            ],
            "/srv/upload",
        )
        .unwrap();
        assert_eq!(
            jobs.iter()
                .map(|job| job.remote_name.as_str())
                .collect::<Vec<_>>(),
            [
                "report.txt",
                "report(1).txt",
                "report(1)(1).txt",
                "report(2).txt",
                ".bashrc",
                ".bashrc(1)"
            ]
        );
    }

    #[test]
    fn sftp_plan_preserves_unicode_and_joins_root_and_trailing_slashes() {
        let paths = vec![PathBuf::from("/tmp/报告.终端.txt")];
        let root = plan_sftp_jobs(&paths, "/").unwrap();
        assert_eq!(root[0].original_name, "报告.终端.txt");
        assert_eq!(root[0].remote_path, "/报告.终端.txt");

        let nested = plan_sftp_jobs(&paths, "/srv/upload///").unwrap();
        assert_eq!(nested[0].remote_path, "/srv/upload/报告.终端.txt");
    }

    #[test]
    fn sftp_plan_rejects_a_path_without_a_basename_atomically() {
        let error = plan_sftp_jobs(&["/tmp/ok".into(), Path::new("/").into()], "/srv").unwrap_err();
        assert_eq!(error.index, 1);
        assert_eq!(error.path, PathBuf::from("/"));
        assert!(plan_sftp_jobs(&[], "/srv").unwrap().is_empty());
    }

    #[test]
    fn overlay_geometry_is_limited_to_the_matching_active_pane() {
        let mut state = DragUploadState::default();
        let hovered = target("pane-a", 1, "/srv");
        state.hover(hovered.clone());
        let pane_rect = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(500.0, 350.0));

        let geometry = active_pane_overlay_geometry(&state, &hovered, pane_rect).unwrap();
        assert_eq!(geometry.rect, pane_rect.shrink(OVERLAY_INSET));
        assert_eq!(geometry.label_position, geometry.rect.center());
        assert!(
            active_pane_overlay_geometry(&state, &target("pane-b", 1, "/srv"), pane_rect).is_none()
        );
        assert!(active_pane_overlay_geometry(
            &state,
            &hovered,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(4.0, 4.0))
        )
        .is_none());
    }

    #[test]
    fn drop_target_debug_redacts_session_and_remote_directory() {
        let debug = format!("{:?}", target("pane-a", 9, "/secret/remote"));
        assert!(!debug.contains("token-9"));
        assert!(!debug.contains("/secret/remote"));
    }
}
