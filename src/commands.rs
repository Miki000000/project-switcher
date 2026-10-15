use std::collections::HashSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use crossterm::style::{StyledContent, Stylize};
use unicode_width::UnicodeWidthStr;

use crate::cache::{self, Cache};
use crate::catalog::{self, Entry, RemoteSource, Searcher};
use crate::cli::{AddArgs, ClassifyTarget, ConfigCommand, ListArgs};
use crate::clone::{self, CloneJob};
use crate::config::{self, Config, tilde};
use crate::discovery;
use crate::giturl::GitUrl;
use crate::model::{Category, LocalKind, LocalRepo, Provider, RemoteRepo, WorkStatus};
use crate::providers::{self, FetchOutcome};

pub fn scan_with_status(cfg: &Config) -> Vec<LocalRepo> {
    let mut locals = discovery::scan(cfg);
    let paths: Vec<PathBuf> = locals
        .iter()
        .filter(|l| l.kind == LocalKind::Git)
        .map(|l| l.path.clone())
        .collect();
    let (tx, rx) = std::sync::mpsc::channel();
    discovery::statuses_parallel(paths, |path, report| {
        let _ = tx.send((path, report));
    });
    drop(tx);
    for (path, report) in rx {
        if let Some(local) = locals.iter_mut().find(|l| l.path == path) {
            discovery::apply_status(local, report);
        }
    }
    locals
}

fn fetch_all(
    cfg: &Config,
    cache: &mut Cache,
) -> Vec<(Provider, FetchOutcome, Option<cache::Diff>)> {
    let outcomes: Vec<(Provider, FetchOutcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = providers::PROVIDERS
            .into_iter()
            .filter(|p| providers::enabled(cfg, *p))
            .map(|p| (p, s.spawn(move || providers::fetch(p))))
            .collect();
        handles
            .into_iter()
            .map(|(p, h)| {
                (
                    p,
                    h.join()
                        .unwrap_or(FetchOutcome::Failed("worker panicked".into())),
                )
            })
            .collect()
    });
    outcomes
        .into_iter()
        .map(|(p, outcome)| {
            let diff = match &outcome {
                FetchOutcome::Fetched(repos) => Some(cache.update(p, repos.clone())),
                _ => None,
            };
            (p, outcome, diff)
        })
        .collect()
}

fn sources<'a>(cfg: &Config, cache: &'a Cache, fresh: &HashSet<Provider>) -> Vec<RemoteSource<'a>> {
    cache
        .providers
        .iter()
        .filter(|(p, _)| providers::enabled(cfg, **p))
        .map(|(p, set)| RemoteSource {
            repos: &set.repos,
            cached: !fresh.contains(p),
        })
        .collect()
}

fn init_colors() {
    if !std::io::stdout().is_terminal() {
        crossterm::style::force_color_output(false);
    }
}

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

fn status_text(status: &WorkStatus) -> String {
    let mut s = if status.changes == 0 {
        "clean".to_string()
    } else {
        format!(
            "{} change{}",
            status.changes,
            if status.changes == 1 { "" } else { "s" }
        )
    };
    if status.ahead > 0 {
        s.push_str(&format!(" ↑{}", status.ahead));
    }
    if status.behind > 0 {
        s.push_str(&format!(" ↓{}", status.behind));
    }
    s
}

fn is_dirty(status: &WorkStatus) -> bool {
    status.changes > 0 || status.ahead > 0
}

