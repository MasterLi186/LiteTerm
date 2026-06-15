use adw::prelude::*;
use gtk::gio;
use gtk::glib;
use gtk::{Orientation, Paned};

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::config::connections::{AuthMethod, ConnectionStore};
use crate::config::settings::Settings;
use super::connection_dialog::{ConnectionDialog, show_password_dialog};
use super::file_browser::{FileBrowserPanel, FileBrowserRefs};
use super::file_browser_session::FileBrowserSession;
use super::monitor_session::MonitorSession;
use super::sidebar::Sidebar;
use super::tabs::TabManager;
use super::terminal::TerminalPane;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub sidebar: Sidebar,
    pub tab_manager: TabManager,
    pub main_paned: Paned,
    pub content_paned: Paned,
    pub file_browser: FileBrowserPanel,
    /// Kept for shortcut handler to create new tabs with correct settings.
    tab_manager_settings: Settings,
}

impl MainWindow {
    pub fn new(app: &adw::Application, settings: &Settings) -> Self {
        // --- Sidebar ---
        let sidebar = Sidebar::new();

        // --- Tab manager (terminal area) ---
        let tab_manager = TabManager::new(settings.clone());

        // Add a default terminal tab (spawns local shell)
        tab_manager.add_tab("Terminal 1");

        // --- File browser panel (bottom of content area) ---
        let file_browser = FileBrowserPanel::new();

        // --- Content paned: terminal tabs (top) | file browser (bottom) ---
        let content_paned = Paned::builder()
            .orientation(Orientation::Vertical)
            .start_child(tab_manager.widget())
            .end_child(file_browser.widget())
            .vexpand(true)
            .hexpand(true)
            .build();
        content_paned.set_position(
            (800 - settings.appearance.file_browser_height) as i32,
        );

        // --- Main paned: sidebar (start) | content (end) ---
        let main_paned = Paned::builder()
            .orientation(Orientation::Horizontal)
            .start_child(sidebar.widget())
            .end_child(&content_paned)
            .vexpand(true)
            .hexpand(true)
            .build();
        main_paned.set_position(settings.appearance.sidebar_width as i32);

        // --- Window chrome with HeaderBar in a vertical box ---
        let header_bar = adw::HeaderBar::new();

        let outer_box = gtk::Box::new(Orientation::Vertical, 0);
        outer_box.append(&header_bar);
        outer_box.append(&main_paned);

        // --- Application window ---
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("GuiShell")
            .default_width(1200)
            .default_height(800)
            .content(&outer_box)
            .build();

        Self {
            window,
            sidebar,
            tab_manager,
            main_paned,
            content_paned,
            file_browser,
            tab_manager_settings: settings.clone(),
        }
    }

