use std::process::{Command, Stdio};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::model::{OwnerKind, Provider, RemoteRepo};

pub const PROVIDERS: [Provider; 2] = [Provider::GitHub, Provider::GitLab];

pub fn enabled(cfg: &Config, provider: Provider) -> bool {
    match provider {
        Provider::GitHub => cfg.providers.github,
        Provider::GitLab => cfg.providers.gitlab,
        Provider::Other => false,
    }
}

fn cli(provider: Provider) -> &'static str {
    match provider {
        Provider::GitHub => "gh",
        Provider::GitLab => "glab",
        Provider::Other => "git",
    }
}

#[derive(Debug, Clone)]
pub enum FetchOutcome {
    Unavailable(String),
    Fetched(Vec<RemoteRepo>),
    Failed(String),
}

pub fn fetch(provider: Provider) -> FetchOutcome {
    let bin = cli(provider);
    match run(bin, &["auth", "status"]) {
        Err(RunError::Missing) => return FetchOutcome::Unavailable(format!("{bin} not installed")),
        Err(RunError::Failed(_)) => {
            return FetchOutcome::Unavailable(format!("{bin} not authenticated"));
        }
        Ok(_) => {}
    }
    let result = match provider {
        Provider::GitHub => fetch_github(),
        Provider::GitLab => fetch_gitlab(),
        Provider::Other => Ok(Vec::new()),
    };
    match result {
        Ok(repos) => FetchOutcome::Fetched(repos),
        Err(e) => FetchOutcome::Failed(e),
    }
}

#[derive(Deserialize)]
struct GhRepo {
    name: String,
    owner: GhOwner,
    private: bool,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    fork: bool,
    description: Option<String>,
    clone_url: String,
    ssh_url: String,
    html_url: String,
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct GhOwner {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

fn github_kind(kind: &str) -> OwnerKind {
    match kind {
        "User" => OwnerKind::User,
        "Organization" => OwnerKind::Organization,
        _ => OwnerKind::Unknown,
    }
}

fn fetch_github() -> Result<Vec<RemoteRepo>, String> {
    let out = run(
        "gh",
        &["api", "--paginate", "user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member"],
    )
    .map_err(|e| e.message("gh"))?;
    let repos: Vec<GhRepo> = parse_pages(&out)?;
    Ok(repos
        .into_iter()
        .map(|r| {
            let host = crate::giturl::GitUrl::parse(&r.clone_url)
                .map_or_else(|| "github.com".into(), |u| u.host);
            RemoteRepo {
                provider: Provider::GitHub,
                host,
                owner_kind: github_kind(&r.owner.kind),
                owner: r.owner.login,
                name: r.name,
                private: r.private,
                archived: r.archived,
                fork: r.fork,
                description: r.description.filter(|d| !d.trim().is_empty()),
                https_url: r.clone_url,
                ssh_url: r.ssh_url,
                web_url: r.html_url,
                updated_at: r.pushed_at,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct GlProject {
    path: String,
    namespace: GlNamespace,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    forked_from_project: Option<serde_json::Value>,
    description: Option<String>,
    http_url_to_repo: String,
    ssh_url_to_repo: String,
    web_url: String,
    last_activity_at: Option<String>,
}

#[derive(Deserialize)]
struct GlNamespace {
    full_path: String,
    kind: String,
}

fn gitlab_kind(kind: &str) -> OwnerKind {
    match kind {
        "user" => OwnerKind::User,
        "group" => OwnerKind::Organization,
        _ => OwnerKind::Unknown,
    }
}

fn fetch_gitlab() -> Result<Vec<RemoteRepo>, String> {
    let out = run(
        "glab",
        &[
            "api",
            "--paginate",
            "projects?membership=true&per_page=100&order_by=last_activity_at&simple=false",
        ],
    )
    .map_err(|e| e.message("glab"))?;
    let projects: Vec<GlProject> = parse_pages(&out)?;
    Ok(projects
        .into_iter()
        .map(|p| {
            let host = crate::giturl::GitUrl::parse(&p.http_url_to_repo)
                .map_or_else(|| "gitlab.com".into(), |u| u.host);
            RemoteRepo {
                provider: Provider::GitLab,
                host,
                owner_kind: gitlab_kind(&p.namespace.kind),
                owner: p.namespace.full_path,
                name: p.path,
                private: p.visibility.as_deref() != Some("public"),
                archived: p.archived,
                fork: p.forked_from_project.is_some(),
                description: p.description.filter(|d| !d.trim().is_empty()),
                https_url: p.http_url_to_repo,
                ssh_url: p.ssh_url_to_repo,
                web_url: p.web_url,
                updated_at: p.last_activity_at,
            }
        })
        .collect())
}

pub fn owner_kind(provider: Provider, host: &str, owner: &str) -> Option<OwnerKind> {
    match provider {
        Provider::GitHub => {
            let out = run(
                "gh",
                &["api", "--hostname", host, &format!("users/{owner}")],
            )
            .ok()?;
            let user: GhOwner = serde_json::from_slice(&out).ok()?;
            Some(github_kind(&user.kind)).filter(|k| *k != OwnerKind::Unknown)
        }
        Provider::GitLab => {
            let path = format!("namespaces/{}", owner.replace('/', "%2F"));
            let out = run("glab", &["api", "--hostname", host, &path]).ok()?;
            let ns: GlNamespace = serde_json::from_slice(&out).ok()?;
            Some(gitlab_kind(&ns.kind)).filter(|k| *k != OwnerKind::Unknown)
        }
        Provider::Other => None,
    }
}

pub fn authenticated(provider: Provider) -> bool {
    provider != Provider::Other && run(cli(provider), &["auth", "status"]).is_ok()
}

fn parse_pages<T: DeserializeOwned>(bytes: &[u8]) -> Result<Vec<T>, String> {
    let mut items = Vec::new();
    for page in serde_json::Deserializer::from_slice(bytes).into_iter::<Vec<T>>() {
        items.extend(page.map_err(|e| format!("unexpected API response: {e}"))?);
    }
    Ok(items)
}

enum RunError {
    Missing,
    Failed(String),
}

impl RunError {
    fn message(self, bin: &str) -> String {
        match self {
            Self::Missing => format!("{bin} not installed"),
            Self::Failed(stderr) => {
                let line = stderr
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("request failed")
                    .trim();
                format!("{bin}: {line}")
            }
        }
    }
}

fn run(bin: &str, args: &[&str]) -> Result<Vec<u8>, RunError> {
    let out = Command::new(bin)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RunError::Missing
            } else {
                RunError::Failed(e.to_string())
            }
        })?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(RunError::Failed(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_concatenated_pages() {
        let pages = br#"[{"a":1},{"a":2}][{"a":3}]"#;
        #[derive(Deserialize)]
        struct A {
            a: u8,
        }
        let items: Vec<A> = parse_pages(pages).unwrap();
        assert_eq!(items.iter().map(|i| i.a).collect::<Vec<_>>(), [1, 2, 3]);
    }
}
