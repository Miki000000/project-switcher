use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Provider, RemoteRepo};

const VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    version: u32,
    pub providers: BTreeMap<Provider, CachedSet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSet {
    pub fetched_at: i64,
    pub repos: Vec<RemoteRepo>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: usize,
    pub removed: usize,
}

impl Cache {
    pub fn load() -> Self {
        let Ok(path) = crate::config::cache_dir().map(|d| d.join("remotes.json")) else {
            return Self::default();
        };
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|c| c.version == VERSION)
            .unwrap_or_default()
    }

    pub fn update(&mut self, provider: Provider, repos: Vec<RemoteRepo>) -> Diff {
        let keys =
            |repos: &[RemoteRepo]| repos.iter().map(RemoteRepo::key).collect::<BTreeSet<_>>();
        let new = keys(&repos);
        let diff = self.providers.get(&provider).map_or(
            Diff {
                added: new.len(),
                removed: 0,
            },
            |old| {
                let old = keys(&old.repos);
                Diff {
                    added: new.difference(&old).count(),
                    removed: old.difference(&new).count(),
                }
            },
        );
        self.providers.insert(
            provider,
            CachedSet {
                fetched_at: now(),
                repos,
            },
        );
        diff
    }

    pub fn save(&mut self) -> Result<()> {
        self.version = VERSION;
        let dir = crate::config::cache_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let tmp = dir.join("remotes.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, dir.join("remotes.json"))?;
        Ok(())
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn age(since: i64) -> String {
    let secs = (now() - since).max(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}
