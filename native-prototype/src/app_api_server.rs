use liteterm_native_api as api;
use std::sync::mpsc;
use winit::event_loop::EventLoopProxy;

use super::UserEvent;

pub(super) struct HttpApiServer {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn reap_http_api_thread(thread: std::thread::JoinHandle<()>) {
    let _ = std::thread::Builder::new()
        .name("native-http-api-reaper".into())
        .spawn(move || {
            let _ = thread.join();
        });
}

impl HttpApiServer {
    pub(super) fn start(
        config: api::ApiServerConfig,
        proxy: EventLoopProxy<UserEvent>,
        outputs: api::OutputRegistry,
    ) -> Result<Self, String> {
        let bridge = api::Bridge::with_default_timeout(move |call| {
            match proxy.send_event(UserEvent::Api(call)) {
                Ok(()) => Ok(()),
                Err(winit::event_loop::EventLoopClosed(UserEvent::Api(call))) => {
                    Err(Box::new(call))
                }
                Err(_) => unreachable!("HTTP bridge only sends Api events"),
            }
        });
        Self::start_with_bridge(config, bridge, outputs)
    }

    pub(super) fn start_with_bridge(
        config: api::ApiServerConfig,
        bridge: api::Bridge,
        outputs: api::OutputRegistry,
    ) -> Result<Self, String> {
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("native-http-api".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(_) => {
                        log::warn!("无法创建 HTTP API 运行时");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let server = match api::start_server(config, bridge, outputs).await {
                        Ok(server) => server,
                        Err(error) => {
                            log::warn!("HTTP API 启动失败（{}）", error.code());
                            return;
                        }
                    };
                    let address = server.address();
                    log::info!("Native HTTP API listening on {address}");
                    let _ = tokio::task::spawn_blocking(move || stop_rx.recv()).await;
                    if let Err(error) = server.shutdown().await {
                        log::warn!("HTTP API 关闭失败（{}）", error.code());
                    }
                });
            })
            .map_err(|_| "无法创建 HTTP API 后台线程".to_string())?;
        Ok(Self {
            stop: Some(stop_tx),
            thread: Some(thread),
        })
    }

    pub(super) fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            reap_http_api_thread(thread);
        }
    }
}

impl Drop for HttpApiServer {
    fn drop(&mut self) {
        self.stop();
    }
}