pub fn list(cfg: &Config, args: &ListArgs) -> Result<()> {
    init_colors();
    let locals = if args.remote {
        Vec::new()
    } else {
        scan_with_status(cfg)
    };
    let mut cache = Cache::load();
    let mut fresh = HashSet::new();
    if args.refresh {
        for (p, outcome, _) in fetch_all(cfg, &mut cache) {
            match outcome {
                FetchOutcome::Fetched(_) => {
                    fresh.insert(p);
                }
                FetchOutcome::Failed(e) => eprintln!("{} {e}", "warning:".yellow()),
                FetchOutcome::Unavailable(_) => {}
            }
        }
        cache.save()?;
    }
    let entries = catalog::build(cfg, &locals, &sources(cfg, &cache, &fresh));
    let hits = Searcher::default().search(&entries, args.query.as_deref().unwrap_or(""), |e| {
        (!args.local || e.is_local())
            && (!args.remote || !e.is_local())
            && (cfg.display.show_folders || !e.is_folder())
            && args.category.is_none_or(|c| e.category == Some(c))
            && args.provider.is_none_or(|p| e.provider == Some(p))
            && args.owner.as_ref().is_none_or(|o| {
                e.owner.as_deref().is_some_and(|eo| {
                    eo.eq_ignore_ascii_case(o)
                        || eo
                            .to_lowercase()
                            .starts_with(&format!("{}/", o.to_lowercase()))
                })
            })
    });
    let rows: Vec<&Entry> = hits.iter().map(|h| &entries[h.idx]).collect();

    if args.json {
        let json: Vec<serde_json::Value> = rows.iter().map(|e| entry_json(e)).collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }
    print_rows(cfg, &rows);
    if let Some(oldest) = cache
        .providers
        .iter()
        .filter(|(p, _)| !fresh.contains(*p))
        .map(|(_, s)| s.fetched_at)
        .min()
        && rows.iter().any(|e| !e.is_local())
    {
        eprintln!(
            "{}",
            format!(
                "remote data cached {} · run `proj refresh` to update",
                cache::age(oldest)
            )
            .dark_grey()
        );
    }
    Ok(())
}

fn entry_json(e: &Entry) -> serde_json::Value {
    let local = e.local.as_ref();
    serde_json::json!({
        "title": e.title,
        "owner": e.owner,
        "name": e.name,
        "provider": e.provider,
        "category": e.category,
        "kind": match local.map(|l| l.kind) {
            Some(LocalKind::Git) => "git",
            Some(LocalKind::Folder) => "folder",
            None => "remote",
        },
        "path": local.map(|l| l.path.clone()),
        "branch": e.branch(),
        "status": local.and_then(|l| l.status.clone()),
        "remotes": local.map(|l| l.remotes.clone()).unwrap_or_default(),
        "url": e.remote.as_ref().map(|r| r.web_url.clone()),
        "private": e.remote.as_ref().map(|r| r.private),
        "cached": e.remote.is_some() && e.remote_cached,
        "destination": (!e.is_local()).then(|| e.rel_path.clone()).flatten(),
    })
}

fn print_rows(cfg: &Config, rows: &[&Entry]) {
    let columns: Vec<[String; 5]> = rows
        .iter()
        .map(|e| {
            let state = match &e.local {
                Some(l) if l.kind == LocalKind::Folder => "folder".to_string(),
                Some(l) => l
                    .status
                    .as_ref()
                    .map_or_else(|| "unknown".into(), status_text),
                None => {
                    let r = e.remote.as_ref().expect("remote-only entry");
                    if r.private {
                        "remote · private".into()
                    } else {
                        "remote".into()
                    }
                }
            };
            let location = match (&e.local, &e.rel_path) {
                (Some(l), _) => tilde(&l.path),
                (None, Some(rel)) => format!("→ {}", tilde(&cfg.root().join(rel))),
                (None, None) => "→ unclassified".into(),
            };
            [
                e.title.clone(),
                e.category.map_or("?", Category::label).to_string(),
                e.provider.map_or("—", Provider::label).to_string(),
                e.branch().unwrap_or("").to_string(),
                format!("{state}\u{0}{location}"),
            ]
        })
        .collect();
    let width = |i: usize| columns.iter().map(|c| c[i].width()).max().unwrap_or(0);
    let widths = [width(0), width(1), width(2), width(3)];
    let state_width = columns
        .iter()
        .map(|c| c[4].split('\0').next().unwrap_or("").width())
        .max()
        .unwrap_or(0);

    let mut out = std::io::stdout().lock();
    for (e, c) in rows.iter().zip(&columns) {
        let (state, location) = c[4].split_once('\0').unwrap_or((&c[4], ""));
        let marker: StyledContent<&str> = match &e.local {
            Some(l) if l.kind == LocalKind::Folder => "▫".dark_grey(),
            Some(_) => "●".green(),
            None => "○".blue(),
        };
        let title = pad(&c[0], widths[0]);
        let title = if e.is_local() {
            title.bold()
        } else {
            title.reset()
        };
        let state_styled = match &e.local {
            Some(l) if l.kind == LocalKind::Git => match &l.status {
                Some(s) if is_dirty(s) => pad(state, state_width).yellow(),
                Some(_) => pad(state, state_width).green(),
                None => pad(state, state_width).dark_grey(),
            },
            Some(_) => pad(state, state_width).dark_grey(),
            None => pad(state, state_width).blue(),
        };
        let _ = writeln!(
            out,
            "{marker} {title}  {}  {}  {}  {state_styled}  {}",
            pad(&c[1], widths[1]),
            pad(&c[2], widths[2]).dark_grey(),
            pad(&c[3], widths[3]).cyan(),
            location.dark_grey(),
        );
    }
}

