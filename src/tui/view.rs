use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::app::{App, CloneTask, Pane, ProviderState, Tone};
use super::theme::{self, Icons};
use crate::catalog::Entry;
use crate::config::tilde;
use crate::model::{Category, LocalKind, Provider};

const ROW_HEIGHT: u16 = 3;

pub fn render(f: &mut Frame, app: &App) {
    let icons = Icons::get(app.cfg.display.icons);
    let area = f.area();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border());
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, area);
    if inner.height < 9 || inner.width < 24 {
        f.render_widget(
            Paragraph::new("Terminal too small").style(theme::muted()),
            inner,
        );
        return;
    }
    let [header, search, _, list, separator, hints, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let focused = !app.dialog_open() && app.prompt.is_none();
    render_header(f, app, icons, header);
    render_search(
        f,
        &app.main,
        icons,
        search,
        "Filter by name, owner, branch, path…",
        focused,
    );
    render_list(f, app, &app.main, icons, list, false);
    render_separator(f, app, separator);
    render_hints(f, app, hints);
    render_status(f, app, icons, status);

    if let Some(scale) = app.dialog_scale() {
        dim(f, area);
        render_add_dialog(f, app, icons, scale);
    }
    if app.prompt.is_some() {
        dim(f, area);
        render_prompt(f, app);
    }
}

fn dim(f: &mut Frame, area: Rect) {
    f.buffer_mut().set_style(
        area,
        Style::new().fg(theme::MUTED).add_modifier(Modifier::DIM),
    );
}

fn render_header(f: &mut Frame, app: &App, icons: &Icons, area: Rect) {
    let left = vec![
        Span::styled(icons.app, Style::new().fg(theme::ACCENT)),
        Span::raw(" "),
        Span::styled("proj", theme::title()),
    ];
    let mut right = Vec::new();
    if app.busy() {
        right.push(Span::styled(
            format!("{} ", app.spinner()),
            Style::new().fg(theme::ACCENT),
        ));
    }
    right.push(Span::styled(tilde(&app.cfg.root()), theme::muted()));
    f.render_widget(Paragraph::new(split_line(left, right, area.width)), area);
}

