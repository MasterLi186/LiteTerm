use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, GestureClick, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow, Separator,
};

use crate::config::connections::ConnectionStore;

/// Keys used with `set_data` / `data` on ListBoxRow to stash connection identity.
const KEY_GROUP_ID: &str = "gs-group-id";
const KEY_HOST_ID: &str = "gs-host-id";

pub struct Sidebar {
    container: GtkBox,
    connection_list: ListBox,
    monitor_area: GtkBox,
    add_button: Button,
}

impl Sidebar {
    pub fn new() -> Self {
        let container = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .width_request(200)
            .build();

        // --- Connection list header with "+" button ---
        let header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .margin_start(8)
            .margin_end(4)
            .margin_top(8)
            .margin_bottom(4)
            .build();

        let conn_label = Label::builder()
            .label("Connections")
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        conn_label.add_css_class("heading");
        header_box.append(&conn_label);

        let add_button = Button::builder()
            .icon_name("list-add-symbolic")
            .has_frame(false)
            .tooltip_text("Add connection")
            .build();
        add_button.add_css_class("flat");
        add_button.add_css_class("circular");
        header_box.append(&add_button);

        container.append(&header_box);

        let connection_list = ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        connection_list.add_css_class("navigation-sidebar");

        let conn_scrolled = ScrolledWindow::builder()
            .vexpand(true)
            .child(&connection_list)
            .build();
        container.append(&conn_scrolled);

        // --- Separator ---
        let separator = Separator::new(Orientation::Horizontal);
        container.append(&separator);

        // --- Monitor section ---
        let monitor_label = Label::builder()
            .label("Monitor")
            .halign(gtk::Align::Start)
            .margin_start(8)
            .margin_top(8)
            .margin_bottom(4)
            .build();
        monitor_label.add_css_class("heading");
        container.append(&monitor_label);

        let monitor_area = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        let monitor_scrolled = ScrolledWindow::builder()
            .vexpand(true)
            .child(&monitor_area)
            .build();
        container.append(&monitor_scrolled);

        Self {
            container,
            connection_list,
            monitor_area,
            add_button,
        }
    }

    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn connection_list(&self) -> &ListBox {
        &self.connection_list
    }

    pub fn monitor_area(&self) -> &GtkBox {
        &self.monitor_area
    }

    pub fn add_button(&self) -> &Button {
        &self.add_button
    }

