//! rgitk-gui — a git commit graph viewer with smooth curved lanes,
//! rendered with egui/eframe. Same commit-graph data model as the
//! terminal `rgitk`, but drawn with cubic-bezier lane transitions
//! instead of ASCII, to match tools like GitKraken/Sourcetree.

use std::collections::HashMap;
use std::process::Command;

use chrono::{FixedOffset, TimeZone};
use eframe::egui;
use egui::{
    epaint::{CubicBezierShape, PathShape},
    Color32, Pos2, Rect, Sense, Stroke, Vec2,
};
use git2::{Oid, Repository};

const LANE_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x89, 0xb4, 0xfa), // blue
    Color32::from_rgb(0xf3, 0x8b, 0xa8), // red/pink
    Color32::from_rgb(0xa6, 0xe3, 0xa1), // green
    Color32::from_rgb(0xcb, 0xa6, 0xf7), // mauve
    Color32::from_rgb(0xf9, 0xe2, 0xaf), // yellow
    Color32::from_rgb(0x89, 0xdc, 0xeb), // sky
];

const ROW_HEIGHT: f32 = 20.0;
const LANE_WIDTH: f32 = 16.0;
const NODE_RADIUS: f32 = 3.5;
const LINE_WIDTH: f32 = 1.8;

fn lane_color(lane: usize) -> Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

struct CommitRow {
    oid: Oid,
    short: String,
    lane: usize,
    passthrough: Vec<usize>,  // lanes alive *before* this commit (drawn straight through)
    parent_lanes: Vec<usize>, // lane each parent lands in (same lane = trunk, else a curve)
    author: String,
    email: String,
    time: i64,
    offset_min: i32,
    summary: String,
    branches: Vec<String>,
    tags: Vec<String>,
    is_head: bool,
}

fn collect_decorations(
    repo: &Repository,
) -> (HashMap<Oid, Vec<String>>, HashMap<Oid, Vec<String>>, Option<Oid>) {
    let mut branches: HashMap<Oid, Vec<String>> = HashMap::new();
    let mut tags: HashMap<Oid, Vec<String>> = HashMap::new();
    let head_oid = repo.head().ok().and_then(|h| h.target());

    if let Ok(refs) = repo.references() {
        for r in refs.flatten() {
            let Some(name) = r.name() else { continue };
            let (label, is_tag) = if let Some(b) = name.strip_prefix("refs/heads/") {
                (b.to_string(), false)
            } else if let Some(t) = name.strip_prefix("refs/tags/") {
                (t.to_string(), true)
            } else if let Some(rb) = name.strip_prefix("refs/remotes/") {
                (rb.to_string(), false)
            } else {
                continue;
            };
            if let Ok(oid) = r.peel_to_commit().map(|c| c.id()) {
                if is_tag {
                    tags.entry(oid).or_default().push(label);
                } else {
                    branches.entry(oid).or_default().push(label);
                }
            }
        }
    }
    (branches, tags, head_oid)
}

fn build_rows(repo: &Repository, limit: usize, all_refs: bool) -> Result<Vec<CommitRow>, git2::Error> {
    let (branch_map, tag_map, head_oid) = collect_decorations(repo);

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

    if all_refs {
        if let Ok(refs) = repo.references() {
            for r in refs.flatten() {
                if let Some(name) = r.name() {
                    if name.starts_with("refs/heads/")
                        || name.starts_with("refs/tags/")
                        || name.starts_with("refs/remotes/")
                    {
                        if let Ok(oid) = r.peel_to_commit().map(|c| c.id()) {
                            let _ = revwalk.push(oid);
                        }
                    }
                }
            }
        }
    } else {
        revwalk.push_head()?;
    }

    let mut active_lanes: HashMap<Oid, usize> = HashMap::new();
    let mut next_free_lane: usize = 0;
    let mut rows = Vec::new();

    for oid_res in revwalk.take(limit) {
        let oid = oid_res?;
        let commit = repo.find_commit(oid)?;
        let parents: Vec<Oid> = commit.parent_ids().collect();

        // Lane for this commit: reuse if a prior child already reserved one for us.
        let my_lane = if let Some(l) = active_lanes.remove(&oid) {
            l
        } else {
            let l = next_free_lane;
            next_free_lane += 1;
            l
        };

        // Snapshot of lanes that were already alive *before* this commit touches
        // anything — these get drawn as plain straight pass-through lines.
        let mut passthrough: Vec<usize> = active_lanes
            .values()
            .copied()
            .filter(|&l| l != my_lane)
            .collect();
        passthrough.sort_unstable();
        passthrough.dedup();

        // Now place each parent into a lane (first parent continues our lane).
        let mut parent_lanes = Vec::with_capacity(parents.len());
        for (i, pid) in parents.iter().enumerate() {
            let lane = if let Some(&l) = active_lanes.get(pid) {
                l
            } else {
                let l = if i == 0 {
                    my_lane
                } else {
                    let l = next_free_lane;
                    next_free_lane += 1;
                    l
                };
                active_lanes.insert(*pid, l);
                l
            };
            parent_lanes.push(lane);
        }

        let author = commit.author();
        let time = commit.time();
        let oid_str = oid.to_string();

        rows.push(CommitRow {
            oid,
            short: oid_str[..7.min(oid_str.len())].to_string(),
            lane: my_lane,
            passthrough,
            parent_lanes,
            author: author.name().unwrap_or("unknown").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: time.seconds(),
            offset_min: time.offset_minutes(),
            summary: commit.summary().unwrap_or("").to_string(),
            branches: branch_map.get(&oid).cloned().unwrap_or_default(),
            tags: tag_map.get(&oid).cloned().unwrap_or_default(),
            is_head: head_oid == Some(oid),
        });
    }

    Ok(rows)
}

