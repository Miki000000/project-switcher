use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use crate::config::Config;
use crate::model::{GitRemote, LocalKind, LocalRepo, WorkStatus};

pub fn scan(cfg: &Config) -> Vec<LocalRepo> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut dirs = vec![cfg.root()];
    dirs.extend(cfg.extra_dirs());
    for dir in dirs {
        let Ok(canonical) = dir.canonicalize() else {
            continue;
        };
        if !seen.insert(canonical) {
            continue;
        }
        walk_children(&dir, cfg.scan_depth, cfg, &mut out);
    }
    let mut unique = std::collections::HashSet::new();
    out.retain(|r: &LocalRepo| {
        unique.insert(r.path.canonicalize().unwrap_or_else(|_| r.path.clone()))
    });
    out
}

fn walk_children(dir: &Path, depth: usize, cfg: &Config, out: &mut Vec<LocalRepo>) {
    for child in subdirs(dir, cfg) {
        if is_git_dir(&child) {
            out.push(inspect_git(child));
            continue;
        }
        let before = out.len();
        if depth > 1 {
            walk_children(&child, depth - 1, cfg, out);
        }
        if !out[before..].iter().any(|r| r.kind == LocalKind::Git) {
            out.truncate(before);
            out.push(folder(child));
        }
    }
}

fn subdirs(dir: &Path, cfg: &Config) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !cfg.ignore.iter().any(|i| *i == name)
        })
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

pub fn is_git_dir(path: &Path) -> bool {
    let dot_git = path.join(".git");
    dot_git.is_file() || dot_git.join("HEAD").is_file()
}

fn folder(path: PathBuf) -> LocalRepo {
    let modified = mtime(&path);
    LocalRepo {
        path,
        kind: LocalKind::Folder,
        branch: None,
        remotes: Vec::new(),
        status: None,
        modified,
    }
}

pub fn inspect_git(path: PathBuf) -> LocalRepo {
    let (git_dir, common_dir) = resolve_git_dirs(&path);
    let branch = fs::read_to_string(git_dir.join("HEAD"))
        .ok()
        .and_then(|head| parse_head(&head));
    let remotes = fs::read_to_string(common_dir.join("config"))
        .map(|c| parse_remotes(&c))
        .unwrap_or_default();
    let modified = mtime(&git_dir.join("index")).max(mtime(&git_dir.join("HEAD")));
    LocalRepo {
        path,
        kind: LocalKind::Git,
        branch,
        remotes,
        status: None,
        modified,
    }
}

fn resolve_git_dirs(worktree: &Path) -> (PathBuf, PathBuf) {
    let dot_git = worktree.join(".git");
    let git_dir = match fs::read_to_string(&dot_git) {
        Ok(content) => match content.trim().strip_prefix("gitdir:") {
            Some(target) => worktree.join(target.trim()),
            None => dot_git,
        },
        Err(_) => dot_git,
    };
    let common = fs::read_to_string(git_dir.join("commondir"))
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|_| git_dir.clone());
    (git_dir, common)
}

fn parse_head(head: &str) -> Option<String> {
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(r) => Some(
            r.trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(r.trim())
                .to_string(),
        ),
        None if head.len() >= 7 => Some(head[..7].to_string()),
        None => None,
    }
}

fn parse_remotes(config: &str) -> Vec<GitRemote> {
    let mut remotes: Vec<GitRemote> = Vec::new();
    let mut current: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .strip_prefix("remote")
                .and_then(|r| {
                    r.trim()
                        .strip_prefix('"')?
                        .strip_suffix('"')
                        .map(String::from)
                });
            continue;
        }
        let Some(name) = &current else { continue };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") && !remotes.iter().any(|r| r.name == *name) {
            remotes.push(GitRemote {
                name: name.clone(),
                url: value.trim().trim_matches('"').to_string(),
            });
        }
    }
    remotes.sort_by_key(|r| r.name != "origin");
    remotes
}

fn mtime(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub branch: Option<String>,
    pub status: WorkStatus,
}

pub fn git_status(path: &Path) -> Option<StatusReport> {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "--branch",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_status(&String::from_utf8_lossy(&out.stdout)))
}

