use gtk::prelude::*;
use gtk::{Box as GtkBox, Button, Label, Notebook, Orientation};

use crate::config::settings::Settings;
use super::terminal::TerminalPane;

pub struct TabManager {
    notebook: Notebook,
    settings: Settings,
}

impl TabManager {
    pub fn new(settings: Settings) -> Self {
        let notebook = Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .build();

        Self { notebook, settings }
    }

    pub fn widget(&self) -> &Notebook {
        &self.notebook
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Add a new terminal tab with the given label text and spawn a local shell.
    /// Returns the TerminalPane so the caller can feed data into it.
    pub fn add_tab(&self, label_text: &str) -> TerminalPane {
        let terminal = self.add_tab_raw(label_text);
        terminal.spawn_local_shell();
        terminal
    }

    /// Add a new terminal tab without spawning anything.
    /// Used when the caller will connect an SSH session instead.
    pub fn add_tab_raw(&self, label_text: &str) -> TerminalPane {
        let terminal = TerminalPane::new(&self.settings.terminal);

        // Build a tab label with text + close button
        let tab_label_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();

        let label = Label::new(Some(label_text));
        tab_label_box.append(&label);

        let close_button = Button::builder()
            .icon_name("window-close-symbolic")
            .has_frame(false)
            .build();
        close_button.add_css_class("flat");
        close_button.add_css_class("circular");
        tab_label_box.append(&close_button);

        let page_widget = terminal.widget().clone();
        self.notebook.append_page(&page_widget, Some(&tab_label_box));

        // Wire up the close button to remove the page
        let nb = self.notebook.clone();
        let pw = page_widget.clone();
        close_button.connect_clicked(move |_| {
            if let Some(page_num) = nb.page_num(&pw) {
                nb.remove_page(Some(page_num));
            }
        });

        // Focus the new tab
        let n = self.notebook.n_pages();
        self.notebook.set_current_page(Some(n - 1));

        terminal
    }

    pub fn tab_count(&self) -> u32 {
        self.notebook.n_pages()
    }
}