fn render_search(
    f: &mut Frame,
    pane: &Pane,
    icons: &Icons,
    area: Rect,
    placeholder: &str,
    focused: bool,
) {
    let border = if focused {
        Style::new().fg(theme::ACCENT)
    } else {
        theme::border()
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border);
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, area);
    let prefix = format!("{} ", icons.search);
    let mut spans = vec![Span::styled(
        prefix.clone(),
        if focused {
            Style::new().fg(theme::ACCENT)
        } else {
            theme::muted()
        },
    )];
    if pane.input.text.is_empty() {
        spans.push(Span::styled(placeholder.to_string(), theme::muted()));
    } else {
        spans.push(Span::styled(pane.input.text.clone(), theme::text()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), inner);
    if focused {
        let before: String = pane.input.text.chars().take(pane.input.cursor).collect();
        let x = inner.x + (prefix.width() + before.width()) as u16;
        f.set_cursor_position(Position::new(
            x.min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

fn render_list(f: &mut Frame, app: &App, pane: &Pane, icons: &Icons, area: Rect, dialog: bool) {
    let rows = pane.row_count();
    if rows == 0 {
        let text = empty_message(app, pane, dialog);
        let y = area.y + area.height.saturating_sub(1) / 3;
        f.render_widget(
            Paragraph::new(text).style(theme::muted()).centered(),
            Rect {
                y,
                height: 1,
                ..area
            },
        );
        return;
    }
    let visible = area.height.div_ceil(ROW_HEIGHT).max(1) as usize;
    let visible = if (visible as u16) * ROW_HEIGHT - 1 > area.height {
        visible - 1
    } else {
        visible
    }
    .max(1);
    let offset = pane.scroll_into_view(visible);
    let animations = app.animations();
    let url_rows = usize::from(pane.url.is_some());
    for (slot, row) in (offset..rows.min(offset + visible)).enumerate() {
        let y = area.y + slot as u16 * ROW_HEIGHT;
        let height = 2.min(area.bottom() - y);
        let selected = row == pane.selected;
        let (lines, reveal) = if row < url_rows {
            (url_row(app, pane, icons, selected, area.width - 2), 1.0)
        } else {
            let hit = &pane.hits[row - url_rows];
            let entry = &app.entries[hit.idx];
            let reveal = if animations {
                pane.row_reveal(&entry.id)
            } else {
                1.0
            };
            (
                entry_row(
                    app,
                    icons,
                    entry,
                    &hit.title_indices,
                    selected,
                    area.width - 2,
                ),
                reveal,
            )
        };
        if reveal <= 0.02 {
            continue;
        }
        let shift = ((1.0 - reveal) * 4.0).round() as u16;
        let rect = Rect {
            x: area.x + 2 + shift,
            y,
            width: (area.width - 2).saturating_sub(shift),
            height,
        };
        let mut paragraph = Paragraph::new(lines);
        if reveal < 0.7 {
            paragraph = paragraph.style(Style::new().add_modifier(Modifier::DIM));
        }
        f.render_widget(paragraph, rect);
    }

    let position = if animations {
        pane.visual_selection()
    } else {
        pane.selected as f32
    };
    let bar_y = area.y as f32 + (position - offset as f32) * ROW_HEIGHT as f32;
    let bar_y = bar_y.round() as i32;
    for dy in 0..2 {
        let y = bar_y + dy;
        if y >= area.y as i32 && y < area.bottom() as i32 {
            f.buffer_mut()[(area.x, y as u16)]
                .set_symbol(icons.selected)
                .set_style(Style::new().fg(theme::ACCENT));
        }
    }
}

fn empty_message(app: &App, pane: &Pane, dialog: bool) -> String {
    let loading = app.providers.values().any(|s| *s == ProviderState::Loading);
    let query = &pane.input.text;
    match dialog {
        false if query.is_empty() => format!(
            "No repositories under {} yet · Ctrl+A to clone one",
            tilde(&app.cfg.root())
        ),
        false => format!("No matches for “{query}”"),
        true if loading => format!("{} Fetching remote repositories…", app.spinner()),
        true if app.cache.providers.is_empty() => {
            "No authenticated provider · run `gh auth login` / `glab auth login`, or paste a Git URL"
                .into()
        }
        true if query.is_empty() => "Every remote repository is already cloned".into(),
        true => "No matches · paste a Git URL to clone it directly".into(),
    }
}

fn url_row<'a>(app: &App, pane: &Pane, icons: &Icons, selected: bool, width: u16) -> Vec<Line<'a>> {
    let url = pane.url.as_ref().expect("url row");
    let provider = Provider::from_host(&url.host);
    let title_style = if selected {
        theme::title()
    } else {
        theme::text()
    };
    let line1 = split_line(
        vec![
            Span::styled(icons.link, Style::new().fg(theme::ACCENT)),
            Span::raw(" "),
            Span::styled(format!("Clone {}", url.path), title_style),
        ],
        vec![Span::styled(
            format!("{} {}", provider_icon(icons, provider), url.host),
            theme::muted(),
        )],
        width,
    );
    let status = match &app.classifying {
        Some(_) => Span::styled(
            format!("{} detecting owner…", app.spinner()),
            Style::new().fg(theme::ACCENT),
        ),
        None => Span::styled("from URL", Style::new().fg(theme::REMOTE)),
    };
    let line2 = split_line(
        vec![
            Span::raw("  "),
            Span::styled(pane.input.text.trim().to_string(), theme::muted()),
        ],
        vec![status],
        width,
    );
    vec![line1, line2]
}

fn entry_row<'a>(
    app: &App,
    icons: &Icons,
    e: &'a Entry,
    matched: &[u32],
    selected: bool,
    width: u16,
) -> Vec<Line<'a>> {
    let local = e.local.as_ref();
    let (icon, color) = match (local.map(|l| l.kind), e.category) {
        (Some(LocalKind::Folder), _) => (icons.folder, theme::FOLDER),
        (None, _) => (icons.remote, theme::REMOTE),
        (Some(LocalKind::Git), Some(Category::Companies)) => (icons.company, theme::COMPANY),
        (Some(LocalKind::Git), _) => (icons.personal, theme::PERSONAL),
    };
    let base = if selected {
        theme::title()
    } else {
        theme::text()
    };
    let mut left = vec![Span::styled(icon, Style::new().fg(color)), Span::raw(" ")];
    left.extend(highlighted(&e.title, matched, base));
    if let Some(r) = &e.remote {
        if r.private {
            left.push(Span::styled(format!(" {}", icons.lock), theme::muted()));
        }
        if r.archived {
            left.push(Span::styled(format!(" {}", icons.archived), theme::muted()));
        }
    }
    let right1 = match local {
        Some(l) if l.kind == LocalKind::Git => vec![
            Span::styled(format!("{} ", icons.branch), theme::muted()),
            Span::styled(
                l.branch.clone().unwrap_or_else(|| "detached".into()),
                theme::text(),
            ),
        ],
        Some(_) => Vec::new(),
        None => {
            let provider = e.provider.unwrap_or(Provider::Other);
            vec![Span::styled(
                format!("{} {}", provider_icon(icons, provider), provider.label()),
                theme::muted(),
            )]
        }
    };

    let cloning = app.clone.as_ref().filter(|_| app.is_cloning(e));
    let left2 = match cloning {
        Some(task) => vec![
            Span::raw("  "),
            Span::styled(task.phase.clone(), Style::new().fg(theme::ACCENT)),
        ],
        None => vec![
            Span::raw("  "),
            Span::styled(details(app, e).join(" · "), theme::muted()),
        ],
    };
    let right2 = match cloning {
        Some(task) => progress_spans(app, task, 14),
        None => state_spans(icons, e),
    };
    vec![
        split_line(left, right1, width),
        split_line(left2, right2, width),
    ]
}

fn details(app: &App, e: &Entry) -> Vec<String> {
    let local = e.local.as_ref();
    let mut details: Vec<String> = Vec::new();
    match local {
        Some(l) if l.kind == LocalKind::Folder => details.push("Folder".into()),
        _ => details.push(e.category.map_or("Unclassified", Category::label).into()),
    }
    if let (Some(l), Some(p)) = (local, e.provider)
        && l.kind == LocalKind::Git
    {
        details.push(p.label().into());
    }
    match local {
        Some(l) if app.cfg.display.show_paths => details.push(tilde(&l.path)),
        None => {
            if let Some(d) = e.remote.as_ref().and_then(|r| r.description.as_deref()) {
                details.push(d.to_string());
            } else if let Some(rel) = &e.rel_path {
                details.push(format!("→ {rel}"));
            }
        }
        _ => {}
    }
    details
}

fn state_spans<'a>(icons: &Icons, e: &Entry) -> Vec<Span<'a>> {
    let Some(local) = &e.local else {
        return vec![Span::styled(
            format!("{} not cloned", icons.download),
            Style::new().fg(theme::REMOTE),
        )];
    };
    if local.kind == LocalKind::Folder {
        return vec![Span::styled("no git", theme::muted())];
    }
    let Some(status) = &local.status else {
        return vec![Span::styled("…", theme::muted())];
    };
    let mut spans = Vec::new();
    if status.ahead > 0 {
        spans.push(Span::styled(
            format!("{}{} ", icons.ahead, status.ahead),
            Style::new().fg(theme::WARNING),
        ));
    }
    if status.behind > 0 {
        spans.push(Span::styled(
            format!("{}{} ", icons.behind, status.behind),
            Style::new().fg(theme::REMOTE),
        ));
    }
    spans.push(match status.changes {
        0 => Span::styled(
            format!("{} Clean", icons.clean),
            Style::new().fg(theme::SUCCESS),
        ),
        1 => Span::styled(
            format!("{} 1 change", icons.dirty),
            Style::new().fg(theme::WARNING),
        ),
        n => Span::styled(
            format!("{} {n} changes", icons.dirty),
            Style::new().fg(theme::WARNING),
        ),
    });
    spans
}

fn progress_spans<'a>(app: &App, task: &CloneTask, width: usize) -> Vec<Span<'a>> {
    let accent = Style::new().fg(if task.cancelling {
        theme::WARNING
    } else {
        theme::ACCENT
    });
    match task.percent {
        Some(pct) => {
            let filled = (pct as usize * width).div_ceil(100).min(width);
            vec![
                Span::styled("━".repeat(filled), accent),
                Span::styled("─".repeat(width - filled), theme::muted()),
                Span::styled(format!(" {pct:>3}%"), accent),
            ]
        }
        None if app.animations() => {
            let segment = 4;
            let period = (width + segment) as u128;
            let pos = (app.elapsed().as_millis() / 60 % period) as usize;
            let bar: String = (0..width)
                .map(|i| {
                    if i + segment > pos && i < pos {
                        '━'
                    } else {
                        '─'
                    }
                })
                .collect();
            let (lit_start, lit_end) = (pos.saturating_sub(segment), pos.min(width));
            let chars: Vec<char> = bar.chars().collect();
            vec![
                Span::styled(
                    chars[..lit_start].iter().collect::<String>(),
                    theme::muted(),
                ),
                Span::styled(chars[lit_start..lit_end].iter().collect::<String>(), accent),
                Span::styled(chars[lit_end..].iter().collect::<String>(), theme::muted()),
                Span::styled("     ", accent),
            ]
        }
        None => vec![Span::styled(format!("{} working", app.spinner()), accent)],
    }
}

fn render_separator(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let line = if app.animations() && app.busy() {
        let segment = (width / 6).max(6);
        let period = (width + segment) as u128;
        let head = (app.elapsed().as_millis() / 12 % period) as usize;
        let start = head.saturating_sub(segment);
        let end = head.min(width);
        Line::from(vec![
            Span::styled("─".repeat(start), theme::border()),
            Span::styled("━".repeat(end - start), Style::new().fg(theme::ACCENT)),
            Span::styled("─".repeat(width - end), theme::border()),
        ])
    } else {
        Line::from(Span::styled("─".repeat(width), theme::border()))
    };
    f.render_widget(Paragraph::new(line), area);
}

fn key<'a>(k: &'static str) -> Span<'a> {
    Span::styled(k, theme::title())
}