    /// Wire up sidebar interactions (add, double-click, right-click) to the
    /// connection dialog and tab manager.
    pub fn setup_connections(&self, store: Rc<RefCell<ConnectionStore>>) {
        let conn_path = Settings::config_dir().join("connections.toml");

        // --- "+" button opens the connection dialog ---
        {
            let window = self.window.clone();
            let store = Rc::clone(&store);
            let conn_path = conn_path.clone();
            let sidebar_list = self.sidebar.connection_list().clone();

            self.sidebar.connect_add_clicked(move || {
                let dialog = ConnectionDialog::new(&window);

                let store2 = Rc::clone(&store);
                let path2 = conn_path.clone();
                let list2 = sidebar_list.clone();

                dialog.connect_save(move |group_id, host_id, config| {
                    let mut s = store2.borrow_mut();
                    // Ensure the group exists
                    if !s.groups.contains_key(&group_id) {
                        s.add_group(&group_id, &group_id, "#3584e4");
                    }
                    s.add_host(&group_id, &host_id, config);
                    if let Err(e) = s.save_to(&path2) {
                        log::error!("Failed to save connections: {}", e);
                    }
                    // Refresh sidebar
                    refresh_sidebar(&list2, &s);
                });

                dialog.show_add();
            });
        }

        // --- Double-click on a host row → open SSH connection ---
        {
            let store = Rc::clone(&store);
            let notebook = self.tab_manager.widget().clone();
            let settings = self.tab_manager_settings.clone();
            let window = self.window.clone();
            let monitor_area = self.sidebar.monitor_area().clone();
            let file_browser = self.file_browser.clone_refs();

            self.sidebar.connect_row_activated(move |group_id, host_id| {
                open_ssh_connection(
                    &window,
                    &store.borrow(),
                    &group_id,
                    &host_id,
                    &notebook,
                    &settings,
                    &monitor_area,
                    &file_browser,
                );
            });
        }

        // --- Right-click context menu ---
        {
            let store_connect = Rc::clone(&store);
            let notebook_connect = self.tab_manager.widget().clone();
            let settings_connect = self.tab_manager_settings.clone();
            let window_connect = self.window.clone();
            let monitor_area_connect = self.sidebar.monitor_area().clone();
            let file_browser_connect = self.file_browser.clone_refs();

            let store_edit = Rc::clone(&store);
            let conn_path_edit = conn_path.clone();
            let window_edit = self.window.clone();
            let sidebar_list_edit = self.sidebar.connection_list().clone();

            let store_delete = Rc::clone(&store);
            let conn_path_delete = conn_path.clone();
            let sidebar_list_delete = self.sidebar.connection_list().clone();

            self.sidebar.setup_context_menu(
                // Connect
                move |group_id, host_id| {
                    open_ssh_connection(
                        &window_connect,
                        &store_connect.borrow(),
                        &group_id,
                        &host_id,
                        &notebook_connect,
                        &settings_connect,
                        &monitor_area_connect,
                        &file_browser_connect,
                    );
                },
                // Edit
                move |group_id, host_id| {
                    let s = store_edit.borrow();
                    if let Some(group) = s.groups.get(&group_id) {
                        if let Some(host) = group.hosts.get(&host_id) {
                            let dialog = ConnectionDialog::new(&window_edit);

                            let store2 = Rc::clone(&store_edit);
                            let path2 = conn_path_edit.clone();
                            let list2 = sidebar_list_edit.clone();

                            dialog.connect_save(move |gid, hid, config| {
                                let mut s = store2.borrow_mut();
                                if !s.groups.contains_key(&gid) {
                                    s.add_group(&gid, &gid, "#3584e4");
                                }
                                s.add_host(&gid, &hid, config);
                                if let Err(e) = s.save_to(&path2) {
                                    log::error!("Failed to save connections: {}", e);
                                }
                                refresh_sidebar(&list2, &s);
                            });

                            dialog.show_edit(&group_id, &host_id, host);
                        }
                    }
                },
                // Delete
                move |group_id, host_id| {
                    let mut s = store_delete.borrow_mut();
                    s.remove_host(&group_id, &host_id);
                    if let Err(e) = s.save_to(&conn_path_delete) {
                        log::error!("Failed to save connections: {}", e);
                    }
                    refresh_sidebar(&sidebar_list_delete, &s);
                },
            );
        }
    }

