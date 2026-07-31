use std::process::Command;

use eframe::egui;
use egui::{
    epaint::{CubicBezierShape, PathShape},
    Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2,
};
use git2::Repository;

use crate::commit::{self, CommitRow};
use crate::config;
use crate::diff;

pub struct App {
    pub repo_path: String,
    pub rows: Vec<CommitRow>,
    pub selected: Option<usize>,
    pub search: String,
    pub diff_text: Option<String>,
    pub limit: usize,
    pub all_refs: bool,
    pub error: Option<String>,
    pub graph_width: f32,
    pub changed_files: Vec<String>,
    pub selected_file: Option<String>,
    pub file_diff: Option<String>,
}

impl App {
    pub fn new(repo_path: String, limit: usize, all_refs: bool) -> Self {
        let mut app = App {
            repo_path,
            rows: Vec::new(),
            selected: None,
            search: String::new(),
            diff_text: None,
            limit,
            all_refs,
            error: None,
            graph_width: 0.0,
            changed_files: Vec::new(),
            selected_file: None,
            file_diff: None,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match Repository::discover(&self.repo_path) {
            Ok(repo) => match commit::build_rows(&repo, self.limit, self.all_refs) {
                Ok(rows) => {
                    self.graph_width =
                        config::GRAPH_PAD_LEFT + commit::max_lanes(&rows) as f32 * config::LANE_WIDTH + config::GRAPH_PAD_RIGHT;
                    self.rows = rows;
                    self.selected = if self.rows.is_empty() { None } else { Some(0) };
                    self.diff_text = None;
                    self.changed_files.clear();
                    self.selected_file = None;
                    self.file_diff = None;
                    if self.selected.is_some() {
                        self.load_changed_files();
                        self.load_diff(false);
                    }
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("failed to read commits: {e}")),
            },
            Err(e) => self.error = Some(format!("not a git repository: {e}")),
        }
    }

    fn load_changed_files(&mut self) {
        if let Some(i) = self.selected {
            let hash = self.rows[i].oid.to_string();
            if let Ok(out) = Command::new("git")
                .current_dir(&self.repo_path)
                .args(&["diff-tree", "--no-commit-id", "-r", "--name-only", &hash])
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                self.changed_files = text.lines().filter(|l| !l.is_empty()).map(String::from).collect();
            }
        }
    }

    fn load_file_diff(&mut self, file: &str) {
        if let Some(i) = self.selected {
            let hash = self.rows[i].oid.to_string();
            if let Ok(out) = Command::new("git")
                .current_dir(&self.repo_path)
                .args(&["show", &hash, "--", file])
                .output()
            {
                self.file_diff = Some(String::from_utf8_lossy(&out.stdout).to_string());
                self.selected_file = Some(file.to_string());
            }
        }
    }

    fn load_diff(&mut self, stat_only: bool) {
        if let Some(i) = self.selected {
            let hash = self.rows[i].oid.to_string();
            let args: &[&str] = if stat_only {
                &["diff-tree", "--no-commit-id", "-r", "--stat", &hash]
            } else {
                &["show", &hash]
            };
            let out = Command::new("git")
                .current_dir(&self.repo_path)
                .args(args)
                .output();
            self.diff_text = match out {
                Ok(o) => Some(String::from_utf8_lossy(&o.stdout).to_string()),
                Err(e) => Some(format!("failed to run git show: {e}")),
            };
        }
    }

