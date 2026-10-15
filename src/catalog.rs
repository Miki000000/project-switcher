use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};

use crate::config::{Config, tilde};
use crate::giturl::GitUrl;
use crate::model::{Category, LocalKind, LocalRepo, Provider, RemoteRepo};

pub struct RemoteSource<'a> {
    pub repos: &'a [RemoteRepo],
    pub cached: bool,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub owner: Option<String>,
    pub name: String,
    pub provider: Option<Provider>,
    pub category: Option<Category>,
    pub local: Option<LocalRepo>,
    pub remote: Option<RemoteRepo>,
    pub remote_cached: bool,
    pub rel_path: Option<String>,
    pub recency: i64,
    fields: Vec<Utf32String>,
}

const MIN_RELATIVE_SCORE: u32 = 70;

impl Entry {
    pub fn is_local(&self) -> bool {
        self.local.is_some()
    }

    pub fn is_folder(&self) -> bool {
        self.local
            .as_ref()
            .is_some_and(|l| l.kind == LocalKind::Folder)
    }

    pub fn branch(&self) -> Option<&str> {
        self.local.as_ref().and_then(|l| l.branch.as_deref())
    }
}

pub fn build(cfg: &Config, locals: &[LocalRepo], sources: &[RemoteSource]) -> Vec<Entry> {
    let mut by_key: HashMap<String, (usize, usize)> = HashMap::new();
    let mut by_path: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut hosts: HashSet<String> = HashSet::new();
    for (s, source) in sources.iter().enumerate() {
        for (r, repo) in source.repos.iter().enumerate() {
            by_key.entry(repo.key()).or_insert((s, r));
            by_path
                .entry(repo.full_name().to_ascii_lowercase())
                .or_default()
                .push((s, r));
            hosts.insert(repo.host.to_ascii_lowercase());
        }
    }

    let root = cfg.root();
    let mut claimed: HashSet<(usize, usize)> = HashSet::new();
    let mut entries = Vec::with_capacity(locals.len() + by_key.len());

    for local in locals {
        let urls: Vec<GitUrl> = local
            .remotes
            .iter()
            .filter_map(|r| GitUrl::parse(&r.url))
            .collect();
        let matched = urls
            .iter()
            .find_map(|u| by_key.get(&u.key()).copied())
            .or_else(|| {
                urls.iter().find_map(|u| {
                    if hosts.contains(&u.host.to_ascii_lowercase()) {
                        return None;
                    }
                    match by_path.get(&u.path.to_ascii_lowercase())?.as_slice() {
                        [only] => Some(*only),
                        _ => None,
                    }
                })
            });
        if let Some(m) = matched {
            claimed.insert(m);
        }
        let remote = matched.map(|(s, r)| &sources[s].repos[r]);
        let url = urls.first();

        let (owner, name) = match (remote, url) {
            (Some(r), _) => (Some(r.owner.clone()), r.name.clone()),
            (None, Some(u)) => (Some(u.owner().to_string()), u.name().to_string()),
            (None, None) => (None, local.dir_name()),
        };
        let title = match &owner {
            Some(o) => format!("{o}/{name}"),
            None => local_label(cfg, &local.path),
        };
        let category = owner
            .as_deref()
            .and_then(|o| cfg.classify_override(o, &name))
            .or_else(|| remote.and_then(|r| r.owner_kind.category()))
            .or_else(|| path_category(cfg, &local.path))
            .unwrap_or(Category::Personal);
        let provider = remote
            .map(|r| r.provider)
            .or_else(|| url.map(|u| Provider::from_host(&u.host)));
        let rel_path = Some(
            local
                .path
                .strip_prefix(&root)
                .map_or_else(|_| tilde(&local.path), |p| p.display().to_string()),
        );
        entries.push(finish(Entry {
            id: format!("local:{}", local.path.display()),
            title,
            owner,
            name,
            provider,
            category: Some(category),
            local: Some(local.clone()),
            remote: remote.cloned(),
            remote_cached: matched.is_some_and(|(s, _)| sources[s].cached),
            rel_path,
            recency: local.modified,
            fields: Vec::new(),
        }));
    }

    for (s, source) in sources.iter().enumerate() {
        for (r, repo) in source.repos.iter().enumerate() {
            if claimed.contains(&(s, r)) || by_key.get(&repo.key()) != Some(&(s, r)) {
                continue;
            }
            let category = cfg
                .classify_override(&repo.owner, &repo.name)
                .or_else(|| repo.owner_kind.category());
            let rel_path = category.map(|c| {
                let dest = crate::clone::destination(cfg, c, &repo.owner, &repo.name);
                dest.strip_prefix(&root)
                    .map_or_else(|_| tilde(&dest), |p| p.display().to_string())
            });
            entries.push(finish(Entry {
                id: format!("remote:{}", repo.key()),
                title: repo.full_name(),
                owner: Some(repo.owner.clone()),
                name: repo.name.clone(),
                provider: Some(repo.provider),
                category,
                local: None,
                remote: Some(repo.clone()),
                remote_cached: source.cached,
                rel_path,
                recency: repo.updated_at.as_deref().map_or(0, parse_timestamp),
                fields: Vec::new(),
            }));
        }
    }
    entries
}

