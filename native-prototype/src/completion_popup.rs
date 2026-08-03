use crate::smart_completion::CompletionSessionKey;

pub const ROW_HEIGHT: f32 = 26.0;
pub const POPUP_WIDTH: f32 = 420.0;
pub const POPUP_MARGIN: f32 = 4.0;

#[derive(Clone, PartialEq)]
pub struct CompletionPopupSnapshot {
    pub tab_id: String,
    pub pane_id: String,
    pub session: CompletionSessionKey,
    pub bounds: egui::Rect,
    pub cursor: egui::Rect,
    pub candidates: Vec<String>,
    pub selected: usize,
}

impl std::fmt::Debug for CompletionPopupSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompletionPopupSnapshot")
            .field("tab_id", &self.tab_id)
            .field("pane_id", &self.pane_id)
            .field("session", &self.session)
            .field("bounds", &self.bounds)
            .field("cursor", &self.cursor)
            .field("candidates_len", &self.candidates.len())
            .field("selected", &self.selected)
            .finish()
    }
}

impl CompletionPopupSnapshot {
    pub fn new(
        tab_id: String,
        session: CompletionSessionKey,
        blocked: bool,
        bounds: egui::Rect,
        cursor: Option<egui::Rect>,
        candidates: Vec<String>,
        selected: usize,
    ) -> Option<Self> {
        Self::new_for_pane(
            tab_id.clone(),
            tab_id,
            session,
            blocked,
            bounds,
            cursor,
            candidates,
            selected,
        )
    }