    fn find_next(&mut self) {
        if self.search.is_empty() || self.rows.is_empty() {
            return;
        }
        let q = self.search.to_lowercase();
        let n = self.rows.len();
        let start = self.selected.unwrap_or(0);
        for d in 1..=n {
            let i = (start + d) % n;
            let r = &self.rows[i];
            if r.summary.to_lowercase().contains(&q)
                || r.author.to_lowercase().contains(&q)
                || r.short.contains(&q)
            {
                self.selected = Some(i);
                return;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ctrl+Q / Cmd+Q to quit
        if ctx.input(|i| i.key_pressed(egui::Key::Q) && i.modifiers.ctrl) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("gitr — ").strong().color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
                ui.label(egui::RichText::new(&self.repo_path).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("☕ Buy me a coffee").clicked() {
                        let _ = webbrowser::open("https://buymeacoffee.com/lahirusamishka");
                    }
                    ui.separator();
                    ui.label(format!("{} commits", self.rows.len()));
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("⟳ Refresh").clicked() {
                    self.reload();
                }
                ui.separator();
                ui.checkbox(&mut self.all_refs, "all refs");
                ui.separator();
                ui.label("limit");
                let mut limit_str = self.limit.to_string();
                if ui.add(egui::TextEdit::singleline(&mut limit_str).desired_width(60.0)).changed() {
                    if let Ok(v) = limit_str.parse() {
                        self.limit = v;
                    }
                }
                ui.separator();
                ui.label("search");
                let resp = ui.add(egui::TextEdit::singleline(&mut self.search).desired_width(160.0));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.find_next();
                }
                if ui.button("Find").clicked() {
                    self.find_next();
                }
                ui.separator();
                ui.label("Ctrl+Q to exit");
            });
            ui.add_space(4.0);
        });

        if let Some(err) = &self.error {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), err);
            });
            return;
        }

        egui::SidePanel::right("details")
            .min_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                draw_details(self, ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let file_count = self.changed_files.len();
            if file_count > 0 {
                // Reserve space at the bottom for the file list.
                let file_h = file_count as f32 * 19.0 + 44.0;
                let rects = ui.max_rect();
                let (graph_rect, file_rect) = rects.split_top_bottom_at_y(rects.height() - file_h);

                // Graph area
                let mut graph_ui = ui.child_ui(graph_rect, egui::Layout::top_down(egui::Align::LEFT), None);
                egui::ScrollArea::both().auto_shrink([false, false]).show(&mut graph_ui, |ui| {
                    draw_graph_inner(self, ui);
                });

                // File list area
                let mut file_ui = ui.child_ui(file_rect, egui::Layout::top_down(egui::Align::LEFT), None);
                file_ui.separator();
                file_ui.add_space(2.0);
                file_ui.label(egui::RichText::new(format!("files changed  {}", file_count)).size(11.0).color(config::C_SUBTEXT));
                let file_font = FontId::monospace(12.0);
                let files: Vec<String> = self.changed_files.clone();
                let mut clicked: Option<String> = None;
                let sel = self.selected_file.clone();
                for file in &files {
                    let selected = sel.as_deref() == Some(file.as_str());
                    let color = if selected { Color32::from_rgb(0x89, 0xb4, 0xfa) } else { config::C_TEXT };
                    let resp = file_ui.add(
                        egui::Label::new(egui::RichText::new(file).color(color).font(file_font.clone()))
                            .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        clicked = Some(file.clone());
                    }
                }
                if let Some(f) = clicked {
                    self.load_file_diff(&f);
                }
            } else {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    draw_graph_inner(self, ui);
                });
            }
        });
    }
}

fn draw_details(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let Some(i) = app.selected else {
            ui.label("no commit selected");
            return;
        };
        let row_info = {
            let row = &app.rows[i];
            (
                row.oid.to_string(),
                row.author.clone(),
                row.email.clone(),
                commit::format_time(row.time, row.offset_min),
                row.summary.clone(),
                row.branches.clone(),
                row.tags.clone(),
            )
        };
        let (hash, author, email, date, summary, branches, tags) = row_info;

        ui.add_space(6.0);
        ui.label(egui::RichText::new(&summary).strong().size(15.0));
        ui.add_space(6.0);
        ui.monospace(&hash);
        ui.label(format!("{author} <{email}>"));
        ui.label(date);
        if !branches.is_empty() {
            ui.label(format!("branches: {}", branches.join(", ")));
        }
        if !tags.is_empty() {
            ui.label(format!("tags: {}", tags.join(", ")));
        }
        ui.add_space(8.0);
        if let Some(file_diff) = &app.file_diff {
            if let Some(file) = &app.selected_file {
                ui.label(egui::RichText::new(file).strong().size(12.0).color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
            }
            ui.add_space(2.0);
            diff::draw_diff(ui, file_diff);
        } else if let Some(diff) = &app.diff_text {
            diff::draw_diff(ui, diff);
        }
    });
}

