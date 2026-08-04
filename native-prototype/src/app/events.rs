use super::*;

impl ApplicationHandler<UserEvent> for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_requested {
            event_loop.exit();
            return;
        }
        if let Some(batch) = self.drag_upload.take_batch() {
            self.dispatch_drag_upload(batch);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        let now = Instant::now();
        let selection_scrolled = self.tick_selection_auto_scroll(now);
        let selection_auto_scroll_active = self.selection_auto_scroll_lines != 0;
        if selection_scrolled {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        let playback_active = self
            .recording_playbacks
            .values()
            .any(recording::PlaybackState::is_playing);
        if playback_active || selection_auto_scroll_active {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                now + Duration::from_millis(if playback_active { 16 } else { 32 }),
            ));
        } else {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LiteTerm Native")
            .with_decorations(false)
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs).unwrap());
        // Enable platform IME so Preedit/Commit events are delivered.
        window.set_ime_allowed(true);

        let gpu = pollster::block_on(GpuState::new(window.clone()));
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&gpu.device, gpu.format(), None, 1, false);
        let mut renderer = Renderer::new(&gpu);

        // 应用持久化主题/字体（创建首个本地终端之前）
        let theme = match themes::theme_by_name(&self.settings.terminal.color_scheme) {
            Some(theme) => theme,
            None => {
                let warning = format!(
                    "未知配色方案「{}」，已回落为 AdventureTime",
                    self.settings.terminal.color_scheme
                );
                log::warn!("[SETTINGS] {warning}");
                match &mut self.settings_load_warning {
                    Some(existing) => {
                        existing.push_str("；");
                        existing.push_str(&warning);
                    }
                    None => self.settings_load_warning = Some(warning),
                }
                self.settings.terminal.color_scheme = "AdventureTime".to_string();
                themes::theme_by_name("AdventureTime")
                    .expect("AdventureTime theme must exist in catalog")
            }
        };
        renderer.set_theme(theme);
        renderer.set_font(
            &gpu,
            &self.settings.terminal.font,
            self.settings.terminal.font_size,
        );

        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.renderer = Some(renderer);
        self.gpu = Some(gpu);

        // 先初始化 viewport（减去标签栏和侧边栏）再创建终端
        self.sync_terminal_size();
        let (cols, rows) = self.grid_size();
        log::debug!(
            "[INIT] window={}x{} sidebar={} tabbar={} grid={}x{}",
            self.gpu.as_ref().map(|g| g.width).unwrap_or(0),
            self.gpu.as_ref().map(|g| g.height).unwrap_or(0),
            self.sidebar_width,
            self.tab_bar_height,
            cols,
            rows
        );
        if let Some(r) = &self.renderer {
            log::debug!(
                "[INIT] viewport: w={} h={} cell_w={} cell_h={}",
                r.viewport_width,
                r.viewport_height,
                r.cell_size().0,
                r.cell_size().1
            );
        }
        if !self.restore_workspace_session() {
            self.new_local_tab();
        }

        // A single local collector owns both periodic and manual refreshes so an
        // older snapshot can never race a newer, separately spawned collector.
        let proxy_mon = self.proxy.clone();
        let (local_refresh_tx, local_refresh_rx) = mpsc::sync_channel(1);
        self.local_monitor_refresh = Some(local_refresh_tx);
        std::thread::spawn(move || {
            let mut collector = monitor::MonitorCollector::new();
            loop {
                match local_refresh_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                let data = collector.collect();
                if proxy_mon
                    .send_event(UserEvent::Monitor(monitor::MonitorEvent {
                        key: monitor::MonitorKey::Local,
                        generation: 0,
                        result: Ok(Box::new(data)),
                    }))
                    .is_err()
                {
                    break;
                }
            }
        });

        // Cursor blink thread
        let proxy2 = self.proxy.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(530));
            if proxy2.send_event(UserEvent::Redraw).is_err() {
                break;
            }
        });

        self.window = Some(window);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.save_workspace_session();
        if let Some(mut server) = self.api_server.take() {
            server.stop();
        }
        self.shutdown_all_zmodem();
        self.recordings.stop_all();
        self.terminal_logs.stop_all();
        self.tunnel_registry.close_all();
        self.shutdown_remote_monitors();
        self.adb_history_writer.shutdown();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        self.handle_user_event(event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.handle_window_event(event_loop, event);
    }
}
