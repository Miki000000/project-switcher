use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result};

pub const CD_FILE_ENV: &str = "PROJ_CD_FILE";

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

pub fn init_script(shell: Shell, cmd: &str) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!(
            r#"# proj shell integration — add to your shell rc:  eval "$(proj init {shell})"
{cmd}() {{
  local __proj_file __proj_status __proj_dir
  __proj_file="$(mktemp "${{TMPDIR:-/tmp}}/proj-cd.XXXXXX")" || return
  {env}="$__proj_file" command proj "$@"
  __proj_status=$?
  if [ -s "$__proj_file" ]; then
    __proj_dir="$(cat -- "$__proj_file")"
    [ -d "$__proj_dir" ] && cd -- "$__proj_dir"
  fi
  rm -f -- "$__proj_file"
  return $__proj_status
}}
"#,
            shell = if matches!(shell, Shell::Bash) {
                "bash"
            } else {
                "zsh"
            },
            env = CD_FILE_ENV,
        ),
        Shell::Fish => format!(
            r#"# proj shell integration — add to config.fish:  proj init fish | source
function {cmd} --wraps proj --description 'proj: jump between repositories'
    set -l __proj_file (mktemp (set -q TMPDIR; and echo $TMPDIR; or echo /tmp)/proj-cd.XXXXXX); or return
    {env}=$__proj_file command proj $argv
    set -l __proj_status $status
    if test -s $__proj_file
        set -l __proj_dir (cat -- $__proj_file)
        test -d "$__proj_dir"; and cd -- $__proj_dir
    end
    rm -f -- $__proj_file
    return $__proj_status
end
"#,
            env = CD_FILE_ENV,
        ),
    }
}

pub fn request_cd(path: &Path) -> Result<()> {
    if let Some(file) = std::env::var_os(CD_FILE_ENV) {
        return std::fs::write(&file, path.as_os_str().as_encoded_bytes())
            .with_context(|| format!("cannot write {}", Path::new(&file).display()));
    }
    println!("{}", path.display());
    if std::io::stdout().is_terminal() {
        eprintln!(
            "proj: shell integration is not active, so the directory was printed instead.\n      \
             Enable it with: eval \"$(proj init zsh)\"   (or bash / `proj init fish | source`)"
        );
    }
    Ok(())
}
