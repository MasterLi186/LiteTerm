use adw::prelude::*;
use gtk::{
    Box as GtkBox, Button, DropDown, Entry, Grid, Label, Orientation,
    PasswordEntry, SpinButton, StringList,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::config::connections::{AuthMethod, HostConfig};

/// Dialog for adding or editing an SSH connection.
pub struct ConnectionDialog {
    window: adw::Window,
    group_entry: Entry,
    label_entry: Entry,
    host_entry: Entry,
    port_spin: SpinButton,
    user_entry: Entry,
    auth_dropdown: DropDown,
    password_entry: PasswordEntry,
    key_path_entry: Entry,
    key_browse_btn: Button,
    password_row: GtkBox,
    key_row: GtkBox,
    save_btn: Button,
    cancel_btn: Button,
    /// When editing, stores (group_id, host_id) so the save callback
    /// knows it's an update rather than a new entry.
    editing: Rc<RefCell<Option<(String, String)>>>,
}

impl ConnectionDialog {
    pub fn new(parent: &impl IsA<gtk::Window>) -> Self {
        let window = adw::Window::builder()
            .title("Connection")
            .modal(true)
            .default_width(480)
            .default_height(420)
            .transient_for(parent)
            .build();

        // --- Form fields ---
        let group_entry = Entry::builder()
            .placeholder_text("e.g. Production")
            .hexpand(true)
            .build();

        let label_entry = Entry::builder()
            .placeholder_text("Display name")
            .hexpand(true)
            .build();

        let host_entry = Entry::builder()
            .placeholder_text("hostname or IP")
            .hexpand(true)
            .build();

        let port_adj = gtk::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
        let port_spin = SpinButton::builder()
            .adjustment(&port_adj)
            .climb_rate(1.0)
            .digits(0)
            .hexpand(true)
            .build();

        let user_entry = Entry::builder()
            .placeholder_text("username")
            .hexpand(true)
            .build();

        // Auth method dropdown
        let auth_items = StringList::new(&["Password", "Key File", "SSH Agent"]);
        let auth_dropdown = DropDown::builder()
            .model(&auth_items)
            .hexpand(true)
            .build();

        // Password field (shown when auth=Password)
        let password_entry = PasswordEntry::builder()
            .show_peek_icon(true)
            .hexpand(true)
            .build();

        let password_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        let password_label = Label::builder()
            .label("Password:")
            .halign(gtk::Align::End)
            .width_chars(12)
            .build();
        password_row.append(&password_label);
        password_row.append(&password_entry);

        // Key file path (shown when auth=Key)
        let key_path_entry = Entry::builder()
            .placeholder_text("~/.ssh/id_rsa")
            .hexpand(true)
            .build();

        let key_browse_btn = Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Browse for key file")
            .build();

        let key_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        let key_label = Label::builder()
            .label("Key File:")
            .halign(gtk::Align::End)
            .width_chars(12)
            .build();
        key_row.append(&key_label);
        key_row.append(&key_path_entry);
        key_row.append(&key_browse_btn);

        // --- Layout with Grid ---
        let grid = Grid::builder()
            .row_spacing(8)
            .column_spacing(8)
            .margin_start(16)
            .margin_end(16)
            .margin_top(16)
            .margin_bottom(8)
            .build();

        // Attach labels and fields to the grid
        let field_labels = ["Group:", "Label:", "Host:", "Port:", "Username:", "Auth:"];
        for (i, lbl_text) in field_labels.iter().enumerate() {
            let lbl = Label::builder()
                .label(*lbl_text)
                .halign(gtk::Align::End)
                .build();
            grid.attach(&lbl, 0, i as i32, 1, 1);
        }
        grid.attach(&group_entry, 1, 0, 1, 1);
        grid.attach(&label_entry, 1, 1, 1, 1);
        grid.attach(&host_entry, 1, 2, 1, 1);
        grid.attach(&port_spin, 1, 3, 1, 1);
        grid.attach(&user_entry, 1, 4, 1, 1);
        grid.attach(&auth_dropdown, 1, 5, 1, 1);

        // Dynamic auth rows below the grid
        let form_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .build();
        form_box.append(&grid);
        form_box.append(&password_row);
        form_box.append(&key_row);

        // Initially show password row (auth default = Password)
        password_row.set_visible(true);
        key_row.set_visible(false);

        // --- Auth dropdown changes visibility ---
        {
            let pw = password_row.clone();
            let kr = key_row.clone();
            auth_dropdown.connect_selected_notify(move |dd| {
                let idx = dd.selected();
                pw.set_visible(idx == 0); // Password
                kr.set_visible(idx == 1); // Key File
                // idx == 2 => Agent: hide both
            });
        }

        // --- Buttons ---
        let button_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .halign(gtk::Align::End)
            .spacing(8)
            .margin_top(12)
            .margin_end(16)
            .margin_bottom(16)
            .build();

        let cancel_btn = Button::builder().label("Cancel").build();
        let save_btn = Button::builder().label("Save").build();
        save_btn.add_css_class("suggested-action");

        button_box.append(&cancel_btn);
        button_box.append(&save_btn);

        // --- Outer container ---
        let outer = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .build();

        let header = adw::HeaderBar::new();
        outer.append(&header);
        outer.append(&form_box);
        outer.append(&button_box);

        window.set_content(Some(&outer));

        // --- Cancel closes the dialog ---
        {
            let w = window.clone();
            cancel_btn.connect_clicked(move |_| {
                w.close();
            });
        }

        // --- Key file browse button ---
        {
            let kpe = key_path_entry.clone();
            let win = window.clone();
            key_browse_btn.connect_clicked(move |_| {
                let dialog = gtk::FileChooserDialog::new(
                    Some("Select SSH Key"),
                    Some(&win),
                    gtk::FileChooserAction::Open,
                    &[
                        ("Cancel", gtk::ResponseType::Cancel),
                        ("Open", gtk::ResponseType::Accept),
                    ],
                );

                // Try to start in ~/.ssh
                if let Some(home) = dirs::home_dir() {
                    let ssh_dir = home.join(".ssh");
                    if ssh_dir.exists() {
                        let file = gtk::gio::File::for_path(&ssh_dir);
                        let _ = dialog.set_current_folder(Some(&file));
                    }
                }

                let entry = kpe.clone();
                dialog.connect_response(move |dlg, resp| {
                    if resp == gtk::ResponseType::Accept {
                        if let Some(file) = dlg.file() {
                            if let Some(path) = file.path() {
                                entry.set_text(&path.to_string_lossy());
                            }
                        }
                    }
                    dlg.close();
                });

                dialog.show();
            });
        }

        Self {
            window,
            group_entry,
            label_entry,
            host_entry,
            port_spin,
            user_entry,
            auth_dropdown,
            password_entry,
            key_path_entry,
            key_browse_btn,
            password_row,
            key_row,
            save_btn,
            cancel_btn,
            editing: Rc::new(RefCell::new(None)),
        }
    }

    /// Show the dialog for adding a new connection.
    pub fn show_add(&self) {
        self.window.set_title(Some("New Connection"));
        self.group_entry.set_text("");
        self.label_entry.set_text("");
        self.host_entry.set_text("");
        self.port_spin.set_value(22.0);
        self.user_entry.set_text("");
        self.auth_dropdown.set_selected(0);
        self.password_entry.set_text("");
        self.key_path_entry.set_text("");
        *self.editing.borrow_mut() = None;
        self.window.present();
    }

    /// Show the dialog for editing an existing connection.
    pub fn show_edit(&self, group_id: &str, host_id: &str, host: &HostConfig) {
        self.window.set_title(Some("Edit Connection"));
        self.group_entry.set_text(group_id);
        self.label_entry.set_text(&host.label);
        self.host_entry.set_text(&host.host);
        self.port_spin.set_value(host.port as f64);
        self.user_entry.set_text(&host.user);

        let auth_idx = match host.auth {
            AuthMethod::Keyring => 0,
            AuthMethod::Key => 1,
            AuthMethod::Agent => 2,
        };
        self.auth_dropdown.set_selected(auth_idx);
        self.key_path_entry.set_text(&host.key_path);

        *self.editing.borrow_mut() = Some((group_id.to_string(), host_id.to_string()));
        self.window.present();
    }

    /// Connect the "Save" button to a callback.
    ///
    /// The callback receives `(group_id, host_id, HostConfig)`.
    /// `host_id` is derived from the label (lowercased, spaces replaced with `-`).
    pub fn connect_save<F: Fn(String, String, HostConfig) + 'static>(&self, f: F) {
        let group_entry = self.group_entry.clone();
        let label_entry = self.label_entry.clone();
        let host_entry = self.host_entry.clone();
        let port_spin = self.port_spin.clone();
        let user_entry = self.user_entry.clone();
        let auth_dropdown = self.auth_dropdown.clone();
        let _password_entry = self.password_entry.clone();
        let key_path_entry = self.key_path_entry.clone();
        let window = self.window.clone();
        let editing = Rc::clone(&self.editing);

        self.save_btn.connect_clicked(move |_| {
            let host_text = host_entry.text().trim().to_string();
            let user_text = user_entry.text().trim().to_string();

            // Validate required fields
            if host_text.is_empty() || user_text.is_empty() {
                // Simple validation: flash the entry red briefly
                if host_text.is_empty() {
                    host_entry.add_css_class("error");
                } else {
                    host_entry.remove_css_class("error");
                }
                if user_text.is_empty() {
                    user_entry.add_css_class("error");
                } else {
                    user_entry.remove_css_class("error");
                }
                return;
            }
            host_entry.remove_css_class("error");
            user_entry.remove_css_class("error");

            let group_id = {
                let g = group_entry.text().trim().to_string();
                if g.is_empty() {
                    "default".to_string()
                } else {
                    g.to_lowercase().replace(' ', "-")
                }
            };

            let label_text = label_entry.text().trim().to_string();
            let label = if label_text.is_empty() {
                host_text.clone()
            } else {
                label_text
            };

            let host_id = if let Some((_, ref old_id)) = *editing.borrow() {
                old_id.clone()
            } else {
                label.to_lowercase().replace(' ', "-")
            };

            let auth = match auth_dropdown.selected() {
                1 => AuthMethod::Key,
                2 => AuthMethod::Agent,
                _ => AuthMethod::Keyring,
            };

            let key_path = key_path_entry.text().trim().to_string();

            let config = HostConfig {
                label,
                host: host_text,
                port: port_spin.value() as u16,
                user: user_text,
                auth,
                key_path,
                charset: "UTF-8".to_string(),
            };

            f(group_id, host_id, config);
            window.close();
        });
    }
}

/// Show a simple password prompt dialog.
///
/// Calls `callback` with the entered password when the user clicks OK.
pub fn show_password_dialog(
    parent: &impl IsA<gtk::Window>,
    host_label: &str,
    callback: impl Fn(String) + 'static,
) {
    let dialog = gtk::Dialog::builder()
        .title(&format!("Password for {}", host_label))
        .modal(true)
        .transient_for(parent)
        .default_width(350)
        .build();

    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Connect", gtk::ResponseType::Accept);

    let content = dialog.content_area();
    content.set_spacing(12);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(12);
    content.set_margin_bottom(12);

    let prompt = Label::new(Some(&format!("Enter password for {}:", host_label)));
    content.append(&prompt);

    let pw_entry = PasswordEntry::builder()
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    content.append(&pw_entry);

    // Enter key in the password field triggers Accept
    {
        let dlg = dialog.clone();
        pw_entry.connect_activate(move |_| {
            dlg.response(gtk::ResponseType::Accept);
        });
    }

    dialog.connect_response(move |dlg, resp| {
        if resp == gtk::ResponseType::Accept {
            let pw = pw_entry.text().to_string();
            callback(pw);
        }
        dlg.close();
    });

    dialog.show();
}
