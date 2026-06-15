mod config;
mod core;
mod plugin;
mod ui;

use adw::prelude::*;
use adw::Application;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use config::connections::ConnectionStore;
use config::settings::Settings;
use ui::window::MainWindow;

const APP_ID: &str = "com.guishell.app";

fn main() -> glib::ExitCode {
    env_logger::init();

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        // 1. Load settings
        let settings = Settings::load().unwrap_or_default();

        // 2. Load connection store from config dir
        let conn_path = Settings::config_dir().join("connections.toml");
        let connections = ConnectionStore::load_from(&conn_path).unwrap_or_default();
        let store = Rc::new(RefCell::new(connections));

        // 3. Create the main window
        let main_window = MainWindow::new(app, &settings);

        // 4. Populate sidebar with saved connections
        main_window.sidebar.populate_connections(&store.borrow());

        // 5. Wire up connection dialog, sidebar interactions
        main_window.setup_connections(Rc::clone(&store));

        // 6. Register keyboard shortcuts
        main_window.setup_shortcuts(app);

        // 7. Present the window
        main_window.present();
    });

    app.run()
}
