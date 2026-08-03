use crate::{smart_completion::CompletionSessionKey, tab_manager::TabManager, zmodem};
use std::{collections::HashMap, sync::mpsc};

#[derive(Clone)]
pub(super) struct ZmodemControlSlot {
    pub(super) tab_id: String,
    pub(super) pane_id: String,
    pub(super) session: CompletionSessionKey,
    pub(super) commands: zmodem::runtime::RuntimeCommandSender,
    /// A manual send request accepted by the bounded runtime queue but not yet
    /// confirmed by `RuntimeEventKind::Started`.
    pub(super) pending_send: Option<u64>,
    pub(super) capability: zmodem::runtime::RuntimeCapability,
    pub(super) unavailable_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ZmodemControlSendError {
    Full,
    Disconnected,
}

pub(super) fn allocate_zmodem_transfer_id(next: &mut Option<u64>) -> Result<u64, &'static str> {
    let transfer_id = (*next).ok_or("ZMODEM 传输编号已耗尽，请重启应用")?;
    *next = transfer_id.checked_add(1);
    Ok(transfer_id)
}

pub(super) fn observe_zmodem_transfer_id(next: &mut Option<u64>, transfer_id: u64) {
    if next.is_some_and(|next_id| transfer_id >= next_id) {
        *next = transfer_id.checked_add(1);
    }
}

impl ZmodemControlSendError {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Full => "ZMODEM 控制队列已满，请稍后重试",
            Self::Disconnected => "ZMODEM 控制通道已断开，请重新连接",
        }
    }
}

pub(super) fn try_send_zmodem_command(
    commands: &zmodem::runtime::RuntimeCommandSender,
    command: zmodem::runtime::RuntimeCommand,
) -> Result<(), ZmodemControlSendError> {
    commands.try_send(command).map_err(|error| match error {
        mpsc::TrySendError::Full(_) => ZmodemControlSendError::Full,
        mpsc::TrySendError::Disconnected(_) => ZmodemControlSendError::Disconnected,
    })
}

pub(super) fn request_zmodem_send(
    commands: &zmodem::runtime::RuntimeCommandSender,
    pending_send: &mut Option<u64>,
    view: &mut zmodem::ui::PaneZmodemView,
    next_transfer_id: &mut Option<u64>,
    generation: u64,
    paths: Vec<std::path::PathBuf>,
) -> bool {
    if pending_send.is_some() {
        view.show_send_error("已有 ZMODEM 发送请求正在等待确认");
        return false;
    }
    if view.active_transfer_id().is_some() {
        view.show_send_error("已有 ZMODEM 传输正在进行");
        return false;
    }
    let transfer_id = match allocate_zmodem_transfer_id(next_transfer_id) {
        Ok(transfer_id) => transfer_id,
        Err(error) => {
            view.show_send_error(error);
            return false;
        }
    };
    let command = zmodem::runtime::RuntimeCommand::StartSend {
        identity: zmodem::runtime::TransferIdentity {
            transfer_id,
            generation,
        },
        paths,
    };
    match try_send_zmodem_command(commands, command) {
        Ok(()) => {
            *pending_send = Some(transfer_id);
            true
        }
        Err(error) => {
            view.show_send_error(error.message());
            false
        }
    }
}

pub(super) fn shutdown_zmodem_slot(slot: &ZmodemControlSlot) {
    let _ = try_send_zmodem_command(&slot.commands, zmodem::runtime::RuntimeCommand::Shutdown);
}

pub(super) fn replace_zmodem_slot(
    slots: &mut HashMap<String, ZmodemControlSlot>,
    views: &mut HashMap<String, zmodem::ui::PaneZmodemView>,
    slot: ZmodemControlSlot,
) {
    views.remove(&slot.pane_id);
    if let Some(previous) = slots.insert(slot.pane_id.clone(), slot) {
        shutdown_zmodem_slot(&previous);
    }
}

pub(super) fn remove_zmodem_pane_resources(
    slots: &mut HashMap<String, ZmodemControlSlot>,
    views: &mut HashMap<String, zmodem::ui::PaneZmodemView>,
    pane_id: &str,
) {
    if let Some(slot) = slots.remove(pane_id) {
        shutdown_zmodem_slot(&slot);
    }
    views.remove(pane_id);
}

pub(super) fn remove_zmodem_tab_resources(
    slots: &mut HashMap<String, ZmodemControlSlot>,
    views: &mut HashMap<String, zmodem::ui::PaneZmodemView>,
    tab_id: &str,
) {
    let pane_ids = slots
        .values()
        .filter(|slot| slot.tab_id == tab_id)
        .map(|slot| slot.pane_id.clone())
        .collect::<Vec<_>>();
    for pane_id in pane_ids {
        remove_zmodem_pane_resources(slots, views, &pane_id);
    }
}