fn label<'a>(l: &'static str) -> Span<'a> {
    Span::styled(l, theme::muted())
}

fn dot<'a>() -> Span<'a> {
    Span::styled("  ", theme::muted())
}

fn render_hints(f: &mut Frame, app: &App, area: Rect) {
    let enter = match app.main.selected_entry(&app.entries) {
        Some(e) if !e.is_local() => " clone",
        _ => " open",
    };
    let mut left = vec![
        key("↵"),
        label(enter),
        dot(),
        key("^A"),
        label(" add"),
        dot(),
    ];
    if app.clone.is_some() {
        left.extend([key("Esc"), label(" cancel clone")]);
    } else {
        left.extend([
            key("^R"),
            label(" refresh"),
            dot(),
            key("Esc"),
            label(" quit"),
        ]);
    }
    let count = app.main.row_count();
    let right = vec![Span::styled(
        if count == 0 {
            "0".to_string()
        } else {
            format!("{}/{count}", app.main.selected + 1)
        },
        theme::muted(),
    )];
    f.render_widget(Paragraph::new(split_line(left, right, area.width)), area);
}

fn render_status(f: &mut Frame, app: &App, icons: &Icons, area: Rect) {
    let accent = Style::new().fg(theme::ACCENT);
    let line = if let Some(task) = &app.clone {
        let mut spans = vec![
            Span::styled(format!("{} ", app.spinner()), accent),
            Span::styled(
                if task.cancelling {
                    "Cancelling "
                } else {
                    "Cloning "
                },
                accent,
            ),
            Span::styled(task.title.clone(), theme::title()),
            Span::raw("  "),
        ];
        spans.extend(progress_spans(app, task, 20));
        spans.push(Span::styled(format!("  {}", task.phase), theme::muted()));
        Line::from(spans)
    } else if let Some(name) = &app.classifying {
        Line::from(Span::styled(
            format!("{} Detecting the owner type of {name}…", app.spinner()),
            accent,
        ))
    } else if let Some(msg) = &app.message {
        let style = match msg.tone {
            Tone::Info => theme::text(),
            Tone::Success => Style::new().fg(theme::SUCCESS),
            Tone::Error => Style::new().fg(theme::ERROR),
        };
        Line::from(Span::styled(msg.text.clone(), style))
    } else {
        provider_summary(app, icons)
    };
    f.render_widget(
        Paragraph::new(truncate_line(line, area.width as usize)),
        area,
    );
}

