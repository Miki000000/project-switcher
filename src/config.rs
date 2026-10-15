use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::Category;

pub const DEFAULT_TEMPLATE: &str = r#"# proj configuration
# Every key is optional; missing keys fall back to the defaults shown here.

# Root directory managed by proj. Repositories are organized as:
#   <root>/<layout.personal_dir>/<repo>
#   <root>/<layout.companies_dir>/<organization>/<repo>
root = "~/projects"

# Additional directories scanned for existing repositories (never written to).
extra_dirs = []

# How many directory levels below each scanned directory are searched for repositories.
scan_depth = 4

# Directory names skipped while scanning.
ignore = ["node_modules", "target", "vendor", "dist", "build", ".direnv"]

[layout]
personal_dir = "personal"
companies_dir = "companies"

[classify]
# Owners ("org", "group/subgroup") or repositories ("owner/repo") forced into a category.
# Repository rules win over owner rules; both win over the automatic classification.
personal = []
companies = []

[providers]
github = true   # uses the `gh` CLI and its authentication
gitlab = true   # uses the `glab` CLI and its authentication

[display]
icons = "nerd"        # "nerd" (Nerd Font glyphs), "unicode" or "ascii"
show_remote = true    # list not-yet-cloned repositories in the main view
show_folders = true   # list plain (non-git) directories
show_paths = true     # show the local path under each repository
animations = true     # transitions, sliding selection, loading sweeps

[clone]
protocol = "auto"        # "auto" (gh/glab decide), "ssh" or "https"
open_after_clone = true  # cd into a repository once it has been cloned from the TUI
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub root: String,
    pub extra_dirs: Vec<String>,
    pub scan_depth: usize,
    pub ignore: Vec<String>,
    pub layout: Layout,
    pub classify: Classify,
    pub providers: Providers,
    pub display: Display,
    pub clone: CloneConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    pub personal_dir: String,
    pub companies_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Classify {
    pub personal: Vec<String>,
    pub companies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Providers {
    pub github: bool,
    pub gitlab: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSet {
    Nerd,
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Display {
    pub icons: IconSet,
    pub show_remote: bool,
    pub show_folders: bool,
    pub show_paths: bool,
    pub animations: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Auto,
    Ssh,
    Https,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CloneConfig {
    pub protocol: Protocol,
    pub open_after_clone: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root: "~/projects".into(),
            extra_dirs: Vec::new(),
            scan_depth: 4,
            ignore: [
                "node_modules",
                "target",
                "vendor",
                "dist",
                "build",
                ".direnv",
            ]
            .map(String::from)
            .to_vec(),
            layout: Layout::default(),
            classify: Classify::default(),
            providers: Providers::default(),
            display: Display::default(),
            clone: CloneConfig::default(),
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            personal_dir: "personal".into(),
            companies_dir: "companies".into(),
        }
    }
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            github: true,
            gitlab: true,
        }
    }
}

impl Default for Display {
    fn default() -> Self {
        Self {
            icons: IconSet::Nerd,
            show_remote: true,
            show_folders: true,
            show_paths: true,
            animations: true,
        }
    }
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            protocol: Protocol::Auto,
            open_after_clone: true,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        for (key, dir) in [
            ("layout.personal_dir", &self.layout.personal_dir),
            ("layout.companies_dir", &self.layout.companies_dir),
        ] {
            if dir.is_empty() || dir.contains('/') || dir == "." || dir == ".." {
                bail!("{key} must be a single directory name, got {dir:?}");
            }
        }
        if self.layout.personal_dir == self.layout.companies_dir {
            bail!("layout.personal_dir and layout.companies_dir must differ");
        }
        if self.scan_depth == 0 {
            bail!("scan_depth must be at least 1");
        }
        Ok(())
    }

    pub fn root(&self) -> PathBuf {
        expand_tilde(&self.root)
    }

    pub fn extra_dirs(&self) -> Vec<PathBuf> {
        self.extra_dirs.iter().map(|d| expand_tilde(d)).collect()
    }

    pub fn category_dir(&self, category: Category) -> PathBuf {
        self.root().join(match category {
            Category::Personal => &self.layout.personal_dir,
            Category::Companies => &self.layout.companies_dir,
        })
    }

    pub fn classify_override(&self, owner: &str, name: &str) -> Option<Category> {
        let full = format!("{owner}/{name}");
        let rules = [
            (Category::Personal, &self.classify.personal),
            (Category::Companies, &self.classify.companies),
        ];
        for (category, list) in rules {
            if list.iter().any(|r| r.eq_ignore_ascii_case(&full)) {
                return Some(category);
            }
        }
        let owner = owner.to_ascii_lowercase();
        rules.into_iter().find_map(|(category, list)| {
            list.iter()
                .map(|r| r.to_ascii_lowercase())
                .any(|r| owner == r || owner.starts_with(&format!("{r}/")))
                .then_some(category)
        })
    }
}

pub fn config_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("PROJ_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(dirs::config_dir()
        .context("cannot determine XDG config directory")?
        .join("proj/config.toml"))
}

pub fn cache_dir() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("PROJ_CACHE_DIR") {
        return Ok(PathBuf::from(p));
    }
    Ok(dirs::cache_dir()
        .context("cannot determine XDG cache directory")?
        .join("proj"))
}

pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => dirs::home_dir().map_or_else(
            || PathBuf::from(path),
            |h| h.join(rest.trim_start_matches('/')),
        ),
        _ => PathBuf::from(path),
    }
}

pub fn tilde(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_rule_beats_owner_rule() {
        let mut cfg = Config::default();
        cfg.classify.companies = vec!["Acme".into()];
        cfg.classify.personal = vec!["acme/dotfiles".into()];
        assert_eq!(
            cfg.classify_override("acme", "dotfiles"),
            Some(Category::Personal)
        );
        assert_eq!(
            cfg.classify_override("ACME", "api"),
            Some(Category::Companies)
        );
        assert_eq!(
            cfg.classify_override("acme/platform", "api"),
            Some(Category::Companies)
        );
        assert_eq!(cfg.classify_override("acme-labs", "api"), None);
    }

    #[test]
    fn default_template_matches_defaults() {
        let parsed = Config::parse(DEFAULT_TEMPLATE).unwrap();
        assert_eq!(
            toml::to_string(&parsed).unwrap(),
            toml::to_string(&Config::default()).unwrap()
        );
    }

    #[test]
    fn rejects_nested_layout_dirs() {
        assert!(Config::parse("[layout]\ncompanies_dir = \"a/b\"").is_err());
        assert!(Config::parse("[layout]\npersonal_dir = \"x\"\ncompanies_dir = \"x\"").is_err());
    }
}
