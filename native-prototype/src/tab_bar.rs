use egui;
use crate::tab_manager::{TabManager, TabType};

pub struct TabBarAction {
    pub switch_to: Option<usize>,
    pub close: Option<usize>,
    pub new_tab: bool,
}

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
            .inner_margin(egui::Margin::symmetric(4, 0)))
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;

                for (i, tab) in tab_manager.tabs.iter().enumerate() {
                    let is_active = i == tab_manager.active_idx;

                    let dot_color = match &tab.tab_type {
                        TabType::Local { .. } => egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
                        TabType::Ssh { .. } => egui::Color32::from_rgb(0x00, 0xd4, 0xff),
                    };

                    let bg = if is_active {
                        egui::Color32::from_rgb(0x21, 0x26, 0x2d)
                    } else {
                        egui::Color32::from_rgb(0x16, 0x1b, 0x22)
                    };
                    let text_color = if is_active {
                        egui::Color32::from_rgb(0xe6, 0xed, 0xf3)
                    } else {
                        egui::Color32::from_rgb(0x8b, 0x94, 0x9e)
                    };

                    // 整个标签区域
                    let (tab_rect, tab_resp) = ui.allocate_exact_size(
                        egui::vec2(140.0, TAB_BAR_HEIGHT - 4.0),
                        egui::Sense::click(),
                    );

                    // 背景
                    ui.painter().rect_filled(tab_rect, 4.0, bg);
                    if tab_resp.hovered() && !is_active {
                        ui.painter().rect_filled(tab_rect, 4.0,
                            egui::Color32::from_rgba_premultiplied(0x30, 0x36, 0x3d, 0x60));
                    }

                    // 类型圆点
                    let dot_center = egui::pos2(tab_rect.left() + 12.0, tab_rect.center().y);
                    ui.painter().circle_filled(dot_center, 3.0, dot_color);

                    // 标签名
                    let text_pos = egui::pos2(tab_rect.left() + 22.0, tab_rect.center().y);
                    ui.painter().text(
                        text_pos,
                        egui::Align2::LEFT_CENTER,
                        &tab.label,
                        egui::FontId::proportional(11.0),
                        text_color,
                    );

                    // × 关闭按钮区域（右侧 20px）
                    let close_rect = egui::Rect::from_min_size(
                        egui::pos2(tab_rect.right() - 20.0, tab_rect.top()),
                        egui::vec2(20.0, tab_rect.height()),
                    );
                    let close_resp = ui.interact(close_rect, egui::Id::new(("tab_close", i)), egui::Sense::click());

                    // × 文字
                    let close_color = if close_resp.hovered() {
                        egui::Color32::from_rgb(0xf8, 0x51, 0x49)
                    } else {
                        egui::Color32::from_rgb(0x48, 0x4f, 0x58)
                    };
                    ui.painter().text(
                        close_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "×",
                        egui::FontId::proportional(14.0),
                        close_color,
                    );

                    // 事件处理：× 优先
                    if close_resp.clicked() {
                        action.close = Some(i);
                    } else if tab_resp.clicked() {
                        action.switch_to = Some(i);
                    }
                    if tab_resp.middle_clicked() {
                        action.close = Some(i);
                    }
                }

                // [+] 按钮
                let (plus_rect, plus_resp) = ui.allocate_exact_size(
                    egui::vec2(24.0, TAB_BAR_HEIGHT - 4.0),
                    egui::Sense::click(),
                );
                if plus_resp.hovered() {
                    ui.painter().rect_filled(plus_rect, 4.0,
                        egui::Color32::from_rgba_premultiplied(0x30, 0x36, 0x3d, 0x80));
                }
                ui.painter().text(
                    plus_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "+",
                    egui::FontId::proportional(16.0),
                    egui::Color32::from_rgb(0x8b, 0x94, 0x9e),
                );
                if plus_resp.clicked() {
                    action.new_tab = true;
                }
            });
        });

    action
}