pub fn status(cfg: &Config, dirty_only: bool) -> Result<()> {
    init_colors();
    let mut locals: Vec<LocalRepo> = scan_with_status(cfg)
        .into_iter()
        .filter(|l| l.kind == LocalKind::Git)
        .collect();
    let total = locals.len();
    let dirty = locals
        .iter()
        .filter(|l| l.status.as_ref().is_some_and(is_dirty))
        .count();
    if dirty_only {
        locals.retain(|l| l.status.as_ref().is_some_and(is_dirty));
    }
    let entries = catalog::build(cfg, &locals, &[]);
    let hits = Searcher::default().search(&entries, "", |_| true);
    let rows: Vec<&Entry> = hits.iter().map(|h| &entries[h.idx]).collect();
    let name_w = rows.iter().map(|e| e.title.width()).max().unwrap_or(0);
    let branch_w = rows
        .iter()
        .map(|e| e.branch().unwrap_or("").width())
        .max()
        .unwrap_or(0);
    let state_w = rows
        .iter()
        .map(|e| {
            e.local
                .as_ref()
                .and_then(|l| l.status.as_ref())
                .map_or(7, |s| status_text(s).width())
        })
        .max()
        .unwrap_or(0);
    let mut out = std::io::stdout().lock();
    for e in &rows {
        let local = e.local.as_ref().expect("local entry");
        let state = local
            .status
            .as_ref()
            .map_or_else(|| "unknown".into(), status_text);
        let state = match &local.status {
            Some(s) if is_dirty(s) => pad(&state, state_w).yellow(),
            Some(_) => pad(&state, state_w).green(),
            None => pad(&state, state_w).red(),
        };
        let _ = writeln!(
            out,
            "{}  {}  {state}  {}",
            pad(&e.title, name_w).bold(),
            pad(e.branch().unwrap_or("—"), branch_w).cyan(),
            tilde(&local.path).dark_grey(),
        );
    }
    let summary = format!("{total} repositories · {dirty} with uncommitted or unpushed work");
    let _ = writeln!(
        out,
        "{}",
        if rows.is_empty() && dirty_only {
            "All repositories are clean.".to_string()
        } else {
            summary
        }
        .dark_grey()
    );
    Ok(())
}

pub fn refresh(cfg: &Config) -> Result<()> {
    init_colors();
    let mut cache = Cache::load();
    let (locals, results) = std::thread::scope(|s| {
        let locals = s.spawn(|| scan_with_status(cfg));
        let results = fetch_all(cfg, &mut cache);
        (locals.join().unwrap_or_default(), results)
    });
    cache.save()?;

    let repos = locals.iter().filter(|l| l.kind == LocalKind::Git).count();
    let folders = locals.len() - repos;
    let dirty = locals
        .iter()
        .filter(|l| l.status.as_ref().is_some_and(is_dirty))
        .count();
    println!(
        "{} {repos} repositories, {folders} folders, {dirty} with changes",
        pad("Local", 7).bold()
    );
    for (p, outcome, diff) in &results {
        let label = pad(p.label(), 7).bold();
        match (outcome, diff) {
            (FetchOutcome::Fetched(repos), Some(d)) => {
                println!(
                    "{label} {} repositories ({} new, {} removed)",
                    repos.len(),
                    d.added,
                    d.removed
                )
            }
            (FetchOutcome::Unavailable(why), _) => {
                println!("{label} {}", format!("skipped: {why}").dark_grey())
            }
            (FetchOutcome::Failed(e), _) => println!("{label} {}", format!("failed: {e}").red()),
            _ => {}
        }
    }
    let entries = catalog::build(
        cfg,
        &locals,
        &sources(cfg, &cache, &results.iter().map(|r| r.0).collect()),
    );
    let linked = entries
        .iter()
        .filter(|e| e.is_local() && e.remote.is_some())
        .count();
    let remote_only = entries.iter().filter(|e| !e.is_local()).count();
    println!(
        "{} {linked} clones linked to a provider, {remote_only} not cloned yet",
        pad("Total", 7).bold()
    );
    Ok(())
}

