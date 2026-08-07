use crate::tab_manager::{PaneStatus, TabManager, TabPlacement, TabType, MAX_TAB_LABEL_CHARS};
use egui;

pub enum TabBarAction {
    None,
    SwitchTo(usize),
    Close(usize),
    CloseOthers(usize),
    Duplicate(usize),
    Rename(usize),
    Reconnect(usize),
    Reorder {
        dragged_id: String,
        target_id: String,
        placement: TabPlacement,
    },
    OpenBatch,
    OpenTunnels,
    OpenSettings,
    NewTab,
    ToggleMaximize,
    MinimizeWindow,
    CloseWindow,
}

#[derive(Default, Debug)]
pub struct TabDragState {
    dragged_id: Option<String>,
    insertion: Option<(String, TabPlacement, f32)>,
    title_drag_rect: Option<egui::Rect>,
}

impl TabDragState {
    pub(crate) fn title_drag_contains(&self, position: egui::Pos2) -> bool {
        self.title_drag_rect
            .is_some_and(|rect| rect.contains(position))
    }
}

pub const TAB_BAR_HEIGHT: f32 = 38.0;
const TITLE_ACTION_WIDTH: f32 = 40.0;
const TITLE_ACTIONS_RESERVED_WIDTH: f32 = 210.0;
const TAB_ITEM_SPACING: f32 = 2.0;
const TAB_PLUS_WIDTH: f32 = 42.0;
const TAB_WIDTH_MIN: f32 = 72.0;
const TAB_WIDTH_MAX: f32 = 176.0;

/// Compute a tab width that fills the tab strip without encroaching on the
/// fixed action area at the right side of the title bar.
///
/// `TAB_WIDTH_MIN` is the preferred lower bound for normal layouts. If the
/// window is narrower than that allows, the width is reduced further instead
/// of allowing the tab row to overlap the global controls.
fn responsive_tab_width(strip_width: f32, tab_count: usize) -> f32 {
    if tab_count == 0 {
        return TAB_WIDTH_MAX;
    }

    let count = tab_count as f32;
    let gaps = count * TAB_ITEM_SPACING;
    let fitting_width = (strip_width - TAB_PLUS_WIDTH - gaps).max(0.0) / count;
    if fitting_width >= TAB_WIDTH_MIN {
        fitting_width.min(TAB_WIDTH_MAX)
    } else {
        fitting_width
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRenameRequest {
    pub tab_id: String,
    pub label: String,
}

#[derive(Default)]
pub struct TabRenameDialog {
    target_id: Option<String>,
    value: String,
    request_focus: bool,
}

impl TabRenameDialog {
    pub fn open(&mut self, tab_id: String, label: String) {
        self.target_id = Some(tab_id);
        self.value = label;
        self.request_focus = true;
    }

    pub fn is_open(&self) -> bool {
        self.target_id.is_some()
    }

    fn close(&mut self) {
        self.target_id = None;
        self.value.clear();
        self.request_focus = false;
    }

    pub fn render(&mut self, ctx: &egui::Context) -> Option<TabRenameRequest> {
        let target_id = self.target_id.as_ref()?.clone();
        let (escape_pressed, enter_pressed) = ctx.input_mut(|input| {
            let escape = input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            let enter = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            (escape, enter)
        });
        let screen = ctx.input(|input| input.screen_rect);
        let mut submit = false;
        let mut cancel = escape_pressed;

        egui::Area::new(egui::Id::new("tab_rename_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110),
                );
            });

        egui::Window::new("重命名标签页")
            .id(egui::Id::new("tab_rename_dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(360.0, 124.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("标签名称")
                        .size(12.0)
                        .color(egui::Color32::from_rgb(0x8b, 0x94, 0x9e)),
                );
                let response = ui.add_sized(
                    egui::vec2(ui.available_width(), 28.0),
                    egui::TextEdit::singleline(&mut self.value)
                        .desired_width(f32::INFINITY)
                        .char_limit(MAX_TAB_LABEL_CHARS)
                        .text_color(egui::Color32::from_rgb(0xe6, 0xed, 0xf3))
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                if self.request_focus {
                    response.request_focus();
                    if let Some(mut edit_state) = egui::TextEdit::load_state(ui.ctx(), response.id)
                    {
                        edit_state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.value.chars().count()),
                            )));
                        egui::TextEdit::store_state(ui.ctx(), response.id, edit_state);
                    }
                    self.request_focus = false;
                }

                let valid = !self.value.trim().is_empty();
                if enter_pressed && valid {
                    submit = true;
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(valid, egui::Button::new("确定")).clicked() {
                        submit = true;
                    }
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            self.close();
            return None;
        }
        if submit {
            let request = TabRenameRequest {
                tab_id: target_id,
                label: self.value.clone(),
            };
            self.close();
            return Some(request);
        }
        None
    }
}

