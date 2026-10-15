use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use anyhow::{Context, Result, bail};

use crate::config::{Config, Protocol};
use crate::model::{Category, Provider, RemoteRepo};

pub fn destination(cfg: &Config, category: Category, owner: &str, name: &str) -> PathBuf {
    let base = cfg.category_dir(category);
    match category {
        Category::Personal => base.join(name),
        Category::Companies => owner
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(base, |p, s| p.join(s))
            .join(name),
    }
}

#[derive(Debug, Clone)]
enum Source {
    Provider {
        provider: Provider,
        host: String,
        full_name: String,
    },
    Url(String),
}

#[derive(Debug, Clone)]
pub struct CloneJob {
    source: Source,
    pub dest: PathBuf,
}

impl CloneJob {
    pub fn for_remote(cfg: &Config, repo: &RemoteRepo, dest: PathBuf) -> Self {
        let source = match (cfg.clone.protocol, repo.provider) {
            (Protocol::Auto, Provider::GitHub | Provider::GitLab) => Source::Provider {
                provider: repo.provider,
                host: repo.host.clone(),
                full_name: repo.full_name(),
            },
            (Protocol::Ssh, _) => Source::Url(repo.ssh_url.clone()),
            _ => Source::Url(repo.https_url.clone()),
        };
        Self { source, dest }
    }

    pub fn for_url(url: &str, dest: PathBuf) -> Self {
        Self {
            source: Source::Url(url.to_string()),
            dest,
        }
    }

    pub fn check(&self) -> Result<()> {
        if self.dest.exists() {
            bail!("{} already exists", crate::config::tilde(&self.dest));
        }
        Ok(())
    }

    fn prepare(&self) -> Result<Vec<PathBuf>> {
        self.check()?;
        let mut created = Vec::new();
        let mut dir = self.dest.parent();
        while let Some(d) = dir.filter(|d| !d.exists()) {
            created.push(d.to_path_buf());
            dir = d.parent();
        }
        if let Some(parent) = self.dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        Ok(created)
    }

    fn cleanup(&self, created: &[PathBuf]) {
        let _ = std::fs::remove_dir_all(&self.dest);
        for dir in created {
            let _ = std::fs::remove_dir(dir);
        }
    }

    fn command(&self, progress: bool) -> Command {
        let dest = self.dest.as_os_str();
        let mut cmd;
        match &self.source {
            Source::Provider {
                provider: Provider::GitHub,
                host,
                full_name,
            } => {
                cmd = Command::new("gh");
                let repo = if host == "github.com" {
                    full_name.clone()
                } else {
                    format!("{host}/{full_name}")
                };
                cmd.args(["repo", "clone"]).arg(repo).arg(dest);
                if progress {
                    cmd.args(["--", "--progress"]);
                }
            }
            Source::Provider {
                host, full_name, ..
            } => {
                cmd = Command::new("glab");
                let repo = if host == "gitlab.com" {
                    full_name.clone()
                } else {
                    format!("https://{host}/{full_name}")
                };
                cmd.args(["repo", "clone"]).arg(repo).arg(dest);
                if progress {
                    cmd.args(["--", "--progress"]);
                }
            }
            Source::Url(url) => {
                cmd = Command::new("git");
                cmd.arg("clone");
                if progress {
                    cmd.arg("--progress");
                }
                cmd.arg(url).arg(dest);
            }
        }
        cmd
    }

    pub fn run_interactive(&self) -> Result<()> {
        let created = self.prepare()?;
        let mut cmd = self.command(false);
        // SAFETY: only async-signal-safe `signal` calls.
        let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                Ok(())
            });
        }
        let status = cmd.status();
        unsafe {
            libc::signal(libc::SIGINT, previous);
        }
        let status = status.with_context(|| format!("cannot run {:?}", cmd.get_program()))?;
        if status.success() {
            return Ok(());
        }
        self.cleanup(&created);
        if status.signal() == Some(libc::SIGINT) || status.code() == Some(130) {
            return Err(Cancelled.into());
        }
        bail!("clone failed ({status})")
    }

    pub fn run_captured(
        &self,
        control: &Arc<CloneControl>,
        mut on_progress: impl FnMut(&str),
    ) -> Result<()> {
        let created = self.prepare()?;
        let mut cmd = self.command(true);
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.cleanup(&created);
                return Err(e).with_context(|| format!("cannot run {:?}", cmd.get_program()));
            }
        };
        control.pgid.store(child.id() as i32, Ordering::SeqCst);
        if control.is_cancelled() {
            control.signal(libc::SIGTERM);
        }

        let mut stderr = child.stderr.take().expect("stderr piped");
        let mut buf = [0u8; 4096];
        let mut line = Vec::new();
        let mut last_lines: Vec<String> = Vec::new();
        loop {
            let n = match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                if b == b'\r' || b == b'\n' {
                    let text = String::from_utf8_lossy(&line).trim().to_string();
                    line.clear();
                    if !text.is_empty() {
                        on_progress(&text);
                        last_lines.push(text);
                        if last_lines.len() > 20 {
                            last_lines.remove(0);
                        }
                    }
                } else {
                    line.push(b);
                }
            }
        }
        let status = child.wait()?;
        control.pgid.store(0, Ordering::SeqCst);
        if status.success() && !control.is_cancelled() {
            return Ok(());
        }
        self.cleanup(&created);
        if control.is_cancelled() {
            return Err(Cancelled.into());
        }
        let reason = last_lines
            .iter()
            .rev()
            .find(|l| l.starts_with("fatal:") || l.starts_with("ERROR") || l.contains("error"))
            .or(last_lines.last())
            .cloned()
            .unwrap_or_else(|| format!("exited with {status}"));
        bail!("{reason}")
    }
}

#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("clone cancelled; partial files removed")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Default)]
pub struct CloneControl {
    pgid: AtomicI32,
    cancelled: AtomicBool,
}

impl CloneControl {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel(self: &Arc<Self>) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.signal(libc::SIGTERM);
        let control = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            control.signal(libc::SIGKILL);
        });
    }

    fn signal(&self, signal: libc::c_int) {
        let id = self.pgid.load(Ordering::SeqCst);
        if id > 0 {
            // SAFETY: plain signal delivery.
            unsafe {
                libc::kill(-id, signal);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn destinations_follow_layout() {
        let cfg = Config {
            root: "/p".into(),
            ..Config::default()
        };
        assert_eq!(
            destination(&cfg, Category::Personal, "me", "blog"),
            Path::new("/p/personal/blog")
        );
        assert_eq!(
            destination(&cfg, Category::Companies, "acme", "api"),
            Path::new("/p/companies/acme/api")
        );
        assert_eq!(
            destination(&cfg, Category::Companies, "group/sub", "svc"),
            Path::new("/p/companies/group/sub/svc")
        );
    }
}