pub fn add(cfg: &Config, args: &AddArgs, repository: &str) -> Result<()> {
    let cache = Cache::load();
    let url = GitUrl::parse(repository);
    let (provider, host, full_name) = match &url {
        Some(u) => (
            args.provider
                .unwrap_or_else(|| Provider::from_host(&u.host)),
            Some(u.host.clone()),
            u.path.clone(),
        ),
        None => {
            let name = repository.trim().trim_matches('/');
            if name.split('/').count() < 2 || name.contains(char::is_whitespace) {
                bail!("expected a Git URL or owner/name, got {repository:?}");
            }
            let provider = match args.provider {
                Some(p) => p,
                None => providers::PROVIDERS
                    .into_iter()
                    .filter(|p| providers::enabled(cfg, *p))
                    .find(|p| providers::authenticated(*p))
                    .context("no authenticated provider: log in with `gh auth login` / `glab auth login`, or pass a full URL")?,
            };
            (provider, None, name.to_string())
        }
    };
    let (owner, name) = full_name
        .rsplit_once('/')
        .map(|(o, n)| (o.to_string(), n.to_string()))
        .context("missing repository name")?;

    let known: Option<&RemoteRepo> = cache.providers.values().flat_map(|s| &s.repos).find(|r| {
        r.full_name().eq_ignore_ascii_case(&full_name)
            && match &host {
                Some(h) => r.host.eq_ignore_ascii_case(h),
                None => r.provider == provider,
            }
    });

    let key_host = host
        .clone()
        .or_else(|| known.map(|k| k.host.clone()))
        .unwrap_or_else(|| match provider {
            Provider::GitLab => "gitlab.com".into(),
            _ => "github.com".into(),
        });
    let key = crate::model::repo_key(&key_host, &full_name);
    if let Some(existing) = discovery::scan(cfg).into_iter().find(|l| {
        l.remotes
            .iter()
            .filter_map(|r| GitUrl::parse(&r.url))
            .any(|u| u.key() == key)
    }) {
        eprintln!("Already cloned at {}", tilde(&existing.path));
        if !args.no_cd {
            crate::shell::request_cd(&existing.path)?;
        }
        return Ok(());
    }

    let category = match args
        .category
        .or_else(|| cfg.classify_override(&owner, &name))
        .or_else(|| known.and_then(|k| k.owner_kind.category()))
    {
        Some(c) => c,
        None => match providers::owner_kind(provider, &key_host, &owner).and_then(|k| k.category())
        {
            Some(c) => c,
            None => ask_category(&owner, &name)?,
        },
    };
    let dest = clone::destination(cfg, category, &owner, &name);
    let job = match (&url, known) {
        (Some(_), _) => CloneJob::for_url(repository.trim(), dest.clone()),
        (None, Some(remote)) => CloneJob::for_remote(cfg, remote, dest.clone()),
        (None, None) => {
            let remote = synthetic_remote(provider, &key_host, &owner, &name);
            CloneJob::for_remote(cfg, &remote, dest.clone())
        }
    };
    eprintln!(
        "Cloning {} into {}",
        full_name.clone().bold(),
        tilde(&dest).cyan()
    );
    job.run_interactive()?;
    eprintln!("{} {}", "✓ Cloned".green(), tilde(&dest));
    if !args.no_cd {
        crate::shell::request_cd(&dest)?;
    }
    Ok(())
}

fn synthetic_remote(provider: Provider, host: &str, owner: &str, name: &str) -> RemoteRepo {
    RemoteRepo {
        provider,
        host: host.to_string(),
        owner: owner.to_string(),
        name: name.to_string(),
        owner_kind: crate::model::OwnerKind::Unknown,
        private: false,
        archived: false,
        fork: false,
        description: None,
        https_url: format!("https://{host}/{owner}/{name}.git"),
        ssh_url: format!("git@{host}:{owner}/{name}.git"),
        web_url: format!("https://{host}/{owner}/{name}"),
        updated_at: None,
    }
}

fn ask_category(owner: &str, name: &str) -> Result<Category> {
    if !std::io::stdin().is_terminal() {
        bail!(
            "cannot tell whether {owner} is a user or an organization; pass --category personal|companies"
        );
    }
    let stdin = std::io::stdin();
    loop {
        eprint!("Could not classify {owner}/{name}. Clone into [p]ersonal or [c]ompanies? ");
        let mut answer = String::new();
        if stdin.lock().read_line(&mut answer)? == 0 {
            bail!("aborted");
        }
        match answer.trim().to_lowercase().as_str() {
            "p" | "personal" => return Ok(Category::Personal),
            "c" | "companies" | "company" => return Ok(Category::Companies),
            _ => {}
        }
    }
}