    /// Register global keyboard shortcuts on the application.
    pub fn setup_shortcuts(&self, app: &adw::Application) {
        // --- Ctrl+Shift+T  : new tab (with local shell) ---
        let action_new_tab = gio::SimpleAction::new("new-tab", None);
        {
            let tab_mgr = self.tab_manager.widget().clone();
            let settings = self.tab_manager_settings.clone();
            action_new_tab.connect_activate(move |_, _| {
                let n = tab_mgr.n_pages() + 1;
                let label = format!("Terminal {}", n);
                let term = TerminalPane::new(&settings.terminal);
                term.spawn_local_shell();

                let tab_label_box = gtk::Box::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(4)
                    .build();
                let tab_label = gtk::Label::new(Some(&label));
                tab_label_box.append(&tab_label);

                let close_button = gtk::Button::builder()
                    .icon_name("window-close-symbolic")
                    .has_frame(false)
                    .build();
                close_button.add_css_class("flat");
                close_button.add_css_class("circular");
                tab_label_box.append(&close_button);

                let page = term.widget().clone();
                tab_mgr.append_page(&page, Some(&tab_label_box));
                tab_mgr.set_current_page(Some(tab_mgr.n_pages() - 1));

                let nb = tab_mgr.clone();
                let pw = page.clone();
                close_button.connect_clicked(move |_| {
                    if let Some(page_num) = nb.page_num(&pw) {
                        nb.remove_page(Some(page_num));
                    }
                });
            });
        }
        app.add_action(&action_new_tab);
        app.set_accels_for_action("app.new-tab", &["<Control><Shift>t"]);

        // --- Ctrl+Shift+Q  : close current tab ---
        let action_close_tab = gio::SimpleAction::new("close-tab", None);
        {
            let nb = self.tab_manager.widget().clone();
            action_close_tab.connect_activate(move |_, _| {
                if let Some(page_num) = nb.current_page() {
                    nb.remove_page(Some(page_num));
                }
            });
        }
        app.add_action(&action_close_tab);
        app.set_accels_for_action("app.close-tab", &["<Control><Shift>q"]);

        // --- Ctrl+B  : toggle sidebar visibility ---
        let action_toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        {
            let sidebar_widget = self.sidebar.widget().clone();
            action_toggle_sidebar.connect_activate(move |_, _| {
                sidebar_widget.set_visible(!sidebar_widget.is_visible());
            });
        }
        app.add_action(&action_toggle_sidebar);
        app.set_accels_for_action("app.toggle-sidebar", &["<Control>b"]);

        // --- Ctrl+Shift+E  : toggle file browser visibility ---
        let action_toggle_fb = gio::SimpleAction::new("toggle-file-browser", None);
        {
            let content = self.content_paned.clone();
            action_toggle_fb.connect_activate(move |_, _| {
                if let Some(end_child) = content.end_child() {
                    end_child.set_visible(!end_child.is_visible());
                }
            });
        }
        app.add_action(&action_toggle_fb);
        app.set_accels_for_action("app.toggle-file-browser", &["<Control><Shift>e"]);

        // --- F11  : toggle fullscreen ---
        let action_fullscreen = gio::SimpleAction::new("toggle-fullscreen", None);
        {
            let win = self.window.clone();
            action_fullscreen.connect_activate(move |_, _| {
                win.set_fullscreened(!win.is_fullscreen());
            });
        }
        app.add_action(&action_fullscreen);
        app.set_accels_for_action("app.toggle-fullscreen", &["F11"]);
    }

    pub fn present(&self) {
        self.window.present();
    }
}

