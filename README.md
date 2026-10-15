# proj

Find, clone and jump between Git repositories from the terminal.

`proj` scans your projects directory, merges it with every repository you can access on GitHub (`gh`) and GitLab (`glab`), and lets you fuzzy-search all of them. Press Enter on a local repository to `cd` into it; press Enter on a remote one to clone it into the right place first, then `cd` into it.

```
~/projects/
├── personal/            # repositories owned by users
│   └── portfolio/
└── companies/           # repositories owned by organizations / groups
    └── acme/
        └── backend/
```

## Install

From a clone of this repository:

```sh
./install.sh
```

It builds with `cargo` and installs the binary to `~/.local/bin/proj`, replacing any previous install (override the location with `PROJ_BIN_DIR=/some/bin ./install.sh`). It then sets up shell integration, or prints the Home Manager option when your shell rc file is managed by Nix.

With Nix flakes:

```sh
nix profile install github:Miki000000/project-switcher
# or run it once
nix run github:Miki000000/project-switcher
```

With a flake-based NixOS / home-manager config, add the input and use `inputs.project-switcher.packages.${system}.default`.

From source without the script: `cargo install --path .`

Runtime requirements: `git`. Optional: `gh` and/or `glab`, logged in (`gh auth login`, `glab auth login`).

## Shell integration

A program cannot change its parent shell's directory, so `proj` ships a small zsh wrapper function. Directory changing currently targets **zsh**; `install.sh` adds this line to `~/.zshrc` automatically:

```sh
command -v proj >/dev/null && eval "$(proj init zsh)"
```

If `~/.zshrc` is managed by Home Manager (a read-only link into `/nix/store`), the script cannot edit it and prints the option to add instead:

```nix
programs.zsh.initContent = ''eval "$(proj init zsh)"'';
```

Use `--cmd p` to name the function `p` instead. Without the wrapper, `proj` prints the selected path on stdout, so `cd "$(proj)"` also works.

## Usage

| Command | Description |
| --- | --- |
| `proj` | Interactive picker with every local and remote repository |
| `proj add` | Picker opened directly on the add dialog (repositories not cloned yet) |
| `proj add <url \| owner/name>` | Clone a repository into its place and `cd` into it |
| `proj list [query]` | List repositories (`--local`, `--remote`, `--owner`, `--provider`, `--category`, `--refresh`, `--json`) |
| `proj status [--dirty]` | Branch, changes and ahead/behind for local repositories |
| `proj refresh` | Re-scan local directories and re-query providers |
| `proj config` | Show / change configuration (see below) |
| `proj init <shell>` | Print the shell integration |

### Picker keys

| Key | Action |
| --- | --- |
| `Enter` | Open a local repository, or clone a remote one |
| `Ctrl+A` | Open the add dialog: repositories not cloned yet, or paste a Git URL to clone it |
| `Ctrl+R` | Refresh everything (local scan, git status, providers) |
| `Esc` | Close the dialog, cancel a running clone, or quit |
| `Ctrl+X` | Cancel a running clone |
| `↑ ↓`, `Ctrl+P/N`, `Ctrl+K/J`, `Tab` | Move selection |
| `PgUp PgDn` | Move by page |
| `Ctrl+U` / `Ctrl+W` | Clear query / delete word |
| `Ctrl+C` | Quit (a running clone is cancelled first) |

The search matches name, owner, organization, provider, branch, category (`personal`, `companies`) and path, so queries like `backend`, `gitlab`, `feature/auth` or `companies/acme` all work. Each search term has to match within a single field, and matches far weaker than the best one are hidden. Local repositories are listed first, then plain folders, then repositories that are not cloned yet.

Cancelling a clone (in the picker or with Ctrl+C during `proj add`) stops git and deletes the partially downloaded repository, along with any directories created for it.

## How it works

- **Local discovery** walks the root (and `extra_dirs`) up to `scan_depth` levels. Git repositories are listed with branch and status; directories without any repository inside are listed as plain folders. Only `.git` metadata and `git status` are read; nothing is ever modified, pulled or checked out.
- **Remote discovery** runs in the background through `gh api` / `glab api`, using whatever accounts and hosts those tools are logged into. Results are cached in `$XDG_CACHE_HOME/proj/remotes.json`, so remote repositories show up instantly and while offline (the status line says when data is cached).
- **Unification** matches clones to provider repositories by remote URL (SSH and HTTPS, any remote, SSH host aliases included), so each repository appears once, wherever it lives on disk.
- **Classification**: GitHub `User` / GitLab `user` namespaces go to `personal/<repo>`; GitHub `Organization` / GitLab `group` namespaces go to `companies/<org>/<repo>` (subgroups become subdirectories). Local repositories without a known owner use their location, defaulting to Personal. When the owner type cannot be determined, `proj` asks.
- **Cloning** uses `gh repo clone` / `glab repo clone` (or `git clone` for other hosts), so existing credentials are used. Existing directories are never overwritten, and a repository that is already cloned anywhere is opened instead of cloned again.

## Configuration

`$XDG_CONFIG_HOME/proj/config.toml` (override with `PROJ_CONFIG`). Every key is optional. `proj config init` writes this commented template:

```toml
root = "~/projects"
extra_dirs = []
scan_depth = 4
ignore = ["node_modules", "target", "vendor", "dist", "build", ".direnv"]

[layout]
personal_dir = "personal"
companies_dir = "companies"

[classify]
personal = []    # owners ("org", "group/sub") or repos ("owner/repo") forced to Personal
companies = []   # ... forced to Companies

[providers]
github = true
gitlab = true

[display]
icons = "nerd"        # "nerd", "unicode" or "ascii"
show_remote = true
show_folders = true
show_paths = true
animations = true     # dialog transitions, sliding selection, loading sweeps

[clone]
protocol = "auto"        # "auto" (gh/glab decide), "ssh" or "https"
open_after_clone = true
```

```sh
proj config                              # effective configuration
proj config set layout.companies_dir work
proj config set extra_dirs '["~/src"]'
proj config classify my-side-org personal
proj config classify acme/dotfiles auto  # remove manual rules
proj config edit                         # open in $VISUAL / $EDITOR
```

Colors come from the terminal's own palette, so the interface follows your terminal theme. The default icon set uses Nerd Font Material Design glyphs (Kitty ships the symbols built in); use `icons = "unicode"` otherwise. Set `animations = false` for a static interface.

## Development

```sh
nix develop      # or `direnv allow`, or `nix-shell`
cargo run
cargo test
cargo clippy --all-targets
```