pub(super) fn zmodem_ui_capability(
    runtime_capability: zmodem::runtime::RuntimeCapability,
    settings_enabled: bool,
    control_ready: bool,
    unavailable_reason: Option<&str>,
) -> zmodem::ui::ZmodemCapability {
    if runtime_capability == zmodem::runtime::RuntimeCapability::SerialDisabled {
        return zmodem::ui::ZmodemCapability::disabled("串口终端不支持 ZMODEM");
    }
    if !settings_enabled {
        return zmodem::ui::ZmodemCapability::disabled("设置中已禁用 ZMODEM");
    }
    if let Some(reason) = unavailable_reason {
        return zmodem::ui::ZmodemCapability::disabled(reason);
    }
    if !control_ready {
        return zmodem::ui::ZmodemCapability::disabled("ZMODEM 控制通道尚未就绪");
    }
    zmodem::ui::ZmodemCapability::Enabled
}

pub(super) fn zmodem_runtime_event_kind_name(
    kind: &zmodem::runtime::RuntimeEventKind,
) -> &'static str {
    use zmodem::runtime::RuntimeEventKind;
    match kind {
        RuntimeEventKind::Started { .. } => "Started",
        RuntimeEventKind::Receiver(_) => "Receiver",
        RuntimeEventKind::Sender(_) => "Sender",
        RuntimeEventKind::Error(_) => "Error",
        RuntimeEventKind::StaleCommand => "StaleCommand",
        RuntimeEventKind::Finished => "Finished",
    }
}

pub(super) fn zmodem_event_identity_is_current(
    tab_manager: &TabManager,
    slots: &HashMap<String, ZmodemControlSlot>,
    tab_id: &str,
    pane_id: &str,
    session: &CompletionSessionKey,
    runtime_generation: u64,
) -> bool {
    let tab_index = tab_manager.find_by_id(tab_id);
    let pane = tab_index.and_then(|index| tab_manager.tabs[index].pane(pane_id));
    zmodem_identity_components_are_current(
        tab_index.is_some(),
        pane.is_some(),
        pane.map(|pane| pane.completion.session()),
        slots.get(pane_id),
        tab_id,
        pane_id,
        session,
        runtime_generation,
    )
}

pub(super) fn zmodem_identity_components_are_current(
    tab_exists: bool,
    pane_belongs_to_tab: bool,
    current_session: Option<&CompletionSessionKey>,
    slot: Option<&ZmodemControlSlot>,
    tab_id: &str,
    pane_id: &str,
    event_session: &CompletionSessionKey,
    runtime_generation: u64,
) -> bool {
    tab_exists
        && pane_belongs_to_tab
        && current_session == Some(event_session)
        && runtime_generation == event_session.generation
        && slot.is_some_and(|slot| {
            slot.tab_id == tab_id
                && slot.pane_id == pane_id
                && slot.session == *event_session
                && slot.session.generation == runtime_generation
        })
}

pub(super) fn update_zmodem_progress(
    view: &mut zmodem::ui::PaneZmodemView,
    transfer_id: u64,
    filename: String,
    transferred: u64,
    total: u64,
) -> bool {
    let Some(transfer) = view
        .transfer
        .as_mut()
        .filter(|transfer| transfer.transfer_id == transfer_id)
    else {
        return false;
    };
    transfer.filename = filename;
    transfer.transferred = transferred;
    transfer.total = total;
    transfer.status = zmodem::ui::TransferStatus::Transferring;
    true
}

pub(super) fn set_or_create_zmodem_error(
    view: &mut zmodem::ui::PaneZmodemView,
    transfer_id: u64,
    error: String,
) -> bool {
    if view.set_status(
        transfer_id,
        zmodem::ui::TransferStatus::Failed(error.clone()),
    ) {
        return true;
    }
    if view
        .transfer
        .as_ref()
        .is_some_and(|transfer| transfer.transfer_id > transfer_id)
    {
        return false;
    }
    view.set_transfer(zmodem::ui::TransferView {
        transfer_id,
        direction: zmodem::ui::TransferDirection::Receive,
        filename: "ZMODEM 传输".into(),
        transferred: 0,
        total: 0,
        status: zmodem::ui::TransferStatus::Failed(error),
    });
    true
}

#[cfg(test)]
pub(super) fn apply_zmodem_runtime_event(
    view: &mut zmodem::ui::PaneZmodemView,
    event: zmodem::runtime::RuntimeEvent,
) -> bool {
    apply_zmodem_runtime_event_arbitrated(view, &mut None, event)
}