/// Re-populate the sidebar's connection list from the store.
fn refresh_sidebar(listbox: &gtk::ListBox, store: &ConnectionStore) {
    // We need to create a temporary Sidebar-like object to call populate,
    // but since populate_connections is on Sidebar, we inline the logic here.
    // Clear existing children
    while let Some(child) = listbox.first_child() {
        listbox.remove(&child);
    }

    for group in store.groups() {
        let header_label = gtk::Label::builder()
            .label(&group.label)
            .halign(gtk::Align::Start)
            .margin_start(8)
            .margin_top(8)
            .margin_bottom(2)
            .build();
        header_label.add_css_class("heading");

        let header_row = gtk::ListBoxRow::builder()
            .selectable(false)
            .activatable(false)
            .child(&header_label)
            .build();
        listbox.append(&header_row);

        for (host_id, host) in store.hosts_in_group(&group.id) {
            let row_box = gtk::Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(6)
                .margin_start(16)
                .margin_top(2)
                .margin_bottom(2)
                .build();

            let dot = gtk::Label::builder()
                .label("\u{25CF}")
                .build();
            let css = gtk::CssProvider::new();
            let color_css = format!("label {{ color: {}; }}", group.color);
            css.load_from_data(&color_css);
            dot.style_context().add_provider(
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            row_box.append(&dot);

            let name_label = gtk::Label::builder()
                .label(&host.label)
                .halign(gtk::Align::Start)
                .hexpand(true)
                .build();
            row_box.append(&name_label);

            let addr_text = format!("{}:{}", host.host, host.port);
            let addr_label = gtk::Label::builder()
                .label(&addr_text)
                .halign(gtk::Align::End)
                .build();
            addr_label.add_css_class("dim-label");
            row_box.append(&addr_label);

            let host_row = gtk::ListBoxRow::builder()
                .child(&row_box)
                .build();

            unsafe {
                host_row.set_data::<String>("gs-group-id", group.id.clone());
                host_row.set_data::<String>("gs-host-id", host_id.clone());
            }

            listbox.append(&host_row);
        }
    }
}

/// Open an SSH connection for the given host, creating a new tab.
fn open_ssh_connection(
    window: &adw::ApplicationWindow,
    store: &ConnectionStore,
    group_id: &str,
    host_id: &str,
    notebook: &gtk::Notebook,
    settings: &Settings,
    monitor_area: &gtk::Box,
    file_browser: &FileBrowserRefs,
) {
    let conn = match store.get_connection_config(group_id, host_id) {
        Some(c) => c,
        None => {
            log::error!("Connection not found: {}/{}", group_id, host_id);
            return;
        }
    };

    // Get the label for the tab
    let tab_label = store
        .groups
        .get(group_id)
        .and_then(|g| g.hosts.get(host_id))
        .map(|h| h.label.clone())
        .unwrap_or_else(|| format!("{}:{}", conn.host, conn.port));

    // For password auth, prompt first, then connect
    if conn.auth == AuthMethod::Keyring {
        let host = conn.host.clone();
        let port = conn.port;
        let user = conn.user.clone();
        let notebook = notebook.clone();
        let settings = settings.clone();
        let label = tab_label.clone();
        let monitor_area = monitor_area.clone();
        let file_browser = file_browser.clone();

        show_password_dialog(window, &tab_label, move |password| {
            do_ssh_connect(
                &notebook, &settings, &label, &host, port, &user,
                Some(&password), None, &monitor_area, &file_browser,
            );
        });
    } else if conn.auth == AuthMethod::Key {
        let key_path = conn.key_path.clone();
        do_ssh_connect(
            notebook, settings, &tab_label, &conn.host, conn.port, &conn.user,
            None, Some(&key_path), monitor_area, file_browser,
        );
    } else {
        // Agent auth
        do_ssh_connect(
            notebook, settings, &tab_label, &conn.host, conn.port, &conn.user,
            None, None, monitor_area, file_browser,
        );
    }
}

/// Perform the actual SSH connection in a background thread and wire
/// it to a new terminal tab. On success, also starts monitoring and SFTP
/// sessions using independent SSH connections.
fn do_ssh_connect(
    notebook: &gtk::Notebook,
    settings: &Settings,
    tab_label: &str,
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    key_path: Option<&str>,
    monitor_area: &gtk::Box,
    file_browser: &FileBrowserRefs,
) {
    use std::net::TcpStream;

    let terminal = TerminalPane::new(&settings.terminal);

    // Build tab label with close button
    let tab_label_box = gtk::Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    let label_widget = gtk::Label::new(Some(tab_label));
    tab_label_box.append(&label_widget);

    let close_button = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .has_frame(false)
        .build();
    close_button.add_css_class("flat");
    close_button.add_css_class("circular");
    tab_label_box.append(&close_button);

    let page_widget = terminal.widget().clone();
    notebook.append_page(&page_widget, Some(&tab_label_box));
    notebook.set_current_page(Some(notebook.n_pages() - 1));

    let nb = notebook.clone();
    let pw = page_widget.clone();
    close_button.connect_clicked(move |_| {
        if let Some(page_num) = nb.page_num(&pw) {
            nb.remove_page(Some(page_num));
        }
    });

    // Attempt SSH connection in background
    let addr = format!("{}:{}", host, port);
    let user = user.to_string();
    let password = password.map(|s| s.to_string());
    let key_path = key_path.map(|s| s.to_string());

    // Show connecting message
    terminal.feed_data(format!("Connecting to {}...\r\n", addr).as_bytes());

    // Connect on a background thread to avoid blocking the UI.
    // Use std::sync::mpsc + glib::timeout_add_local to check for the result.
    let (tx, rx) = std::sync::mpsc::channel::<Result<Arc<ssh2::Session>, String>>();

    let user_bg = user.clone();
    let password_bg = password.clone();
    let key_path_bg = key_path.clone();

    std::thread::spawn(move || {
        let result = (|| -> Result<Arc<ssh2::Session>, String> {
            let tcp = TcpStream::connect(&addr).map_err(|e| format!("TCP connect failed: {}", e))?;
            let mut sess = ssh2::Session::new().map_err(|e| format!("Session create failed: {}", e))?;
            sess.set_tcp_stream(tcp);
            sess.handshake().map_err(|e| format!("SSH handshake failed: {}", e))?;

            // Authenticate
            if let Some(ref pw) = password_bg {
                sess.userauth_password(&user_bg, pw)
                    .map_err(|e| format!("Password auth failed: {}", e))?;
            } else if let Some(ref kp) = key_path_bg {
                let expanded = shellexpand::tilde(kp).to_string();
                let path = std::path::Path::new(&expanded);
                sess.userauth_pubkey_file(&user_bg, None, path, None)
                    .map_err(|e| format!("Key auth failed: {}", e))?;
            } else {
                // Try agent
                let mut agent = sess.agent().map_err(|e| format!("SSH agent failed: {}", e))?;
                agent.connect().map_err(|e| format!("Agent connect failed: {}", e))?;
                agent.list_identities().map_err(|e| format!("Agent list failed: {}", e))?;

                let mut authenticated = false;
                for identity in agent.identities().unwrap_or_default() {
                    if agent.userauth(&user_bg, &identity).is_ok() {
                        authenticated = true;
                        break;
                    }
                }
                if !authenticated {
                    return Err("SSH agent authentication failed".to_string());
                }
            }

            if !sess.authenticated() {
                return Err("Authentication failed".to_string());
            }

            Ok(Arc::new(sess))
        })();

        let _ = tx.send(result);
    });

    // Capture the connection credentials for monitor/SFTP sessions
    let mon_host = host.to_string();
    let mon_port = port;
    let mon_user = user.clone();
    let mon_password = password.clone();
    let mon_key_path = key_path.clone();
    let monitor_area = monitor_area.clone();
    let file_browser = file_browser.clone();

    // Poll for the SSH result on the main thread
    let term = terminal;
    let rx = std::sync::Mutex::new(rx);
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let rx = rx.lock().unwrap();
        match rx.try_recv() {
            Ok(Ok(session)) => {
                term.feed_data(b"Connected!\r\n");
                term.connect_ssh_channel(session);

                // Start the system monitor (opens its own SSH connection)
                // Clear any previous monitor charts
                while let Some(child) = monitor_area.first_child() {
                    monitor_area.remove(&child);
                }
                let _monitor = MonitorSession::start(
                    &monitor_area,
                    &mon_host,
                    mon_port,
                    &mon_user,
                    mon_password.as_deref(),
                    mon_key_path.as_deref(),
                );
                // Keep the monitor session alive by leaking it into a Box.
                // It will stop when the AtomicBool stop_flag is set on drop,
                // but for now we let it run for the lifetime of the connection.
                // A more sophisticated approach would store it per-tab.
                Box::leak(Box::new(_monitor));

                // Start the SFTP file browser (opens its own SSH connection)
                let fb_panel = FileBrowserPanel::from_refs(&file_browser);
                let _fb = FileBrowserSession::start(
                    &fb_panel,
                    &mon_host,
                    mon_port,
                    &mon_user,
                    mon_password.as_deref(),
                    mon_key_path.as_deref(),
                );
                Box::leak(Box::new(_fb));

                glib::ControlFlow::Break
            }
            Ok(Err(msg)) => {
                term.feed_data(format!("\x1b[31mError: {}\x1b[0m\r\n", msg).as_bytes());
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                glib::ControlFlow::Continue
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                term.feed_data(b"\x1b[31mConnection thread terminated unexpectedly\x1b[0m\r\n");
                glib::ControlFlow::Break
            }
        }
    });
}
