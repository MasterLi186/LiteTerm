use crate::{app_monitor, file_browser, monitor, sftp, ssh, tab_manager::TabManager};
use std::collections::HashMap;

pub(super) fn assert_tab_scoped_resource_invariant(
    browsers: &HashMap<String, file_browser::FileBrowserState>,
    workers: &HashMap<String, sftp::SftpHandle>,
) {
    debug_assert!(workers.iter().all(|(pane_id, worker)| {
        pane_id == worker.pane_id() && browsers.contains_key(worker.tab_id())
    }));
}

pub(super) fn install_tab_scoped_resources(
    browsers: &mut HashMap<String, file_browser::FileBrowserState>,
    workers: &mut HashMap<String, sftp::SftpHandle>,
    tab_id: String,
    browser: file_browser::FileBrowserState,
    worker: sftp::SftpHandle,
) {
    assert_tab_scoped_resource_invariant(browsers, workers);
    browsers.insert(tab_id, browser);
    let pane_id = worker.pane_id().to_string();
    let replaced_worker = workers.insert(pane_id, worker);
    if let Some(replaced_worker) = replaced_worker {
        let _ = replaced_worker.send(sftp::SftpCommand::Shutdown);
    }
    assert_tab_scoped_resource_invariant(browsers, workers);
}

pub(super) fn shutdown_and_remove_tab_scoped_resources(
    browsers: &mut HashMap<String, file_browser::FileBrowserState>,
    workers: &mut HashMap<String, sftp::SftpHandle>,
    tab_id: &str,
) {
    assert_tab_scoped_resource_invariant(browsers, workers);
    browsers.remove(tab_id);
    let pane_ids = workers
        .iter()
        .filter(|(_, worker)| worker.tab_id() == tab_id)
        .map(|(pane_id, _)| pane_id.clone())
        .collect::<Vec<_>>();
    for pane_id in pane_ids {
        if let Some(worker) = workers.remove(&pane_id) {
            let _ = worker.send(sftp::SftpCommand::Shutdown);
        }
    }
    assert_tab_scoped_resource_invariant(browsers, workers);
}

pub(super) fn shutdown_and_remove_pane_worker(
    workers: &mut HashMap<String, sftp::SftpHandle>,
    pane_id: &str,
) {
    if let Some(worker) = workers.remove(pane_id) {
        let _ = worker.send(sftp::SftpCommand::Shutdown);
    }
}

pub(super) fn close_tab_scoped_resources_and_plan(
    tab_manager: &mut TabManager,
    browsers: &mut HashMap<String, file_browser::FileBrowserState>,
    workers: &mut HashMap<String, sftp::SftpHandle>,
    running_monitors: &HashMap<monitor::MonitorKey, ssh::ConnectionParams>,
    index: usize,
) -> Option<app_monitor::RemoteMonitorReconcileActions> {
    let tab_id = tab_manager.tabs.get(index)?.id.clone();
    tab_manager.close(index);
    shutdown_and_remove_tab_scoped_resources(browsers, workers, &tab_id);
    Some(app_monitor::reconcile_actions(
        &tab_manager.remote_monitor_requirements(),
        running_monitors,
    ))
}