fn provider_summary<'a>(app: &App, icons: &Icons) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
    let sep = || Span::styled(" · ", theme::muted());
    let locals = app
        .locals
        .iter()
        .filter(|l| l.kind == LocalKind::Git)
        .count();
    if app.pending_status > 0 {
        spans.push(Span::styled(
            format!("{} ", app.spinner()),
            Style::new().fg(theme::ACCENT),
        ));
    }
    spans.push(Span::styled(format!("{locals} local"), theme::muted()));
    for (provider, state) in &app.providers {
        spans.push(sep());
        let icon = provider_icon(icons, *provider);
        let count = app.remote_count(*provider);
        let cached = app
            .cached_age(*provider)
            .map(|age| format!(" (cached {age})"))
            .unwrap_or_default();
        match state {
            ProviderState::Loading => {
                spans.push(Span::styled(
                    format!("{} ", app.spinner()),
                    Style::new().fg(theme::ACCENT),
                ));
                spans.push(Span::styled(
                    format!("{} {}", icon, provider.label()),
                    theme::muted(),
                ));
                if count > 0 {
                    spans.push(Span::styled(format!(" {count}{cached}"), theme::muted()));
                }
            }
            ProviderState::Ready => {
                spans.push(Span::styled(
                    format!("{icon} {} {count}", provider.label()),
                    theme::muted(),
                ));
            }
            ProviderState::Unavailable(why) => {
                spans.push(Span::styled(
                    format!("{icon} {}: {why}{cached}", provider.label()),
                    theme::muted(),
                ));
            }
            ProviderState::Failed(e) => {
                spans.push(Span::styled(
                    format!("{icon} {} offline{cached}", provider.label()),
                    Style::new().fg(theme::ERROR),
                ));
                spans.push(Span::styled(format!(" — {e}"), theme::muted()));
            }
        }
    }
    if app.providers.is_empty() {
        spans.push(sep());
        spans.push(Span::styled("remote providers disabled", theme::muted()));
    }
    Line::from(spans)
}