fn parse_status(text: &str) -> StatusReport {
    let mut lines = text.lines();
    let mut report = StatusReport {
        branch: None,
        status: WorkStatus::default(),
    };
    if let Some(header) = lines.next().and_then(|l| l.strip_prefix("## ")) {
        let (head, tracking) = header
            .split_once(" [")
            .map_or((header, ""), |(h, t)| (h, t.trim_end_matches(']')));
        let head = head
            .strip_prefix("No commits yet on ")
            .or_else(|| head.strip_prefix("Initial commit on "))
            .unwrap_or(head);
        if !head.starts_with("HEAD (no branch)") {
            report.branch = Some(head.split_once("...").map_or(head, |(b, _)| b).to_string());
        }
        for part in tracking.split(", ") {
            if let Some(n) = part.strip_prefix("ahead ") {
                report.status.ahead = n.parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                report.status.behind = n.parse().unwrap_or(0);
            }
        }
    }
    report.status.changes = lines.filter(|l| !l.is_empty()).count();
    report
}

pub fn statuses_parallel<F>(paths: Vec<PathBuf>, on_result: F)
where
    F: Fn(PathBuf, Option<StatusReport>) + Sync,
{
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8)
        .min(paths.len().max(1));
    let next = AtomicUsize::new(0);
    let paths = &paths;
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i).cloned() else {
                        break;
                    };
                    let report = git_status(&path);
                    on_result(path, report);
                }
            });
        }
    });
}

pub fn apply_status(repo: &mut LocalRepo, report: Option<StatusReport>) {
    if let Some(report) = report {
        if report.branch.is_some() {
            repo.branch = report.branch;
        }
        repo.status = Some(report.status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_header_and_changes() {
        let r =
            parse_status("## main...origin/main [ahead 2, behind 1]\n M src/a.rs\n?? new.txt\n");
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(
            r.status,
            WorkStatus {
                changes: 2,
                ahead: 2,
                behind: 1
            }
        );

        let r = parse_status("## No commits yet on trunk\n");
        assert_eq!(r.branch.as_deref(), Some("trunk"));
        assert_eq!(r.status.changes, 0);

        assert_eq!(parse_status("## HEAD (no branch)\n").branch, None);
        assert_eq!(
            parse_status("## feature/x...upstream/feature/x [gone]\n")
                .branch
                .as_deref(),
            Some("feature/x")
        );
    }

    #[test]
    fn parses_remotes_origin_first() {
        let cfg = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://github.com/up/r.git\n[remote \"origin\"]\n\turl = git@github.com:me/r.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        let remotes = parse_remotes(cfg);
        assert_eq!(
            remotes.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["origin", "upstream"]
        );
        assert_eq!(remotes[0].url, "git@github.com:me/r.git");
    }

    #[test]
    fn parses_head() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/parser\n").as_deref(),
            Some("feature/parser")
        );
        assert_eq!(parse_head("0123456789abcdef\n").as_deref(), Some("0123456"));
    }

    #[test]
    fn containers_are_not_listed_but_plain_folders_are() {
        let tmp = std::env::temp_dir().join(format!("proj-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        for dir in [
            "personal/notes/sub",
            "personal/broken/.git",
            "companies/acme/docs",
        ] {
            fs::create_dir_all(tmp.join(dir)).unwrap();
        }
        for repo in ["personal/app", "companies/acme/api", "work/x/y"] {
            fs::create_dir_all(tmp.join(repo).join(".git")).unwrap();
            fs::write(tmp.join(repo).join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        }
        let cfg = Config {
            root: tmp.display().to_string(),
            ..Config::default()
        };
        let mut found: Vec<(String, LocalKind)> = scan(&cfg)
            .into_iter()
            .map(|r| {
                (
                    r.path.strip_prefix(&tmp).unwrap().display().to_string(),
                    r.kind,
                )
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        fs::remove_dir_all(&tmp).unwrap();
        assert_eq!(
            found,
            [
                ("companies/acme/api".to_string(), LocalKind::Git),
                ("companies/acme/docs".to_string(), LocalKind::Folder),
                ("personal/app".to_string(), LocalKind::Git),
                ("personal/broken".to_string(), LocalKind::Folder),
                ("personal/notes".to_string(), LocalKind::Folder),
                ("work/x/y".to_string(), LocalKind::Git),
            ]
        );
    }
}
