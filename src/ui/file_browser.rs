use gtk::prelude::*;
use gtk::{
    Box as GtkBox, Button, Entry, Label, ListBox, Orientation, Paned, ProgressBar,
    ScrolledWindow,
};

/// Lightweight clonable reference bundle for passing `FileBrowserPanel`
/// widget references through GTK closures that require `'static` types.
/// All fields are GObject references (cheap to clone).
#[derive(Clone)]
pub struct FileBrowserRefs {
    pub local_path_entry: Entry,
    pub remote_path_entry: Entry,
    pub local_list: ListBox,
    pub remote_list: ListBox,
}

/// A dual-pane file browser panel with local and remote file lists
/// and a transfer queue section at the bottom.
pub struct FileBrowserPanel {
    container: GtkBox,
    local_path_entry: Entry,
    remote_path_entry: Entry,
    local_list: ListBox,
    remote_list: ListBox,
    transfer_list: GtkBox,
}

impl FileBrowserPanel {
    /// Create a new dual-pane file browser panel.
    pub fn new() -> Self {
        let container = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(0)
            .vexpand(true)
            .hexpand(true)
            .build();

        // --- Local pane ---
        let local_pane = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .vexpand(true)
            .build();

        let local_header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .build();

        let local_label = Label::builder()
            .label("Local")
            .halign(gtk::Align::Start)
            .build();
        local_label.add_css_class("heading");

        let local_path_entry = Entry::builder()
            .placeholder_text("/home")
            .hexpand(true)
            .build();

        local_header.append(&local_label);
        local_header.append(&local_path_entry);
        local_pane.append(&local_header);

        let local_list = ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        local_list.add_css_class("boxed-list");

        let local_scrolled = ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&local_list)
            .build();
        local_pane.append(&local_scrolled);

        // --- Remote pane ---
        let remote_pane = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .vexpand(true)
            .build();

        let remote_header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .build();

        let remote_label = Label::builder()
            .label("Remote")
            .halign(gtk::Align::Start)
            .build();
        remote_label.add_css_class("heading");

        let remote_path_entry = Entry::builder()
            .placeholder_text("/home")
            .hexpand(true)
            .build();

        remote_header.append(&remote_label);
        remote_header.append(&remote_path_entry);
        remote_pane.append(&remote_header);

        let remote_list = ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        remote_list.add_css_class("boxed-list");

        let remote_scrolled = ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .child(&remote_list)
            .build();
        remote_pane.append(&remote_scrolled);

        // --- Horizontal Paned: local | remote ---
        let paned = Paned::builder()
            .orientation(Orientation::Horizontal)
            .start_child(&local_pane)
            .end_child(&remote_pane)
            .vexpand(true)
            .hexpand(true)
            .build();
        paned.set_position(400);

        container.append(&paned);

        // --- Transfer queue section ---
        let transfer_header = Label::builder()
            .label("Transfers")
            .halign(gtk::Align::Start)
            .margin_start(8)
            .margin_top(4)
            .margin_bottom(2)
            .build();
        transfer_header.add_css_class("heading");
        container.append(&transfer_header);

        let transfer_list = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .build();

        let transfer_scrolled = ScrolledWindow::builder()
            .vexpand(false)
            .height_request(100)
            .child(&transfer_list)
            .build();
        container.append(&transfer_scrolled);

        Self {
            container,
            local_path_entry,
            remote_path_entry,
            local_list,
            remote_list,
            transfer_list,
        }
    }

    /// Returns a reference to the outer container widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Create a clonable bundle of widget references for passing through
    /// closures.
    pub fn clone_refs(&self) -> FileBrowserRefs {
        FileBrowserRefs {
            local_path_entry: self.local_path_entry.clone(),
            remote_path_entry: self.remote_path_entry.clone(),
            local_list: self.local_list.clone(),
            remote_list: self.remote_list.clone(),
        }
    }

    /// Reconstruct a `FileBrowserPanel`-like handle from a `FileBrowserRefs`
    /// bundle. This gives access to the same widgets as the original panel
    /// (useful for the file browser session to call accessor methods).
    pub fn from_refs(refs: &FileBrowserRefs) -> Self {
        // We only need the widget references; the container and transfer_list
        // are not needed for navigation wiring, so we create dummies.
        let container = GtkBox::new(Orientation::Vertical, 0);
        let transfer_list = GtkBox::new(Orientation::Vertical, 0);
        Self {
            container,
            local_path_entry: refs.local_path_entry.clone(),
            remote_path_entry: refs.remote_path_entry.clone(),
            local_list: refs.local_list.clone(),
            remote_list: refs.remote_list.clone(),
            transfer_list,
        }
    }

    /// Returns a reference to the local path entry.
    #[allow(dead_code)]
    pub fn local_path_entry(&self) -> &Entry {
        &self.local_path_entry
    }

    /// Returns a reference to the remote path entry.
    #[allow(dead_code)]
    pub fn remote_path_entry(&self) -> &Entry {
        &self.remote_path_entry
    }

    /// Returns a reference to the local file list.
    #[allow(dead_code)]
    pub fn local_list(&self) -> &ListBox {
        &self.local_list
    }

    /// Returns a reference to the remote file list.
    #[allow(dead_code)]
    pub fn remote_list(&self) -> &ListBox {
        &self.remote_list
    }

    /// Add a transfer row to the transfer queue section.
    ///
    /// Returns the row widget so the caller can update progress or remove it.
    ///
    /// - `filename`: name of the file being transferred
    /// - `direction`: a direction indicator string (e.g. ">>>" for upload, "<<<" for download)
    /// - `size`: human-readable file size (e.g. "12.3 MB")
    pub fn add_transfer_row(&self, filename: &str, direction: &str, size: &str) -> GtkBox {
        let row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_start(4)
            .margin_end(4)
            .margin_top(2)
            .margin_bottom(2)
            .build();

        // Direction icon
        let dir_label = Label::builder()
            .label(direction)
            .width_chars(3)
            .build();
        dir_label.add_css_class("monospace");
        row.append(&dir_label);

        // Filename
        let name_label = Label::builder()
            .label(filename)
            .hexpand(true)
            .halign(gtk::Align::Start)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        row.append(&name_label);

        // Size
        let size_label = Label::builder()
            .label(size)
            .width_chars(10)
            .halign(gtk::Align::End)
            .build();
        size_label.add_css_class("dim-label");
        row.append(&size_label);

        // Progress bar
        let progress = ProgressBar::builder()
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        progress.set_fraction(0.0);
        row.append(&progress);

        // Cancel button
        let cancel_button = Button::builder()
            .icon_name("process-stop-symbolic")
            .has_frame(false)
            .build();
        cancel_button.add_css_class("flat");
        cancel_button.add_css_class("circular");

        // Wire cancel to remove the row from the transfer list
        let transfer_list = self.transfer_list.clone();
        let row_ref = row.clone();
        cancel_button.connect_clicked(move |_| {
            transfer_list.remove(&row_ref);
        });

        row.append(&cancel_button);

        self.transfer_list.append(&row);
        row
    }
}
