use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[value(name = "github")]
    GitHub,
    #[value(name = "gitlab")]
    GitLab,
    Other,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Other => "Git",
        }
    }

    pub fn from_host(host: &str) -> Self {
        let host = host.to_ascii_lowercase();
        if host.contains("github") {
            Self::GitHub
        } else if host.contains("gitlab") {
            Self::GitLab
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Personal,
    #[value(alias = "company", alias = "work")]
    Companies,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Personal => "Personal",
            Self::Companies => "Company",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OwnerKind {
    User,
    Organization,
    Unknown,
}

impl OwnerKind {
    pub fn category(self) -> Option<Category> {
        match self {
            Self::User => Some(Category::Personal),
            Self::Organization => Some(Category::Companies),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRepo {
    pub provider: Provider,
    pub host: String,
    pub owner: String,
    pub name: String,
    pub owner_kind: OwnerKind,
    pub private: bool,
    pub archived: bool,
    pub fork: bool,
    pub description: Option<String>,
    pub https_url: String,
    pub ssh_url: String,
    pub web_url: String,
    pub updated_at: Option<String>,
}

impl RemoteRepo {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn key(&self) -> String {
        repo_key(&self.host, &self.full_name())
    }
}

pub fn repo_key(host: &str, path: &str) -> String {
    format!(
        "{}/{}",
        host.to_ascii_lowercase(),
        path.trim_matches('/').to_ascii_lowercase()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalKind {
    Git,
    Folder,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WorkStatus {
    pub changes: usize,
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalRepo {
    pub path: PathBuf,
    pub kind: LocalKind,
    pub branch: Option<String>,
    pub remotes: Vec<GitRemote>,
    pub status: Option<WorkStatus>,
    pub modified: i64,
}

impl LocalRepo {
    pub fn dir_name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }
}