/// 右键菜单状态
struct ContextMenuState {
    tab_idx: usize,
    capabilities: ContextMenuCapabilities,
}

#[derive(Clone, Copy)]
struct ContextMenuCapabilities {
    can_duplicate_terminal: bool,
    can_reconnect: bool,
}

fn context_menu_capabilities(tab_type: &TabType) -> ContextMenuCapabilities {
    ContextMenuCapabilities {
        can_duplicate_terminal: matches!(tab_type, TabType::Local { .. } | TabType::Ssh { .. }),
        can_reconnect: matches!(tab_type, TabType::Ssh { .. } | TabType::Serial { .. }),
    }
}

fn tab_dot_color(tab_type: &TabType, status: &PaneStatus) -> egui::Color32 {
    match tab_type {
        TabType::Local { .. } => egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
        TabType::Ssh { .. } => egui::Color32::from_rgb(0x00, 0xd4, 0xff),
        TabType::Process { .. } => egui::Color32::from_rgb(0xbc, 0x8c, 0xff),
        TabType::Network { .. } => egui::Color32::from_rgb(0xbc, 0x8c, 0xff),
        TabType::Serial { .. } => match status {
            PaneStatus::Connected => egui::Color32::from_rgb(0x3f, 0xb9, 0x50),
            PaneStatus::Connecting => egui::Color32::from_rgb(0xf1, 0xc4, 0x0f),
            PaneStatus::Idle => egui::Color32::from_rgb(0x6e, 0x76, 0x81),
            PaneStatus::Failed(_) => egui::Color32::from_rgb(0xf8, 0x51, 0x49),
        },
        TabType::Recording { .. } => egui::Color32::from_rgb(0xff, 0x7b, 0x72),
        TabType::Settings => egui::Color32::from_rgb(0x40, 0xcb, 0xd9),
    }
}

#[derive(Clone, Copy)]
enum TopActionIcon {
    Tools,
    Settings,
}

#[derive(Clone, Copy)]
enum WindowActionIcon {
    Minimize,
    Maximize,
    Close,
}

fn window_action_button(ui: &mut egui::Ui, icon: WindowActionIcon) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(TITLE_ACTION_WIDTH, TAB_BAR_HEIGHT),
        egui::Sense::click(),
    );
    let hovered = response.hovered();
    let close = matches!(icon, WindowActionIcon::Close);
    if hovered {
        ui.painter().rect_filled(
            rect,
            0.0,
            if close {
                egui::Color32::from_rgb(0xc4, 0x2b, 0x1c)
            } else {
                egui::Color32::from_rgb(0x2a, 0x31, 0x3a)
            },
        );
    }
    let color = if hovered {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(0xb7, 0xc0, 0xca)
    };
    let stroke = egui::Stroke::new(1.2, color);
    let center = rect.center();
    match icon {
        WindowActionIcon::Minimize => {
            ui.painter().line_segment(
                [
                    center + egui::vec2(-6.0, 3.0),
                    center + egui::vec2(6.0, 3.0),
                ],
                stroke,
            );
        }
        WindowActionIcon::Maximize => {
            ui.painter().rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(11.0, 10.0)),
                0.0,
                stroke,
                egui::epaint::StrokeKind::Inside,
            );
        }
        WindowActionIcon::Close => {
            ui.painter().line_segment(
                [
                    center + egui::vec2(-5.0, -5.0),
                    center + egui::vec2(5.0, 5.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(5.0, -5.0),
                    center + egui::vec2(-5.0, 5.0),
                ],
                stroke,
            );
        }
    }
    response
}

