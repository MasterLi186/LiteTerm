use super::*;

pub fn start_worker_for_pane(
    tab_id: String,
    pane_id: String,
    session: CompletionSessionKey,
    params: crate::ssh::ConnectionParams,
    proxy: EventLoopProxy<crate::UserEvent>,
) -> SftpHandle {
    let (tx, rx) = mpsc::channel();
    let worker_id = new_worker_id();
    let handle_tab_id = tab_id.clone();
    let handle_pane_id = pane_id.clone();
    let handle_session = session.clone();
    std::thread::spawn(move || {
        let events = WorkerEventSink {
            proxy: &proxy,
            worker_id,
            tab_id: &tab_id,
            pane_id: &pane_id,
            session: &session,
        };
        let mut connection = connect_sftp(&params);
        send_connection_result(&events, &tab_id, &connection);

        while let Ok(command) = rx.recv() {
            match command {
                SftpCommand::Shutdown => break,
                SftpCommand::Reconnect => {
                    connection = connect_sftp(&params);
                    send_connection_result(&events, &tab_id, &connection);
                }
                SftpCommand::ListLocal { request_id, path } => {
                    let result = list_local_dir(&path);
                    events.send(SftpEvent::Listed {
                        tab_id: tab_id.clone(),
                        request_id,
                        side: FileSide::Local,
                        path,
                        result,
                    });
                }
                SftpCommand::ListRemote { request_id, path } => {
                    let result = connection
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|(_, sftp, _)| list_remote_dir(sftp, &path));
                    events.send(SftpEvent::Listed {
                        tab_id: tab_id.clone(),
                        request_id,
                        side: FileSide::Remote,
                        path,
                        result,
                    });
                }
                SftpCommand::Upload {
                    transfer_id,
                    local_path,
                    remote_path,
                } => {
                    let result =
                        connection
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|(_, sftp, _)| {
                                upload_local_path(
                                    sftp,
                                    &events,
                                    &tab_id,
                                    &transfer_id,
                                    &local_path,
                                    &remote_path,
                                )
                            });
                    events.send(SftpEvent::TransferFinished {
                        tab_id: tab_id.clone(),
                        transfer_id,
                        direction: TransferDirection::Upload,
                        result,
                    });
                }
                SftpCommand::UploadBatch { uploads } => {
                    for upload in uploads {
                        let result =
                            connection
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|(_, sftp, _)| {
                                    upload_local_path(
                                        sftp,
                                        &events,
                                        &tab_id,
                                        &upload.transfer_id,
                                        &upload.local_path,
                                        &upload.remote_path,
                                    )
                                });
                        events.send(SftpEvent::TransferFinished {
                            tab_id: tab_id.clone(),
                            transfer_id: upload.transfer_id,
                            direction: TransferDirection::Upload,
                            result,
                        });
                    }
                }
                SftpCommand::Download {
                    transfer_id,
                    remote_path,
                    local_path,
                } => {
                    let result =
                        connection
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|(_, sftp, _)| {
                                transfer_download(
                                    sftp,
                                    &events,
                                    &tab_id,
                                    &transfer_id,
                                    &remote_path,
                                    &local_path,
                                )
                            });
                    events.send(SftpEvent::TransferFinished {
                        tab_id: tab_id.clone(),
                        transfer_id,
                        direction: TransferDirection::Download,
                        result,
                    });
                }
                SftpCommand::Create { side, path, kind } => {
                    let result = match side {
                        FileSide::Local => create_local(&expand_local_path(&path), kind),
                        FileSide::Remote => connection
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|(_, sftp, _)| create_remote(sftp, &path, kind)),
                    };
                    events.send(SftpEvent::MutationFinished {
                        tab_id: tab_id.clone(),
                        side,
                        operation: FileOperation::Create,
                        result,
                    });
                }
                SftpCommand::Rename {
                    side,
                    old_path,
                    new_path,
                } => {
                    let result =
                        match side {
                            FileSide::Local => rename_local(
                                &expand_local_path(&old_path),
                                &expand_local_path(&new_path),
                            ),
                            FileSide::Remote => connection.as_ref().map_err(Clone::clone).and_then(
                                |(_, sftp, _)| {
                                    sftp.rename(Path::new(&old_path), Path::new(&new_path), None)
                                        .map_err(|error| {
                                            format!(
                                        "无法将远端路径 {old_path} 重命名为 {new_path}: {error}"
                                    )
                                        })
                                },
                            ),
                        };
                    events.send(SftpEvent::MutationFinished {
                        tab_id: tab_id.clone(),
                        side,
                        operation: FileOperation::Rename,
                        result,
                    });
                }
                SftpCommand::Delete { side, path, is_dir } => {
                    let result =
                        match side {
                            FileSide::Local => delete_local(&expand_local_path(&path), is_dir),
                            FileSide::Remote => connection.as_ref().map_err(Clone::clone).and_then(
                                |(_, sftp, _)| {
                                    let result = if is_dir {
                                        sftp.rmdir(Path::new(&path))
                                    } else {
                                        sftp.unlink(Path::new(&path))
                                    };
                                    result.map_err(|error| {
                                        format!("无法删除远端路径 {path}: {error}")
                                    })
                                },
                            ),
                        };
                    events.send(SftpEvent::MutationFinished {
                        tab_id: tab_id.clone(),
                        side,
                        operation: FileOperation::Delete,
                        result,
                    });
                }
                SftpCommand::ReadCompletionHistory {
                    session: command_session,
                    request,
                    path,
                    max_bytes,
                } => {
                    let result =
                        with_current_completion_session(&session, &command_session, || {
                            connection
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|(_, sftp, _)| {
                                    read_remote_history_tail(sftp, Path::new(&path), max_bytes)
                                })
                        });
                    events.send(SftpEvent::CompletionHistoryRead {
                        tab_id: tab_id.clone(),
                        session: command_session,
                        request,
                        path,
                        result,
                    });
                }
                SftpCommand::WriteCompletionCandidate {
                    session: command_session,
                    request_id,
                    path,
                    bytes,
                } => {
                    let result =
                        with_current_completion_session(&session, &command_session, || {
                            connection
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|(_, sftp, _)| {
                                    write_remote_candidate_atomic(sftp, &path, request_id, &bytes)
                                })
                        });
                    events.send(SftpEvent::CompletionCandidateWritten {
                        tab_id: tab_id.clone(),
                        session: command_session,
                        request_id,
                        result,
                    });
                }
            }
        }
    });
    SftpHandle {
        id: worker_id,
        tab_id: handle_tab_id,
        pane_id: handle_pane_id,
        session: handle_session,
        tx,
    }
}

