use std::cell::RefCell;
use std::rc::Rc;

use cairo::Context;
use gtk::prelude::*;
use gtk::{Box as GtkBox, DrawingArea, Label, Orientation};

/// A chart widget for the sidebar monitoring panel.
///
/// Contains a header row (title + value label) and a Cairo `DrawingArea`
/// that renders line charts or bar charts for CPU, memory, network, etc.
pub struct MonitorChart {
    container: GtkBox,
    drawing_area: DrawingArea,
    label: Label,
    value_label: Label,
}

impl MonitorChart {
    /// Create a new monitor chart with the given title and drawing area height.
    pub fn new(title: &str, height: i32) -> Self {
        let container = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        // Header row: title on the left, value on the right
        let header = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();

        let label = Label::builder()
            .label(title)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .build();
        label.add_css_class("caption");

        let value_label = Label::builder()
            .label("")
            .halign(gtk::Align::End)
            .build();
        value_label.add_css_class("caption");

        header.append(&label);
        header.append(&value_label);
        container.append(&header);

        // Drawing area for the chart
        let drawing_area = DrawingArea::builder()
            .height_request(height)
            .hexpand(true)
            .build();

        container.append(&drawing_area);

        Self {
            container,
            drawing_area,
            label,
            value_label,
        }
    }

    /// Returns a reference to the outer container widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update the value text shown in the header (e.g. "73%").
    pub fn set_value_text(&self, text: &str) {
        self.value_label.set_label(text);
    }

    /// Returns a reference to the title label.
    #[allow(dead_code)]
    pub fn label(&self) -> &Label {
        &self.label
    }

    /// Set the draw function to render a horizontal bar chart.
    ///
    /// Each item is a `(label, fraction)` pair where fraction is 0.0..1.0.
    /// `color` is the (r, g, b) fill color for the bars.
    pub fn set_bar_chart(&self, items: &[(String, f64)], color: (f64, f64, f64)) {
        let items = items.to_vec();
        self.drawing_area
            .set_draw_func(move |_area, cr, width, height| {
                draw_bar_chart(cr, width as f64, height as f64, &items, color);
            });
        self.drawing_area.queue_draw();
    }

    /// Set the draw function to render a line chart with area fill.
    ///
    /// `data` is the time-series values, `max_val` is the Y-axis maximum,
    /// and `color` is an (r, g, b) tuple in 0.0..1.0 range.
    pub fn update_line_chart(&self, data: &[f64], max_val: f64, color: (f64, f64, f64)) {
        let data = data.to_vec();
        let data = Rc::new(RefCell::new(data));
        let max_val = if max_val <= 0.0 { 1.0 } else { max_val };

        let data_clone = Rc::clone(&data);
        self.drawing_area
            .set_draw_func(move |_area, cr, width, height| {
                let w = width as f64;
                let h = height as f64;
                let data = data_clone.borrow();

                // Dark background
                cr.set_source_rgb(0.12, 0.12, 0.15);
                cr.rectangle(0.0, 0.0, w, h);
                let _ = cr.fill();

                // Grid lines (horizontal)
                cr.set_source_rgba(0.3, 0.3, 0.35, 0.5);
                cr.set_line_width(0.5);
                for i in 1..4 {
                    let y = h * (i as f64) / 4.0;
                    cr.move_to(0.0, y);
                    cr.line_to(w, y);
                }
                let _ = cr.stroke();

                if data.is_empty() {
                    return;
                }

                let n = data.len();
                let step = if n > 1 { w / (n as f64 - 1.0) } else { w };

                // Build the line path
                let (r, g, b) = color;

                // Area fill under the line
                cr.new_path();
                cr.move_to(0.0, h);
                for (i, val) in data.iter().enumerate() {
                    let x = i as f64 * step;
                    let y = h - (val.min(max_val) / max_val) * h;
                    cr.line_to(x, y);
                }
                cr.line_to((n as f64 - 1.0) * step, h);
                cr.close_path();
                cr.set_source_rgba(r, g, b, 0.2);
                let _ = cr.fill();

                // Line stroke on top
                cr.new_path();
                for (i, val) in data.iter().enumerate() {
                    let x = i as f64 * step;
                    let y = h - (val.min(max_val) / max_val) * h;
                    if i == 0 {
                        cr.move_to(x, y);
                    } else {
                        cr.line_to(x, y);
                    }
                }
                cr.set_source_rgb(r, g, b);
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            });

        self.drawing_area.queue_draw();
    }
}

/// Draw a horizontal bar chart (e.g. for disk usage).
///
/// Each item is a `(label, fraction)` pair where fraction is 0.0..1.0.
/// `color` is the (r, g, b) fill color for the bars.
pub fn draw_bar_chart(
    cr: &Context,
    w: f64,
    h: f64,
    items: &[(String, f64)],
    color: (f64, f64, f64),
) {
    if items.is_empty() {
        return;
    }

    let (r, g, b) = color;
    let bar_height = 14.0;
    let label_height = 12.0;
    let row_spacing = 6.0;
    let row_height = label_height + bar_height + row_spacing;
    let margin_x = 4.0;
    let bar_width = w - margin_x * 2.0;

    // Dark background
    cr.set_source_rgb(0.12, 0.12, 0.15);
    cr.rectangle(0.0, 0.0, w, h);
    let _ = cr.fill();

    cr.set_font_size(10.0);

    for (i, (label, fraction)) in items.iter().enumerate() {
        let y_base = i as f64 * row_height + 4.0;

        // Label text
        cr.set_source_rgb(0.7, 0.7, 0.7);
        cr.move_to(margin_x, y_base + label_height - 2.0);
        let _ = cr.show_text(label);

        // Bar background
        let bar_y = y_base + label_height;
        cr.set_source_rgb(0.2, 0.2, 0.22);
        cr.rectangle(margin_x, bar_y, bar_width, bar_height);
        let _ = cr.fill();

        // Bar fill
        let fill_width = (fraction.clamp(0.0, 1.0)) * bar_width;
        cr.set_source_rgb(r, g, b);
        cr.rectangle(margin_x, bar_y, fill_width, bar_height);
        let _ = cr.fill();

        // Percentage text on the bar
        let pct_text = format!("{}%", (fraction * 100.0) as u32);
        cr.set_source_rgb(0.9, 0.9, 0.9);
        cr.move_to(margin_x + 4.0, bar_y + bar_height - 3.0);
        let _ = cr.show_text(&pct_text);
    }
}