    pub fn new_for_pane(
        tab_id: String,
        pane_id: String,
        session: CompletionSessionKey,
        blocked: bool,
        bounds: egui::Rect,
        cursor: Option<egui::Rect>,
        candidates: Vec<String>,
        selected: usize,
    ) -> Option<Self> {
        if blocked
            || candidates.is_empty()
            || !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || bounds.width() <= 0.0
            || bounds.height() <= 0.0
        {
            return None;
        }
        let cursor = cursor?;
        if !cursor.min.is_finite()
            || !cursor.max.is_finite()
            || !bounds.intersects(cursor)
            || cursor.width() <= 0.0
            || cursor.height() <= 0.0
        {
            return None;
        }
        let geometry = popup_geometry(bounds, cursor, candidates.len());
        if !geometry.position.is_finite()
            || !geometry.size.is_finite()
            || geometry.size.x <= 0.0
            || geometry.size.y < ROW_HEIGHT
        {
            return None;
        }
        let visible_count = candidates
            .len()
            .min((geometry.size.y / ROW_HEIGHT).floor() as usize);
        if visible_count == 0 {
            return None;
        }
        let global_selected = selected.min(candidates.len() - 1);
        let first_visible = global_selected
            .saturating_add(1)
            .saturating_sub(visible_count)
            .min(candidates.len() - visible_count);
        let candidates = candidates
            .into_iter()
            .skip(first_visible)
            .take(visible_count)
            .collect();
        Some(Self {
            tab_id,
            pane_id,
            session,
            bounds,
            cursor,
            selected: global_selected - first_visible,
            candidates,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupGeometry {
    pub position: egui::Pos2,
    pub size: egui::Vec2,
    pub opens_above: bool,
}

fn finite_rect(rect: egui::Rect) -> egui::Rect {
    if !rect.min.is_finite() || !rect.max.is_finite() {
        return egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::ZERO);
    }
    egui::Rect::from_min_max(
        egui::pos2(rect.min.x.min(rect.max.x), rect.min.y.min(rect.max.y)),
        egui::pos2(rect.min.x.max(rect.max.x), rect.min.y.max(rect.max.y)),
    )
}

pub fn popup_geometry(
    terminal_bounds: egui::Rect,
    cursor: egui::Rect,
    candidate_count: usize,
) -> PopupGeometry {
    let bounds = finite_rect(terminal_bounds);
    let cursor = finite_rect(cursor);
    let horizontal_margin = if bounds.width() >= POPUP_MARGIN * 2.0 {
        POPUP_MARGIN
    } else {
        0.0
    };
    let vertical_margin = if bounds.height() >= POPUP_MARGIN * 2.0 {
        POPUP_MARGIN
    } else {
        0.0
    };
    let available = egui::Rect::from_min_max(
        bounds.min + egui::vec2(horizontal_margin, vertical_margin),
        bounds.max - egui::vec2(horizontal_margin, vertical_margin),
    );
    let desired_height = ROW_HEIGHT * candidate_count as f32;
    let size = egui::vec2(
        POPUP_WIDTH.min(available.width()).max(0.0),
        desired_height.min(available.height()).max(0.0),
    );
    let cursor_x = if cursor.min.x.is_finite() {
        cursor.min.x
    } else {
        available.min.x
    };
    let below_y = cursor.max.y + POPUP_MARGIN;
    let opens_above = below_y + size.y > available.max.y;
    let desired_y = if opens_above {
        cursor.min.y - POPUP_MARGIN - size.y
    } else {
        below_y
    };
    let max_x = available.max.x - size.x;
    let max_y = available.max.y - size.y;

    PopupGeometry {
        position: egui::pos2(
            cursor_x.clamp(available.min.x, max_x),
            desired_y.clamp(available.min.y, max_y),
        ),
        size,
        opens_above,
    }
}

pub fn render(ctx: &egui::Context, snapshot: &CompletionPopupSnapshot) {
    let geometry = popup_geometry(snapshot.bounds, snapshot.cursor, snapshot.candidates.len());
    if geometry.size.x <= 0.0 || geometry.size.y <= 0.0 {
        return;
    }

    egui::Area::new(egui::Id::new((
        "bash_completion",
        &snapshot.tab_id,
        &snapshot.pane_id,
    )))
    .order(egui::Order::Foreground)
    .interactable(false)
    .fade_in(false)
    .fixed_pos(geometry.position)
    .show(ctx, |ui| {
        ui.set_min_size(geometry.size);
        ui.set_max_size(geometry.size);
        let popup_rect = egui::Rect::from_min_size(ui.min_rect().min, geometry.size);
        ui.painter().rect(
            popup_rect,
            4.0,
            egui::Color32::from_rgb(0x1c, 0x20, 0x28),
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d)),
            egui::StrokeKind::Inside,
        );
        let painter = ui.painter().with_clip_rect(popup_rect);
        for (index, candidate) in snapshot.candidates.iter().enumerate() {
            let row = egui::Rect::from_min_size(
                popup_rect.min + egui::vec2(0.0, index as f32 * ROW_HEIGHT),
                egui::vec2(geometry.size.x, ROW_HEIGHT),
            );
            if index == snapshot.selected {
                painter.rect_filled(row, 0.0, egui::Color32::from_rgb(0x30, 0x36, 0x3d));
            }
            painter.text(
                row.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                candidate,
                egui::FontId::monospace(13.0),
                egui::Color32::from_rgb(0xc9, 0xd1, 0xd9),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart_completion::CompletionSessionKey;

    fn popup_rect(geometry: PopupGeometry) -> egui::Rect {
        egui::Rect::from_min_size(geometry.position, geometry.size)
    }

    fn snapshot(
        blocked: bool,
        bounds: egui::Rect,
        cursor: Option<egui::Rect>,
        candidates: Vec<String>,
    ) -> Option<CompletionPopupSnapshot> {
        CompletionPopupSnapshot::new(
            "tab-a".into(),
            CompletionSessionKey::new_for_test(3, "session"),
            blocked,
            bounds,
            cursor,
            candidates,
            0,
        )
    }

    #[test]
    fn snapshot_rejects_blocked_empty_missing_and_offscreen_inputs() {
        let bounds = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 600.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(10.0, 20.0));

        assert!(snapshot(true, bounds, Some(cursor), vec!["git status".into()]).is_none());
        assert!(snapshot(false, bounds, Some(cursor), Vec::new()).is_none());
        assert!(snapshot(false, bounds, None, vec!["git status".into()]).is_none());
        assert!(snapshot(
            false,
            bounds,
            Some(egui::Rect::from_min_size(
                egui::pos2(900.0, 100.0),
                egui::vec2(10.0, 20.0),
            )),
            vec!["git status".into()],
        )
        .is_none());
    }

    #[test]
    fn snapshot_rejects_nonfinite_and_nonpositive_geometry() {
        let bounds = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 600.0));
        let nonfinite_cursor =
            egui::Rect::from_min_size(egui::pos2(f32::NAN, 100.0), egui::vec2(10.0, 20.0));
        assert!(snapshot(
            false,
            bounds,
            Some(nonfinite_cursor),
            vec!["git status".into()],
        )
        .is_none());
        assert!(snapshot(
            false,
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(0.0, 100.0)),
            Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 10.0),
                egui::vec2(1.0, 1.0),
            )),
            vec!["git status".into()],
        )
        .is_none());
    }

    #[test]
    fn valid_snapshot_preserves_interaction_identity_and_selection() {
        let bounds = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 600.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(10.0, 20.0));
        let session = CompletionSessionKey::new_for_test(3, "session");

        let snapshot = CompletionPopupSnapshot::new(
            "tab-a".into(),
            session.clone(),
            false,
            bounds,
            Some(cursor),
            vec!["git status".into(), "git log".into()],
            1,
        )
        .expect("valid popup snapshot");

        assert_eq!(snapshot.tab_id, "tab-a");
        assert_eq!(snapshot.session, session);
        assert_eq!(snapshot.bounds, bounds);
        assert_eq!(snapshot.cursor, cursor);
        assert_eq!(snapshot.candidates, ["git status", "git log"]);
        assert_eq!(snapshot.selected, 1);
    }

    #[test]
    fn snapshot_rejects_viewport_with_less_than_one_full_row() {
        let bounds = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, ROW_HEIGHT + POPUP_MARGIN * 2.0 - 1.0),
        );
        let cursor = egui::Rect::from_min_size(egui::pos2(2.0, 2.0), egui::vec2(2.0, 2.0));

        assert!(snapshot(false, bounds, Some(cursor), vec!["git status".into()],).is_none());
    }

    #[test]
    fn snapshot_windows_candidates_around_global_selection() {
        for visible_rows in [1, 2] {
            let bounds = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, ROW_HEIGHT * visible_rows as f32 + POPUP_MARGIN * 2.0),
            );
            let cursor = egui::Rect::from_min_size(egui::pos2(2.0, 2.0), egui::vec2(2.0, 2.0));

            let snapshot = CompletionPopupSnapshot::new(
                "tab-a".into(),
                CompletionSessionKey::new_for_test(3, "session"),
                false,
                bounds,
                Some(cursor),
                vec!["one".into(), "two".into(), "three".into(), "four".into()],
                3,
            )
            .unwrap();

            assert_eq!(snapshot.candidates.len(), visible_rows);
            assert_eq!(snapshot.selected, visible_rows - 1);
            assert_eq!(snapshot.candidates[snapshot.selected], "four");
        }
    }

    #[test]
    fn snapshot_debug_redacts_candidate_text() {
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(2.0, 2.0), egui::vec2(2.0, 2.0));
        let snapshot = CompletionPopupSnapshot::new(
            "tab-a".into(),
            CompletionSessionKey::new_for_test(3, "secret-session"),
            false,
            bounds,
            Some(cursor),
            vec!["export SECRET=candidate-value".into()],
            0,
        )
        .unwrap();

        let debug = format!("{snapshot:?}");
        assert!(debug.contains("tab-a"));
        assert!(debug.contains("candidates_len"));
        assert!(!debug.contains("candidate-value"));
        assert!(!debug.contains("secret-session"));
    }

    #[test]
    fn popup_opens_below_cursor_when_space_allows() {
        let bounds = egui::Rect::from_min_size(egui::pos2(220.0, 34.0), egui::vec2(800.0, 600.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(400.0, 100.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 4);
        assert!(!geometry.opens_above);
        assert!(geometry.position.y >= cursor.bottom());
        assert!(bounds.contains_rect(popup_rect(geometry)));
    }

    #[test]
    fn popup_flips_above_and_clamps_inside_terminal() {
        let bounds = egui::Rect::from_min_size(egui::pos2(220.0, 34.0), egui::vec2(300.0, 160.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(500.0, 170.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 8);
        assert!(geometry.opens_above);
        assert!(bounds.contains_rect(popup_rect(geometry)));
    }

    #[test]
    fn narrow_terminal_clamps_the_entire_popup_rect() {
        let bounds = egui::Rect::from_min_size(egui::pos2(50.0, 20.0), egui::vec2(6.0, 120.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(55.0, 40.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 3);

        assert!(geometry.size.x <= bounds.width());
        assert!(geometry.size.y <= bounds.height());
        assert!(bounds.contains_rect(popup_rect(geometry)));
    }

    #[test]
    fn tiny_terminal_and_zero_candidates_stay_finite_and_inside() {
        let bounds = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(1.0, 1.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(10.0, 20.0));
        let geometry = popup_geometry(bounds, cursor, 0);

        assert!(geometry.position.is_finite());
        assert!(geometry.size.is_finite());
        assert!(bounds.contains_rect(popup_rect(geometry)));
    }

    #[test]
    fn nonfinite_geometry_inputs_produce_a_safe_finite_rect() {
        let bounds = egui::Rect::from_min_max(
            egui::pos2(f32::NAN, f32::NEG_INFINITY),
            egui::pos2(f32::INFINITY, f32::NAN),
        );
        let cursor = egui::Rect::from_min_max(
            egui::pos2(f32::NAN, f32::INFINITY),
            egui::pos2(f32::NEG_INFINITY, f32::NAN),
        );
        let geometry = popup_geometry(bounds, cursor, usize::MAX);

        assert!(geometry.position.is_finite());
        assert!(geometry.size.is_finite());
        assert!(geometry.size.x >= 0.0);
        assert!(geometry.size.y >= 0.0);
    }

    #[test]
    fn render_paints_only_at_the_cursor_anchored_popup_rect() {
        let ctx = egui::Context::default();
        let bounds = egui::Rect::from_min_size(egui::pos2(220.0, 34.0), egui::vec2(800.0, 600.0));
        let cursor = egui::Rect::from_min_size(egui::pos2(400.0, 100.0), egui::vec2(10.0, 20.0));
        let expected = popup_rect(popup_geometry(bounds, cursor, 2));
        let mut painted = egui::Rect::NOTHING;
        for _ in 0..2 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    let snapshot = snapshot(
                        false,
                        bounds,
                        Some(cursor),
                        vec!["git status".into(), "git log".into()],
                    )
                    .unwrap();
                    render(ctx, &snapshot);
                },
            );
            painted = output
                .shapes
                .iter()
                .filter(|shape| !matches!(&shape.shape, egui::epaint::Shape::Noop))
                .fold(egui::Rect::NOTHING, |rect, shape| {
                    rect.union(shape.shape.visual_bounding_rect())
                });
        }

        assert!(painted.intersects(expected), "{painted:?} vs {expected:?}");
        assert!(
            painted.width() <= expected.width() + 2.0,
            "{painted:?} vs {expected:?}"
        );
        assert!(
            painted.height() <= expected.height() + 2.0,
            "{painted:?} vs {expected:?}"
        );
    }
}