fn top_action_button(
    ui: &mut egui::Ui,
    icon: TopActionIcon,
    tooltip: &'static str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(TITLE_ACTION_WIDTH, TAB_BAR_HEIGHT),
        egui::Sense::click(),
    );
    let response = response.on_hover_text(tooltip);
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_rgb(0x2a, 0x31, 0x3a));
    }
    let color = if response.hovered() {
        egui::Color32::from_rgb(0x00, 0xd4, 0xff)
    } else {
        egui::Color32::from_rgb(0x8b, 0x94, 0x9e)
    };
    let stroke = egui::Stroke::new(1.25, color);
    let center = rect.center();
    match icon {
        TopActionIcon::Tools => {
            let body =
                egui::Rect::from_center_size(center + egui::vec2(0.0, 2.0), egui::vec2(15.0, 10.0));
            ui.painter()
                .rect_stroke(body, 2.0, stroke, egui::epaint::StrokeKind::Inside);
            ui.painter().line_segment(
                [
                    center + egui::vec2(-4.0, -5.0),
                    center + egui::vec2(-2.0, -7.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(-2.0, -7.0),
                    center + egui::vec2(2.0, -7.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(2.0, -7.0),
                    center + egui::vec2(4.0, -5.0),
                ],
                stroke,
            );
        }
        TopActionIcon::Settings => {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                "⚙",
                egui::FontId::proportional(15.0),
                color,
            );
        }
    }
    response
}

