use crate::app::Tab;
use crate::theme;

pub fn show(ui: &mut egui::Ui, tab: &mut Tab, compact: bool) {
    ui.add_space(6.0);

    for (t, label) in [
        (Tab::Processes, "Processes"),
        (Tab::Performance, "Performance"),
        (Tab::Startup, "Startup apps"),
        (Tab::Services, "Services"),
    ] {
        let selected = *tab == t;
        let resp = side_button(ui, t, label, selected, compact);
        if resp.clicked() {
            *tab = t;
        }
    }

    // Settings pinned to the bottom of the sidebar via a bottom-up layout.
    // Same widget as the main tabs so selection feels consistent.
    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        ui.add_space(6.0);
        let selected = *tab == Tab::Settings;
        let resp = side_button(ui, Tab::Settings, "Settings", selected, compact);
        if resp.clicked() {
            *tab = Tab::Settings;
        }
    });
}

fn side_button(
    ui: &mut egui::Ui,
    tab: Tab,
    label: &str,
    selected: bool,
    compact: bool,
) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), 38.0);
    let (rect, resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    let bg = if selected {
        egui::Color32::from_rgba_unmultiplied(0x60, 0xCD, 0xFF, 38)
    } else if resp.hovered() {
        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(6), bg);
    if selected {
        let bar = egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0, 8.0),
            egui::vec2(3.0, rect.height() - 16.0),
        );
        ui.painter()
            .rect_filled(bar, egui::CornerRadius::same(2), theme::ACCENT);
    }
    let fg = if selected { theme::ACCENT } else { theme::TEXT };

    // In compact mode the icon is centered in the strip and the label is
    // surfaced through hover. Otherwise icon sits at x=14 with the label next
    // to it at x=42.
    let icon_size = 16.0;
    let icon_x = if compact {
        rect.center().x - icon_size / 2.0
    } else {
        rect.min.x + 14.0
    };
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(icon_x, rect.min.y + (rect.height() - icon_size) / 2.0),
        egui::vec2(icon_size, icon_size),
    );
    paint_icon(ui.painter(), tab, icon_rect, fg);

    if !compact {
        ui.painter().text(
            rect.min + egui::vec2(42.0, rect.height() / 2.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.5),
            fg,
        );
        resp
    } else {
        resp.on_hover_text(label)
    }
}

fn paint_icon(painter: &egui::Painter, tab: Tab, rect: egui::Rect, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    match tab {
        Tab::Processes => {
            // Three rows: a small filled dot on the left, a line on the right.
            // Mimics W11 Task Manager's "list" icon.
            let rows = 3;
            let dot_r = 1.6;
            let line_h_pad = 1.5;
            let row_step = rect.height() / rows as f32;
            for i in 0..rows {
                let cy = rect.min.y + row_step * (i as f32 + 0.5);
                let cx_dot = rect.min.x + 2.0;
                painter.circle_filled(egui::pos2(cx_dot, cy), dot_r, color);
                let line_start = egui::pos2(cx_dot + 4.0, cy);
                let line_end = egui::pos2(rect.max.x - line_h_pad, cy);
                painter.line_segment([line_start, line_end], stroke);
            }
        }
        Tab::Performance => {
            // Mini bar chart: three vertical bars of varying height.
            let bars = [0.45_f32, 0.85, 0.65];
            let bar_w = 3.0;
            let gap = 2.0;
            let total_w = bar_w * bars.len() as f32 + gap * (bars.len() as f32 - 1.0);
            let start_x = rect.center().x - total_w / 2.0;
            for (i, h_ratio) in bars.iter().enumerate() {
                let x = start_x + i as f32 * (bar_w + gap);
                let h = rect.height() * h_ratio;
                let bar =
                    egui::Rect::from_min_size(egui::pos2(x, rect.max.y - h), egui::vec2(bar_w, h));
                painter.rect_filled(bar, egui::CornerRadius::same(1), color);
            }
        }
        Tab::Startup => {
            // Up arrow inside a rounded square.
            let inset = 1.0;
            let inner = rect.shrink(inset);
            painter.rect_stroke(
                inner,
                egui::CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            let cx = inner.center().x;
            let top = inner.min.y + 3.0;
            let bottom = inner.max.y - 3.0;
            painter.line_segment([egui::pos2(cx, top), egui::pos2(cx, bottom)], stroke);
            painter.line_segment(
                [egui::pos2(cx, top), egui::pos2(cx - 3.0, top + 3.0)],
                stroke,
            );
            painter.line_segment(
                [egui::pos2(cx, top), egui::pos2(cx + 3.0, top + 3.0)],
                stroke,
            );
        }
        Tab::Services => {
            // Power button glyph (open ring + vertical bar through the top)
            // — visually distinct from the Settings gear and reads as
            // "start/stop daemons".
            let center = rect.center();
            let r = rect.width() * 0.40;
            // Ring with a notch at the top by drawing two arcs as a circle then
            // covering the top with a vertical mark.
            painter.circle_stroke(center, r, egui::Stroke::new(1.8, color));
            // Vertical mark on top of the circle
            painter.line_segment(
                [
                    egui::pos2(center.x, rect.min.y + 1.0),
                    egui::pos2(center.x, center.y - r * 0.25),
                ],
                egui::Stroke::new(2.0, color),
            );
            // Small fill at the top of the ring to cover the arc — gives a
            // cleaner "power" look.
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(center.x - 1.5, center.y - r - 0.5),
                    egui::vec2(3.0, 2.0),
                ),
                egui::CornerRadius::ZERO,
                ui_color_match(painter),
            );
        }
        Tab::Settings => {
            // Gear: same drawing as the previous floating button.
            let center = rect.center();
            let outer = rect.width() / 2.0 - 0.5;
            let stroke_w = egui::Stroke::new(1.6, color);
            let teeth = 8;
            for i in 0..teeth {
                let angle = std::f32::consts::TAU * (i as f32 / teeth as f32);
                let dir = egui::vec2(angle.cos(), angle.sin());
                let p_in = center + dir * (outer * 0.72);
                let p_out = center + dir * outer;
                painter.line_segment([p_in, p_out], egui::Stroke::new(2.2, color));
            }
            painter.circle_stroke(center, outer * 0.7, stroke_w);
            painter.circle_stroke(center, outer * 0.34, stroke_w);
        }
    }
}

/// Return the panel's background color so we can punch a small "negative
/// space" hole through painted shapes (used by the Services power icon to
/// hide the top of the ring under the vertical mark).
fn ui_color_match(_painter: &egui::Painter) -> egui::Color32 {
    theme::SIDEBAR_BG
}
