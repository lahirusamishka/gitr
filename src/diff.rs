use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

struct SideRow {
    header: Option<String>,
    left: Option<(String, String, bool)>,
    right: Option<(String, String, bool)>,
}

pub fn draw_diff(ui: &mut egui::Ui, diff: &str) {
    let bg = Color32::from_rgb(0x1e, 0x1e, 0x2e);
    let line_h = 20.0;
    let font = egui::FontId::monospace(12.0);
    let ln_font = egui::FontId::monospace(10.0);

    let rows = parse_side_by_side(diff);
    let total_h = rows.len() as f32 * line_h + 4.0;

    egui::ScrollArea::vertical().show(ui, |ui| {
        let avail = ui.available_width();
        let col_w = (avail - 2.0) / 2.0;
        let ln_w = 44.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(avail, total_h), Sense::hover());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        painter.rect_filled(rect, 0.0, bg);

        let div_x = origin.x + col_w;
        painter.line_segment(
            [Pos2::new(div_x, origin.y), Pos2::new(div_x, origin.y + total_h)],
            Stroke::new(1.0_f32, Color32::from_rgb(0x36, 0x39, 0x4f)),
        );

        for (i, row) in rows.iter().enumerate() {
            let y = origin.y + 2.0 + i as f32 * line_h;

            if let Some(hdr) = &row.header {
                painter.text(
                    Pos2::new(origin.x + 6.0, y + line_h / 2.0),
                    egui::Align2::LEFT_CENTER,
                    hdr,
                    font.clone(),
                    Color32::from_rgb(0x89, 0xb4, 0xfa),
                );
                continue;
            }

            if let Some((ln, text, is_del)) = &row.left {
                let r = Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(col_w, line_h));
                if *is_del {
                    painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x3c, 0x1a, 0x1a, 200));
                }
                painter.text(
                    Pos2::new(origin.x + 2.0, y + line_h / 2.0),
                    egui::Align2::LEFT_CENTER,
                    ln,
                    ln_font.clone(),
                    Color32::from_rgb(0x58, 0x5b, 0x70),
                );
                painter.text(
                    Pos2::new(origin.x + ln_w, y + line_h / 2.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font.clone(),
                    if *is_del { Color32::from_rgb(0xf3, 0x8b, 0xa8) } else { Color32::from_rgb(0xcd, 0xd6, 0xf4) },
                );
            }

            if let Some((ln, text, is_add)) = &row.right {
                let r = Rect::from_min_size(Pos2::new(div_x, y), Vec2::new(col_w, line_h));
                if *is_add {
                    painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(0x1a, 0x3c, 0x1a, 200));
                }
                painter.text(
                    Pos2::new(div_x + 2.0, y + line_h / 2.0),
                    egui::Align2::LEFT_CENTER,
                    ln,
                    ln_font.clone(),
                    Color32::from_rgb(0x58, 0x5b, 0x70),
                );
                painter.text(
                    Pos2::new(div_x + ln_w, y + line_h / 2.0),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font.clone(),
                    if *is_add { Color32::from_rgb(0xa6, 0xe3, 0xa1) } else { Color32::from_rgb(0xcd, 0xd6, 0xf4) },
                );
            }
        }
    });
}

fn parse_side_by_side(diff: &str) -> Vec<SideRow> {
    let mut rows = Vec::new();
    let mut in_hunk = false;
    let mut old_ln = 0usize;
    let mut new_ln = 0usize;

    for raw in diff.lines() {
        if raw.starts_with("diff --git") || raw.starts_with("--- ") || raw.starts_with("+++ ") {
            rows.push(SideRow { header: Some(raw.to_string()), left: None, right: None });
            in_hunk = false;
            continue;
        }
        if raw.starts_with("@@") {
            in_hunk = true;
            let parts: Vec<&str> = raw.split_whitespace().collect();
            let old_part = parts.get(1).copied().unwrap_or("-0,0").trim_start_matches('-');
            let new_part = parts.get(2).copied().unwrap_or("+0,0").trim_start_matches('+');
            old_ln = old_part.split(',').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            new_ln = new_part.split(',').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            rows.push(SideRow { header: Some(raw.to_string()), left: None, right: None });
            continue;
        }
        if !in_hunk {
            continue;
        }

        let ch = raw.chars().next().unwrap_or(' ');
        match ch {
            ' ' => {
                old_ln += 1; new_ln += 1;
                let text = &raw[1..];
                rows.push(SideRow {
                    header: None,
                    left: Some((format!("{}", old_ln), text.to_string(), false)),
                    right: Some((format!("{}", new_ln), text.to_string(), false)),
                });
            }
            '-' => {
                old_ln += 1;
                rows.push(SideRow {
                    header: None,
                    left: Some((format!("{}", old_ln), raw[1..].to_string(), true)),
                    right: None,
                });
            }
            '+' => {
                new_ln += 1;
                rows.push(SideRow {
                    header: None,
                    left: None,
                    right: Some((format!("{}", new_ln), raw[1..].to_string(), true)),
                });
            }
            _ => {}
        }
    }
    rows
}
