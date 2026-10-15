use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::model::{Category, Provider};
use crate::shell::Shell;

#[derive(Parser)]
#[command(
    name = "proj",
    version,
    about = "Find, clone and jump between Git repositories",
    long_about = "Find, clone and jump between Git repositories.\n\n\
        Run without arguments to open the interactive picker. Selecting a local repository \
        changes into it; selecting a remote one clones it into the right place first.\n\
        Enable directory changing with:  eval \"$(proj init zsh)\""
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Open the add view, or clone a repository by URL or owner/name
    Add(AddArgs),
    /// List known repositories (local and cached remote)
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Re-scan local repositories and re-query GitHub/GitLab
    Refresh,
    /// Show branch and working-tree state of local repositories
    #[command(visible_alias = "st")]
    Status {
        /// Only repositories with uncommitted changes or unpushed commits
        #[arg(long)]
        dirty: bool,
    },
    /// Show or change the configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigCommand>,
    },
    /// Print the shell integration script (enables `cd` on selection)
    Init {
        shell: Shell,
        /// Name of the shell function to define
        #[arg(long, default_value = "proj")]
        cmd: String,
    },
}

#[derive(Args)]
pub struct AddArgs {
    /// Git URL (SSH or HTTPS) or `owner/name`; omit to open the interactive add view
    pub repository: Option<String>,
    /// Provider used for `owner/name` (defaults to the first authenticated one)
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,
    /// Force the destination category instead of detecting it from the owner
    #[arg(long, value_enum)]
    pub category: Option<Category>,
    /// Do not change into the repository after cloning
    #[arg(long)]
    pub no_cd: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// Fuzzy filter (same matching as the interactive picker)
    pub query: Option<String>,
    /// Only repositories of this owner, organization or group
    #[arg(long)]
    pub owner: Option<String>,
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,
    #[arg(long, value_enum)]
    pub category: Option<Category>,
    /// Only repositories present on disk
    #[arg(long, conflicts_with = "remote")]
    pub local: bool,
    /// Only repositories not cloned yet
    #[arg(long)]
    pub remote: bool,
    /// Query providers instead of using cached remote data
    #[arg(long)]
    pub refresh: bool,
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Print the effective configuration
    Show,
    /// Print the configuration file path
    Path,
    /// Write a commented default configuration file
    Init,
    /// Open the configuration file in $VISUAL / $EDITOR
    Edit,
    /// Print a value, e.g. `proj config get layout.companies_dir`
    Get { key: String },
    /// Set a value, e.g. `proj config set root ~/code` or `proj config set extra_dirs '["~/src"]'`
    Set { key: String, value: String },
    /// Remove a value so the default applies again
    Unset { key: String },
    /// Pin an owner (`org`) or repository (`owner/name`) to a category
    Classify {
        target: String,
        category: ClassifyTarget,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ClassifyTarget {
    Personal,
    #[value(alias = "company", alias = "work")]
    Companies,
    /// Remove manual rules and use automatic detection
    Auto,
}