    /// Connect a callback for the "+" button.
    pub fn connect_add_clicked<F: Fn() + 'static>(&self, f: F) {
        self.add_button.connect_clicked(move |_| {
            f();
        });
    }

    /// Connect a callback for double-click on a host row.
    ///
    /// The callback receives `(group_id, host_id)`.
    pub fn connect_row_activated<F: Fn(String, String) + Clone + 'static>(&self, f: F) {
        self.connection_list.connect_row_activated(move |_listbox, row| {
            let group_id: Option<String> = unsafe { row.data::<String>(KEY_GROUP_ID).map(|p| p.as_ref().clone()) };
            let host_id: Option<String> = unsafe { row.data::<String>(KEY_HOST_ID).map(|p| p.as_ref().clone()) };

            if let (Some(gid), Some(hid)) = (group_id, host_id) {
                f(gid, hid);
            }
        });
    }

    /// Connect callbacks for the right-click context menu on host rows.
    ///
    /// - `on_connect(group_id, host_id)` — open a connection
    /// - `on_edit(group_id, host_id)` — edit the connection
    /// - `on_delete(group_id, host_id)` — delete the connection
    pub fn setup_context_menu<C, E, D>(&self, on_connect: C, on_edit: E, on_delete: D)
    where
        C: Fn(String, String) + Clone + 'static,
        E: Fn(String, String) + Clone + 'static,
        D: Fn(String, String) + Clone + 'static,
    {
        // We use a GestureClick with button 3 (right-click) on the ListBox
        let gesture = GestureClick::builder()
            .button(3) // right-click
            .build();

        let listbox = self.connection_list.clone();
        let on_connect = on_connect.clone();
        let on_edit = on_edit.clone();
        let on_delete = on_delete.clone();

        gesture.connect_pressed(move |_gesture, _n_press, _x, y| {
            // Find the row at this position
            if let Some(row) = listbox.row_at_y(y as i32) {
                let group_id: Option<String> =
                    unsafe { row.data::<String>(KEY_GROUP_ID).map(|p| p.as_ref().clone()) };
                let host_id: Option<String> =
                    unsafe { row.data::<String>(KEY_HOST_ID).map(|p| p.as_ref().clone()) };

                if let (Some(gid), Some(hid)) = (group_id, host_id) {
                    // Build a popover with action buttons
                    let pop_box = GtkBox::builder()
                        .orientation(Orientation::Vertical)
                        .spacing(2)
                        .margin_start(4)
                        .margin_end(4)
                        .margin_top(4)
                        .margin_bottom(4)
                        .build();

                    let connect_btn = Button::builder()
                        .label("Connect")
                        .has_frame(false)
                        .build();
                    let edit_btn = Button::builder()
                        .label("Edit")
                        .has_frame(false)
                        .build();
                    let delete_btn = Button::builder()
                        .label("Delete")
                        .has_frame(false)
                        .build();
                    delete_btn.add_css_class("destructive-action");

                    pop_box.append(&connect_btn);
                    pop_box.append(&edit_btn);
                    pop_box.append(&delete_btn);

                    let popover = gtk::Popover::builder()
                        .child(&pop_box)
                        .has_arrow(true)
                        .build();
                    popover.set_parent(&row);

                    let gid2 = gid.clone();
                    let hid2 = hid.clone();
                    let oc = on_connect.clone();
                    let pop = popover.clone();
                    connect_btn.connect_clicked(move |_| {
                        pop.popdown();
                        oc(gid2.clone(), hid2.clone());
                    });

                    let gid2 = gid.clone();
                    let hid2 = hid.clone();
                    let oe = on_edit.clone();
                    let pop = popover.clone();
                    edit_btn.connect_clicked(move |_| {
                        pop.popdown();
                        oe(gid2.clone(), hid2.clone());
                    });

                    let gid2 = gid.clone();
                    let hid2 = hid.clone();
                    let od = on_delete.clone();
                    let pop = popover.clone();
                    delete_btn.connect_clicked(move |_| {
                        pop.popdown();
                        od(gid2.clone(), hid2.clone());
                    });

                    popover.popup();
                }
            }
        });

        self.connection_list.add_controller(gesture);
    }

    /// Populate the connection list from a `ConnectionStore`.
    ///
    /// For each group a non-selectable header row is inserted, followed by
    /// selectable rows for every host in that group. Each host row shows a
    /// colored dot (using the group color), the host label, and the address.
    ///
    /// Each host row stores the group_id and host_id as widget data so that
    /// activation and context menu callbacks can identify the connection.
    pub fn populate_connections(&self, store: &ConnectionStore) {
        // Clear existing children
        while let Some(child) = self.connection_list.first_child() {
            self.connection_list.remove(&child);
        }

        for group in store.groups() {
            // --- Group header (non-selectable) ---
            let header_label = Label::builder()
                .label(&group.label)
                .halign(gtk::Align::Start)
                .margin_start(8)
                .margin_top(8)
                .margin_bottom(2)
                .build();
            header_label.add_css_class("heading");

            let header_row = ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .child(&header_label)
                .build();
            self.connection_list.append(&header_row);

            // --- Host rows ---
            for (host_id, host) in store.hosts_in_group(&group.id) {
                let row_box = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(6)
                    .margin_start(16)
                    .margin_top(2)
                    .margin_bottom(2)
                    .build();

                // Colored dot using the group color
                let dot = Label::builder()
                    .label("\u{25CF}") // filled circle
                    .build();
                let css = gtk::CssProvider::new();
                let color_css = format!("label {{ color: {}; }}", group.color);
                css.load_from_data(&color_css);
                dot.style_context().add_provider(
                    &css,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
                row_box.append(&dot);

                // Host label
                let name_label = Label::builder()
                    .label(&host.label)
                    .halign(gtk::Align::Start)
                    .hexpand(true)
                    .build();
                row_box.append(&name_label);

                // Address (dimmed)
                let addr_text = format!("{}:{}", host.host, host.port);
                let addr_label = Label::builder()
                    .label(&addr_text)
                    .halign(gtk::Align::End)
                    .build();
                addr_label.add_css_class("dim-label");
                row_box.append(&addr_label);

                let host_row = ListBoxRow::builder()
                    .child(&row_box)
                    .build();

                // Store group_id and host_id on the row for later retrieval
                unsafe {
                    host_row.set_data::<String>(KEY_GROUP_ID, group.id.clone());
                    host_row.set_data::<String>(KEY_HOST_ID, host_id.clone());
                }

                self.connection_list.append(&host_row);
            }
        }
    }
}
