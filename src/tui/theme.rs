use ratatui::style::{Color, Modifier, Style};

use crate::config::IconSet;

pub const ACCENT: Color = Color::Cyan;
pub const MUTED: Color = Color::DarkGray;
pub const SUCCESS: Color = Color::Green;
pub const WARNING: Color = Color::Yellow;
pub const ERROR: Color = Color::Red;
pub const REMOTE: Color = Color::Blue;
pub const COMPANY: Color = Color::Magenta;
pub const PERSONAL: Color = Color::Cyan;
pub const FOLDER: Color = Color::Yellow;

pub fn text() -> Style {
    Style::new().fg(Color::Reset)
}

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn border() -> Style {
    Style::new().fg(MUTED)
}

pub fn title() -> Style {
    Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD)
}

pub fn highlight() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub struct Icons {
    pub app: &'static str,
    pub search: &'static str,
    pub personal: &'static str,
    pub company: &'static str,
    pub folder: &'static str,
    pub remote: &'static str,
    pub branch: &'static str,
    pub lock: &'static str,
    pub archived: &'static str,
    pub github: &'static str,
    pub gitlab: &'static str,
    pub git: &'static str,
    pub clean: &'static str,
    pub dirty: &'static str,
    pub ahead: &'static str,
    pub behind: &'static str,
    pub add: &'static str,
    pub link: &'static str,
    pub download: &'static str,
    pub selected: &'static str,
}

impl Icons {
    pub fn get(set: IconSet) -> &'static Self {
        match set {
            IconSet::Nerd => &NERD,
            IconSet::Unicode => &UNICODE,
            IconSet::Ascii => &ASCII,
        }
    }
}

static NERD: Icons = Icons {
    app: "\u{f018d}",      // md-console
    search: "\u{f0349}",   // md-magnify
    personal: "\u{f024c}", // md-folder_account
    company: "\u{f00d6}",  // md-briefcase
    folder: "\u{f0256}",   // md-folder_outline
    remote: "\u{f0b7d}",   // md-cloud_download_outline
    branch: "\u{f062c}",   // md-source_branch
    lock: "\u{f0341}",     // md-lock_outline
    archived: "\u{f120e}", // md-archive_outline
    github: "\u{f02a4}",   // md-github
    gitlab: "\u{f0ba0}",   // md-gitlab
    git: "\u{f02a2}",      // md-git
    clean: "\u{f05e1}",    // md-check_circle_outline
    dirty: "\u{f08d5}",    // md-circle_edit_outline
    ahead: "\u{f005d}",    // md-arrow_up
    behind: "\u{f0045}",   // md-arrow_down
    add: "\u{f0b9d}",      // md-folder_plus_outline
    link: "\u{f0339}",     // md-link_variant
    download: "\u{f0997}", // md-progress_download
    selected: "▌",
};

static UNICODE: Icons = Icons {
    app: "❯",
    search: "⌕",
    personal: "◆",
    company: "▣",
    folder: "▢",
    remote: "◇",
    branch: "⎇",
    lock: "⊘",
    archived: "▤",
    github: "◉",
    gitlab: "◉",
    git: "◉",
    clean: "✓",
    dirty: "●",
    ahead: "↑",
    behind: "↓",
    add: "+",
    link: "⇢",
    download: "↓",
    selected: "▌",
};

static ASCII: Icons = Icons {
    app: ">",
    search: "/",
    personal: "*",
    company: "#",
    folder: "+",
    remote: "o",
    branch: "@",
    lock: "!",
    archived: "a",
    github: "-",
    gitlab: "-",
    git: "-",
    clean: "ok",
    dirty: "*",
    ahead: "^",
    behind: "v",
    add: "+",
    link: "~",
    download: "v",
    selected: ">",
};