pub(super) fn completion_command_session_is_current(
    worker_session: &CompletionSessionKey,
    command_session: &CompletionSessionKey,
) -> bool {
    worker_session == command_session
}

pub(super) fn with_current_completion_session<T>(
    worker_session: &CompletionSessionKey,
    command_session: &CompletionSessionKey,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if !completion_command_session_is_current(worker_session, command_session) {
        return Err("补全会话已失效".to_string());
    }
    operation()
}

fn send_connection_result(
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    connection: &Result<(ssh2::Session, ssh2::Sftp, String), String>,
) {
    let event = match connection {
        Ok((_, _, home)) => SftpEvent::Ready {
            tab_id: tab_id.to_string(),
            home: home.clone(),
        },
        Err(error) => SftpEvent::Failed {
            tab_id: tab_id.to_string(),
            error: error.clone(),
        },
    };
    events.send(event);
}

struct WorkerEventSink<'a> {
    proxy: &'a EventLoopProxy<crate::UserEvent>,
    worker_id: SftpWorkerId,
    tab_id: &'a str,
    pane_id: &'a str,
    session: &'a CompletionSessionKey,
}

impl WorkerEventSink<'_> {
    fn send(&self, event: SftpEvent) {
        let _ = self
            .proxy
            .send_event(crate::UserEvent::Sftp(SftpWorkerEvent {
                worker_id: self.worker_id,
                tab_id: self.tab_id.to_string(),
                pane_id: self.pane_id.to_string(),
                session: self.session.clone(),
                event,
            }));
    }
}

fn emit_progress(
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    direction: TransferDirection,
    transferred: u64,
    total: u64,
) {
    events.send(SftpEvent::TransferProgress {
        tab_id: tab_id.to_string(),
        transfer_id: transfer_id.to_string(),
        direction,
        transferred,
        total,
    });
}