fn finish(mut e: Entry) -> Entry {
    let mut fields = vec![e.title.clone()];
    if let Some(c) = e.category {
        fields.push(c.label().to_string());
        fields.push(
            match c {
                Category::Personal => "personal",
                Category::Companies => "companies",
            }
            .to_string(),
        );
    }
    if let Some(p) = e.provider {
        fields.push(p.label().to_string());
    }
    if let Some(b) = e.local.as_ref().and_then(|l| l.branch.as_deref()) {
        fields.push(b.to_string());
    }
    if let Some(p) = &e.rel_path {
        fields.push(p.clone());
    }
    if e.is_folder() {
        fields.push("folder".to_string());
    }
    e.fields = fields.into_iter().map(Utf32String::from).collect();
    e
}

fn local_label(cfg: &Config, path: &Path) -> String {
    let rel = std::iter::once(cfg.root())
        .chain(cfg.extra_dirs())
        .find_map(|base| path.strip_prefix(base).ok().map(PathBuf::from));
    let Some(rel) = rel else { return tilde(path) };
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.len() > 1
        && (parts[0] == cfg.layout.personal_dir || parts[0] == cfg.layout.companies_dir)
    {
        parts.remove(0);
    }
    parts.join("/")
}

fn path_category(cfg: &Config, path: &Path) -> Option<Category> {
    [Category::Personal, Category::Companies]
        .into_iter()
        .find(|c| path.starts_with(cfg.category_dir(*c)))
}

fn parse_timestamp(ts: &str) -> i64 {
    let num = |range: std::ops::Range<usize>| ts.get(range).and_then(|s| s.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d), Some(h), Some(min), Some(s)) = (
        num(0..4),
        num(5..7),
        num(8..10),
        num(11..13),
        num(14..16),
        num(17..19),
    ) else {
        return 0;
    };
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400 + h * 3600 + min * 60 + s
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub idx: usize,
    pub title_indices: Vec<u32>,
}

pub struct Searcher {
    matcher: Matcher,
    indices: Vec<u32>,
}

impl Default for Searcher {
    fn default() -> Self {
        Self {
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()),
            indices: Vec::new(),
        }
    }
}

