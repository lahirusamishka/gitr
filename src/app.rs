use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eframe::egui;
use egui::{
    epaint::{CubicBezierShape, PathShape},
    text::LayoutJob,
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
    pub selected_is_staged: bool,
    pub side_by_side: bool,
    pub show_about: bool,
    pub current_branch: String,
    pub staged_files: Vec<String>,
    pub unstaged_files: Vec<String>,
    pub needs_reload: Arc<AtomicBool>,
}

impl App {
    pub fn new(repo_path: String, limit: usize, all_refs: bool) -> Self {
        let needs_reload = Arc::new(AtomicBool::new(false));
        let watch_flag = needs_reload.clone();
        let watch_path = repo_path.clone();
        std::thread::spawn(move || {
            let mut last_status = String::new();
            let head_path = format!("{watch_path}/.git/HEAD");
            let mut last_head: Option<std::time::SystemTime> = std::fs::metadata(&head_path).ok().and_then(|m| m.modified().ok());
            let refs_dir = format!("{watch_path}/.git/refs");
            let mut last_ref_times: Vec<(String, std::time::SystemTime)> = Vec::new();
            let scan_refs = |times: &mut Vec<(String, std::time::SystemTime)>| {
                let mut changed = false;
                let mut found = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&refs_dir) {
                    for e in entries.flatten() {
                        let path = e.path();
                        if path.is_dir() {
                            if let Ok(sub) = std::fs::read_dir(&path) {
                                for f in sub.flatten() {
                                    if let Ok(meta) = f.metadata() {
                                        if let Ok(m) = meta.modified() {
                                            let s = f.path().to_string_lossy().to_string();
                                            found.push((s.clone(), m));
                                            if !times.iter().any(|(p, t)| p == &s && t == &m) {
                                                changed = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                *times = found;
                changed
            };
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(out) = std::process::Command::new("git")
                    .current_dir(&watch_path)
                    .args(&["status", "--porcelain"])
                    .output()
                {
                    let cur = String::from_utf8_lossy(&out.stdout).to_string();
                    if cur != last_status {
                        watch_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        last_status = cur;
                    }
                }
                if let Ok(meta) = std::fs::metadata(&head_path) {
                    if let Ok(modified) = meta.modified() {
                        if last_head.map_or(true, |t| t != modified) {
                            watch_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                            last_head = Some(modified);
                        }
                    }
                }
                if scan_refs(&mut last_ref_times) {
                    watch_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
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
            selected_is_staged: false,
            side_by_side: false,
            show_about: false,
            current_branch: String::new(),
            staged_files: Vec::new(),
            unstaged_files: Vec::new(),
            needs_reload,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match Repository::discover(&self.repo_path) {
            Ok(repo) => match commit::build_rows(&repo, self.limit, self.all_refs) {
                Ok(mut rows) => {
                    self.current_branch = String::from_utf8_lossy(
                        &Command::new("git").current_dir(&self.repo_path).args(&["symbolic-ref", "--short", "HEAD"]).output().map(|o| o.stdout).unwrap_or_default()
                    ).trim().to_string();
                    self.load_status();
                    if !self.unstaged_files.is_empty() || !self.staged_files.is_empty() {
                        if let Some(head_idx) = rows.iter().position(|r| r.is_head) {
                            let head_lane = rows[head_idx].lane;
                            let has_staged = !self.staged_files.is_empty();
                            let has_unstaged = !self.unstaged_files.is_empty();
                            let summary = match (has_staged, has_unstaged) {
                                (true, false) => "staged changes",
                                (false, true) => "unstaged changes",
                                _ => "working tree changes",
                            };
                            rows.insert(0, commit::CommitRow {
                                oid: git2::Oid::zero(),
                                short: String::new(),
                                lane: head_lane,
                                passthrough: Vec::new(),
                                parent_lanes: vec![head_lane],
                                author: String::new(),
                                email: String::new(),
                                time: 0,
                                offset_min: 0,
                                summary: summary.to_string(),
                                branches: Vec::new(),
                                tags: Vec::new(),
                                is_head: false,
                                is_working: true,
                            });
                        }
                    }
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
                        if !self.rows[0].is_working {
                            self.load_diff(false);
                        }
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
            if self.rows[i].is_working {
                let mut merged = self.staged_files.clone();
                for f in &self.unstaged_files {
                    if !merged.contains(f) {
                        merged.push(f.clone());
                    }
                }
                self.changed_files = merged;
                return;
            }
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
            if self.rows[i].is_working {
                let args = if self.selected_is_staged {
                    vec!["diff", "--cached", "--", file]
                } else {
                    vec!["diff", "--", file]
                };
                if let Ok(out) = Command::new("git")
                    .current_dir(&self.repo_path)
                    .args(&args)
                    .output()
                {
                    self.file_diff = Some(String::from_utf8_lossy(&out.stdout).to_string());
                    self.selected_file = Some(file.to_string());
                }
                return;
            }
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
            if self.rows[i].is_working {
                let out = Command::new("git")
                    .current_dir(&self.repo_path)
                    .args(&["diff", "HEAD"])
                    .output();
                self.diff_text = match out {
                    Ok(o) => Some(String::from_utf8_lossy(&o.stdout).to_string()),
                    Err(e) => Some(format!("failed to run git diff: {e}")),
                };
                return;
            }
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

    fn load_status(&mut self) {
        if let Ok(out) = Command::new("git")
            .current_dir(&self.repo_path)
            .args(&["status", "--porcelain"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            self.staged_files.clear();
            self.unstaged_files.clear();
            for line in text.lines() {
                if line.len() < 3 { continue; }
                let staged = line.as_bytes()[0] as char;
                let unstaged = line.as_bytes()[1] as char;
                let file = line[3..].to_string();
                if staged != ' ' {
                    self.staged_files.push(file.clone());
                }
                if unstaged != ' ' {
                    self.unstaged_files.push(file);
                }
            }
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
        if self.needs_reload.swap(false, Ordering::SeqCst) {
            self.reload();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        if ctx.input(|i| i.key_pressed(egui::Key::Q) && i.modifiers.ctrl) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::R) && i.modifiers.ctrl) {
            self.reload();
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.add(egui::Button::new("Refresh").shortcut_text("Ctrl+R")).clicked() {
                        self.reload();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new("Exit").shortcut_text("Ctrl+Q")).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About gitr").clicked() {
                        self.show_about = true;
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{} commits ", self.rows.len()));
                    if ui.button("☕ Buy me a coffee").clicked() {
                        let _ = webbrowser::open("https://buymeacoffee.com/lahirusamishka");
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("gitr").strong().color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let repo_display = std::path::Path::new(&self.repo_path).file_name().and_then(|n| n.to_str()).map(|s| s.to_owned()).unwrap_or_else(|| {
                    std::fs::canonicalize(&self.repo_path).ok().and_then(|p| p.file_name().and_then(|n| n.to_str().map(|s| s.to_owned()))).unwrap_or(self.repo_path.clone())
                });
                ui.label(egui::RichText::new(repo_display).strong().size(14.0).color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
                ui.add_space(8.0);
                let branch = &self.current_branch;
                if !branch.is_empty() {
                    let fg = Color32::from_rgb(0xcd, 0xd6, 0xf4);
                    let bg = Color32::from_rgb(0x31, 0x32, 0x44);
                    let galley = ui.painter().layout_no_wrap(format!(" ⎇ {branch} "), FontId::proportional(12.0), fg);
                    let w = galley.size().x + 16.0;
                    let h = 20.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
                    let p = ui.painter_at(rect);
                    p.rect_filled(rect, 6.0, bg);
                    p.galley(rect.min + egui::vec2(8.0, (h - galley.size().y) / 2.0), galley, fg);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                });
            });
            ui.add_space(4.0);
        });

        egui::Window::new("About gitr")
            .open(&mut self.show_about)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("gitr").strong().size(18.0).color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
                ui.add_space(2.0);
                ui.label("Version 0.1.0");
                ui.separator();
                ui.add_space(4.0);
                ui.label("Inspired by gitg (GNOME Git graphical interface) and gitk.");
                ui.add_space(4.0);
                ui.label("I love the git graph tree view and gitk is very easy to access in the terminal. I wanted both combined into one tool — so I built it with Rust.");
                ui.add_space(4.0);
                ui.label("gitr (r = Rust) brings together the visual tree of gitg and the quick accessibility of gitk, making it easy to browse your repository and check file diffs at a glance.");
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Fully open source.").strong().size(12.0).color(Color32::from_rgb(0xa6, 0xe3, 0xa1)));
                ui.add_space(4.0);
                ui.hyperlink_to("GitHub", "https://github.com/lahirusamishka/gitr");
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Made with ☕ by Lahiru Samishka").size(11.0).color(config::C_SUBTEXT));
            });

        if let Some(err) = &self.error {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), err);
            });
            return;
        }

        egui::SidePanel::right("details")
            .min_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                draw_details(self, ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let file_count = self.changed_files.len();
            if file_count > 0 {
                let is_working = self.selected.map(|i| self.rows[i].is_working).unwrap_or(false);
                let file_h = if is_working {
                    (self.staged_files.len() + self.unstaged_files.len()) as f32 * 19.0 + 68.0
                } else {
                    file_count as f32 * 19.0 + 44.0
                };
                let rects = ui.max_rect();
                let (graph_rect, file_rect) = rects.split_top_bottom_at_y(rects.height() - file_h);

                let mut graph_ui = ui.child_ui(graph_rect, egui::Layout::top_down(egui::Align::LEFT), None);
                egui::ScrollArea::both().auto_shrink([false, false]).show(&mut graph_ui, |ui| {
                    draw_graph_inner(self, ui);
                });

                let mut file_ui = ui.child_ui(file_rect, egui::Layout::top_down(egui::Align::LEFT), None);
                file_ui.separator();
                file_ui.add_space(2.0);
                let file_font = FontId::monospace(12.0);
                let mut clicked: Option<String> = None;
                let sel = self.selected_file.clone();

                if is_working {
                    if !self.staged_files.is_empty() {
                        file_ui.label(egui::RichText::new(format!("staged  {}", self.staged_files.len())).size(11.0).color(Color32::from_rgb(0xf9, 0xe2, 0xaf)));
                        for file in &self.staged_files {
                            let selected = sel.as_deref() == Some(file.as_str());
                            let color = if selected { Color32::from_rgb(0x89, 0xb4, 0xfa) } else { Color32::from_rgb(0xf9, 0xe2, 0xaf) };
                            let resp = file_ui.add(
                                egui::Label::new(egui::RichText::new(format!("  {file}")).color(color).font(file_font.clone()))
                                    .sense(Sense::click()),
                            ).on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                self.selected_is_staged = true;
                                clicked = Some(file.clone());
                            }
                        }
                    }
                    if !self.unstaged_files.is_empty() {
                        file_ui.add_space(2.0);
                        file_ui.label(egui::RichText::new(format!("unstaged  {}", self.unstaged_files.len())).size(11.0).color(Color32::from_rgb(0xf3, 0x8b, 0xa8)));
                        for file in &self.unstaged_files {
                            let selected = sel.as_deref() == Some(file.as_str());
                            let color = if selected { Color32::from_rgb(0x89, 0xb4, 0xfa) } else { Color32::from_rgb(0xf3, 0x8b, 0xa8) };
                            let resp = file_ui.add(
                                egui::Label::new(egui::RichText::new(format!("  {file}")).color(color).font(file_font.clone()))
                                    .sense(Sense::click()),
                            ).on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                self.selected_is_staged = false;
                                clicked = Some(file.clone());
                            }
                        }
                    }
                } else {
                    file_ui.label(egui::RichText::new(format!("files changed  {}", file_count)).size(11.0).color(config::C_SUBTEXT));
                    let files: Vec<String> = self.changed_files.clone();
                    for file in &files {
                        let selected = sel.as_deref() == Some(file.as_str());
                        let color = if selected { Color32::from_rgb(0x89, 0xb4, 0xfa) } else { config::C_TEXT };
                        let resp = file_ui.add(
                            egui::Label::new(egui::RichText::new(format!("  {file}")).color(color).font(file_font.clone()))
                                .sense(Sense::click()),
                        ).on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                self.selected_is_staged = false;
                                clicked = Some(file.clone());
                            }
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
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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
        ui.horizontal(|ui| {
            let btn = if app.side_by_side { "Text view" } else { "Side-by-side" };
            if ui.button(btn).clicked() {
                app.side_by_side = !app.side_by_side;
            }
        });
        let (label, text): (Option<String>, &str) = if let Some(file_diff) = &app.file_diff {
            (app.selected_file.clone(), file_diff.as_str())
        } else if let Some(diff) = &app.diff_text {
            (None, diff.as_str())
        } else {
            (None, "")
        };
        if let Some(file) = &label {
            ui.label(egui::RichText::new(file).strong().size(12.0).color(Color32::from_rgb(0x89, 0xb4, 0xfa)));
            ui.add_space(2.0);
        }
        if app.side_by_side {
            diff::draw_diff(ui, text);
        } else {
            let font = FontId::monospace(13.0);
            let mut job = LayoutJob::default();
            for line in text.lines() {
                if line.is_empty() {
                    job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), Color32::TRANSPARENT));
                    continue;
                }
                let ch = line.chars().next().unwrap_or(' ');
                let (fg, bg) = match ch {
                    '+' => (
                        Color32::from_rgb(0xa6, 0xe3, 0xa1),
                        Color32::from_rgba_unmultiplied(0x1a, 0x3c, 0x1a, 180),
                    ),
                    '-' => (
                        Color32::from_rgb(0xf3, 0x8b, 0xa8),
                        Color32::from_rgba_unmultiplied(0x3c, 0x1a, 0x1a, 180),
                    ),
                    '@' => (
                        Color32::from_rgb(0x89, 0xb4, 0xfa),
                        Color32::TRANSPARENT,
                    ),
                    _ => (
                        Color32::from_rgb(0xba, 0xbe, 0xcc),
                        Color32::TRANSPARENT,
                    ),
                };
                let mut fmt = egui::TextFormat::simple(font.clone(), fg);
                fmt.background = bg;
                job.append(&format!("{}\n", line), 0.0, fmt);
            }

            egui::Frame::none().show(ui, |ui| {
                egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add(egui::Label::new(job.clone()));
                });
            });
        }
    });
}

fn draw_graph_inner(app: &mut App, ui: &mut egui::Ui) {
    let total_height = app.rows.len() as f32 * config::ROW_HEIGHT + config::ROW_HEIGHT;
    let text_col_x = app.graph_width;
        let width = ui.available_width().max(text_col_x + 400.0);
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
                if app.rows[idx].is_working {
                    app.selected_is_staged = false;
                    app.load_status();
                    let has_staged = !app.staged_files.is_empty();
                    let has_unstaged = !app.unstaged_files.is_empty();
                    app.rows[idx].summary = match (has_staged, has_unstaged) {
                        (true, false) => "staged changes",
                        (false, true) => "unstaged changes",
                        _ => "working tree changes",
                    }.to_string();
                    app.load_changed_files();
                } else {
                    app.load_changed_files();
                    app.load_diff(false);
                }
            }
        }

