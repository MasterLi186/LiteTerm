use crate::{
    monitor, process_manager, remote_monitor, sidebar::Sidebar, ssh, tab_manager::TabManager,
};
use std::collections::{HashMap, HashSet};

use super::UserEvent;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RemoteMonitorReconcileActions {
    pub(super) starts: Vec<(monitor::MonitorKey, ssh::ConnectionParams)>,
    pub(super) stops: Vec<monitor::MonitorKey>,
}

fn monitor_key_sort_key(key: &monitor::MonitorKey) -> (&str, &str, u16) {
    match key {
        monitor::MonitorKey::Local => ("", "", 0),
        monitor::MonitorKey::Remote { user, host, port } => (user, host, *port),
    }
}

pub(super) fn reconcile_actions(
    required: &HashMap<monitor::MonitorKey, ssh::ConnectionParams>,
    running: &HashMap<monitor::MonitorKey, ssh::ConnectionParams>,
) -> RemoteMonitorReconcileActions {
    let mut starts = required
        .iter()
        .filter(|(key, params)| running.get(*key) != Some(*params))
        .map(|(key, params)| (key.clone(), params.clone()))
        .collect::<Vec<_>>();
    starts.sort_by(|(left, _), (right, _)| {
        monitor_key_sort_key(left).cmp(&monitor_key_sort_key(right))
    });

    let mut stops = running
        .iter()
        .filter(|(key, params)| required.get(*key) != Some(*params))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    stops.sort_by(|left, right| monitor_key_sort_key(left).cmp(&monitor_key_sort_key(right)));

    RemoteMonitorReconcileActions { starts, stops }
}

pub(super) fn next_remote_monitor_generation(counter: &mut u64) -> u64 {
    *counter = counter.wrapping_add(1).max(1);
    *counter
}

pub(super) fn user_event_from_remote(event: remote_monitor::RemoteMonitorEvent) -> UserEvent {
    match event {
        remote_monitor::RemoteMonitorEvent::Update {
            key,
            generation,
            data,
        } => UserEvent::Monitor(monitor::MonitorEvent {
            key,
            generation,
            result: Ok(data),
        }),
        remote_monitor::RemoteMonitorEvent::Failed {
            key,
            generation,
            error,
        } => UserEvent::Monitor(monitor::MonitorEvent {
            key,
            generation,
            result: Err(error),
        }),
        remote_monitor::RemoteMonitorEvent::ProcessDetail {
            key,
            generation,
            requester,
            request_id,
            result,
        } => UserEvent::ProcessDetail {
            key,
            generation,
            requester,
            request_id,
            result,
        },
        remote_monitor::RemoteMonitorEvent::NetworkDetail {
            key,
            generation,
            requester,
            request_id,
            result,
        } => UserEvent::NetworkDetail {
            key,
            generation,
            requester,
            request_id,
            result,
        },
    }
}

pub(super) fn monitor_event_is_current(
    key: &monitor::MonitorKey,
    generation: u64,
    remote_generations: &HashMap<monitor::MonitorKey, u64>,
) -> bool {
    match key {
        monitor::MonitorKey::Local => generation == 0,
        monitor::MonitorKey::Remote { .. } => remote_generations.get(key) == Some(&generation),
    }
}

fn safe_monitor_error(error: String) -> String {
    const PREFIX: &str = "监控更新失败：";
    const MAX_CHARS: usize = 160;
    let detail = error
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_CHARS - PREFIX.chars().count())
        .collect::<String>();
    if detail.is_empty() {
        "监控更新失败".into()
    } else {
        format!("{PREFIX}{detail}")
    }
}

pub(super) fn apply_monitor_event(
    slots: &mut HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    event: monitor::MonitorEvent,
    remote_generations: &HashMap<monitor::MonitorKey, u64>,
) -> bool {
    if !monitor_event_is_current(&event.key, event.generation, remote_generations) {
        return false;
    }

    let slot = slots.entry(event.key).or_default();
    match event.result {
        Ok(data) => {
            slot.data = Some(*data);
            slot.error = None;
        }
        Err(error) => slot.error = Some(safe_monitor_error(error)),
    }
    true
}

pub(super) fn apply_monitor_event_and_update_sidebar(
    slots: &mut HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    sidebar: &mut Sidebar,
    event: monitor::MonitorEvent,
    remote_generations: &HashMap<monitor::MonitorKey, u64>,
) -> bool {
    let key = event.key.clone();
    let has_snapshot = event.result.is_ok();
    if !apply_monitor_event(slots, event, remote_generations) {
        return false;
    }
    if has_snapshot {
        if let Some(data) = slots.get(&key).and_then(|slot| slot.data.as_ref()) {
            sidebar.on_monitor_update(&key, data);
        }
    }
    true
}

pub(super) fn active_monitor_slot<'a>(
    slots: &'a HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    active_key: &monitor::MonitorKey,
) -> Option<&'a monitor::MonitorSlot> {
    slots.get(active_key)
}

pub(super) fn active_monitor_snapshot<'a>(
    slots: &'a HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    active_key: &monitor::MonitorKey,
) -> Option<&'a monitor::MonitorData> {
    active_monitor_slot(slots, active_key).and_then(|slot| slot.data.as_ref())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_process_detail_event(
    process_managers: &mut HashMap<String, process_manager::ProcessManagerState>,
    remote_generations: &HashMap<monitor::MonitorKey, u64>,
    key: &monitor::MonitorKey,
    generation: u64,
    requester: &str,
    request_id: u64,
    result: Result<Box<monitor::ProcessDetail>, String>,
) -> bool {
    if !monitor_event_is_current(key, generation, remote_generations) {
        return false;
    }
    let Some(state) = process_managers.get_mut(requester) else {
        return false;
    };
    if state.target() != key {
        return false;
    }
    state.apply_detail(request_id, result.map(|detail| *detail))
}

pub(super) fn remove_monitor_slots(
    slots: &mut HashMap<monitor::MonitorKey, monitor::MonitorSlot>,
    keys: &[monitor::MonitorKey],
) {
    for key in keys {
        slots.remove(key);
    }
}

fn monitor_keys_in_tabs(tab_manager: &TabManager) -> HashSet<monitor::MonitorKey> {
    tab_manager
        .tabs
        .iter()
        .map(|tab| tab.monitor_key())
        .collect()
}

pub(super) fn prune_sidebar_monitor_views(sidebar: &mut Sidebar, tab_manager: &TabManager) {
    sidebar.retain_monitor_views(&monitor_keys_in_tabs(tab_manager));
}