fn ensure_remote_dir(sftp: &ssh2::Sftp, remote_path: &str) -> Result<(), String> {
    match sftp.stat(Path::new(remote_path)) {
        Ok(stat) if stat.is_dir() => Ok(()),
        Ok(_) => Err(format!("远端目标已存在且不是目录: {remote_path}")),
        Err(_) => sftp
            .mkdir(Path::new(remote_path), 0o755)
            .map_err(|error| format!("无法创建远端目录 {remote_path}: {error}")),
    }
}

fn upload_local_path(
    sftp: &ssh2::Sftp,
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    local_path: &str,
    remote_path: &str,
) -> Result<(), String> {
    let local_path = expand_local_path(local_path);
    let metadata = std::fs::symlink_metadata(&local_path)
        .map_err(|error| format!("无法读取本地路径 {}: {error}", local_path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("不支持上传符号链接: {}", local_path.display()));
    }
    if metadata.is_file() {
        return transfer_upload_file(
            sftp,
            events,
            tab_id,
            transfer_id,
            &local_path,
            remote_path,
            0,
            metadata.len(),
        );
    }
    if !metadata.is_dir() {
        return Err(format!("不支持上传特殊文件: {}", local_path.display()));
    }

    let plan = build_local_upload_plan(&local_path)?;
    ensure_remote_dir(sftp, remote_path)?;
    let mut completed = 0_u64;
    for entry in &plan.entries {
        let target = remote_plan_path(remote_path, &entry.relative)?;
        if entry.is_dir {
            ensure_remote_dir(sftp, &target)?;
        } else {
            transfer_upload_file(
                sftp,
                events,
                tab_id,
                transfer_id,
                &entry.source,
                &target,
                completed,
                plan.total_bytes,
            )?;
            completed = completed.saturating_add(entry.size);
        }
    }
    emit_progress(
        events,
        tab_id,
        transfer_id,
        TransferDirection::Upload,
        completed,
        plan.total_bytes,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn transfer_upload_file(
    sftp: &ssh2::Sftp,
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    local_path: &Path,
    remote_path: &str,
    completed_before: u64,
    total: u64,
) -> Result<(), String> {
    let mut source = std::fs::File::open(local_path)
        .map_err(|error| format!("无法打开本地文件 {}: {error}", local_path.display()))?;
    let mut destination = sftp
        .create(Path::new(remote_path))
        .map_err(|e| format!("无法创建远端文件 {remote_path}: {e}"))?;
    copy_with_progress_offset(
        &mut source,
        &mut destination,
        completed_before,
        total,
        events,
        tab_id,
        transfer_id,
        TransferDirection::Upload,
    )
}

fn transfer_download(
    sftp: &ssh2::Sftp,
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    remote_path: &str,
    local_path: &str,
) -> Result<(), String> {
    let mut source = sftp
        .open(Path::new(remote_path))
        .map_err(|e| format!("无法打开远端文件 {remote_path}: {e}"))?;
    let total = source.stat().map_err(|e| e.to_string())?.size.unwrap_or(0);
    let mut destination = std::fs::File::create(local_path)
        .map_err(|e| format!("无法创建本地文件 {local_path}: {e}"))?;
    copy_with_progress(
        &mut source,
        &mut destination,
        total,
        events,
        tab_id,
        transfer_id,
        TransferDirection::Download,
    )
}

fn copy_with_progress(
    source: &mut dyn Read,
    destination: &mut dyn Write,
    total: u64,
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    direction: TransferDirection,
) -> Result<(), String> {
    copy_with_progress_offset(
        source,
        destination,
        0,
        total,
        events,
        tab_id,
        transfer_id,
        direction,
    )
}

#[allow(clippy::too_many_arguments)]
fn copy_with_progress_offset(
    source: &mut dyn Read,
    destination: &mut dyn Write,
    completed_before: u64,
    total: u64,
    events: &WorkerEventSink<'_>,
    tab_id: &str,
    transfer_id: &str,
    direction: TransferDirection,
) -> Result<(), String> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut transferred = 0_u64;
    let mut throttle = ProgressThrottle::new(Instant::now());
    loop {
        let count = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        transferred += count as u64;
        let overall = completed_before.saturating_add(transferred);
        if throttle.should_emit(Instant::now(), overall, total) {
            emit_progress(events, tab_id, transfer_id, direction, overall, total);
        }
    }
    destination.flush().map_err(|e| e.to_string())?;
    emit_progress(
        events,
        tab_id,
        transfer_id,
        direction,
        completed_before.saturating_add(transferred),
        total,
    );
    Ok(())
}