fn scaled_rect(area: Rect, width: u16, height: u16, scale: f32) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let w = (width as f32 * (0.7 + 0.3 * scale)).round() as u16;
    let h = ((height as f32) * scale).round().max(3.0) as u16;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h.min(height),
    }
}

fn render_add_dialog(f: &mut Frame, app: &App, icons: &Icons, scale: f32) {
    let area = f.area();
    let target_w = (area.width * 4 / 5).clamp(40, 120);
    let target_h = (area.height * 4 / 5).clamp(14, 40);
    let rect = scaled_rect(area, target_w, target_h, scale);
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::ACCENT))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(icons.add, Style::new().fg(theme::ACCENT)),
            Span::styled(" Add repository ", theme::title()),
        ]));
    let inner = block.inner(rect).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, rect);
    if scale < 0.85 || inner.height < 8 {
        return;
    }
    let [search, _, list, separator, hints] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let Some(dialog) = &app.add else {
        return;
    };
    let focused = app.prompt.is_none() && app.dialog_open();
    render_search(
        f,
        &dialog.pane,
        icons,
        search,
        "Search repositories to clone, or paste a Git URL…",
        focused,
    );
    render_list(f, app, &dialog.pane, icons, list, true);
    f.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(separator.width as usize),
            theme::border(),
        )),
        separator,
    );
    let enter = if dialog.pane.row_count() == 0 {
        " —"
    } else {
        " clone"
    };
    let left = vec![
        key("↵"),
        label(enter),
        dot(),
        key("^R"),
        label(" refresh"),
        dot(),
        key("Esc"),
        label(" close"),
    ];
    let loading = app.providers.values().any(|s| *s == ProviderState::Loading);
    let count = dialog.pane.row_count();
    let right = if loading {
        vec![Span::styled(
            format!("{} fetching", app.spinner()),
            Style::new().fg(theme::ACCENT),
        )]
    } else {
        vec![Span::styled(
            if count == 0 {
                "0".to_string()
            } else {
                format!("{}/{count}", dialog.pane.selected + 1)
            },
            theme::muted(),
        )]
    };
    f.render_widget(Paragraph::new(split_line(left, right, hints.width)), hints);
}