fn format_time(secs: i64, offset_min: i32) -> String {
    let tz = FixedOffset::east_opt(offset_min * 60).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    match tz.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        _ => String::from("?"),
    }
}

fn max_lanes(rows: &[CommitRow]) -> usize {
    let mut m = 0usize;
    for r in rows {
        m = m.max(r.lane + 1);
        for &l in &r.passthrough {
            m = m.max(l + 1);
        }
        for &l in &r.parent_lanes {
            m = m.max(l + 1);
        }
    }
    m.max(1)
}

struct App {
    repo_path: String,
    rows: Vec<CommitRow>,
    selected: Option<usize>,
    search: String,
    diff_text: Option<String>,
    limit: usize,
    all_refs: bool,
    error: Option<String>,
    graph_width: f32,
}

impl App {
    fn new(repo_path: String, limit: usize, all_refs: bool) -> Self {
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
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        match Repository::discover(&self.repo_path) {
            Ok(repo) => match build_rows(&repo, self.limit, self.all_refs) {
                Ok(rows) => {
                    self.graph_width = max_lanes(&rows) as f32 * LANE_WIDTH + 12.0;
                    self.rows = rows;
                    self.selected = if self.rows.is_empty() { None } else { Some(0) };
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("failed to read commits: {e}")),
            },
            Err(e) => self.error = Some(format!("not a git repository: {e}")),
        }
    }

    fn load_diff(&mut self, full: bool) {
        if let Some(i) = self.selected {
            let hash = self.rows[i].oid.to_string();
            let mut args = vec!["show"];
            if !full {
                args.push("--stat");
            }
            args.push(&hash);
            let out = Command::new("git")
                .current_dir(&self.repo_path)
                .args(&args)
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
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&self.repo_path).strong());
                ui.separator();
                if ui.button("Refresh").clicked() {
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
                if ui.button("Find next").clicked() {
                    self.find_next();
                }
                ui.separator();
                ui.label(format!("{} commits", self.rows.len()));
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
            .min_width(360.0)
            .resizable(true)
            .show(ctx, |ui| {
                draw_details(self, ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            draw_graph(self, ui);
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
                format_time(row.time, row.offset_min),
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
            if ui.button("Full diff").clicked() {
                app.load_diff(true);
            }
            if ui.button("Stat only").clicked() {
                app.load_diff(false);
            }
            if ui.button("Clear").clicked() {
                app.diff_text = None;
            }
        });
        ui.separator();
        if let Some(diff) = &app.diff_text {
            egui::ScrollArea::vertical()
                .id_source("diff_scroll")
                .max_height(f32::INFINITY)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut diff.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                });
        }
    });
}

