use std::collections::HashSet;

const BACKDROP_ALPHA: u8 = 153;
const PANEL: egui::Color32 = egui::Color32::from_rgb(0x16, 0x1b, 0x22);
const BORDER: egui::Color32 = egui::Color32::from_rgb(0x30, 0x36, 0x3d);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe6, 0xed, 0xf3);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x94, 0x9e);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xff);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchTarget {
    pub id: String,
    pub label: String,
    pub identity: String,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchResult {
    pub sent: Vec<String>,
    pub failed: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchCommandAction {
    None,
    Close,
    Execute {
        command: String,
        tab_ids: Vec<String>,
    },
}

#[derive(Default)]
pub struct BatchCommandDialog {
    visible: bool,
    command: String,
    selected: HashSet<String>,
    result: Option<BatchResult>,
    request_focus: bool,
}

impl BatchCommandDialog {
    pub fn open(&mut self, targets: &[BatchTarget]) {
        self.visible = true;
        self.selected = targets
            .iter()
            .filter(|target| target.connected)
            .map(|target| target.id.clone())
            .collect();
        self.result = None;
        self.request_focus = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.request_focus = false;
    }

    pub fn is_open(&self) -> bool {
        self.visible
    }

    pub fn set_result(&mut self, result: BatchResult) {
        self.result = Some(result);
    }

    pub fn show(&mut self, ctx: &egui::Context, targets: &[BatchTarget]) -> BatchCommandAction {
        if !self.visible {
            return BatchCommandAction::None;
        }
        self.reconcile_targets(targets);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close();
            return BatchCommandAction::Close;
        }

        let screen = ctx.input(|input| input.screen_rect());
        let backdrop_clicked = egui::Area::new(egui::Id::new("batch_command_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_black_alpha(BACKDROP_ALPHA),
                );
                response.clicked()
            })
            .inner;

        let mut action = BatchCommandAction::None;
        let mut close_clicked = false;
        egui::Window::new("批量命令")
            .id(egui::Id::new("batch_command_dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(egui::vec2(480.0, 430.0))
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(8.0)
                    .inner_margin(14.0),
            )
            .show(ctx, |ui| {
                let connected = targets.iter().filter(|target| target.connected).count();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "目标终端 ({}/{connected})",
                            self.selected.len()
                        ))
                        .size(12.0)
                        .strong()
                        .color(TEXT),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let all_selected = connected > 0 && self.selected.len() == connected;
                        if ui
                            .small_button(if all_selected {
                                "取消全选"
                            } else {
                                "全选"
                            })
                            .clicked()
                        {
                            if all_selected {
                                self.selected.clear();
                            } else {
                                self.selected = targets
                                    .iter()
                                    .filter(|target| target.connected)
                                    .map(|target| target.id.clone())
                                    .collect();
                            }
                        }
                    });
                });
                ui.add_space(6.0);

                egui::ScrollArea::vertical()
                    .id_salt("batch_target_list")
                    .max_height(210.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if targets.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("暂无打开的 SSH 连接").color(MUTED));
                            });
                        }
                        for target in targets {
                            let mut checked = self.selected.contains(&target.id);
                            ui.add_enabled_ui(target.connected, |ui| {
                                egui::Frame::new()
                                    .fill(egui::Color32::from_rgb(0x0d, 0x11, 0x17))
                                    .stroke(egui::Stroke::new(1.0, BORDER))
                                    .corner_radius(4.0)
                                    .inner_margin(egui::Margin::symmetric(8, 5))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if ui.checkbox(&mut checked, "").changed() {
                                                if checked {
                                                    self.selected.insert(target.id.clone());
                                                } else {
                                                    self.selected.remove(&target.id);
                                                }
                                            }
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new(&target.label)
                                                        .size(12.0)
                                                        .color(TEXT),
                                                );
                                                ui.label(
                                                    egui::RichText::new(if target.connected {
                                                        target.identity.as_str()
                                                    } else {
                                                        "尚未连接"
                                                    })
                                                    .size(10.0)
                                                    .color(MUTED),
                                                );
                                            });
                                        });
                                    });
                            });
                            ui.add_space(4.0);
                        }
                    });

                ui.add_space(8.0);
                ui.label(egui::RichText::new("命令").size(12.0).color(MUTED));
                let edit = ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(&mut self.command)
                        .hint_text("输入要发送的命令…")
                        .text_color(TEXT),
                );
                if self.request_focus {
                    edit.request_focus();
                    self.request_focus = false;
                }
                ui.label(
                    egui::RichText::new("命令会立即写入目标终端当前交互行并执行")
                        .size(10.0)
                        .color(egui::Color32::from_rgb(0xf1, 0xfa, 0x8c)),
                );

                let valid = !self.command.trim().is_empty() && !self.selected.is_empty();
                let enter = edit.has_focus()
                    && ui.input(|input| {
                        input.events.iter().any(|event| match event {
                            egui::Event::Key {
                                key,
                                pressed,
                                repeat,
                                ..
                            } => accepts_submit_key(*key, *pressed, *repeat),
                            _ => false,
                        })
                    });
                let execute_clicked = ui
                    .add_enabled(
                        valid,
                        egui::Button::new(egui::RichText::new("发送").strong().color(ACCENT))
                            .min_size(egui::vec2(ui.available_width(), 28.0)),
                    )
                    .clicked();
                if valid && (enter || execute_clicked) {
                    action = self.execute_action(targets);
                }

                if let Some(result) = &self.result {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("已发送到 {} 个终端", result.sent.len()))
                            .size(11.0)
                            .color(TEXT),
                    );
                    if !result.failed.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("发送失败：{}", result.failed.join("、")))
                                .size(10.0)
                                .color(egui::Color32::from_rgb(0xff, 0x6b, 0x6b)),
                        );
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close_clicked = ui.button("关闭").clicked();
                });
            });

        if backdrop_clicked || close_clicked {
            self.close();
            return BatchCommandAction::Close;
        }
        action
    }

    fn reconcile_targets(&mut self, targets: &[BatchTarget]) {
        let connected = targets
            .iter()
            .filter(|target| target.connected)
            .map(|target| target.id.as_str())
            .collect::<HashSet<_>>();
        self.selected
            .retain(|tab_id| connected.contains(tab_id.as_str()));
    }

    fn execute_action(&self, targets: &[BatchTarget]) -> BatchCommandAction {
        let mut tab_ids = targets
            .iter()
            .filter(|target| target.connected && self.selected.contains(&target.id))
            .map(|target| target.id.clone())
            .collect::<Vec<_>>();
        tab_ids.sort();
        if self.command.trim().is_empty() || tab_ids.is_empty() {
            BatchCommandAction::None
        } else {
            BatchCommandAction::Execute {
                command: self.command.trim().to_owned(),
                tab_ids,
            }
        }
    }
}

