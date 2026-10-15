use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::cache::{self, Cache};
use crate::catalog::{self, Entry, Hit, RemoteSource, Searcher};
use crate::clone::{self, Cancelled, CloneControl, CloneJob};
use crate::config::{Config, tilde};
use crate::discovery::{self, StatusReport};
use crate::giturl::GitUrl;
use crate::model::{Category, LocalKind, LocalRepo, Provider, RemoteRepo};
use crate::providers::{self, FetchOutcome};

pub const DIALOG_OPEN: Duration = Duration::from_millis(180);
pub const DIALOG_CLOSE: Duration = Duration::from_millis(120);
const SELECTION_SLIDE: Duration = Duration::from_millis(110);
const ROW_FADE: Duration = Duration::from_millis(160);
const ROW_STAGGER: Duration = Duration::from_millis(22);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Browse,
    Add,
}

pub enum Outcome {
    Cd(PathBuf),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderState {
    Loading,
    Ready,
    Unavailable(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Error,
}

pub struct Message {
    pub text: String,
    pub tone: Tone,
    at: Instant,
}

#[derive(Debug, Clone)]
pub struct PendingClone {
    pub owner: String,
    pub name: String,
    pub key: String,
    source: PendingSource,
}

#[derive(Debug, Clone)]
enum PendingSource {
    Remote(RemoteRepo),
    Url(String),
}

impl PendingClone {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

pub struct CloneTask {
    pub key: String,
    pub title: String,
    pub phase: String,
    pub percent: Option<u8>,
    pub cancelling: bool,
    control: Arc<CloneControl>,
}

impl CloneTask {
    fn update(&mut self, line: &str) {
        let line = line.strip_prefix("remote:").unwrap_or(line).trim();
        match line.split_once(':') {
            Some((phase, rest)) if rest.contains('%') => {
                self.phase = phase.trim().to_string();
                self.percent = rest
                    .split('%')
                    .next()
                    .and_then(|n| n.trim().rsplit(' ').next())
                    .and_then(|n| n.parse().ok());
            }
            _ => {
                self.phase = line.trim_end_matches("...").to_string();
                self.percent = None;
            }
        }
    }
}

pub struct CategoryPrompt {
    pub pending: PendingClone,
    pub choice: Category,
}

enum CloneEnd {
    Done(PathBuf),
    Cancelled,
    Failed(String),
}

enum Msg {
    Status(PathBuf, Option<StatusReport>),
    Remote(Provider, FetchOutcome),
    Classified(Box<PendingClone>, Option<Category>),
    CloneProgress(String),
    CloneDone(CloneEnd),
}

struct RefreshReport {
    added: usize,
    removed: usize,
}

#[derive(Default)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
}

impl Input {
    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn insert(&mut self, s: &str) {
        let at = self.byte_at(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.text.chars().count() {
            let at = self.byte_at(self.cursor);
            self.text.remove(at);
        }
    }

    fn delete_word(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
        self.text.replace_range(a..b, "");
        self.cursor = start;
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn edit(&mut self, code: KeyCode, ctrl: bool) -> Option<bool> {
        match (code, ctrl) {
            (KeyCode::Char('u'), true) => self.clear(),
            (KeyCode::Char('w'), true) | (KeyCode::Backspace, true) => self.delete_word(),
            (KeyCode::Backspace, false) => self.backspace(),
            (KeyCode::Delete, _) => self.delete(),
            (KeyCode::Char(c), false) => self.insert(c.encode_utf8(&mut [0; 4])),
            (KeyCode::Left, _) => {
                self.cursor = self.cursor.saturating_sub(1);
                return Some(false);
            }
            (KeyCode::Right, _) => {
                self.cursor = (self.cursor + 1).min(self.text.chars().count());
                return Some(false);
            }
            (KeyCode::Home, _) | (KeyCode::Char('b'), true) => {
                self.cursor = 0;
                return Some(false);
            }
            (KeyCode::End, _) | (KeyCode::Char('e'), true) => {
                self.cursor = self.text.chars().count();
                return Some(false);
            }
            _ => return None,
        }
        Some(true)
    }
}

pub struct Pane {
    pub input: Input,
    pub hits: Vec<Hit>,
    pub url: Option<GitUrl>,
    pub selected: usize,
    offset: Cell<usize>,
    sel_from: f32,
    sel_at: Instant,
    born: HashMap<String, RowAnim>,
}

#[derive(Clone, Copy)]
struct RowAnim {
    row: usize,
    since: Instant,
    order: u32,
}

impl Pane {
    fn new() -> Self {
        Self {
            input: Input::default(),
            hits: Vec::new(),
            url: None,
            selected: 0,
            offset: Cell::new(0),
            sel_from: 0.0,
            sel_at: Instant::now(),
            born: HashMap::new(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.hits.len() + usize::from(self.url.is_some())
    }

    fn url_rows(&self) -> usize {
        usize::from(self.url.is_some())
    }

    pub fn selected_entry<'a>(&self, entries: &'a [Entry]) -> Option<&'a Entry> {
        let i = self.selected.checked_sub(self.url_rows())?;
        self.hits.get(i).map(|h| &entries[h.idx])
    }

    pub fn url_selected(&self) -> bool {
        self.url.is_some() && self.selected == 0
    }

    fn refilter(
        &mut self,
        entries: &[Entry],
        searcher: &mut Searcher,
        include: impl Fn(&Entry) -> bool,
        allow_url: bool,
        keep_selection: bool,
    ) {
        let previous = keep_selection
            .then(|| self.selected_entry(entries).map(|e| e.id.clone()))
            .flatten();
        let before = self.visual_selection();
        self.hits = searcher.search(entries, &self.input.text, include);
        self.url = allow_url.then(|| GitUrl::parse(&self.input.text)).flatten();
        let now = Instant::now();
        let url_rows = self.url_rows();
        let first_visible = self.offset.get();
        let previous_rows = std::mem::take(&mut self.born);
        let mut order = 0;
        for (i, hit) in self.hits.iter().enumerate() {
            let id = &entries[hit.idx].id;
            let row = i + url_rows;
            let anim = match previous_rows.get(id) {
                Some(prev) if prev.row == row => RowAnim { row, ..*prev },
                _ => {
                    let anim = RowAnim {
                        row,
                        since: now,
                        order,
                    };
                    if row >= first_visible {
                        order += 1;
                    }
                    anim
                }
            };
            self.born.insert(id.clone(), anim);
        }
        let selected = previous
            .and_then(|id| self.hits.iter().position(|h| entries[h.idx].id == id))
            .map_or(if keep_selection { self.selected } else { 0 }, |i| {
                i + self.url_rows()
            })
            .min(self.row_count().saturating_sub(1));
        if !keep_selection {
            self.offset.set(0);
        }
        self.select(selected, before);
    }

    fn select(&mut self, row: usize, from: f32) {
        if row != self.selected {
            self.sel_from = from;
            self.sel_at = Instant::now();
        }
        self.selected = row;
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        let row = (self.selected as isize + delta).clamp(0, count as isize - 1) as usize;
        let from = self.visual_selection();
        self.select(row, from);
    }

    pub fn visual_selection(&self) -> f32 {
        let t = progress(self.sel_at, SELECTION_SLIDE);
        let to = self.selected as f32;
        self.sel_from + (to - self.sel_from) * ease_out(t)
    }

    pub fn row_reveal(&self, id: &str) -> f32 {
        self.born.get(id).map_or(1.0, |anim| {
            let delay = ROW_STAGGER * anim.order.min(12);
            let elapsed = anim.since.elapsed().saturating_sub(delay);
            ease_out((elapsed.as_secs_f32() / ROW_FADE.as_secs_f32()).min(1.0))
        })
    }

    pub fn scroll_into_view(&self, visible: usize) -> usize {
        let visible = visible.max(1);
        let mut offset = self.offset.get();
        if self.selected < offset {
            offset = self.selected;
        } else if self.selected >= offset + visible {
            offset = self.selected + 1 - visible;
        }
        offset = offset.min(self.row_count().saturating_sub(visible));
        self.offset.set(offset);
        offset
    }

    fn animating(&self) -> bool {
        self.sel_at.elapsed() < SELECTION_SLIDE
            || self
                .born
                .values()
                .any(|anim| anim.since.elapsed() < ROW_FADE + ROW_STAGGER * 12)
    }
}

pub struct AddDialog {
    pub pane: Pane,
    opened_at: Instant,
    closing_at: Option<Instant>,
}

pub fn progress(since: Instant, duration: Duration) -> f32 {
    (since.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.0)
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub struct App {
    pub cfg: Config,
    pub main: Pane,
    pub add: Option<AddDialog>,
    pub locals: Vec<LocalRepo>,
    pub cache: Cache,
    fresh: HashSet<Provider>,
    pub providers: BTreeMap<Provider, ProviderState>,
    pub entries: Vec<Entry>,
    pub pending_status: usize,
    pub clone: Option<CloneTask>,
    pub classifying: Option<String>,
    pub prompt: Option<CategoryPrompt>,
    pub message: Option<Message>,
    pub outcome: Option<Outcome>,
    quit_after_cancel: bool,
    refresh: Option<RefreshReport>,
    started: Instant,
    searcher: Searcher,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    pub fn new(cfg: Config, view: View) -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            locals: discovery::scan(&cfg),
            cache: Cache::load(),
            cfg,
            main: Pane::new(),
            add: None,
            fresh: HashSet::new(),
            providers: BTreeMap::new(),
            entries: Vec::new(),
            pending_status: 0,
            clone: None,
            classifying: None,
            prompt: None,
            message: None,
            outcome: None,
            quit_after_cancel: false,
            refresh: None,
            started: Instant::now(),
            searcher: Searcher::default(),
            tx,
            rx,
        };
        app.start_status_scan();
        app.start_remote_fetch();
        app.rebuild();
        if view == View::Add {
            app.open_add();
        }
        app
    }

    pub fn animations(&self) -> bool {
        self.cfg.display.animations
    }

    fn start_status_scan(&mut self) {
        let paths: Vec<PathBuf> = self
            .locals
            .iter()
            .filter(|l| l.kind == LocalKind::Git)
            .map(|l| l.path.clone())
            .collect();
        self.pending_status = paths.len();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            discovery::statuses_parallel(paths, |path, report| {
                let _ = tx.send(Msg::Status(path, report));
            });
        });
    }

    fn start_remote_fetch(&mut self) {
        for provider in providers::PROVIDERS {
            if !providers::enabled(&self.cfg, provider) {
                continue;
            }
            self.providers.insert(provider, ProviderState::Loading);
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Remote(provider, providers::fetch(provider)));
            });
        }
    }

    pub fn is_refreshing(&self) -> bool {
        self.pending_status > 0
            || self
                .providers
                .values()
                .any(|s| *s == ProviderState::Loading)
    }

    pub fn busy(&self) -> bool {
        self.is_refreshing() || self.clone.is_some() || self.classifying.is_some()
    }

    pub fn animating(&self) -> bool {
        self.animations()
            && (self.busy()
                || self.main.animating()
                || self.add.as_ref().is_some_and(|d| {
                    d.pane.animating()
                        || d.opened_at.elapsed() < DIALOG_OPEN
                        || d.closing_at.is_some()
                }))
    }

    pub fn dialog_scale(&self) -> Option<f32> {
        let dialog = self.add.as_ref()?;
        if !self.animations() {
            return Some(1.0);
        }
        Some(match dialog.closing_at {
            Some(at) => 1.0 - ease_out(progress(at, DIALOG_CLOSE)),
            None => ease_out(progress(dialog.opened_at, DIALOG_OPEN)),
        })
    }

    pub fn dialog_open(&self) -> bool {
        self.add.as_ref().is_some_and(|d| d.closing_at.is_none())
    }

    pub fn drain(&mut self) {
        let mut dirty = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(path, report) => {
                    dirty = true;
                    self.pending_status = self.pending_status.saturating_sub(1);
                    if let Some(local) = self.locals.iter_mut().find(|l| l.path == path) {
                        discovery::apply_status(local, report);
                    }
                }
                Msg::Remote(provider, outcome) => {
                    dirty = true;
                    self.on_remote(provider, outcome);
                }
                Msg::Classified(pending, category) => {
                    self.classifying = None;
                    match category {
                        Some(c) => self.start_clone(*pending, c),
                        None => {
                            self.prompt = Some(CategoryPrompt {
                                pending: *pending,
                                choice: Category::Personal,
                            })
                        }
                    }
                }
                Msg::CloneProgress(line) => {
                    if let Some(task) = &mut self.clone {
                        task.update(&line);
                    }
                }
                Msg::CloneDone(end) => {
                    dirty = true;
                    self.on_clone_done(end);
                }
            }
        }
        if dirty {
            self.rebuild();
        }
        if self.refresh.is_some() && !self.is_refreshing() {
            let report = self.refresh.take().expect("checked");
            let failures: Vec<String> = self
                .providers
                .iter()
                .filter_map(|(p, s)| match s {
                    ProviderState::Failed(e) => Some(format!("{}: {e}", p.label())),
                    _ => None,
                })
                .collect();
            if failures.is_empty() {
                self.notify(
                    Tone::Success,
                    format!(
                        "Refreshed · {} new, {} removed",
                        report.added, report.removed
                    ),
                );
            } else {
                self.notify(
                    Tone::Error,
                    format!("Refresh incomplete · {}", failures.join(" · ")),
                );
            }
        }
        if self
            .message
            .as_ref()
            .is_some_and(|m| m.tone != Tone::Error && m.at.elapsed() > Duration::from_secs(6))
        {
            self.message = None;
        }
        if self
            .add
            .as_ref()
            .and_then(|d| d.closing_at)
            .is_some_and(|at| !self.animations() || at.elapsed() >= DIALOG_CLOSE)
        {
            self.add = None;
        }
    }

    fn on_remote(&mut self, provider: Provider, outcome: FetchOutcome) {
        let state = match outcome {
            FetchOutcome::Fetched(repos) => {
                let diff = self.cache.update(provider, repos);
                if let Some(r) = &mut self.refresh {
                    r.added += diff.added;
                    r.removed += diff.removed;
                }
                self.fresh.insert(provider);
                if let Err(e) = self.cache.save() {
                    self.notify(Tone::Error, format!("Cannot write cache: {e}"));
                }
                ProviderState::Ready
            }
            FetchOutcome::Unavailable(why) => ProviderState::Unavailable(why),
            FetchOutcome::Failed(e) => ProviderState::Failed(e),
        };
        self.providers.insert(provider, state);
    }

    fn on_clone_done(&mut self, end: CloneEnd) {
        let Some(task) = self.clone.take() else {
            return;
        };
        match end {
            CloneEnd::Done(dest) if self.cfg.clone.open_after_clone && !self.quit_after_cancel => {
                self.outcome = Some(Outcome::Cd(dest))
            }
            CloneEnd::Done(dest) => {
                self.notify(
                    Tone::Success,
                    format!("Cloned {} into {}", task.title, tilde(&dest)),
                );
                self.locals = discovery::scan(&self.cfg);
                self.start_status_scan();
            }
            CloneEnd::Cancelled => self.notify(
                Tone::Info,
                format!("Cancelled cloning {} · partial files removed", task.title),
            ),
            CloneEnd::Failed(e) => {
                self.notify(Tone::Error, format!("Clone of {} failed: {e}", task.title))
            }
        }
        if self.quit_after_cancel {
            self.outcome = Some(Outcome::Quit);
        }
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            tone,
            at: Instant::now(),
        });
    }

    pub fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[(self.started.elapsed().as_millis() / 80) as usize % FRAMES.len()]
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn rebuild(&mut self) {
        let sources: Vec<RemoteSource> = self
            .cache
            .providers
            .iter()
            .filter(|(p, _)| providers::enabled(&self.cfg, **p))
            .map(|(p, set)| RemoteSource {
                repos: &set.repos,
                cached: !self.fresh.contains(p),
            })
            .collect();
        self.entries = catalog::build(&self.cfg, &self.locals, &sources);
        self.refilter_main(true);
        self.refilter_add(true);
    }

    fn refilter_main(&mut self, keep_selection: bool) {
        let display = &self.cfg.display;
        self.main.refilter(
            &self.entries,
            &mut self.searcher,
            |e| (display.show_remote || e.is_local()) && (display.show_folders || !e.is_folder()),
            false,
            keep_selection,
        );
    }

    fn refilter_add(&mut self, keep_selection: bool) {
        if let Some(dialog) = &mut self.add {
            dialog.pane.refilter(
                &self.entries,
                &mut self.searcher,
                |e| !e.is_local(),
                true,
                keep_selection,
            );
        }
    }

    pub fn is_cloning(&self, entry: &Entry) -> bool {
        match (&self.clone, &entry.remote) {
            (Some(task), Some(remote)) => !entry.is_local() && remote.key() == task.key,
            _ => false,
        }
    }

    fn open_add(&mut self) {
        self.add = Some(AddDialog {
            pane: Pane::new(),
            opened_at: Instant::now(),
            closing_at: None,
        });
        self.refilter_add(false);
    }

    fn close_add(&mut self) {
        if let Some(dialog) = &mut self.add
            && dialog.closing_at.is_none()
        {
            dialog.closing_at = Some(Instant::now());
        }
    }

    fn refresh_all(&mut self) {
        if self.is_refreshing() {
            self.notify(Tone::Info, "Refresh already in progress");
            return;
        }
        self.locals = discovery::scan(&self.cfg);
        self.start_status_scan();
        self.start_remote_fetch();
        self.refresh = Some(RefreshReport {
            added: 0,
            removed: 0,
        });
        self.message = None;
        self.rebuild();
    }

    fn activate(&mut self) {
        let in_dialog = self.dialog_open();
        let pane = if in_dialog {
            &self.add.as_ref().expect("dialog open").pane
        } else {
            &self.main
        };
        if pane.url_selected() {
            let url = pane.url.clone().expect("url row");
            let raw = pane.input.text.trim().to_string();
            self.close_add();
            return self.clone_url(url, raw);
        }
        let Some(entry) = pane.selected_entry(&self.entries).cloned() else {
            return;
        };
        if let Some(local) = &entry.local {
            self.outcome = Some(Outcome::Cd(local.path.clone()));
            return;
        }
        let Some(remote) = entry.remote else { return };
        self.close_add();
        let pending = PendingClone {
            owner: remote.owner.clone(),
            name: remote.name.clone(),
            key: remote.key(),
            source: PendingSource::Remote(remote),
        };
        match entry.category {
            Some(c) => self.start_clone(pending, c),
            None => {
                self.prompt = Some(CategoryPrompt {
                    pending,
                    choice: Category::Personal,
                })
            }
        }
    }

    fn clone_url(&mut self, url: GitUrl, raw: String) {
        let key = url.key();
        if let Some(local) = self.locals.iter().find(|l| {
            l.remotes
                .iter()
                .any(|r| GitUrl::parse(&r.url).is_some_and(|u| u.key() == key))
        }) {
            self.outcome = Some(Outcome::Cd(local.path.clone()));
            return;
        }
        let known = self
            .entries
            .iter()
            .filter_map(|e| e.remote.as_ref())
            .find(|r| r.key() == key);
        let pending = PendingClone {
            owner: url.owner().to_string(),
            name: url.name().to_string(),
            key,
            source: PendingSource::Url(raw),
        };
        let category = self
            .cfg
            .classify_override(&pending.owner, &pending.name)
            .or_else(|| known.and_then(|r| r.owner_kind.category()));
        if let Some(c) = category {
            return self.start_clone(pending, c);
        }
        if self.classifying.is_some() {
            return self.notify(Tone::Info, "Still detecting the owner of the previous URL");
        }
        let provider = Provider::from_host(&url.host);
        if provider == Provider::Other {
            self.prompt = Some(CategoryPrompt {
                pending,
                choice: Category::Personal,
            });
            return;
        }
        self.classifying = Some(pending.full_name());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let category =
                providers::owner_kind(provider, &url.host, url.owner()).and_then(|k| k.category());
            let _ = tx.send(Msg::Classified(Box::new(pending), category));
        });
    }

    fn start_clone(&mut self, pending: PendingClone, category: Category) {
        if let Some(task) = &self.clone {
            return self.notify(
                Tone::Info,
                format!("Wait for {} to finish cloning", task.title),
            );
        }
        let dest = clone::destination(&self.cfg, category, &pending.owner, &pending.name);
        let job = match &pending.source {
            PendingSource::Remote(remote) => CloneJob::for_remote(&self.cfg, remote, dest.clone()),
            PendingSource::Url(url) => CloneJob::for_url(url, dest.clone()),
        };
        if let Err(e) = job.check() {
            return self.notify(
                Tone::Error,
                format!("Cannot clone {}: {e}", pending.full_name()),
            );
        }
        let control = Arc::new(CloneControl::default());
        self.clone = Some(CloneTask {
            key: pending.key.clone(),
            title: pending.full_name(),
            phase: format!("Starting clone into {}", tilde(&dest)),
            percent: None,
            cancelling: false,
            control: Arc::clone(&control),
        });
        self.message = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = job.run_captured(&control, |line| {
                let _ = progress_tx.send(Msg::CloneProgress(line.to_string()));
            });
            let end = match result {
                Ok(()) => CloneEnd::Done(dest),
                Err(e) if e.is::<Cancelled>() => CloneEnd::Cancelled,
                Err(e) => CloneEnd::Failed(e.to_string()),
            };
            let _ = tx.send(Msg::CloneDone(end));
        });
    }

    fn cancel_clone(&mut self) {
        if let Some(task) = &mut self.clone {
            task.cancelling = true;
            task.phase = "Cancelling and removing partial files".into();
            task.percent = None;
            task.control.cancel();
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        if self.prompt.is_some() {
            return;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if self.dialog_open() {
            self.add
                .as_mut()
                .expect("dialog")
                .pane
                .input
                .insert(text.trim());
            self.refilter_add(false);
        } else {
            self.main.input.insert(text.trim());
            self.refilter_main(false);
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if self.clone.is_some() {
                self.quit_after_cancel = true;
                self.cancel_clone();
            } else {
                self.outcome = Some(Outcome::Quit);
            }
            return;
        }
        if self.prompt.is_some() {
            return self.on_prompt_key(key);
        }
        let in_dialog = self.dialog_open();
        match (key.code, ctrl) {
            (KeyCode::Esc, _) if in_dialog => self.close_add(),
            (KeyCode::Esc, _) if self.clone.is_some() => self.cancel_clone(),
            (KeyCode::Esc, _) => self.outcome = Some(Outcome::Quit),
            (KeyCode::Char('x'), true) if self.clone.is_some() => self.cancel_clone(),
            (KeyCode::Enter, _) => self.activate(),
            (KeyCode::Char('a'), true) if in_dialog => self.close_add(),
            (KeyCode::Char('a'), true) => self.open_add(),
            (KeyCode::Char('r'), true) => self.refresh_all(),
            (KeyCode::Up, _) | (KeyCode::Char('p' | 'k'), true) | (KeyCode::BackTab, _) => {
                self.focused_pane().move_selection(-1)
            }
            (KeyCode::Down, _) | (KeyCode::Char('n' | 'j'), true) | (KeyCode::Tab, _) => {
                self.focused_pane().move_selection(1)
            }
            (KeyCode::PageUp, _) => self.focused_pane().move_selection(-8),
            (KeyCode::PageDown, _) => self.focused_pane().move_selection(8),
            (code, ctrl) => {
                if self.focused_pane().input.edit(code, ctrl) == Some(true) {
                    if in_dialog {
                        self.refilter_add(false);
                    } else {
                        self.refilter_main(false);
                    }
                }
            }
        }
    }

    fn focused_pane(&mut self) -> &mut Pane {
        match &mut self.add {
            Some(dialog) if dialog.closing_at.is_none() => &mut dialog.pane,
            _ => &mut self.main,
        }
    }

    fn on_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Char('p') => prompt.choice = Category::Personal,
            KeyCode::Char('c') => prompt.choice = Category::Companies,
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Tab
            | KeyCode::BackTab => {
                prompt.choice = match prompt.choice {
                    Category::Personal => Category::Companies,
                    Category::Companies => Category::Personal,
                };
            }
            KeyCode::Enter => {
                let prompt = self.prompt.take().expect("checked");
                self.start_clone(prompt.pending, prompt.choice);
            }
            _ => {}
        }
    }

    pub fn cached_age(&self, provider: Provider) -> Option<String> {
        (!self.fresh.contains(&provider))
            .then(|| {
                self.cache
                    .providers
                    .get(&provider)
                    .map(|s| cache::age(s.fetched_at))
            })
            .flatten()
    }

    pub fn remote_count(&self, provider: Provider) -> usize {
        self.cache
            .providers
            .get(&provider)
            .map_or(0, |s| s.repos.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> CloneTask {
        CloneTask {
            key: String::new(),
            title: String::new(),
            phase: String::new(),
            percent: None,
            cancelling: false,
            control: Arc::new(CloneControl::default()),
        }
    }

    #[test]
    fn parses_git_progress_lines() {
        let mut t = task();
        t.update("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s");
        assert_eq!(
            (t.phase.as_str(), t.percent),
            ("Receiving objects", Some(45))
        );
        t.update("remote: Counting objects: 100% (13/13), done.");
        assert_eq!(
            (t.phase.as_str(), t.percent),
            ("Counting objects", Some(100))
        );
        t.update("Cloning into '/tmp/x'...");
        assert_eq!(
            (t.phase.as_str(), t.percent),
            ("Cloning into '/tmp/x'", None)
        );
    }
}