fn draw_graph(app: &mut App, ui: &mut egui::Ui) {
    let total_height = app.rows.len() as f32 * ROW_HEIGHT + ROW_HEIGHT;
    let text_col_x = app.graph_width;

    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width().max(text_col_x + 600.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, total_height), Sense::click());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        // Handle clicks -> select row
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let row_idx = ((pos.y - origin.y) / ROW_HEIGHT) as usize;
                if row_idx < app.rows.len() {
                    app.selected = Some(row_idx);
                    app.diff_text = None;
                }
            }
        }

        let y_center = |i: usize| origin.y + i as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
        let x_lane = |l: usize| origin.x + 8.0 + l as f32 * LANE_WIDTH;

        // Selection highlight
        if let Some(sel) = app.selected {
            let top = origin.y + sel as f32 * ROW_HEIGHT;
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rect.min.x, top), Vec2::new(rect.width(), ROW_HEIGHT)),
                0.0,
                Color32::from_rgba_unmultiplied(0x31, 0x32, 0x44, 160),
            );
        }

        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let yc_next = y_center(i + 1);

            // 1. straight pass-through lanes (unrelated to this commit)
            for &l in &row.passthrough {
                let x = x_lane(l);
                painter.line_segment(
                    [Pos2::new(x, yc), Pos2::new(x, yc_next)],
                    Stroke::new(LINE_WIDTH, lane_color(l)),
                );
            }

            // 2. connectors to each parent: straight if same lane, curved otherwise
            let x_my = x_lane(row.lane);
            for &pl in &row.parent_lanes {
                let x_p = x_lane(pl);
                if pl == row.lane {
                    painter.line_segment(
                        [Pos2::new(x_my, yc), Pos2::new(x_p, yc_next)],
                        Stroke::new(LINE_WIDTH, lane_color(row.lane)),
                    );
                } else {
                    let mid_y = (yc + yc_next) / 2.0;
                    let points = [
                        Pos2::new(x_my, yc),
                        Pos2::new(x_my, mid_y),
                        Pos2::new(x_p, mid_y),
                        Pos2::new(x_p, yc_next),
                    ];
                    let bez = CubicBezierShape::from_points_stroke(
                        points,
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(LINE_WIDTH, lane_color(pl)),
                    );
                    painter.add(bez);
                }
            }

            // 3. node circle
            let node_color = lane_color(row.lane);
            painter.circle_filled(Pos2::new(x_my, yc), NODE_RADIUS, node_color);
            if row.is_head {
                painter.circle_stroke(
                    Pos2::new(x_my, yc),
                    NODE_RADIUS + 2.0,
                    Stroke::new(1.4_f32, Color32::WHITE),
                );
            }

            // 4. text: hash, badges, message — to the right of the lane gutter
            let mut tx = text_col_x + 6.0;
            let text_y = yc;

            painter.text(
                Pos2::new(tx, text_y),
                egui::Align2::LEFT_CENTER,
                &row.short,
                egui::FontId::monospace(11.0),
                Color32::from_rgb(0xa6, 0xe3, 0xa1),
            );
            tx += 54.0;

            if row.is_head {
                let galley = painter.layout_no_wrap(
                    "HEAD".into(),
                    egui::FontId::monospace(10.0),
                    Color32::from_rgb(0x11, 0x11, 0x1b),
                );
                let badge_rect = Rect::from_min_size(
                    Pos2::new(tx, text_y - 7.0),
                    Vec2::new(galley.size().x + 6.0, 14.0),
                );
                painter.rect_filled(badge_rect, 2.0, Color32::from_rgb(0xa6, 0xe3, 0xa1));
                painter.galley(badge_rect.min + Vec2::new(3.0, 0.0), galley, Color32::BLACK);
                tx += badge_rect.width() + 4.0;
            }
            for b in &row.branches {
                tx = draw_badge(&painter, tx, text_y, b, Color32::from_rgb(0x89, 0xdc, 0xeb));
            }
            for t in &row.tags {
                tx = draw_badge(&painter, tx, text_y, t, Color32::from_rgb(0xfa, 0xb3, 0x87));
            }

            painter.text(
                Pos2::new(tx, text_y),
                egui::Align2::LEFT_CENTER,
                &row.summary,
                egui::FontId::proportional(12.0),
                Color32::from_rgb(0xcd, 0xd6, 0xf4),
            );
        }

        // draw a faint frame around the whole content so long scroll areas read clearly
        let _ = PathShape::convex_polygon(vec![], Color32::TRANSPARENT, Stroke::NONE); // no-op keeps import used
    });
}

fn draw_badge(painter: &egui::Painter, x: f32, y: f32, label: &str, color: Color32) -> f32 {
    let galley = painter.layout_no_wrap(label.to_string(), egui::FontId::monospace(10.0), Color32::BLACK);
    let rect = Rect::from_min_size(Pos2::new(x, y - 7.0), Vec2::new(galley.size().x + 6.0, 14.0));
    painter.rect_filled(rect, 2.0, color);
    painter.galley(rect.min + Vec2::new(3.0, 0.0), galley, Color32::BLACK);
    rect.max.x + 4.0
}

fn print_help() {
    println!("rgitk-gui — git commit graph viewer with smooth curved lanes\n");
    println!("USAGE:");
    println!("  rgitk-gui [path] [--limit N] [--current] [--help]\n");
    println!("OPTIONS:");
    println!("  path        repository path (default: current directory)");
    println!("  --limit N   max commits to load (default: 1000)");
    println!("  --current   only walk the current branch (default: all refs)");
    println!("  --help      show this message");
}

fn main() -> eframe::Result<()> {
    let mut path = String::from(".");
    let mut limit: usize = 1000;
    let mut all_refs = true;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--limit" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    limit = v.parse().unwrap_or(1000);
                }
            }
            "--current" => all_refs = false,
            other => path = other.to_string(),
        }
        i += 1;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "rgitk-gui",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(path, limit, all_refs)))),
    )
}
