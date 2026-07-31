use std::collections::HashMap;

use chrono::{FixedOffset, TimeZone};
use git2::{Oid, Repository};

pub struct CommitRow {
    pub oid: Oid,
    pub short: String,
    pub lane: usize,
    pub passthrough: Vec<usize>,
    pub parent_lanes: Vec<usize>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub offset_min: i32,
    pub summary: String,
    pub branches: Vec<String>,
    pub tags: Vec<String>,
    pub is_head: bool,
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

pub fn build_rows(repo: &Repository, limit: usize, all_refs: bool) -> Result<Vec<CommitRow>, git2::Error> {
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
    let mut free_lanes: Vec<usize> = Vec::new();

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

        let my_lane = if let Some(l) = active_lanes.remove(&oid) {
            l
        } else {
            alloc_lane(&mut free_lanes, &mut next_free_lane)
        };

        let mut passthrough: Vec<usize> = active_lanes
            .values()
            .copied()
            .filter(|&l| l != my_lane)
            .collect();
        passthrough.sort_unstable();
        passthrough.dedup();

        let mut alive_before: Vec<usize> = passthrough.clone();
        alive_before.push(my_lane);

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

        for l in alive_before {
            if !active_lanes.values().any(|&al| al == l) && !free_lanes.contains(&l) {
                free_lanes.push(l);
            }
        }
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

pub fn format_time(secs: i64, offset_min: i32) -> String {
    let tz = FixedOffset::east_opt(offset_min * 60).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    match tz.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        _ => String::from("?"),
    }
}

pub fn max_lanes(rows: &[CommitRow]) -> usize {
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
