use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
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

pub enum UpdateState {
    Idle,
    Checking,
    Available { version: String, url: String, notes: String },
    Downloading,
    Ready { path: String },
    Failed(String),
    UpToDate,
}

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
    pub initial_load: bool,
    pub diff_loading: Arc<AtomicBool>,
    pub file_diff_loading: Arc<AtomicBool>,
    pub pending_diff: Arc<std::sync::Mutex<Option<String>>>,
    pub pending_file_diff: Arc<std::sync::Mutex<Option<(String, String, bool)>>>,
    pub context_branch: Option<String>,
    pub show_rename: bool,
    pub rename_old: String,
    pub rename_new: String,
    pub confirm_delete: Option<String>,
    pub del_origin: bool,
    pub confirm_checkout: Option<String>,
    pub update_state: UpdateState,
    pub pending_update: Arc<std::sync::Mutex<Option<UpdateState>>>,
    pub dl_progress: Option<(Arc<std::sync::Mutex<u64>>, Arc<std::sync::Mutex<u64>>)>,
    pub replace_failed: bool,
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
        let app = App {
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
            initial_load: true,
            diff_loading: Arc::new(AtomicBool::new(false)),
            file_diff_loading: Arc::new(AtomicBool::new(false)),
            pending_diff: Arc::new(std::sync::Mutex::new(None)),
            pending_file_diff: Arc::new(std::sync::Mutex::new(None)),
            context_branch: None,
            show_rename: false,
            rename_old: String::new(),
            rename_new: String::new(),
            confirm_delete: None,
            del_origin: false,
            confirm_checkout: None,
            update_state: UpdateState::Idle,
            pending_update: Arc::new(std::sync::Mutex::new(None)),
            dl_progress: None,
            replace_failed: false,
        };
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
                                is_stash: false,
                            });
                        }
                    }
                    if let Ok(out) = Command::new("git").current_dir(&self.repo_path).args(&["stash", "list", "--format=%gd|%H|%s"]).output() {
                        let text = String::from_utf8_lossy(&out.stdout);
                        for line in text.lines() {
                            let parts: Vec<&str> = line.splitn(3, '|').collect();
                            if parts.len() < 2 { continue; }
                            let stash_ref = parts[0].to_string();
                            let stash_oid = parts[1];
                            let msg = parts.get(2).unwrap_or(&"").to_string();
                            let parent_oid = String::from_utf8_lossy(
                                &Command::new("git").current_dir(&self.repo_path).args(&["rev-parse", &format!("{stash_oid}^")]).output().map(|o| o.stdout).unwrap_or_default()
                            ).trim().to_string();
                            let parent_lane = rows.iter().find(|r| r.oid.to_string() == parent_oid).map(|r| r.lane).unwrap_or(0);
                            let stash_lane = rows.iter().map(|r| r.lane).max().unwrap_or(0) + 1;
                            rows.insert(1, commit::CommitRow {
                                oid: git2::Oid::from_str(stash_oid).unwrap_or(git2::Oid::zero()),
                                short: stash_ref.clone(),
                                lane: stash_lane,
                                passthrough: Vec::new(),
                                parent_lanes: vec![parent_lane],
                                author: String::new(),
                                email: String::new(),
                                time: 0,
                                offset_min: 0,
                                summary: msg,
                                branches: vec![stash_ref],
                                tags: Vec::new(),
                                is_head: false,
                                is_working: false,
                                is_stash: true,
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
            self.file_diff_loading.store(true, Ordering::SeqCst);
            self.file_diff = None;
            let repo = self.repo_path.clone();
            let f = file.to_string();
            let pending = self.pending_file_diff.clone();
            let is_staged = self.selected_is_staged;
            let is_working = self.rows[i].is_working;
            let oid = self.rows[i].oid.to_string();
            std::thread::spawn(move || {
                let args: Vec<String> = if is_working {
                    if is_staged {
                        vec!["diff".into(), "--cached".into(), "--".into(), f.clone()]
                    } else {
                        vec!["diff".into(), "--".into(), f.clone()]
                    }
                } else {
                    vec!["show".into(), oid, "--".into(), f.clone()]
                };
                let out = std::process::Command::new("git").current_dir(&repo).args(&args).output();
                let diff = match out {
                    Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                    Err(e) => format!("failed: {e}"),
                };
                *pending.lock().unwrap() = Some((f, diff, is_staged));
            });
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
        if let Some(state) = self.pending_update.lock().unwrap().take() {
            self.update_state = state;
        }
        if self.initial_load {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(40.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.add(egui::Spinner::new());
                    ui.label("Loading repository…");
                });
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            self.reload();
            self.initial_load = false;
            return;
        }
        if let Some(text) = self.pending_diff.lock().unwrap().take() {
            self.diff_text = Some(text);
            self.diff_loading.store(false, Ordering::SeqCst);
        }
        if let Some((file, diff, staged)) = self.pending_file_diff.lock().unwrap().take() {
            self.file_diff = Some(diff);
            self.selected_file = Some(file);
            self.selected_is_staged = staged;
            self.file_diff_loading.store(false, Ordering::SeqCst);
        }
        if self.needs_reload.swap(false, Ordering::SeqCst) {
            self.reload();
        }
        if self.diff_loading.load(Ordering::SeqCst) || self.file_diff_loading.load(Ordering::SeqCst) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
        }
        if ctx.input(|i| (i.key_pressed(egui::Key::Q) || i.key_pressed(egui::Key::C)) && i.modifiers.ctrl)
            || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
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
                    if ui.button("Check for Updates").clicked() {
                        self.update_state = UpdateState::Checking;
                        let repo = config::REPO.to_string();
                        let current = config::VERSION.to_string();
                        let pending = self.pending_update.clone();
                        std::thread::spawn(move || {
                            let result = check_update(&repo, &current);
                            *pending.lock().unwrap() = Some(result);
                        });
                    }
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
                    let galley = ui.painter().layout_no_wrap(format!(" {branch} "), FontId::proportional(12.0), fg);
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
                ui.label(egui::RichText::new(format!("Version {}", config::VERSION)).size(12.0).color(config::C_SUBTEXT));
                ui.separator();
                ui.add_space(4.0);
                ui.label("Inspired by gitg (GNOME Git graphical interface) and gitk.");
                ui.add_space(4.0);
                ui.label("I love the git graph tree view and gitk is very easy to access in the terminal. I wanted both combined into one tool - so I built it with Rust.");
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

        match &self.update_state {
            UpdateState::Checking => {
                egui::Window::new("Update")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new());
                            ui.label("Checking for updates…");
                        });
                    });
            }
            UpdateState::Available { version, url, notes } => {
                let v = version.clone();
                let u = url.clone();
                let n = notes.clone();
                egui::Window::new("Update Available")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(egui::RichText::new(format!("gitr v{v}")).strong().size(16.0).color(Color32::from_rgb(0xa6, 0xe3, 0xa1)));
                        ui.add_space(4.0);
                        if !n.is_empty() {
                            ui.add(egui::Label::new(&n).wrap());
                            ui.add_space(4.0);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("Download & Install").clicked() {
                                let pending = self.pending_update.clone();
                                let dl_url = u.clone();
                                let dest = std::env::temp_dir().join("gitr-update").to_string_lossy().to_string();
                                let dl = Arc::new(std::sync::Mutex::new(0u64));
                                let dt = Arc::new(std::sync::Mutex::new(0u64));
                                let dl2 = dl.clone();
                                let dt2 = dt.clone();
                                let d = dest.clone();
                                self.dl_progress = Some((dl.clone(), dt.clone()));
                                std::thread::spawn(move || {
                                    match download_update(&dl_url, &d, &dl2, &dt2) {
                                        Ok(()) => {
                                            *pending.lock().unwrap() = Some(UpdateState::Ready { path: d });
                                        }
                                        Err(e) => {
                                            *pending.lock().unwrap() = Some(UpdateState::Failed(e));
                                        }
                                    }
                                });
                                self.update_state = UpdateState::Downloading;
                            }
                            if ui.button("Later").clicked() {
                                self.update_state = UpdateState::Idle;
                            }
                        });
                    });
            }
            UpdateState::Downloading { .. } => {
                let (d, t) = self.dl_progress.as_ref().map(|(dl, dt)| {
                    (*dl.lock().unwrap(), *dt.lock().unwrap())
                }).unwrap_or((0, 0));
                egui::Window::new("Downloading")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        let pct = if t > 0 { d as f64 / t as f64 } else { 0.0 };
                        let mb_d = d as f64 / 1_048_576.0;
                        let mb_t = t as f64 / 1_048_576.0;
                        ui.add(egui::ProgressBar::new(pct as f32).text(format!("{mb_d:.1} MB / {mb_t:.1} MB")));
                        ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    });
            }
            UpdateState::Ready { path } => {
                let p = path.clone();
                let mut close = false;
                let exe = std::env::var("APPIMAGE").ok()
                    .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()))
                    .unwrap_or_default();
                let is_appimage = std::env::var("APPIMAGE").is_ok();
                egui::Window::new("Update Ready")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("Download complete.");
                        ui.add_space(4.0);
                        let file_size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        if file_size == 0 {
                            ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), "Downloaded file is empty or missing.");
                        }
                        if ui.button("Replace & Restart").clicked() {
                            let replaced = if file_size == 0 { false } else {
                                let backup = format!("{exe}.bak");
                                let _ = std::fs::copy(&exe, &backup);
                                if is_appimage {
                                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
                                }
                                // Try direct copy first
                                let mut ok = std::fs::copy(&p, &exe).is_ok();
                                // Try pkexec (GUI privilege escalation) if direct fails
                                if !ok {
                                    ok = std::process::Command::new("pkexec")
                                        .args(&["cp", &p, &exe])
                                        .status().map(|s| s.success()).unwrap_or(false);
                                }
                                if ok {
                                    let _ = std::fs::remove_file(&backup);
                                }
                                ok
                            };
                            if replaced {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                close = true;
                            } else {
                                self.replace_failed = true;
                            }
                        }
                        if self.replace_failed {
                            ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), "Could not replace the binary.");
                            ui.add_space(2.0);
                            ui.label("Run this in your terminal:");
                            ui.monospace(format!("cp {p} {exe}"));
                            ui.add_space(2.0);
                            ui.label("Then make it executable:");
                            ui.monospace(format!("chmod +x {exe}"));
                        }
                        if ui.button("Cancel").clicked() {
                            let _ = std::fs::remove_file(&p);
                            self.update_state = UpdateState::Idle;
                            self.replace_failed = false;
                            close = true;
                        }
                    });
                if close { self.update_state = UpdateState::Idle; }
            }
            UpdateState::Failed(msg) => {
                let m = msg.clone();
                let mut close = false;
                egui::Window::new("Update Failed")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), &m);
                        ui.add_space(4.0);
                        ui.label("Download manually:");
                        ui.hyperlink_to("GitHub Releases", format!("https://github.com/{}/releases", config::REPO));
                        if ui.button("OK").clicked() {
                            close = true;
                        }
                    });
                if close { self.update_state = UpdateState::Idle; }
            }
            UpdateState::UpToDate => {
                let mut close = false;
                egui::Window::new("Update")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(egui::RichText::new("You're up to date ✓").color(Color32::from_rgb(0xa6, 0xe3, 0xa1)).size(14.0));
                        if ui.button("OK").clicked() {
                            close = true;
                        }
                    });
                if close { self.update_state = UpdateState::Idle; }
            }
            _ => {}
        }

        if self.show_rename {
            let old = self.rename_old.clone();
            let repo_path = self.repo_path.clone();
            let mut close = false;
            egui::Window::new("Rename branch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Rename \"{old}\" to:"));
                    ui.text_edit_singleline(&mut self.rename_new);
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked() {
                            let _ = std::process::Command::new("git")
                                .current_dir(&repo_path)
                                .args(&["branch", "-m", &old, &self.rename_new])
                                .output();
                            self.reload();
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.show_rename = false;
            }
        }

        if let Some(branch) = &self.confirm_delete.clone() {
            let del_branch = branch.clone();
            let repo_path = self.repo_path.clone();
            let mut close = false;
            egui::Window::new("Delete branch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Delete \"{del_branch}\"?"));
                    ui.add_space(4.0);
                    ui.checkbox(&mut self.del_origin, "also delete from origin");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Yes").clicked() {
                            let _ = std::process::Command::new("git").current_dir(&repo_path).args(&["branch", "-d", &del_branch]).output();
                            if self.del_origin {
                                let _ = std::process::Command::new("git").current_dir(&repo_path).args(&["push", "origin", "--delete", &del_branch]).output();
                            }
                            self.reload();
                            close = true;
                        }
                        if ui.button("No").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.confirm_delete = None;
            }
        }

        if let Some(branch) = &self.confirm_checkout.clone() {
            let target = branch.clone();
            let repo_path = self.repo_path.clone();
            let has_staged = !self.staged_files.is_empty();
            let has_unstaged = !self.unstaged_files.is_empty();
            let has_changes = has_staged || has_unstaged;
            let mut close = false;
            egui::Window::new("Switch branch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(format!("Switch to \"{target}\"?")).strong().size(14.0));
                    ui.add_space(6.0);
                    if has_changes {
                        ui.colored_label(Color32::from_rgb(0xf3, 0x8b, 0xa8), "You have uncommitted changes.");
                        ui.label("These will be stashed before switching.");
                        ui.add_space(6.0);
                    }
                    ui.horizontal(|ui| {
                        if has_changes {
                            if ui.button("Stash & Checkout").clicked() {
                                let _ = std::process::Command::new("git").current_dir(&repo_path).args(&["stash"]).output();
                                let _ = std::process::Command::new("git").current_dir(&repo_path).args(&["checkout", &target]).output();
                                self.reload();
                                close = true;
                            }
                        } else {
                            if ui.button("Checkout").clicked() {
                                let _ = std::process::Command::new("git").current_dir(&repo_path).args(&["checkout", &target]).output();
                                self.reload();
                                close = true;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if close {
                self.confirm_checkout = None;
            }
        }
    }
}

fn draw_details(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let Some(i) = app.selected else {
            ui.label("no commit selected");
            return;
        };
        if app.diff_loading.load(Ordering::SeqCst) || app.file_diff_loading.load(Ordering::SeqCst) {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label("Loading diff…");
            });
            return;
        }
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
        let avail_w = ui.available_width().max(text_col_x);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(avail_w, total_height), Sense::click());
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
                    app.diff_loading.store(true, Ordering::SeqCst);
                    app.diff_text = None;
                    let repo = app.repo_path.clone();
                    let hash = app.rows[idx].oid.to_string();
                    let pending = app.pending_diff.clone();
                    std::thread::spawn(move || {
                        let out = std::process::Command::new("git").current_dir(&repo).args(&["show", &hash]).output();
                        let text = match out {
                            Ok(o) => Some(String::from_utf8_lossy(&o.stdout).to_string()),
                            Err(e) => Some(format!("failed to run git show: {e}")),
                        };
                        *pending.lock().unwrap() = text;
                    });
                }
            }
        }

        let y_center = |i: usize| origin.y + i as f32 * config::ROW_HEIGHT + config::ROW_HEIGHT / 2.0;
        let x_lane = |l: usize| origin.x + config::GRAPH_PAD_LEFT + l as f32 * config::LANE_WIDTH;

        let meta_w = config::COL_HASH + 8.0 + config::COL_DATE + 8.0 + config::COL_AUTHOR + 12.0 + config::GRAPH_PAD_RIGHT;
        let space_for_msg = avail_w - text_col_x - meta_w;
        let show_hash = space_for_msg > 120.0;
        let show_date = space_for_msg > 220.0;
        let show_author = space_for_msg > 340.0;
        let x_hash = rect.max.x - config::GRAPH_PAD_RIGHT;
        let x_date = if show_date { x_hash - config::COL_HASH - 8.0 } else { x_hash };
        let x_author = if show_author { x_date - config::COL_DATE - 8.0 } else { x_date };
        let x_msg_end = if show_author { x_author - config::COL_AUTHOR - 12.0 } else { avail_w - 10.0 };

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
            if row.is_stash {
                painter.circle_filled(center, config::NODE_RADIUS + 1.5, Color32::from_rgb(0x1e, 0x1e, 0x2e));
                painter.circle_stroke(center, config::NODE_RADIUS, Stroke::new(2.2_f32, Color32::from_rgb(0xcb, 0xa6, 0xf7)));
            } else if row.is_working {
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

        response.context_menu(|ui| {
            if let Some(branch) = &app.context_branch.clone() {
                let is_stash = branch.starts_with("stash@{");
                ui.set_min_width(200.0);
                let label_color = if is_stash { Color32::from_rgb(0xcb, 0xa6, 0xf7) } else { Color32::from_rgb(0x89, 0xb4, 0xfa) };
                ui.label(egui::RichText::new(branch).strong().size(12.0).color(label_color));
                ui.separator();
                if is_stash {
                    if ui.button("Apply stash").clicked() {
                        let _ = std::process::Command::new("git").current_dir(&app.repo_path).args(&["stash", "apply", branch]).output();
                        app.reload();
                    }
                    if ui.button("Drop stash").clicked() {
                        let _ = std::process::Command::new("git").current_dir(&app.repo_path).args(&["stash", "drop", branch]).output();
                        app.reload();
                    }
                } else {
                    let is_current = branch == &app.current_branch;
                    let is_remote = branch.contains('/');
                    if is_remote {
                        if ui.button("Checkout as local branch").clicked() {
                            let local = branch.split('/').last().unwrap_or(branch);
                            let _ = std::process::Command::new("git").current_dir(&app.repo_path).args(&["checkout", "-b", local, branch]).output();
                            app.reload();
                        }
                    } else {
                        if ui.button("Checkout").clicked() {
                            app.confirm_checkout = Some(branch.clone());
                        }
                    }
                    if !is_remote {
                        if ui.button("Rename branch…").clicked() {
                            app.rename_old = branch.clone();
                            app.rename_new = branch.clone();
                            app.show_rename = true;
                        }
                        if !is_current && ui.button("Delete branch").clicked() {
                            app.confirm_delete = Some(branch.clone());
                            app.del_origin = false;
                        }
                    }
                }
            }
        });

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
                let bg = if row.is_stash {
                    Color32::from_rgb(0xcb, 0xa6, 0xf7)
                } else if is_current { Color32::from_rgb(0xa6, 0xe3, 0xa1) } else { Color32::from_rgb(0x89, 0xb4, 0xfa) };
                let pill_start = tx;
                tx = draw_pill(&painter, tx, yc, b, Color32::from_rgb(0x1e, 0x1e, 0x2e), bg);
                if let Some(pos) = response.hover_pos() {
                    let row_top = origin.y + i as f32 * config::ROW_HEIGHT;
                    if pos.y >= row_top && pos.y < row_top + config::ROW_HEIGHT && pos.x >= pill_start && pos.x < tx {
                        app.context_branch = Some(b.clone());
                    }
                }
            }
            for t in &row.tags {
                if tx >= cap_x { break; }
                tx = draw_pill(&painter, tx, yc, t, Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xfa, 0xb3, 0x87));
            }

            let msg_color = if row.is_stash {
                Color32::from_rgb(0xcb, 0xa6, 0xf7)
            } else if row.is_working {
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

            if show_author {
                let author = elide(&painter, &row.author, FontId::proportional(11.5), config::COL_AUTHOR);
                painter.text(
                    Pos2::new(x_author, yc),
                    egui::Align2::RIGHT_CENTER,
                    &author,
                    FontId::proportional(11.5),
                    config::C_SUBTEXT,
                );
            }

            if show_date {
                painter.text(
                    Pos2::new(x_date, yc),
                    egui::Align2::RIGHT_CENTER,
                    commit::format_time(row.time, row.offset_min),
                    FontId::proportional(11.5),
                    config::C_SUBTEXT,
                );
            }

            if show_hash {
                painter.text(
                    Pos2::new(x_hash, yc),
                    egui::Align2::RIGHT_CENTER,
                    &row.short,
                    FontId::monospace(11.0),
                    config::C_HASH,
                );
            }
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

fn check_update(repo: &str, current: &str) -> UpdateState {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    match ureq::get(&url).set("User-Agent", "gitr").call() {
        Ok(resp) => {
            let body = match resp.into_string() {
                Ok(b) => b,
                Err(_) => return UpdateState::Failed("Failed to read response".into()),
            };
            let json: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => return UpdateState::Failed("Invalid response".into()),
            };
            let raw_tag = json["tag_name"].as_str().unwrap_or("");
            let notes = json["body"].as_str().unwrap_or("").to_string();
            let assets = json["assets"].as_array().map(|a| {
                a.iter().filter_map(|a| {
                    let name = a["name"].as_str()?;
                    let browser = a["browser_download_url"].as_str()?;
                    Some((name.to_string(), browser.to_string()))
                }).collect::<Vec<_>>()
            }).unwrap_or_default();
            if raw_tag.is_empty() {
                return UpdateState::Failed("No releases found".into());
            }
            // Only compare version tags (starting with "v"), skip "latest" / rolling tags
            if let Some(ver) = raw_tag.strip_prefix('v') {
                if ver == current {
                    return UpdateState::UpToDate;
                }
                // Find AppImage or binary URL
                let url = assets.iter().find(|(n, _)| n.contains("x86_64.AppImage"))
                    .or_else(|| assets.iter().find(|(n, _)| n.contains("linux-x86_64.tar.gz")))
                    .map(|(_, u)| u.clone())
                    .unwrap_or_else(|| format!("https://github.com/{repo}/releases/tag/{raw_tag}"));
                return UpdateState::Available { version: ver.to_string(), url, notes };
            }
            // Non-version tag (e.g. "latest") — assume we're up to date
            UpdateState::UpToDate
        }
        Err(e) => UpdateState::Failed(format!("Network error: {e}")),
    }
}

fn download_update(url: &str, dest: &str, downloaded: &Arc<std::sync::Mutex<u64>>, total: &Arc<std::sync::Mutex<u64>>) -> Result<(), String> {
    let resp = ureq::get(url).set("User-Agent", "gitr").call().map_err(|e| format!("Download failed: {e}"))?;
    let len = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    *total.lock().unwrap() = len;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("Cannot create file: {e}"))?;
    let mut buf = [0u8; 65536];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Read error: {e}"))?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| format!("Write error: {e}"))?;
        done += n as u64;
        *downloaded.lock().unwrap() = done;
    }
    Ok(())
}