fn render_prompt(f: &mut Frame, app: &App) {
    let Some(prompt) = &app.prompt else { return };
    let area = f.area();
    let rect = scaled_rect(area, area.width.saturating_sub(4).min(72), 9, 1.0);
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::ACCENT))
        .title(Span::styled(" Clone destination ", theme::title()));
    let inner = block.inner(rect).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, rect);
    let p = &prompt.pending;
    let option = |category: Category| {
        let dest = crate::clone::destination(&app.cfg, category, &p.owner, &p.name);
        let chosen = prompt.choice == category;
        Line::from(vec![
            Span::styled(
                if chosen { "● " } else { "○ " },
                if chosen {
                    theme::highlight()
                } else {
                    theme::muted()
                },
            ),
            Span::styled(
                format!("{:<9}", category.label()),
                if chosen {
                    theme::title()
                } else {
                    theme::text()
                },
            ),
            Span::styled(format!("→ {}", tilde(&dest)), theme::muted()),
        ])
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Could not tell whether ", theme::text()),
            Span::styled(p.owner.clone(), theme::title()),
            Span::styled(" is a user or an organization.", theme::text()),
        ]),
        Line::from(Span::styled(
            format!("Where should {} be cloned?", p.full_name()),
            theme::text(),
        )),
        Line::default(),
        option(Category::Personal),
        option(Category::Companies),
        Line::default(),
        Line::from(vec![
            key("↵"),
            label(" clone"),
            dot(),
            key("←/→"),
            label(" choose"),
            dot(),
            key("Esc"),
            label(" cancel"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn provider_icon(icons: &Icons, provider: Provider) -> &'static str {
    match provider {
        Provider::GitHub => icons.github,
        Provider::GitLab => icons.gitlab,
        Provider::Other => icons.git,
    }
}

fn highlighted<'a>(title: &str, matched: &[u32], base: Style) -> Vec<Span<'a>> {
    if matched.is_empty() {
        return vec![Span::styled(title.to_string(), base)];
    }
    let hl = base.fg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_hl = false;
    for (i, c) in title.chars().enumerate() {
        let is_hl = matched.binary_search(&(i as u32)).is_ok();
        if is_hl != current_hl && !current.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut current),
                if current_hl { hl } else { base },
            ));
        }
        current_hl = is_hl;
        current.push(c);
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, if current_hl { hl } else { base }));
    }
    spans
}

fn split_line<'a>(left: Vec<Span<'a>>, right: Vec<Span<'a>>, width: u16) -> Line<'a> {
    let width = width as usize;
    let right = truncate_spans(right, width * 45 / 100);
    let right_width = spans_width(&right);
    let left = truncate_spans(
        left,
        width.saturating_sub(right_width + usize::from(right_width > 0)),
    );
    let fill = width.saturating_sub(spans_width(&left) + right_width);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(fill)));
    spans.extend(right);
    Line::from(spans)
}

fn truncate_line(line: Line<'_>, width: usize) -> Line<'_> {
    Line::from(truncate_spans(line.spans, width))
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

fn truncate_spans<'a>(spans: Vec<Span<'a>>, max: usize) -> Vec<Span<'a>> {
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.content.width();
        if used + w < max {
            used += w;
            out.push(span);
            continue;
        }
        let text = truncate(&span.content, max - used);
        out.push(Span::styled(text, span.style));
        break;
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_line_fills_exact_width() {
        let line = split_line(
            vec![Span::raw("datanorte/frontend-application")],
            vec![Span::raw("develop")],
            24,
        );
        assert_eq!(spans_width(&line.spans), 24);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "datanorte/front… develop");
    }
}
