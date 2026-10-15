#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitUrl {
    pub host: String,
    pub path: String,
}

impl GitUrl {
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
            if !matches!(
                scheme,
                "https" | "http" | "ssh" | "git" | "git+ssh" | "ssh+git"
            ) {
                return None;
            }
            let (authority, path) = rest.split_once('/')?;
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let host = host.split_once(':').map_or(host, |(h, _)| h);
            (host, path)
        } else {
            let (authority, path) = url.split_once(':')?;
            if authority.contains('/') || authority.is_empty() {
                return None;
            }
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            (host, path)
        };
        let path = path.trim_matches('/');
        let path = path
            .strip_suffix(".git")
            .unwrap_or(path)
            .trim_end_matches('/');
        if host.is_empty() || !path.contains('/') || path.split('/').any(str::is_empty) {
            return None;
        }
        Some(Self {
            host: host.to_string(),
            path: path.to_string(),
        })
    }

    pub fn owner(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(o, _)| o)
    }

    pub fn name(&self) -> &str {
        self.path.rsplit_once('/').map_or(&self.path, |(_, n)| n)
    }

    pub fn key(&self) -> String {
        crate::model::repo_key(&self.host, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(url: &str) -> Option<(String, String)> {
        GitUrl::parse(url).map(|u| (u.host, u.path))
    }

    #[test]
    fn parses_common_forms() {
        let want = Some(("github.com".to_string(), "org/repo".to_string()));
        assert_eq!(p("https://github.com/org/repo"), want);
        assert_eq!(p("https://github.com/org/repo.git/"), want);
        assert_eq!(p("https://user:token@github.com/org/repo.git"), want);
        assert_eq!(p("git@github.com:org/repo.git"), want);
        assert_eq!(p("ssh://git@github.com:22/org/repo.git"), want);
        assert_eq!(p("git+ssh://github.com/org/repo"), want);
    }

    #[test]
    fn keeps_gitlab_subgroups() {
        let u = GitUrl::parse("git@gitlab.com:group/sub/repo.git").unwrap();
        assert_eq!((u.owner(), u.name()), ("group/sub", "repo"));
    }

    #[test]
    fn ssh_and_https_share_key() {
        let a = GitUrl::parse("git@GitHub.com:Org/Repo.git").unwrap();
        let b = GitUrl::parse("https://github.com/org/repo").unwrap();
        assert_eq!(a.key(), b.key());
    }

    #[test]
    fn rejects_local_and_incomplete() {
        assert_eq!(p("/srv/git/repo.git"), None);
        assert_eq!(p("./relative/repo"), None);
        assert_eq!(p("file:///srv/repo.git"), None);
        assert_eq!(p("https://github.com/org"), None);
        assert_eq!(p("rust-lang/rust"), None);
    }
}