pub fn config(action: Option<ConfigCommand>) -> Result<()> {
    let path = config::config_path()?;
    match action.unwrap_or(ConfigCommand::Show) {
        ConfigCommand::Show => {
            let cfg = Config::load()?;
            let note = if path.exists() {
                ""
            } else {
                " (not created yet; showing defaults)"
            };
            println!(
                "# {}{note}\n{}",
                tilde(&path),
                toml::to_string_pretty(&cfg)?
            );
        }
        ConfigCommand::Path => println!("{}", path.display()),
        ConfigCommand::Init => {
            if path.exists() {
                bail!("{} already exists", tilde(&path));
            }
            write_config(&path, config::DEFAULT_TEMPLATE)?;
            eprintln!("Wrote {}", tilde(&path));
        }
        ConfigCommand::Edit => {
            if !path.exists() {
                write_config(&path, config::DEFAULT_TEMPLATE)?;
            }
            let editor = std::env::var("VISUAL")
                .or_else(|_| std::env::var("EDITOR"))
                .unwrap_or_else(|_| "vi".into());
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{editor} \"$1\""))
                .arg("sh")
                .arg(&path)
                .status()
                .with_context(|| format!("cannot launch {editor}"))?;
            if !status.success() {
                bail!("{editor} exited with {status}");
            }
            Config::load()?;
        }
        ConfigCommand::Get { key } => {
            let value = toml::Value::try_from(Config::load()?)?;
            let found = key
                .split('.')
                .try_fold(&value, |v, k| v.get(k))
                .with_context(|| format!("unknown key {key:?}"))?;
            match found {
                toml::Value::String(s) => println!("{s}"),
                toml::Value::Table(_) => print!("{}", toml::to_string_pretty(found)?),
                other => println!("{other}"),
            }
        }
        ConfigCommand::Set { key, value } => {
            let parsed = value
                .parse::<toml_edit::Value>()
                .unwrap_or_else(|_| toml_edit::Value::from(value.as_str()));
            edit_config(&path, |doc| {
                let (table, leaf) = table_for(doc, &key, true)?;
                table.insert(leaf, toml_edit::Item::Value(parsed));
                Ok(())
            })?;
        }
        ConfigCommand::Unset { key } => {
            edit_config(&path, |doc| {
                if let Ok((table, leaf)) = table_for(doc, &key, false) {
                    table.remove(leaf);
                }
                Ok(())
            })?;
        }
        ConfigCommand::Classify { target, category } => {
            let target = target.trim().trim_matches('/').to_string();
            edit_config(&path, |doc| {
                let classify = doc.entry("classify").or_insert(toml_edit::table());
                let classify = classify
                    .as_table_like_mut()
                    .context("[classify] is not a table")?;
                for (list, cat) in [
                    ("personal", ClassifyTarget::Personal),
                    ("companies", ClassifyTarget::Companies),
                ] {
                    let item = classify
                        .entry(list)
                        .or_insert(toml_edit::value(toml_edit::Array::new()));
                    let array = item
                        .as_array_mut()
                        .with_context(|| format!("classify.{list} is not an array"))?;
                    array.retain(|v| !v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(&target)));
                    if std::mem::discriminant(&cat) == std::mem::discriminant(&category) {
                        array.push(target.as_str());
                    }
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

fn write_config(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

fn edit_config(
    path: &std::path::Path,
    f: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<()>,
) -> Result<()> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|_| config::DEFAULT_TEMPLATE.to_string());
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("cannot parse {}", path.display()))?;
    f(&mut doc)?;
    let text = doc.to_string();
    Config::parse(&text).context("refusing to write an invalid configuration")?;
    write_config(path, &text)?;
    eprintln!("Updated {}", tilde(path));
    Ok(())
}

fn table_for<'a, 'k>(
    doc: &'a mut toml_edit::DocumentMut,
    key: &'k str,
    create: bool,
) -> Result<(&'a mut dyn toml_edit::TableLike, &'k str)> {
    let mut parts: Vec<&str> = key.split('.').collect();
    let leaf = parts.pop().filter(|l| !l.is_empty()).context("empty key")?;
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for part in parts {
        if !create && table.get(part).is_none() {
            bail!("unknown key {key:?}");
        }
        table = table
            .entry(part)
            .or_insert(toml_edit::table())
            .as_table_like_mut()
            .with_context(|| format!("{part} is not a table"))?;
    }
    Ok((table, leaf))
}