pub fn render_tab_bar(
    ctx: &egui::Context,
    tab_manager: &TabManager,
    drag: &mut TabDragState,
) -> TabBarAction {
    let mut action = TabBarAction::None;
    drag.title_drag_rect = None;

    // 持久化的右键菜单状态
    let menu_id = egui::Id::new("tab_context_menu");
    let mut show_menu: Option<ContextMenuState> = None;

    egui::TopBottomPanel::top("tab_bar")
        .exact_height(TAB_BAR_HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x16, 0x1b, 0x22))
                .inner_margin(egui::Margin::symmetric(4, 0)),
        )
        .show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let tabs_right =
                (panel_rect.right() - TITLE_ACTIONS_RESERVED_WIDTH).max(panel_rect.left() + 1.0);
            let tabs_rect = egui::Rect::from_min_max(
                panel_rect.left_top(),
                egui::pos2(tabs_right, panel_rect.bottom()),
            );
            let controls_rect = egui::Rect::from_min_max(
                egui::pos2(tabs_right, panel_rect.top()),
                panel_rect.right_bottom(),
            );
            ui.painter().rect_filled(
                controls_rect,
                0.0,
                egui::Color32::from_rgb(0x13, 0x18, 0x20),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(tabs_right, panel_rect.top() + 5.0),
                    egui::pos2(tabs_right, panel_rect.bottom() - 5.0),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d)),
            );

            let tab_width = responsive_tab_width(tabs_rect.width(), tab_manager.tabs.len());
            let mut strip_right = tabs_rect.left();
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(tabs_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = TAB_ITEM_SPACING;
                    ui.set_clip_rect(ui.clip_rect().intersect(tabs_rect));
                    let mut drag_target_seen = false;

                    for (i, tab) in tab_manager.tabs.iter().enumerate() {
                        let is_active = i == tab_manager.active_idx;

                        let dot_color = tab_dot_color(&tab.tab_type, &tab.status);

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

                        let (tab_rect, tab_resp) = ui.allocate_exact_size(
                            egui::vec2(tab_width, TAB_BAR_HEIGHT - 6.0),
                            egui::Sense::click_and_drag(),
                        );

                        // 背景
                        ui.painter().rect_filled(tab_rect, 4.0, bg);
                        if tab_resp.hovered() && !is_active {
                            ui.painter().rect_filled(
                                tab_rect,
                                4.0,
                                egui::Color32::from_rgba_unmultiplied(0x30, 0x36, 0x3d, 0x60),
                            );
                        }

                        // 类型圆点
                        let dot_center = egui::pos2(tab_rect.left() + 12.0, tab_rect.center().y);
                        ui.painter().circle_filled(dot_center, 3.0, dot_color);

                        // 标签名只允许绘制到关闭按钮左侧。长的 SSH/进程标题不能覆盖 × 和 +。
                        let text_pos = egui::pos2(tab_rect.left() + 22.0, tab_rect.center().y);
                        let label_clip = egui::Rect::from_min_max(
                            egui::pos2(tab_rect.left() + 20.0, tab_rect.top()),
                            egui::pos2(tab_rect.right() - 22.0, tab_rect.bottom()),
                        );
                        ui.painter().with_clip_rect(label_clip).text(
                            text_pos,
                            egui::Align2::LEFT_CENTER,
                            &tab.label,
                            egui::FontId::proportional(13.0),
                            text_color,
                        );

                        // × 关闭按钮
                        let close_rect = egui::Rect::from_min_size(
                            egui::pos2(tab_rect.right() - 20.0, tab_rect.top()),
                            egui::vec2(20.0, tab_rect.height()),
                        );
                        let close_resp = ui.interact(
                            close_rect,
                            egui::Id::new(("tab_close", i)),
                            egui::Sense::click(),
                        );

                        let close_color = if close_resp.hovered() {
                            egui::Color32::from_rgb(0xf8, 0x51, 0x49)
                        } else {
                            egui::Color32::from_rgb(0x48, 0x4f, 0x58)
                        };
                        ui.painter().text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "×",
                            egui::FontId::proportional(16.0),
                            close_color,
                        );

                        // 事件
                        if close_resp.clicked() {
                            action = TabBarAction::Close(i);
                        } else if tab_resp.clicked() {
                            action = TabBarAction::SwitchTo(i);
                        }
                        if tab_resp.drag_started()
                            && ctx
                                .input(|input| input.pointer.interact_pos())
                                .is_some_and(|position| !close_rect.contains(position))
                        {
                            drag.dragged_id = Some(tab.id.clone());
                            drag.insertion = None;
                        }
                        if drag.dragged_id.as_deref().is_some_and(|id| id != tab.id)
                            && tab_resp.hovered()
                        {
                            if let Some(position) = ctx.input(|input| input.pointer.interact_pos())
                            {
                                drag_target_seen = true;
                                let placement = if position.x < tab_rect.center().x {
                                    TabPlacement::Before
                                } else {
                                    TabPlacement::After
                                };
                                let x = match placement {
                                    TabPlacement::Before => tab_rect.left(),
                                    TabPlacement::After => tab_rect.right(),
                                };
                                drag.insertion = Some((tab.id.clone(), placement, x));
                            }
                        }
                        if tab_resp.middle_clicked() {
                            action = TabBarAction::Close(i);
                        }

                        // 右键菜单触发：记住鼠标位置
                        if tab_resp.secondary_clicked() {
                            let capabilities = context_menu_capabilities(&tab.tab_type);
                            let pos = ctx.input(|i| i.pointer.interact_pos().unwrap_or_default());
                            show_menu = Some(ContextMenuState {
                                tab_idx: i,
                                capabilities,
                            });
                            ctx.memory_mut(|mem| {
                                mem.data.insert_temp(
                                    menu_id,
                                    (
                                        i,
                                        capabilities.can_duplicate_terminal,
                                        capabilities.can_reconnect,
                                        pos,
                                    ),
                                )
                            });
                        }
                    }
                    if drag.dragged_id.is_some() && !drag_target_seen {
                        drag.insertion = None;
                    }

                    // [+] 按钮
                    let (plus_rect, plus_resp) = ui.allocate_exact_size(
                        egui::vec2(TAB_PLUS_WIDTH, TAB_BAR_HEIGHT - 4.0),
                        egui::Sense::click(),
                    );
                    if plus_resp.hovered() {
                        ui.painter().rect_filled(
                            plus_rect,
                            5.0,
                            egui::Color32::from_rgba_unmultiplied(0x00, 0xd4, 0xff, 0x24),
                        );
                    }
                    ui.painter().text(
                        plus_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "+",
                        egui::FontId::proportional(26.0),
                        if plus_resp.hovered() {
                            egui::Color32::from_rgb(0x00, 0xd4, 0xff)
                        } else {
                            egui::Color32::from_rgb(0xc9, 0xd1, 0xd9)
                        },
                    );
                    if plus_resp.clicked() {
                        action = TabBarAction::NewTab;
                    }
                    strip_right = plus_rect.right();
                },
            );
            // Only the genuinely blank part of the bar is draggable. Registering the whole
            // title bar would steal tab reorder and close-button gestures.
            let drag_right = tabs_rect.right();
            if drag_right > strip_right + 4.0 {
                let drag_rect = egui::Rect::from_min_max(
                    egui::pos2(strip_right + 4.0, ui.max_rect().top()),
                    egui::pos2(drag_right, ui.max_rect().bottom()),
                );
                let title_drag = ui.interact(
                    drag_rect,
                    egui::Id::new("custom_title_bar_drag_region"),
                    egui::Sense::click_and_drag(),
                );
                drag.title_drag_rect = Some(drag_rect);
                if title_drag.double_clicked() {
                    action = TabBarAction::ToggleMaximize;
                }
            }
            if let Some((_, _, x)) = &drag.insertion {
                let rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(*x, rect.top() + 2.0),
                        egui::pos2(*x, rect.bottom() - 2.0),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(0x00, 0xd4, 0xff)),
                );
            }
        });

    let tools_menu_id = egui::Id::new("tab_bar_tools_menu_open");
    let mut tools_opened_this_frame = false;
    egui::Area::new(egui::Id::new("tab_bar_top_actions"))
        .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if top_action_button(ui, TopActionIcon::Tools, "工具").clicked() {
                    ctx.memory_mut(|memory| {
                        let open = memory.data.get_temp::<bool>(tools_menu_id).unwrap_or(false);
                        memory.data.insert_temp(tools_menu_id, !open);
                    });
                    tools_opened_this_frame = true;
                }
                if top_action_button(ui, TopActionIcon::Settings, "设置").clicked() {
                    action = TabBarAction::OpenSettings;
                }
                let separator = ui
                    .allocate_exact_size(egui::vec2(8.0, TAB_BAR_HEIGHT), egui::Sense::hover())
                    .0;
                ui.painter().line_segment(
                    [
                        egui::pos2(separator.center().x, separator.center().y - 9.0),
                        egui::pos2(separator.center().x, separator.center().y + 9.0),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d)),
                );
                if window_action_button(ui, WindowActionIcon::Minimize)
                    .on_hover_text("最小化")
                    .clicked()
                {
                    action = TabBarAction::MinimizeWindow;
                }
                if window_action_button(ui, WindowActionIcon::Maximize)
                    .on_hover_text("最大化 / 还原")
                    .clicked()
                {
                    action = TabBarAction::ToggleMaximize;
                }
                if window_action_button(ui, WindowActionIcon::Close)
                    .on_hover_text("关闭")
                    .clicked()
                {
                    action = TabBarAction::CloseWindow;
                }
            });
        });

    if ctx.memory(|memory| memory.data.get_temp::<bool>(tools_menu_id).unwrap_or(false)) {
        let screen = ctx.input(|input| input.screen_rect);
        let width = 176.0;
        let position = egui::pos2(
            (screen.right() - TITLE_ACTIONS_RESERVED_WIDTH + TITLE_ACTION_WIDTH - width)
                .max(screen.left()),
            TAB_BAR_HEIGHT,
        );
        let menu = egui::Window::new("tools_menu")
            .id(egui::Id::new("tab_bar_tools_menu"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .movable(false)
            .fixed_pos(position)
            .default_width(width)
            .min_width(width)
            .max_width(width)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x1c, 0x20, 0x28))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                    ))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(4)),
            )
            .show(ctx, |ui| {
                let item = |ui: &mut egui::Ui, label: &str| {
                    ui.add_sized(
                        egui::vec2(width - 8.0, 30.0),
                        egui::Button::new(label).frame(false),
                    )
                };
                if item(ui, "批量命令").clicked() {
                    action = TabBarAction::OpenBatch;
                }
                if item(ui, "SSH 隧道").clicked() {
                    action = TabBarAction::OpenTunnels;
                }
            });
        let menu_rect = menu.map(|response| response.response.rect);
        let close = matches!(&action, TabBarAction::OpenBatch | TabBarAction::OpenTunnels)
            || (!tools_opened_this_frame
                && ctx.input(|input| {
                    input.pointer.any_pressed()
                        && input.pointer.interact_pos().is_some_and(|position| {
                            menu_rect.is_none_or(|rect| !rect.contains(position))
                        })
                }));
        if close {
            ctx.memory_mut(|memory| memory.data.insert_temp(tools_menu_id, false));
        }
    }

    if ctx.input(|input| input.pointer.any_released()) {
        if let (Some(dragged_id), Some((target_id, placement, _))) =
            (drag.dragged_id.take(), drag.insertion.take())
        {
            action = TabBarAction::Reorder {
                dragged_id,
                target_id,
                placement,
            };
        } else {
            drag.dragged_id = None;
            drag.insertion = None;
        }
    }

    // 右键菜单窗口（在 panel 外渲染，避免被裁剪）
    let menu_state: Option<(usize, bool, bool, egui::Pos2)> =
        ctx.memory(|mem| mem.data.get_temp(menu_id));
    if show_menu.is_some() || menu_state.is_some() {
        let (tab_idx, can_duplicate_terminal, can_reconnect, menu_pos) = show_menu
            .map(|m| {
                let pos = ctx.input(|i| i.pointer.interact_pos().unwrap_or_default());
                (
                    m.tab_idx,
                    m.capabilities.can_duplicate_terminal,
                    m.capabilities.can_reconnect,
                    pos,
                )
            })
            .or(menu_state)
            .unwrap();

        let mut close_menu = false;
        egui::Window::new("tab_menu")
            .id(egui::Id::new("tab_context_menu_window"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .movable(false)
            .fixed_pos(menu_pos)
            .fixed_size(egui::vec2(150.0, 0.0))
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0x1c, 0x20, 0x28))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                    ))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(4)),
            )
            .show(ctx, |ui| {
                ui.style_mut().visuals.override_text_color =
                    Some(egui::Color32::from_rgb(0xc9, 0xd1, 0xd9));

                let item = |ui: &mut egui::Ui, label: &str| -> bool {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(142.0, 26.0), egui::Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            3.0,
                            egui::Color32::from_rgb(0x30, 0x36, 0x3d),
                        );
                    }
                    ui.painter().text(
                        rect.left_center() + egui::vec2(8.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(12.0),
                        egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
                    );
                    resp.clicked()
                };

                if item(ui, "重命名") {
                    action = TabBarAction::Rename(tab_idx);
                    close_menu = true;
                }
                if can_duplicate_terminal {
                    if item(ui, "复制标签页") {
                        action = TabBarAction::Duplicate(tab_idx);
                        close_menu = true;
                    }
                }
                if can_reconnect {
                    ui.separator();
                    if item(ui, "重新连接") {
                        action = TabBarAction::Reconnect(tab_idx);
                        close_menu = true;
                    }
                }
                ui.separator();
                if item(ui, "关闭") {
                    action = TabBarAction::Close(tab_idx);
                    close_menu = true;
                }
                if item(ui, "关闭其他") {
                    action = TabBarAction::CloseOthers(tab_idx);
                    close_menu = true;
                }
            });

        // 选择了菜单项 → 关闭
        if close_menu {
            ctx.memory_mut(|mem| mem.data.remove::<(usize, bool, bool, egui::Pos2)>(menu_id));
        }
        // 点击菜单外关闭（下一帧检测到非菜单区域的 click）
        if !close_menu
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().map_or(false, |p| {
                        let menu_rect =
                            egui::Rect::from_min_size(menu_pos, egui::vec2(150.0, 160.0));
                        !menu_rect.contains(p)
                    })
            })
        {
            ctx.memory_mut(|mem| mem.data.remove::<(usize, bool, bool, egui::Pos2)>(menu_id));
        }
    }

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn render_rename_frame(
        ctx: &egui::Context,
        dialog: &mut TabRenameDialog,
        events: Vec<egui::Event>,
    ) -> Option<TabRenameRequest> {
        let mut request = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| request = dialog.render(ctx),
        );
        request
    }

    fn process_tab_type() -> TabType {
        TabType::Process {
            label: "远端进程".into(),
            key: crate::monitor::MonitorKey::remote("alice", "process.example", 22),
            params: Some(crate::ssh::ConnectionParams {
                host: "process.example".into(),
                port: 22,
                user: "alice".into(),
                auth: "key".into(),
                key_path: String::new(),
                password: String::new(),
            }),
        }
    }

    #[test]
    fn process_tab_has_a_distinct_visual_identity() {
        let process = tab_dot_color(&process_tab_type(), &PaneStatus::Connected);
        let local = tab_dot_color(
            &TabType::Local {
                shell_path: "sh".into(),
            },
            &PaneStatus::Connected,
        );
        let ssh = tab_dot_color(
            &TabType::Ssh {
                label: "SSH".into(),
                params: crate::ssh::ConnectionParams {
                    host: "ssh.example".into(),
                    port: 22,
                    user: "alice".into(),
                    auth: "key".into(),
                    key_path: String::new(),
                    password: String::new(),
                },
            },
            &PaneStatus::Connected,
        );

        assert_ne!(process, local);
        assert_ne!(process, ssh);
    }

    #[test]
    fn process_tab_has_no_terminal_duplicate_or_reconnect_actions() {
        let capabilities = context_menu_capabilities(&process_tab_type());

        assert!(!capabilities.can_duplicate_terminal);
        assert!(!capabilities.can_reconnect);
    }

    #[test]
    fn serial_tab_supports_reconnect_without_duplicate() {
        let capabilities = context_menu_capabilities(&TabType::Serial {
            spec: crate::serial::SerialSpec {
                device: "/dev/ttyUSB1".into(),
                display_name: "FT232R USB UART".into(),
                serial_number: Some("A10LCL3D".into()),
                baud_rate: crate::serial::DEFAULT_BAUD_RATE,
            },
        });

        assert!(!capabilities.can_duplicate_terminal);
        assert!(capabilities.can_reconnect);
    }

    #[test]
    fn serial_tab_dot_reflects_connection_state() {
        let serial = TabType::Serial {
            spec: crate::serial::SerialSpec {
                device: "/dev/ttyUSB1".into(),
                display_name: "FT232R USB UART".into(),
                serial_number: Some("A10LCL3D".into()),
                baud_rate: crate::serial::DEFAULT_BAUD_RATE,
            },
        };

        let connected = tab_dot_color(&serial, &PaneStatus::Connected);
        let connecting = tab_dot_color(&serial, &PaneStatus::Connecting);
        let disconnected = tab_dot_color(&serial, &PaneStatus::Idle);
        let failed = tab_dot_color(&serial, &PaneStatus::Failed("打开失败".into()));

        assert_ne!(connected, connecting);
        assert_ne!(connected, disconnected);
        assert_ne!(disconnected, failed);
    }

    #[test]
    fn title_actions_share_one_hit_target_size_and_vertical_center() {
        let ctx = egui::Context::default();
        let mut rects = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 100.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        rects.push(top_action_button(ui, TopActionIcon::Settings, "设置").rect);
                        rects.push(window_action_button(ui, WindowActionIcon::Minimize).rect);
                        rects.push(window_action_button(ui, WindowActionIcon::Maximize).rect);
                        rects.push(window_action_button(ui, WindowActionIcon::Close).rect);
                    });
                });
            },
        );

        assert!(rects.iter().all(|rect| rect.width() == TITLE_ACTION_WIDTH));
        assert!(rects.iter().all(|rect| rect.height() == TAB_BAR_HEIGHT));
        assert!(rects
            .windows(2)
            .all(|pair| pair[0].center().y == pair[1].center().y));
    }

    #[test]
    fn responsive_tab_width_reserves_plus_button_and_spacing() {
        let strip_width = 1_060.0;
        let tab_count = 9;
        let width = responsive_tab_width(strip_width, tab_count);
        let used = width * tab_count as f32 + TAB_ITEM_SPACING * tab_count as f32 + TAB_PLUS_WIDTH;

        assert!(width < TAB_WIDTH_MAX);
        assert!(width >= TAB_WIDTH_MIN);
        assert!(used <= strip_width + f32::EPSILON);
    }

    #[test]
    fn responsive_tab_width_caps_single_tab_at_the_normal_width() {
        assert_eq!(responsive_tab_width(1_060.0, 1), TAB_WIDTH_MAX);
    }

    #[test]
    fn responsive_tab_width_still_fits_when_the_strip_is_very_tight() {
        let strip_width = 180.0;
        let tab_count = 4;
        let width = responsive_tab_width(strip_width, tab_count);
        let used = width * tab_count as f32 + TAB_ITEM_SPACING * tab_count as f32 + TAB_PLUS_WIDTH;

        assert!(width < TAB_WIDTH_MIN);
        assert!(used <= strip_width + f32::EPSILON);
    }

    #[test]
    fn rename_dialog_open_tracks_target_prefills_label_and_close_resets_state() {
        let mut dialog = TabRenameDialog::default();
        dialog.open("tab-7".into(), "生产环境".into());

        assert!(dialog.is_open());
        assert_eq!(dialog.target_id.as_deref(), Some("tab-7"));
        assert_eq!(dialog.value, "生产环境");
        assert!(dialog.request_focus);

        dialog.close();
        assert!(!dialog.is_open());
        assert!(dialog.value.is_empty());
        assert!(!dialog.request_focus);
    }

    #[test]
    fn rename_dialog_submits_enter_on_its_first_focused_frame_once() {
        let ctx = egui::Context::default();
        let mut dialog = TabRenameDialog::default();
        dialog.open("tab-first".into(), "生产环境".into());

        let request = render_rename_frame(&ctx, &mut dialog, vec![key_event(egui::Key::Enter)]);

        assert_eq!(
            request,
            Some(TabRenameRequest {
                tab_id: "tab-first".into(),
                label: "生产环境".into(),
            })
        );
        assert!(!dialog.is_open());
        assert_eq!(render_rename_frame(&ctx, &mut dialog, Vec::new()), None);
    }

    #[test]
    fn rename_dialog_submits_enter_when_text_edit_already_has_focus() {
        let ctx = egui::Context::default();
        let mut dialog = TabRenameDialog::default();
        dialog.open("tab-focused".into(), "日志".into());

        assert_eq!(render_rename_frame(&ctx, &mut dialog, Vec::new()), None);
        assert!(ctx.memory(|memory| memory.focused().is_some()));

        let request = render_rename_frame(&ctx, &mut dialog, vec![key_event(egui::Key::Enter)]);
        assert_eq!(
            request,
            Some(TabRenameRequest {
                tab_id: "tab-focused".into(),
                label: "日志".into(),
            })
        );
        assert!(!dialog.is_open());
    }

    #[test]
    fn rename_dialog_whitespace_enter_keeps_dialog_open_and_focused() {
        let ctx = egui::Context::default();
        let mut dialog = TabRenameDialog::default();
        dialog.open("tab-blank".into(), " \t ".into());

        let request = render_rename_frame(&ctx, &mut dialog, vec![key_event(egui::Key::Enter)]);

        assert_eq!(request, None);
        assert!(dialog.is_open());
        assert_eq!(dialog.value, " \t ");
        assert!(ctx.memory(|memory| memory.focused().is_some()));
    }

    #[test]
    fn rename_dialog_escape_cancels_with_priority_over_enter() {
        let ctx = egui::Context::default();
        let mut dialog = TabRenameDialog::default();
        dialog.open("tab-cancel".into(), "不可提交".into());

        let request = render_rename_frame(
            &ctx,
            &mut dialog,
            vec![key_event(egui::Key::Enter), key_event(egui::Key::Escape)],
        );

        assert_eq!(request, None);
        assert!(!dialog.is_open());
    }
}