fn draw_graph_inner(app: &mut App, ui: &mut egui::Ui) {
    let total_height = app.rows.len() as f32 * config::ROW_HEIGHT + config::ROW_HEIGHT;
    let text_col_x = app.graph_width;
        let width = ui.available_width().max(text_col_x + 600.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, total_height), Sense::click());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        let hover_row = response.hover_pos().and_then(|pos| {
            let idx = ((pos.y - origin.y) / config::ROW_HEIGHT) as usize;
            (idx < app.rows.len()).then_some(idx)
        });

        if response.clicked() {
            if let Some(idx) = hover_row {
                app.selected = Some(idx);
                app.diff_text = None;
                app.file_diff = None;
                app.selected_file = None;
                app.load_changed_files();
                app.load_diff(false);
            }
        }

        let y_center = |i: usize| origin.y + i as f32 * config::ROW_HEIGHT + config::ROW_HEIGHT / 2.0;
        let x_lane = |l: usize| origin.x + config::GRAPH_PAD_LEFT + l as f32 * config::LANE_WIDTH;

        let x_hash = rect.max.x - config::GRAPH_PAD_RIGHT;
        let x_date = x_hash - config::COL_HASH;
        let x_author = x_date - config::COL_DATE;
        let x_msg_end = x_author - config::COL_AUTHOR - 12.0;

        for i in 0..app.rows.len() {
            let top = origin.y + i as f32 * config::ROW_HEIGHT;
            let full = Rect::from_min_size(Pos2::new(rect.min.x, top), Vec2::new(rect.width(), config::ROW_HEIGHT));
            if app.selected == Some(i) {
                painter.rect_filled(full, 0.0, config::C_SEL);
            } else if hover_row == Some(i) {
                painter.rect_filled(full, 0.0, config::C_HOVER);
            }
        }

        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let yc_next = y_center(i + 1);

            for &l in &row.passthrough {
                let x = x_lane(l);
                painter.line_segment(
                    [Pos2::new(x, yc), Pos2::new(x, yc_next)],
                    Stroke::new(config::LINE_WIDTH, config::lane_color(l)),
                );
            }

            let x_my = x_lane(row.lane);
            for &pl in &row.parent_lanes {
                let x_p = x_lane(pl);
                if pl == row.lane {
                    painter.line_segment(
                        [Pos2::new(x_my, yc), Pos2::new(x_p, yc_next)],
                        Stroke::new(config::LINE_WIDTH, config::lane_color(row.lane)),
                    );
                } else {
                    let dx = (x_p - x_my).abs();
                    let ease = (config::ROW_HEIGHT * 0.5).max(dx * 0.35).min(config::ROW_HEIGHT * 0.85);
                    let points = [
                        Pos2::new(x_my, yc),
                        Pos2::new(x_my, yc + ease),
                        Pos2::new(x_p, yc_next - ease),
                        Pos2::new(x_p, yc_next),
                    ];
                    let bez = CubicBezierShape::from_points_stroke(
                        points,
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(config::LINE_WIDTH, config::lane_color(pl)),
                    );
                    painter.add(bez);
                }
            }
        }

        for (i, row) in app.rows.iter().enumerate() {
            let center = Pos2::new(x_lane(row.lane), y_center(i));
            let node_color = config::lane_color(row.lane);
            if row.is_head {
                painter.circle_filled(center, config::NODE_RADIUS + 1.5, Color32::from_rgb(0x1e, 0x1e, 0x2e));
                painter.circle_stroke(center, config::NODE_RADIUS, Stroke::new(2.2_f32, node_color));
            } else {
                painter.circle_filled(center, config::NODE_RADIUS, node_color);
            }
        }

        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let mut tx = text_col_x;

            // Cap pills so they don't extend into the metadata columns.
            let cap_x = x_author - 20.0;
            if row.is_head && tx < cap_x {
                tx = draw_pill(&painter, tx, yc, "HEAD", Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xf3, 0x8b, 0xa8));
            }
            for b in &row.branches {
                if tx >= cap_x { break; }
                tx = draw_pill(&painter, tx, yc, b, Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0x89, 0xb4, 0xfa));
            }
            for t in &row.tags {
                if tx >= cap_x { break; }
                tx = draw_pill(&painter, tx, yc, t, Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xfa, 0xb3, 0x87));
            }

            let msg = elide(&painter, &row.summary, FontId::proportional(12.5), x_msg_end - tx);
            painter.text(
                Pos2::new(tx, yc),
                egui::Align2::LEFT_CENTER,
                &msg,
                FontId::proportional(12.5),
                config::C_TEXT,
            );

            let author = elide(&painter, &row.author, FontId::proportional(11.5), config::COL_AUTHOR - 8.0);
            painter.text(
                Pos2::new(x_author, yc),
                egui::Align2::LEFT_CENTER,
                &author,
                FontId::proportional(11.5),
                config::C_SUBTEXT,
            );

            painter.text(
                Pos2::new(x_date, yc),
                egui::Align2::RIGHT_CENTER,
                commit::format_time(row.time, row.offset_min),
                FontId::proportional(11.5),
                config::C_SUBTEXT,
            );

            painter.text(
                Pos2::new(x_hash, yc),
                egui::Align2::RIGHT_CENTER,
                &row.short,
                FontId::monospace(11.0),
                config::C_HASH,
            );
        }

        let _ = PathShape::convex_polygon(vec![], Color32::TRANSPARENT, Stroke::NONE);
}

fn draw_pill(painter: &egui::Painter, x: f32, y: f32, label: &str, fg: Color32, bg: Color32) -> f32 {
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(label.to_string(), font, fg);
    let w = galley.size().x + 12.0;
    let h = 16.0;
    let rect = Rect::from_min_size(Pos2::new(x, y - h / 2.0), Vec2::new(w, h));
    painter.rect_filled(rect, 8.0, bg);
    painter.galley(rect.min + Vec2::new(6.0, (h - galley.size().y) / 2.0), galley, fg);
    rect.max.x + 5.0
}

fn elide(painter: &egui::Painter, text: &str, font: FontId, max_w: f32) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    let full = painter.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE);
    if full.size().x <= max_w {
        return text.to_string();
    }
    let mut end = text.len();
    while end > 0 {
        if text.is_char_boundary(end) {
            let candidate = format!("{}…", &text[..end]);
            let g = painter.layout_no_wrap(candidate.clone(), font.clone(), Color32::WHITE);
            if g.size().x <= max_w {
                return candidate;
            }
        }
        end -= 1;
    }
    String::from("…")
}
