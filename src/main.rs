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

const ROW_HEIGHT: f32 = 24.0;
const LANE_WIDTH: f32 = 14.0;
const NODE_RADIUS: f32 = 5.0;
const LINE_WIDTH: f32 = 2.0;
const GRAPH_PAD_LEFT: f32 = 12.0;
const GRAPH_PAD_RIGHT: f32 = 16.0;

// column widths for the right-aligned metadata block (author | date | hash)
const COL_AUTHOR: f32 = 150.0;
const COL_DATE: f32 = 130.0;
const COL_HASH: f32 = 72.0;

// palette (Catppuccin Mocha)
const C_TEXT: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
const C_SUBTEXT: Color32 = Color32::from_rgb(0x93, 0x99, 0xb2);
const C_HASH: Color32 = Color32::from_rgb(0xf9, 0xe2, 0xaf);
const C_SEL: Color32 = Color32::from_rgba_premultiplied(0x31, 0x32, 0x44, 200);
const C_HOVER: Color32 = Color32::from_rgba_premultiplied(0x28, 0x28, 0x38, 120);

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
    // Reclaimed lane columns, kept sorted so the lowest free lane is reused
    // first — this keeps the graph packed to the left.
    let mut free_lanes: Vec<usize> = Vec::new();

    // Allocate the lowest available lane: prefer a recycled one, else a new one.
    let alloc_lane = |free_lanes: &mut Vec<usize>, next_free_lane: &mut usize| -> usize {
        if let Some(l) = free_lanes.pop() {
            l
        } else {
            let l = *next_free_lane;
            *next_free_lane += 1;
            l
        }
    };

    let mut rows = Vec::new();

    for oid_res in revwalk.take(limit) {
        let oid = oid_res?;
        let commit = repo.find_commit(oid)?;
        let parents: Vec<Oid> = commit.parent_ids().collect();

        // Lane for this commit: reuse if a prior child already reserved one for us.
        let my_lane = if let Some(l) = active_lanes.remove(&oid) {
            l
        } else {
            alloc_lane(&mut free_lanes, &mut next_free_lane)
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

        // Lanes that were alive going *into* this row.
        let mut alive_before: Vec<usize> = passthrough.clone();
        alive_before.push(my_lane);

        // Now place each parent into a lane (first parent continues our lane).
        let mut parent_lanes = Vec::with_capacity(parents.len());
        for (i, pid) in parents.iter().enumerate() {
            let lane = if let Some(&l) = active_lanes.get(pid) {
                l
            } else {
                let l = if i == 0 {
                    my_lane
                } else {
                    alloc_lane(&mut free_lanes, &mut next_free_lane)
                };
                active_lanes.insert(*pid, l);
                l
            };
            parent_lanes.push(lane);
        }

        // Reclaim any lane that was alive before this row but no longer has a
        // future commit waiting on it — a branch tip or a fully-merged lane.
        // Those columns become available for later diverging branches so the
        // graph stays packed to the left instead of drifting right.
        for l in alive_before {
            if !active_lanes.values().any(|&al| al == l) && !free_lanes.contains(&l) {
                free_lanes.push(l);
            }
        }
        // Keep highest lane at the end so pop() hands out the lowest first.
        free_lanes.sort_unstable_by(|a, b| b.cmp(a));

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
                    self.graph_width =
                        GRAPH_PAD_LEFT + max_lanes(&rows) as f32 * LANE_WIDTH + GRAPH_PAD_RIGHT;
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
            .min_width(520.0)
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
            draw_diff(ui, diff);
        }
    });
}
fn draw_diff(ui: &mut egui::Ui, diff: &str) {
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

        // Dark background
        painter.rect_filled(rect, 0.0, bg);

        // Center divider
        let div_x = origin.x + col_w;
        painter.line_segment(
            [Pos2::new(div_x, origin.y), Pos2::new(div_x, origin.y + total_h)],
            Stroke::new(1.0_f32, Color32::from_rgb(0x36, 0x39, 0x4f)),
        );

        for (i, row) in rows.iter().enumerate() {
            let y = origin.y + 2.0 + i as f32 * line_h;

            // --- header spanning both sides ---
            if row.header.is_some() {
                if let Some(hdr) = &row.header {
                    painter.text(
                        Pos2::new(origin.x + 6.0, y + line_h / 2.0),
                        egui::Align2::LEFT_CENTER,
                        hdr,
                        font.clone(),
                        Color32::from_rgb(0x89, 0xb4, 0xfa),
                    );
                }
                continue;
            }

            // --- left side ---
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

            // --- right side ---
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

struct SideRow {
    header: Option<String>,
    left: Option<(String, String, bool)>,
    right: Option<(String, String, bool)>,
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

fn draw_graph(app: &mut App, ui: &mut egui::Ui) {
    let total_height = app.rows.len() as f32 * ROW_HEIGHT + ROW_HEIGHT;
    let text_col_x = app.graph_width;

    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width().max(text_col_x + 600.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, total_height), Sense::click());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        // pointer -> which row is under the cursor
        let hover_row = response.hover_pos().and_then(|pos| {
            let idx = ((pos.y - origin.y) / ROW_HEIGHT) as usize;
            (idx < app.rows.len()).then_some(idx)
        });

        if response.clicked() {
            if let Some(idx) = hover_row {
                app.selected = Some(idx);
                app.diff_text = None;
            }
        }

        let y_center = |i: usize| origin.y + i as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
        let x_lane = |l: usize| origin.x + GRAPH_PAD_LEFT + l as f32 * LANE_WIDTH;

        // right-aligned metadata column anchors
        let x_hash = rect.max.x - GRAPH_PAD_RIGHT;
        let x_date = x_hash - COL_HASH;
        let x_author = x_date - COL_DATE;
        let x_msg_end = x_author - COL_AUTHOR - 12.0;

        // ---- row backgrounds (hover + selection) ----
        for i in 0..app.rows.len() {
            let top = origin.y + i as f32 * ROW_HEIGHT;
            let full = Rect::from_min_size(Pos2::new(rect.min.x, top), Vec2::new(rect.width(), ROW_HEIGHT));
            if app.selected == Some(i) {
                painter.rect_filled(full, 0.0, C_SEL);
            } else if hover_row == Some(i) {
                painter.rect_filled(full, 0.0, C_HOVER);
            }
        }

        // ---- graph lanes ----
        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let yc_next = y_center(i + 1);

            // pass-through lanes (unrelated to this commit) drawn straight
            for &l in &row.passthrough {
                let x = x_lane(l);
                painter.line_segment(
                    [Pos2::new(x, yc), Pos2::new(x, yc_next)],
                    Stroke::new(LINE_WIDTH, lane_color(l)),
                );
            }

            // connectors to each parent: straight if same lane, smooth S-curve otherwise
            let x_my = x_lane(row.lane);
            for &pl in &row.parent_lanes {
                let x_p = x_lane(pl);
                if pl == row.lane {
                    painter.line_segment(
                        [Pos2::new(x_my, yc), Pos2::new(x_p, yc_next)],
                        Stroke::new(LINE_WIDTH, lane_color(row.lane)),
                    );
                } else {
                    // Vertical control-point offset scales with the horizontal
                    // distance travelled, so longer lane jumps curve out wider
                    // and "lazier" — the flowing-ribbon look of Git Graph.
                    let dx = (x_p - x_my).abs();
                    let ease = (ROW_HEIGHT * 0.5).max(dx * 0.35).min(ROW_HEIGHT * 0.85);
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
                        Stroke::new(LINE_WIDTH, lane_color(pl)),
                    );
                    painter.add(bez);
                }
            }
        }

        // ---- nodes (drawn last so they sit on top of every curve) ----
        for (i, row) in app.rows.iter().enumerate() {
            let center = Pos2::new(x_lane(row.lane), y_center(i));
            let node_color = lane_color(row.lane);
            if row.is_head {
                painter.circle_filled(center, NODE_RADIUS + 1.5, Color32::from_rgb(0x1e, 0x1e, 0x2e));
                painter.circle_stroke(center, NODE_RADIUS, Stroke::new(2.2_f32, node_color));
            } else {
                painter.circle_filled(center, NODE_RADIUS, node_color);
            }
        }

        // ---- text columns ----
        for (i, row) in app.rows.iter().enumerate() {
            let yc = y_center(i);
            let mut tx = text_col_x;

            // ref pills: HEAD, branches, tags
            if row.is_head {
                tx = draw_pill(&painter, tx, yc, "HEAD", C_TEXT, Color32::from_rgb(0xf3, 0x8b, 0xa8));
            }
            for b in &row.branches {
                tx = draw_pill(&painter, tx, yc, b, C_TEXT, Color32::from_rgb(0x89, 0xb4, 0xfa));
            }
            for t in &row.tags {
                tx = draw_pill(&painter, tx, yc, t, Color32::from_rgb(0x1e, 0x1e, 0x2e), Color32::from_rgb(0xfa, 0xb3, 0x87));
            }

            // commit message (truncated to fit before the metadata columns)
            let msg = elide(&painter, &row.summary, egui::FontId::proportional(12.5), x_msg_end - tx);
            painter.text(
                Pos2::new(tx, yc),
                egui::Align2::LEFT_CENTER,
                &msg,
                egui::FontId::proportional(12.5),
                C_TEXT,
            );

            // author (right block, left-aligned within its column)
            let author = elide(&painter, &row.author, egui::FontId::proportional(11.5), COL_AUTHOR - 8.0);
            painter.text(
                Pos2::new(x_author, yc),
                egui::Align2::LEFT_CENTER,
                &author,
                egui::FontId::proportional(11.5),
                C_SUBTEXT,
            );

            // date
            painter.text(
                Pos2::new(x_date, yc),
                egui::Align2::LEFT_CENTER,
                format_time(row.time, row.offset_min),
                egui::FontId::proportional(11.5),
                C_SUBTEXT,
            );

            // short hash (right aligned)
            painter.text(
                Pos2::new(x_hash, yc),
                egui::Align2::RIGHT_CENTER,
                &row.short,
                egui::FontId::monospace(11.0),
                C_HASH,
            );
        }

        let _ = PathShape::convex_polygon(vec![], Color32::TRANSPARENT, Stroke::NONE); // keep import used
    });
}

/// A rounded ref pill. Returns the x position just after the pill.
fn draw_pill(painter: &egui::Painter, x: f32, y: f32, label: &str, fg: Color32, bg: Color32) -> f32 {
    let font = egui::FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(label.to_string(), font, fg);
    let w = galley.size().x + 12.0;
    let h = 16.0;
    let rect = Rect::from_min_size(Pos2::new(x, y - h / 2.0), Vec2::new(w, h));
    painter.rect_filled(rect, 8.0, bg);
    painter.galley(rect.min + Vec2::new(6.0, (h - galley.size().y) / 2.0), galley, fg);
    rect.max.x + 5.0
}

/// Truncate `text` with an ellipsis so it fits within `max_w` pixels.
fn elide(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32) -> String {
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