        let y_center = |i: usize| origin.y + i as f32 * config::ROW_HEIGHT + config::ROW_HEIGHT / 2.0;
        let x_lane = |l: usize| origin.x + config::GRAPH_PAD_LEFT + l as f32 * config::LANE_WIDTH;

        let x_hash = rect.max.x - config::GRAPH_PAD_RIGHT;
        let x_date = x_hash - config::COL_HASH - 8.0;
        let x_author = x_date - config::COL_DATE - 8.0;
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
            if row.is_working {
                let node_color = if !app.staged_files.is_empty() && app.unstaged_files.is_empty() {
                    Color32::from_rgb(0xf9, 0xe2, 0xaf)
                } else if app.staged_files.is_empty() && !app.unstaged_files.is_empty() {
                    Color32::from_rgb(0xf3, 0x8b, 0xa8)
                } else {
                    Color32::from_rgb(0xf9, 0xe2, 0xaf)
                };
                painter.circle_filled(center, config::NODE_RADIUS + 1.5, Color32::from_rgb(0x1e, 0x1e, 0x2e));
                painter.circle_stroke(center, config::NODE_RADIUS, Stroke::new(2.2_f32, node_color));
            } else {
                let node_color = config::lane_color(row.lane);
                if row.is_head {
                    painter.circle_filled(center, config::NODE_RADIUS + 1.5, Color32::from_rgb(0x1e, 0x1e, 0x2e));
                    painter.circle_stroke(center, config::NODE_RADIUS, Stroke::new(2.2_f32, node_color));
                } else {
                    painter.circle_filled(center, config::NODE_RADIUS, node_color);
                }
            }
        }

        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let mut tx = text_col_x;

            // Cap pills so they don't extend into the metadata columns.
            let cap_x = x_author - 20.0;
            if row.is_working && tx < cap_x {
                let has_staged = !app.staged_files.is_empty();
                let has_unstaged = !app.unstaged_files.is_empty();
                let (pill_label, pill_bg) = match (has_staged, has_unstaged) {
                    (true, false) => ("STAGED", Color32::from_rgb(0xf9, 0xe2, 0xaf)),
                    (false, true) => ("UNSTAGED", Color32::from_rgb(0xf3, 0x8b, 0xa8)),
                    _ => ("WORKING", Color32::from_rgb(0xf9, 0xe2, 0xaf)),
                };
                tx = draw_pill(&painter, tx, yc, pill_label, Color32::from_rgb(0x1e, 0x1e, 0x2e), pill_bg);
            }
            if row.is_head && tx < cap_x {
                tx = draw_pill(&painter, tx, yc, "HEAD", Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xf3, 0x8b, 0xa8));
            }
            for b in &row.branches {
                if tx >= cap_x { break; }
                let is_current = b == &app.current_branch;
                let bg = if is_current { Color32::from_rgb(0xa6, 0xe3, 0xa1) } else { Color32::from_rgb(0x89, 0xb4, 0xfa) };
                tx = draw_pill(&painter, tx, yc, b, Color32::from_rgb(0x1e, 0x1e, 0x2e), bg);
            }
            for t in &row.tags {
                if tx >= cap_x { break; }
                tx = draw_pill(&painter, tx, yc, t, Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xfa, 0xb3, 0x87));
            }

            let msg_color = if row.is_working {
                if !app.staged_files.is_empty() && app.unstaged_files.is_empty() {
                    Color32::from_rgb(0xf9, 0xe2, 0xaf)
                } else if app.staged_files.is_empty() && !app.unstaged_files.is_empty() {
                    Color32::from_rgb(0xf3, 0x8b, 0xa8)
                } else {
                    Color32::from_rgb(0xf9, 0xe2, 0xaf)
                }
            } else { config::C_TEXT };
            let msg = elide(&painter, &row.summary, FontId::proportional(12.5), x_msg_end - tx);
            painter.text(
                Pos2::new(tx, yc),
                egui::Align2::LEFT_CENTER,
                &msg,
                FontId::proportional(12.5),
                msg_color,
            );

            let author = elide(&painter, &row.author, FontId::proportional(11.5), config::COL_AUTHOR);
            painter.text(
                Pos2::new(x_author, yc),
                egui::Align2::RIGHT_CENTER,
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
