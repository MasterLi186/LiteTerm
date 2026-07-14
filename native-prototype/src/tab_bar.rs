use egui;
use crate::tab_manager::{TabManager, TabType};

pub struct TabBarAction {
    pub switch_to: Option<usize>,
    pub close: Option<usize>,
    pub new_tab: bool,
}

/// Tab bar height in logical pixels
pub const TAB_BAR_HEIGHT: f32 = 28.0;

pub fn render_tab_bar(ctx: &egui::Context, tab_manager: &TabManager) -> TabBarAction {
    let mut action = TabBarAction {
        switch_to: None,
        close: None,
        new_tab: false,
    };

    egui::TopBottomPanel::top("tab_bar")
        .exact_height(TAB_BAR_HEIGHT)
        .frame(egui::Frame::new()
            .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
            .inner_margin(egui::Margin::symmetric(4.0, 0.0)))
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                for (i, tab) in tab_manager.tabs.iter().enumerate() {
                    let is_active = i == tab_manager.active_idx;
                    let bg = if is_active {
                        egui::Color32::from_rgb(0x21, 0x26, 0x2d)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let text_color = if is_active {
                        egui::Color32::from_rgb(0xe6, 0xed, 0xf3)
                    } else {
                        egui::Color32::from_rgb(0x8b, 0x94, 0x9e)
                    };

                    // Type indicator color
                    let dot_color = match &tab.tab_type {
                        TabType::Local { .. } => egui::Color32::from_rgb(0x3f, 0xb9, 0x50), // green
                        TabType::Ssh { .. } => egui::Color32::from_rgb(0x00, 0xd4, 0xff), // cyan
                    };

                    let tab_frame = egui::Frame::new()
                        .fill(bg)
                        .rounding(egui::Rounding::same(4.0))
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0));

                    let resp = tab_frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Dot
                            let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot_rect.center(), 3.0, dot_color);

                            // Label
                            ui.label(egui::RichText::new(&tab.label).size(11.0).color(text_color));

                            // Close button
                            let close_resp = ui.add(
                                egui::Button::new(egui::RichText::new("×").size(12.0).color(
                                    egui::Color32::from_rgb(0x48, 0x4f, 0x58)
                                ))
                                .frame(false)
                            );
                            if close_resp.clicked() {
                                action.close = Some(i);
                            }
                        });
                    });

                    let full_resp = resp.response.interact(egui::Sense::click());
                    if full_resp.clicked() {
                        action.switch_to = Some(i);
                    }
                    if full_resp.middle_clicked() {
                        action.close = Some(i);
                    }
                }

                // [+] button
                let plus_resp = ui.add(
                    egui::Button::new(
                        egui::RichText::new("+").size(14.0).color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e))
                    )
                    .frame(false)
                );
                if plus_resp.clicked() {
                    action.new_tab = true;
                }
            });
        });

    action
}