impl Searcher {
    pub fn search(
        &mut self,
        entries: &[Entry],
        query: &str,
        include: impl Fn(&Entry) -> bool,
    ) -> Vec<Hit> {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(Hit, u32)> = Vec::new();
        'entries: for (idx, e) in entries.iter().enumerate().filter(|(_, e)| include(e)) {
            let mut score = 0u32;
            for atom in &pattern.atoms {
                let best = e
                    .fields
                    .iter()
                    .filter_map(|f| atom.score(f.slice(..), &mut self.matcher))
                    .max();
                match best {
                    Some(s) => score += u32::from(s),
                    None => continue 'entries,
                }
            }
            self.indices.clear();
            for atom in &pattern.atoms {
                let before = self.indices.len();
                if atom
                    .indices(e.fields[0].slice(..), &mut self.matcher, &mut self.indices)
                    .is_none()
                {
                    self.indices.truncate(before);
                }
            }
            self.indices.sort_unstable();
            self.indices.dedup();
            let title_indices = self.indices.clone();
            scored.push((Hit { idx, title_indices }, score));
        }
        let best = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);
        scored.retain(|(_, s)| *s * 100 >= best * MIN_RELATIVE_SCORE);
        let tier = |e: &Entry| match &e.local {
            Some(l) if l.kind == LocalKind::Git => 0,
            Some(_) => 1,
            None => 2,
        };
        scored.sort_by(|(a, sa), (b, sb)| {
            let (ea, eb) = (&entries[a.idx], &entries[b.idx]);
            tier(ea)
                .cmp(&tier(eb))
                .then(sb.cmp(sa))
                .then(eb.recency.cmp(&ea.recency))
                .then_with(|| ea.title.to_lowercase().cmp(&eb.title.to_lowercase()))
        });
        scored.into_iter().map(|(h, _)| h).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GitRemote, OwnerKind};

    fn remote(owner: &str, name: &str, kind: OwnerKind) -> RemoteRepo {
        RemoteRepo {
            provider: Provider::GitHub,
            host: "github.com".into(),
            owner: owner.into(),
            name: name.into(),
            owner_kind: kind,
            private: false,
            archived: false,
            fork: false,
            description: None,
            https_url: format!("https://github.com/{owner}/{name}.git"),
            ssh_url: format!("git@github.com:{owner}/{name}.git"),
            web_url: String::new(),
            updated_at: Some("2024-01-01T00:00:00Z".into()),
        }
    }

    fn local(cfg: &Config, rel: &str, url: Option<&str>) -> LocalRepo {
        LocalRepo {
            path: cfg.root().join(rel),
            kind: LocalKind::Git,
            branch: Some("main".into()),
            remotes: url
                .map(|u| GitRemote {
                    name: "origin".into(),
                    url: u.into(),
                })
                .into_iter()
                .collect(),
            status: None,
            modified: 0,
        }
    }

    fn cfg() -> Config {
        Config {
            root: "/p".into(),
            ..Config::default()
        }
    }

    #[test]
    fn clone_and_remote_merge_into_one_entry() {
        let cfg = cfg();
        let repos = [
            remote("Acme", "api", OwnerKind::Organization),
            remote("me", "blog", OwnerKind::User),
        ];
        let locals = [local(
            &cfg,
            "somewhere/api",
            Some("git@github.com:acme/api.git"),
        )];
        let entries = build(
            &cfg,
            &locals,
            &[RemoteSource {
                repos: &repos,
                cached: false,
            }],
        );
        assert_eq!(entries.len(), 2);
        let api = &entries[0];
        assert!(api.is_local() && api.remote.is_some());
        assert_eq!(api.category, Some(Category::Companies));
        assert_eq!(api.title, "Acme/api");
        let blog = &entries[1];
        assert!(!blog.is_local());
        assert_eq!(blog.rel_path.as_deref(), Some("personal/blog"));
    }

    #[test]
    fn ssh_alias_matches_by_unique_path() {
        let cfg = cfg();
        let repos = [remote("acme", "api", OwnerKind::Organization)];
        let locals = [local(
            &cfg,
            "personal/api",
            Some("git@github-work:acme/api.git"),
        )];
        let entries = build(
            &cfg,
            &locals,
            &[RemoteSource {
                repos: &repos,
                cached: true,
            }],
        );
        assert_eq!(entries.len(), 1);
        assert!(entries[0].remote_cached);
        assert_eq!(entries[0].category, Some(Category::Companies));
    }

    #[test]
    fn local_without_remote_uses_path_label_and_category() {
        let cfg = cfg();
        let locals = [
            local(&cfg, "companies/acme/tool", None),
            local(&cfg, "personal/notes", None),
        ];
        let entries = build(&cfg, &locals, &[]);
        assert_eq!(
            (entries[0].title.as_str(), entries[0].category),
            ("acme/tool", Some(Category::Companies))
        );
        assert_eq!(
            (entries[1].title.as_str(), entries[1].category),
            ("notes", Some(Category::Personal))
        );
    }

    #[test]
    fn unknown_owner_has_no_category_or_destination() {
        let cfg = cfg();
        let repos = [remote("ghost", "x", OwnerKind::Unknown)];
        let entries = build(
            &cfg,
            &[],
            &[RemoteSource {
                repos: &repos,
                cached: false,
            }],
        );
        assert_eq!(
            (entries[0].category, entries[0].rel_path.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn local_entries_rank_before_comparable_remote_matches() {
        let cfg = cfg();
        let repos = [remote("acme", "backend", OwnerKind::Organization)];
        let locals = [local(&cfg, "personal/old-backend-notes", None)];
        let entries = build(
            &cfg,
            &locals,
            &[RemoteSource {
                repos: &repos,
                cached: false,
            }],
        );
        let hits = Searcher::default().search(&entries, "backend", |_| true);
        let order: Vec<&str> = hits.iter().map(|h| entries[h.idx].title.as_str()).collect();
        assert_eq!(order, ["old-backend-notes", "acme/backend"]);
    }

    #[test]
    fn scattered_matches_across_or_within_fields_are_dropped() {
        let cfg = cfg();
        let locals = [
            local(
                &cfg,
                "work/dentalume/applications/dentalume-front",
                Some("git@github.com:SalusCorp/dentalume-front.git"),
            ),
            local(&cfg, "personal/code-presentations", None),
        ];
        let repos = [
            remote("Data-Norte", "api", OwnerKind::Organization),
            remote("BeroLab", "beroboard-legacy", OwnerKind::Organization),
            remote("me", "blog", OwnerKind::User),
        ];
        let entries = build(
            &cfg,
            &locals,
            &[RemoteSource {
                repos: &repos,
                cached: false,
            }],
        );
        let mut s = Searcher::default();
        let titles = |s: &mut Searcher, q: &str| -> Vec<String> {
            s.search(&entries, q, |_| true)
                .iter()
                .map(|h| entries[h.idx].title.clone())
                .collect()
        };
        assert_eq!(titles(&mut s, "datanorte"), ["Data-Norte/api"]);
        assert_eq!(titles(&mut s, "blog"), ["me/blog"]);
    }

    #[test]
    fn search_covers_branch_category_and_path() {
        let cfg = cfg();
        let mut l = local(
            &cfg,
            "companies/datanorte/api",
            Some("https://github.com/datanorte/api"),
        );
        l.branch = Some("feature/auth".into());
        let entries = build(&cfg, &[l], &[]);
        let mut s = Searcher::default();
        for q in ["feature/auth", "companies/datanorte", "company", "github"] {
            assert_eq!(s.search(&entries, q, |_| true).len(), 1, "query {q}");
        }
        assert!(s.search(&entries, "personal", |_| true).is_empty());
        let hit = &s.search(&entries, "api", |_| true)[0];
        assert_eq!(hit.title_indices, [10, 11, 12]);
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse_timestamp("2024-03-01T12:00:00.000Z"), 1_709_294_400);
        assert_eq!(parse_timestamp("garbage"), 0);
    }
}