fn accepts_submit_key(key: egui::Key, pressed: bool, repeat: bool) -> bool {
    key == egui::Key::Enter && pressed && !repeat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets() -> Vec<BatchTarget> {
        vec![
            BatchTarget {
                id: "b".into(),
                label: "B".into(),
                identity: "bob@b:22".into(),
                connected: true,
            },
            BatchTarget {
                id: "a".into(),
                label: "A".into(),
                identity: "alice@a:22".into(),
                connected: true,
            },
            BatchTarget {
                id: "pending".into(),
                label: "连接中".into(),
                identity: "p@host:22".into(),
                connected: false,
            },
        ]
    }

    #[test]
    fn open_selects_only_connected_targets() {
        let mut dialog = BatchCommandDialog::default();
        dialog.open(&targets());
        assert!(dialog.is_open());
        assert_eq!(dialog.selected, HashSet::from(["a".into(), "b".into()]));
    }

    #[test]
    fn reconcile_drops_closed_and_disconnected_targets() {
        let mut dialog = BatchCommandDialog::default();
        dialog.open(&targets());
        dialog.reconcile_targets(&[BatchTarget {
            id: "a".into(),
            label: "A".into(),
            identity: "alice@a:22".into(),
            connected: false,
        }]);
        assert!(dialog.selected.is_empty());
    }

    #[test]
    fn execute_is_stable_and_rejects_empty_input_or_selection() {
        let mut dialog = BatchCommandDialog::default();
        dialog.open(&targets());
        assert_eq!(dialog.execute_action(&targets()), BatchCommandAction::None);

        dialog.command = "  uname -a  ".into();
        assert_eq!(
            dialog.execute_action(&targets()),
            BatchCommandAction::Execute {
                command: "uname -a".into(),
                tab_ids: vec!["a".into(), "b".into()],
            }
        );

        dialog.selected.clear();
        assert_eq!(dialog.execute_action(&targets()), BatchCommandAction::None);
    }

    #[test]
    fn result_can_be_replaced_without_closing_dialog() {
        let mut dialog = BatchCommandDialog::default();
        dialog.open(&targets());
        dialog.set_result(BatchResult {
            sent: vec!["A".into()],
            failed: vec!["B".into()],
        });
        assert!(dialog.is_open());
        assert_eq!(dialog.result.as_ref().unwrap().failed, ["B"]);
    }

    #[test]
    fn repeated_or_released_enter_does_not_submit_again() {
        assert!(accepts_submit_key(egui::Key::Enter, true, false));
        assert!(!accepts_submit_key(egui::Key::Enter, true, true));
        assert!(!accepts_submit_key(egui::Key::Enter, false, false));
        assert!(!accepts_submit_key(egui::Key::Tab, true, false));
    }
}