pub(super) fn apply_zmodem_runtime_event_arbitrated(
    view: &mut zmodem::ui::PaneZmodemView,
    pending_send: &mut Option<u64>,
    event: zmodem::runtime::RuntimeEvent,
) -> bool {
    use zmodem::receiver::ReceiverEvent;
    use zmodem::runtime::{RuntimeEventKind, TransferDirection};
    use zmodem::sender::SenderAction;
    use zmodem::ui::{TransferStatus, TransferView};

    let transfer_id = event.identity.transfer_id;
    match event.kind {
        RuntimeEventKind::Started {
            direction,
            filename,
            total,
        } => {
            if direction == TransferDirection::Send && *pending_send == Some(transfer_id) {
                *pending_send = None;
            }
            let direction = match direction {
                TransferDirection::Send => zmodem::ui::TransferDirection::Send,
                TransferDirection::Receive => zmodem::ui::TransferDirection::Receive,
            };
            let fallback = match direction {
                zmodem::ui::TransferDirection::Send => "准备发送文件…",
                zmodem::ui::TransferDirection::Receive => "等待接收文件…",
            };
            view.set_transfer(TransferView {
                transfer_id,
                direction,
                filename: filename.unwrap_or_else(|| fallback.into()),
                transferred: 0,
                total: total.unwrap_or(0),
                status: TransferStatus::Transferring,
            });
            true
        }
        RuntimeEventKind::Receiver(receiver) => match receiver {
            ReceiverEvent::Progress {
                bytes_received,
                total,
                filename,
            } => update_zmodem_progress(view, transfer_id, filename, bytes_received, total),
            ReceiverEvent::FileComplete { path, size } => {
                let Some(transfer) = view
                    .transfer
                    .as_mut()
                    .filter(|transfer| transfer.transfer_id == transfer_id)
                else {
                    return false;
                };
                transfer.filename = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "接收文件".into());
                transfer.transferred = size;
                transfer.total = size;
                transfer.status = TransferStatus::Transferring;
                true
            }
            ReceiverEvent::SessionComplete => {
                view.set_status(transfer_id, TransferStatus::Completed)
            }
            ReceiverEvent::Error(error) => {
                view.set_status(transfer_id, TransferStatus::Failed(error.to_string()))
            }
            ReceiverEvent::Cancelled => view.set_status(transfer_id, TransferStatus::Cancelled),
        },
        RuntimeEventKind::Sender(sender) => match sender {
            SenderAction::Progress {
                bytes_sent,
                total,
                filename,
            } => update_zmodem_progress(view, transfer_id, filename, bytes_sent, total),
            SenderAction::FileComplete(filename) => {
                let Some(transfer) = view
                    .transfer
                    .as_mut()
                    .filter(|transfer| transfer.transfer_id == transfer_id)
                else {
                    return false;
                };
                transfer.filename = filename;
                transfer.transferred = transfer.total;
                transfer.status = TransferStatus::Transferring;
                true
            }
            SenderAction::AllComplete => view.set_status(transfer_id, TransferStatus::Completed),
            SenderAction::Error(error) => {
                view.set_status(transfer_id, TransferStatus::Failed(error.to_string()))
            }
            SenderAction::Send(_) | SenderAction::None => view
                .transfer
                .as_ref()
                .is_some_and(|transfer| transfer.transfer_id == transfer_id),
        },
        RuntimeEventKind::Error(error) => {
            if *pending_send == Some(transfer_id) {
                *pending_send = None;
                if view.active_transfer_id().is_some() {
                    view.show_transfer_error(error.to_string());
                } else {
                    view.show_send_error(error.to_string());
                }
                return true;
            }
            set_or_create_zmodem_error(view, transfer_id, error.to_string())
        }
        RuntimeEventKind::StaleCommand if *pending_send == Some(transfer_id) => {
            *pending_send = None;
            if view.active_transfer_id().is_some() {
                view.show_transfer_error("ZMODEM 请求已过期");
            } else {
                view.show_send_error("ZMODEM 请求已过期");
            }
            true
        }
        RuntimeEventKind::StaleCommand => view.set_status(
            transfer_id,
            TransferStatus::Failed("ZMODEM 请求已过期".into()),
        ),
        RuntimeEventKind::Finished => {
            let Some(transfer) = view
                .transfer
                .as_mut()
                .filter(|transfer| transfer.transfer_id == transfer_id)
            else {
                return false;
            };
            if transfer.status.is_active() {
                transfer.status = TransferStatus::Completed;
            }
            true
        }
    }
}
