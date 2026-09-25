//! Sync from the viewer. The Sync tab is always in the toolbar. The first
//! click on it, and the Plan button after that, look for the Kobo and read
//! the plan on a background task, because the device database sits on a
//! USB volume and opening it takes a moment. The plan fills the pane, one
//! row per action. The Sync button runs every action, because the device
//! holds what the library holds and nothing less. The actions run one per
//! background task, so each row lands as it runs and the window draws
//! between them. Each task takes the `Library` and the `Kobo` values with
//! it and hands them back in a `Handoff`.
//!
//! After the last action one more task deletes the macOS `._` files,
//! writes the Kobo rows, and reads progress and words back. Then the pane
//! says what the sync came to.
//!
//! The Eject button is always in the pane. It closes the device database
//! before the unmount, so an eject ends the plan whatever it comes to, and
//! Plan starts over. With no plan in view the eject looks for the Kobo
//! first.
//!
//! The planning, the read back, and the eject have no count to show, so a
//! block slides along the bar under the head while they run. The window's
//! frames move it, through `Message::Tick`, only while one of the three
//! runs.
//!
//! The device refuses row writes on a firmware version the app has not
//! been run against. Sync then holds the replacements back: the rows read
//! "held back" and do not run. The viewer has no switch for it, and the
//! reason says which CLI flag runs them anyway.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use epubsync_core::device::{Action, Device, RowUpdate};
use epubsync_core::kobo::{self, Kobo, eject};
use epubsync_core::library::Library;
use epubsync_core::sync as core_sync;
use iced::widget::{button, column, container, progress_bar, responsive, row, space, text};
use iced::{Center, Element, Fill, Length, Task, padding};

use crate::handoff::Handoff;
use crate::theme::{BODY, MONO, SANS_MEDIUM};
use crate::{Message, Open, format, table, theme};

/// Where the pane is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No plan is in view. The pane asks for Plan.
    Idle,
    /// The planning task is out.
    Planning,
    /// The plan is in view and nothing runs.
    Planned,
    /// The actions run, one per task.
    Sending,
    /// The Kobo rows and the read back run.
    Reading,
    /// The sync is over.
    Done,
    /// The eject task is out.
    Ejecting,
    /// The volume is unmounted.
    Ejected,
}

/// What one action came to.
#[derive(Debug, Clone, PartialEq)]
enum State {
    /// The action has not run.
    Waiting,
    /// The action is on the task now.
    Running,
    Done,
    /// The action failed. The text is the whole error chain.
    Failed(String),
}

/// One row of the plan: the action, the book's title, and what the action
/// came to.
#[derive(Debug, Clone)]
struct Step {
    action: Action,
    /// The book's title. A delete has none: the book left the library.
    title: Option<String>,
    /// Whether the write gate holds the replacement back. A held row does
    /// not run.
    held: bool,
    state: State,
}

impl Step {
    /// The word in the row's last column. A row Cancel dropped reads
    /// "skipped".
    fn word(&self, phase: Phase, cancelled: bool) -> &str {
        match &self.state {
            State::Running => "running",
            State::Done => DONE[slot(&self.action)],
            State::Failed(_) => "failed",
            State::Waiting if self.held => "held back",
            State::Waiting if phase == Phase::Planned => "",
            State::Waiting if cancelled => "skipped",
            State::Waiting => "waiting",
        }
    }
}

/// What the Kobo rows and the read back came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// The macOS `._` files deleted from the book folder.
    dot_files: usize,
    /// The Kobo rows the update changed.
    rows: usize,
    /// The books the device gave progress for.
    books: usize,
    /// The books whose progress differs from the last sync's.
    changed: usize,
    /// The looked-up words the library did not hold.
    words: usize,
}

/// What an eject came to, as the pane says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ejected {
    /// The volume is unmounted and the device told to disconnect.
    Yes,
    /// The path is a folder, not a mounted volume.
    NotAVolume(PathBuf),
    /// The eject failed. The text is the whole error chain.
    Failed(String),
}

/// The device a plan is for.
struct Found {
    /// None while a task has it.
    kobo: Option<Kobo>,
    serial: String,
    root: PathBuf,
    db_version: Option<i64>,
    /// Why the device refuses row writes, when it does.
    gate: Option<String>,
}

/// A device and one row per action for it, as the planning task hands
/// them back.
pub struct Plan {
    found: Found,
    steps: Vec<Step>,
}

/// The sync pane's state: the phase, the device and its plan once the
/// planning found one, and what the last sync came to.
pub struct Sync {
    phase: Phase,
    found: Option<Found>,
    /// One row per action. Empty while no plan is in view.
    steps: Vec<Step>,
    /// The step on the task now.
    current: Option<usize>,
    /// When the phase in view began, and the time of the frame drawn
    /// last. The activity bar's block moves by the time between them.
    since: Instant,
    now: Instant,
    /// When the Sync button was pressed.
    started: Instant,
    /// How long the sync took, from the Sync button to the end of the
    /// read back.
    took: Duration,
    /// Whether Cancel cut the plan short.
    cancelled: bool,
    counts: Counts,
    /// What the eject came to, once one has run.
    ejected: Option<Ejected>,
    /// The planning, action, read back, or eject that failed first, as
    /// one sentence.
    error: Option<String>,
}

/// The one Kobo under the mount folders the CLI scans.
fn detect() -> Result<Kobo> {
    kobo::detect_one(&kobo::default_roots())
        .map_err(|e| anyhow!("{e}. Plug in one Kobo and try again."))
}

/// Opens the device database and reads the plan, one row per action. The
/// replacements the write gate holds back come last.
pub fn plan(library: &Library, mut kobo: Kobo) -> Result<Plan> {
    kobo.open_db(false)?;
    let gate = core_sync::gate(core_sync::plan(library, &kobo)?, &kobo);
    let (reason, actions, held) = match gate {
        core_sync::Gate::Open(actions) => (None, actions, Vec::new()),
        core_sync::Gate::Closed {
            kept,
            skipped,
            reason,
        } => (Some(reason), kept, skipped),
    };
    let step = |action: Action, held: bool| Step {
        title: match action {
            // A delete's book left the library, so there is no row to
            // read a title from.
            Action::Delete { .. } => None,
            _ => library.get(action.id()).ok().map(|b| b.metadata.title),
        },
        action,
        held,
        state: State::Waiting,
    };
    let mut steps: Vec<Step> = actions.into_iter().map(|a| step(a, false)).collect();
    steps.extend(held.into_iter().map(|a| step(a, true)));
    Ok(Plan {
        found: Found {
            serial: kobo.serial().to_string(),
            root: kobo.root.clone(),
            db_version: kobo.db_version(),
            kobo: Some(kobo),
            gate: reason,
        },
        steps,
    })
}

impl Sync {
    /// A pane with no plan in it.
    pub fn new() -> Sync {
        let now = Instant::now();
        Sync {
            phase: Phase::Idle,
            found: None,
            steps: Vec::new(),
            current: None,
            since: now,
            now,
            started: now,
            took: Duration::ZERO,
            cancelled: false,
            counts: Counts::default(),
            ejected: None,
            error: None,
        }
    }

    /// How many rows the plan holds, held back or not.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether no plan is in view and no task is out.
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Whether a task holds the library.
    pub fn running(&self) -> bool {
        matches!(
            self.phase,
            Phase::Planning | Phase::Sending | Phase::Reading
        )
    }

    /// Whether a task is out.
    fn busy(&self) -> bool {
        self.running() || self.phase == Phase::Ejecting
    }

    /// Whether the activity bar is in view. It needs a frame per draw to
    /// move.
    pub fn animating(&self) -> bool {
        matches!(
            self.phase,
            Phase::Planning | Phase::Reading | Phase::Ejecting
        )
    }

    /// Records the time of a frame, which moves the activity bar.
    pub fn tick(&mut self, at: Instant) {
        self.now = at;
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.since = Instant::now();
        self.now = self.since;
    }

    /// Drops what is in view and starts the task that looks for the Kobo
    /// and reads the plan. The task takes the library and hands it back
    /// in `Message::SyncPlanned`. Does nothing while a task is out or
    /// while the import holds the library.
    pub fn make_plan(&mut self, library: &mut Option<Library>) -> Task<Message> {
        if self.busy() {
            return Task::none();
        }
        let Some(lib) = library.take() else {
            return Task::none();
        };
        *self = Sync::new();
        self.enter(Phase::Planning);
        Task::perform(
            async move {
                let plan = detect()
                    .and_then(|kobo| plan(&lib, kobo))
                    .map_err(|e| format!("{e:#}"));
                Handoff::new((lib, plan))
            },
            Message::SyncPlanned,
        )
    }

    /// Takes the library back from the planning task and puts the plan in
    /// view, or the reason there is none.
    pub fn planned(&mut self, handed: Option<(Library, Result<Plan, String>)>) -> Option<Library> {
        let library = handed.map(|(library, plan)| {
            match plan {
                Ok(plan) => {
                    self.found = Some(plan.found);
                    self.steps = plan.steps;
                }
                Err(why) => self.error = Some(why),
            }
            library
        });
        self.enter(if self.found.is_some() {
            Phase::Planned
        } else {
            Phase::Idle
        });
        library
    }

    /// Starts the sync. The actions run one per task, and the row
    /// updates and the read back follow.
    pub fn run(&mut self, library: &mut Option<Library>) -> Task<Message> {
        if self.phase != Phase::Planned || library.is_none() {
            return Task::none();
        }
        self.enter(Phase::Sending);
        self.started = Instant::now();
        self.step(library)
    }

    /// Starts the next action, or, with none left, the task that
    /// writes the Kobo rows and reads progress back. Does nothing while a
    /// task is out.
    pub fn step(&mut self, library: &mut Option<Library>) -> Task<Message> {
        if self.phase != Phase::Sending || self.current.is_some() || library.is_none() {
            return Task::none();
        }
        let next = if self.cancelled { None } else { self.next() };
        let Some(found) = &mut self.found else {
            return Task::none();
        };
        let Some(mut kobo) = found.kobo.take() else {
            return Task::none();
        };
        let rows = found.gate.is_none();
        let mut lib = library.take().expect("the state holds the library");
        match next {
            Some(index) => {
                self.current = Some(index);
                self.steps[index].state = State::Running;
                let action = self.steps[index].action.clone();
                Task::perform(
                    async move {
                        let result = send(&mut lib, &mut kobo, &action);
                        (Handoff::new((lib, kobo)), result)
                    },
                    |(handoff, result)| Message::Synced(handoff, result),
                )
            }
            None => {
                self.enter(Phase::Reading);
                Task::perform(
                    async move {
                        let counts =
                            read_back(&mut lib, &mut kobo, rows).map_err(|e| format!("{e:#}"));
                        (Handoff::new((lib, kobo)), counts)
                    },
                    |(handoff, counts)| Message::SyncRead(handoff, counts),
                )
            }
        }
    }

    /// The row of the first action that has not run and is not held.
    fn next(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|s| !s.held && s.state == State::Waiting)
    }

    /// Takes the library and the device back from the task and records
    /// what the action came to. A failed action keeps the sync going and
    /// puts its error in the head.
    pub fn synced(
        &mut self,
        handed: Option<(Library, Kobo)>,
        result: Result<(), String>,
    ) -> Option<Library> {
        let library = self.take_back(handed);
        if let Some(index) = self.current.take() {
            self.steps[index].state = match result {
                Ok(()) => State::Done,
                Err(why) => {
                    let step = &self.steps[index];
                    self.error.get_or_insert(format!(
                        "{} {} failed: {why}",
                        VERB[slot(&step.action)],
                        step.action.id()
                    ));
                    State::Failed(why)
                }
            };
        }
        library
    }

    /// Takes the library and the device back and ends the sync.
    pub fn read(
        &mut self,
        handed: Option<(Library, Kobo)>,
        counts: Result<Counts, String>,
    ) -> Option<Library> {
        let library = self.take_back(handed);
        self.enter(Phase::Done);
        self.took = self.started.elapsed();
        match counts {
            Ok(counts) => self.counts = counts,
            Err(why) => {
                self.error
                    .get_or_insert(format!("Reading the device failed: {why}"));
            }
        }
        library
    }

    /// Keeps the device and gives the library back to the caller.
    fn take_back(&mut self, handed: Option<(Library, Kobo)>) -> Option<Library> {
        let (library, kobo) = handed?;
        if let Some(found) = &mut self.found {
            found.kobo = Some(kobo);
        }
        Some(library)
    }

    /// Drops the actions that have not started. The action on the task
    /// finishes, because one action is one call, and then the row updates
    /// and the read back run.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Starts the task that closes the device database, unmounts the
    /// volume, and tells the device the USB session is over. With no
    /// device in view the task looks for the Kobo first. Does nothing
    /// while a task is out.
    pub fn eject(&mut self) -> Task<Message> {
        if self.busy() {
            return Task::none();
        }
        let kobo = self.found.as_mut().and_then(|f| f.kobo.take());
        self.error = None;
        self.enter(Phase::Ejecting);
        Task::perform(async move { eject_one(kobo) }, Message::Ejected)
    }

    /// Records what the eject came to and drops the plan, because the
    /// device database is closed. A failed eject leaves the pane idle
    /// with the error, so Plan and Eject can run again.
    pub fn ejected(&mut self, ejected: Ejected) {
        self.found = None;
        self.steps.clear();
        match ejected {
            Ejected::Failed(why) => {
                self.enter(Phase::Idle);
                self.error = Some(format!("The eject failed: {why}"));
            }
            done => {
                self.enter(Phase::Ejected);
                self.ejected = Some(done);
            }
        }
    }

    /// The rows the sync ran or is to run.
    fn to_run(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| !s.held)
    }

    /// The rows that ran, whatever they came to. This is how far the
    /// sync is through the plan.
    fn ran(&self) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .filter(|s| matches!(s.state, State::Done | State::Failed(_)))
    }

    /// The rows that ran and did what they say.
    fn done(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|s| s.state == State::Done)
    }

    /// How many rows failed.
    fn failed(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.state, State::Failed(_)))
            .count()
    }
}

/// Runs one action and updates the book's `sent` row.
fn send(library: &mut Library, kobo: &mut Kobo, action: &Action) -> Result<(), String> {
    core_sync::apply(library, kobo, std::slice::from_ref(action), |_| {})
        .map_err(|e| format!("{e:#}"))
}

/// Deletes the macOS `._` files, writes the Kobo rows, reads progress and
/// words back, and closes the device database. The row updates run only
/// with the write gate open, as in the CLI.
fn read_back(library: &mut Library, kobo: &mut Kobo, write_rows: bool) -> Result<Counts> {
    let dot_files = kobo.remove_dot_underscore_files()?;
    let rows = if write_rows {
        core_sync::update_rows(library, kobo)?
            .iter()
            .filter(|(_, outcome)| *outcome == RowUpdate::Updated)
            .count()
    } else {
        0
    };
    let back = core_sync::read_back(library, kobo)?;
    kobo.finish()?;
    Ok(Counts {
        dot_files,
        rows,
        books: back.progress.len(),
        changed: back.changed,
        words: back.words.len(),
    })
}

/// Closes the database of the given device, or of the Kobo found under
/// the mount folders when there is none, and ejects its volume.
fn eject_one(kobo: Option<Kobo>) -> Ejected {
    let eject = || -> Result<Ejected> {
        let mut kobo = match kobo {
            Some(kobo) => kobo,
            None => detect()?,
        };
        kobo.finish()?;
        Ok(match kobo.eject()? {
            eject::Ejected::Yes => Ejected::Yes,
            eject::Ejected::NotAVolume => Ejected::NotAVolume(kobo.root.clone()),
        })
    };
    eject().unwrap_or_else(|e| Ejected::Failed(format!("{e:#}")))
}

/// The verb for each kind of action, as the Action column holds it, in
/// the order `slot` gives.
const VERB: [&str; 4] = ["send", "replace", "send again", "delete"];

/// The verb for each kind of action that ran, in the order `slot` gives.
const DONE: [&str; 4] = ["sent", "replaced", "sent again", "deleted"];

/// The position of an action's kind in `VERB`, in `DONE`, and in a tally.
fn slot(action: &Action) -> usize {
    match action {
        Action::Send { .. } => 0,
        Action::Replace { .. } => 1,
        Action::SendAgain { .. } => 2,
        Action::Delete { .. } => 3,
    }
}

/// A count per kind of action, joined as "3 to send · 2 to delete". Each
/// count is followed by `before` and the kind's entry in `words`. A kind
/// with no action is left out, and no action at all gives an empty
/// string.
fn tally<'a>(steps: impl Iterator<Item = &'a Step>, before: &str, words: [&str; 4]) -> String {
    let mut counts = [0usize; 4];
    for step in steps {
        counts[slot(&step.action)] += 1;
    }
    counts
        .iter()
        .zip(words)
        .filter(|(n, _)| **n > 0)
        .map(|(n, word)| format!("{n} {before}{word}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The first line of the head, by phase.
fn headline(sync: &Sync) -> String {
    match sync.phase {
        Phase::Idle => "Press Plan to look for the Kobo".to_string(),
        Phase::Planning => "Looking for the Kobo".to_string(),
        Phase::Planned => plan_line(sync),
        Phase::Sending => sending_line(sync),
        Phase::Reading => "Writing the Kobo rows and reading progress back".to_string(),
        Phase::Done => summary(sync),
        Phase::Ejecting => "Ejecting the Kobo".to_string(),
        Phase::Ejected => ejected_line(sync),
    }
}

/// The first line while the plan is in view: what the sync is to do.
fn plan_line(sync: &Sync) -> String {
    let tally = tally(sync.to_run(), "to ", VERB);
    if tally.is_empty() {
        "Nothing to send. Sync reads progress back.".to_string()
    } else {
        tally
    }
}

/// "3 of 12 · send Pride and Prejudice", with the verb of the action on
/// the task. "Cancelling" takes its place after Cancel.
fn sending_line(sync: &Sync) -> String {
    if sync.cancelled {
        return "Cancelling".to_string();
    }
    let total = sync.to_run().count();
    let n = sync.ran().count() + 1;
    // The update that ends one action starts the next, so a drawn frame
    // in this phase always has an action on the task.
    let Some(step) = sync.current.map(|i| &sync.steps[i]) else {
        return String::new();
    };
    let what = step
        .title
        .clone()
        .unwrap_or_else(|| format!("book {}", step.action.id()));
    format!("{n} of {total} · {} {what}", VERB[slot(&step.action)])
}

/// The first line when the sync is over: "Synced in 1 min 14 s", or
/// "Cancelled after 20 s" when Cancel cut the plan short.
fn summary(sync: &Sync) -> String {
    let took = format::elapsed(sync.took);
    if sync.cancelled {
        format!("Cancelled after {took}")
    } else {
        format!("Synced in {took}")
    }
}

/// The first line once the eject has run. A path that is a plain folder
/// has nothing to eject.
fn ejected_line(sync: &Sync) -> String {
    match &sync.ejected {
        Some(Ejected::NotAVolume(root)) => format!(
            "{} is a folder, not a mounted volume. Nothing to eject.",
            root.display()
        ),
        _ => "Ejected. Unplug the Kobo.".to_string(),
    }
}

/// The line under the first once a sync has run: what ran, what the Kobo
/// rows came to, and what the read back found. A part with no count is
/// left out.
fn counts_line(sync: &Sync) -> String {
    let counts = &sync.counts;
    let mut parts = Vec::new();
    let done = tally(sync.done(), "", DONE);
    if !done.is_empty() {
        parts.push(done);
    }
    if sync.failed() > 0 {
        parts.push(format!("{} failed", sync.failed()));
    }
    if counts.dot_files > 0 {
        parts.push(format!("{} macOS ._ files deleted", counts.dot_files));
    }
    if counts.rows > 0 {
        parts.push(format!("{} Kobo rows written", counts.rows));
    }
    if counts.books > 0 {
        parts.push(format!(
            "progress for {}, {} changed",
            crate::count(counts.books, "book"),
            counts.changed
        ));
    }
    if counts.words > 0 {
        parts.push(crate::count(counts.words, "new word"));
    }
    if parts.is_empty() {
        return "Nothing changed".to_string();
    }
    parts.join(" · ")
}

/// The time the activity bar's block takes from one end and back.
const SWEEP: Duration = Duration::from_millis(1600);

/// Where the activity bar's block sits after `elapsed`: 0 at the left
/// end, 1 at the right, and 0 again after one `SWEEP`.
fn slide(elapsed: Duration) -> f32 {
    let t = (elapsed.as_secs_f32() / SWEEP.as_secs_f32()).fract();
    if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
}

/// The widths of the pane's columns. The title takes the rest.
const ACTION: f32 = 112.0;
const ID: f32 = 56.0;
const STATE: f32 = 150.0;

/// The sync pane: the head with the device and the buttons, then the plan
/// as a table, one row per action.
pub fn view<'a>(open: &'a Open, sync: &'a Sync) -> Element<'a, Message> {
    let header = |name: &'static str, width: Length| {
        table::cell(
            theme::label(name).style(theme::text_color(|c| c.muted)),
            width,
        )
    };
    let headers = row![
        space().width(table::MARK),
        header("Action", Length::Fixed(ACTION)),
        header("ID", Length::Fixed(ID)),
        header("Title", Fill),
        header("State", Length::Fixed(STATE)),
    ]
    .height(theme::HEADER)
    .align_y(Center);
    let body = responsive(move |size| {
        table::rows(open.scroll, size, sync.steps.len(), None, |i| {
            step_row(sync, i)
        })
    });
    column![
        head(open, sync),
        theme::hline(),
        table::frame(headers.into(), body)
    ]
    .into()
}

/// One row: the verb, the book id, the title, and the state.
fn step_row(sync: &Sync, index: usize) -> Element<'_, Message> {
    let step = &sync.steps[index];
    let title: Element<'_, Message> = match &step.title {
        Some(title) => table::line(title).font(SANS_MEDIUM).into(),
        None => table::line("no longer in the library")
            .style(theme::text_color(|c| c.faint))
            .into(),
    };
    let state = table::line(step.word(sync.phase, sync.cancelled)).style(match step.state {
        State::Done => theme::text_color(|c| c.finished),
        State::Failed(_) => theme::text_color(|c| c.danger),
        State::Running => theme::text_color(|c| c.accent),
        State::Waiting => theme::text_color(|c| c.faint),
    });
    let cells = row![
        space().width(table::MARK),
        table::cell(
            table::line(VERB[slot(&step.action)]).style(theme::text_color(|c| c.ink_2)),
            Length::Fixed(ACTION)
        ),
        table::cell(
            table::line(step.action.id())
                .font(MONO)
                .size(12)
                .style(theme::text_color(|c| c.muted)),
            Length::Fixed(ID)
        ),
        table::cell(title, Fill),
        table::cell(state, Length::Fixed(STATE)),
    ]
    .height(Fill)
    .align_y(Center);
    column![
        container(cells).width(Fill).height(table::ROW),
        theme::hline()
    ]
    .into()
}

/// The head of the pane: the first line with the buttons, the device,
/// what the last sync came to, the bar while a task runs, the write
/// gate's reason, and the first error.
fn head<'a>(open: &'a Open, sync: &'a Sync) -> Element<'a, Message> {
    let top = row![
        text(headline(sync)).size(BODY).font(SANS_MEDIUM),
        space().width(Fill)
    ]
    .extend(buttons(open, sync))
    .spacing(8)
    .align_y(Center);
    let mut body = column![top].spacing(6);
    if let Some(found) = &sync.found {
        body = body.push(device_line(found));
    }
    if sync.phase == Phase::Done {
        body = body.push(muted(counts_line(sync)));
    }
    if sync.phase == Phase::Sending {
        let total = sync.to_run().count();
        let done = sync.ran().count();
        body = body.push(
            container(
                progress_bar(0.0..=total.max(1) as f32, done as f32)
                    .girth(4)
                    .style(theme::import_bar),
            )
            .padding(padding::top(2).right(6)),
        );
    } else if sync.animating() {
        let at = slide(sync.now.saturating_duration_since(sync.since));
        body = body.push(container(activity_bar(at)).padding(padding::top(2).right(6)));
    }
    if let Some(found) = &sync.found
        && let Some(reason) = &found.gate
    {
        body = body.push(
            text(reason)
                .size(BODY)
                .style(theme::text_color(|c| c.reading)),
        );
    }
    if let Some(error) = &sync.error {
        body = body.push(
            text(error)
                .size(BODY)
                .style(theme::text_color(|c| c.danger)),
        );
    }
    container(body)
        .width(Fill)
        .padding(padding::all(10).left(14).right(8))
        .style(theme::ground(|c| c.window))
        .into()
}

/// The device the plan is for: the serial, the volume, and the database
/// version when the volume has a database.
fn device_line(found: &Found) -> Element<'_, Message> {
    let line = row![
        muted(format!("Kobo {}", found.serial)),
        text(found.root.display().to_string())
            .font(MONO)
            .size(11)
            .style(theme::text_color(|c| c.muted)),
    ]
    .spacing(10)
    .align_y(Center);
    line.extend(
        found
            .db_version
            .map(|v| muted(format!("database version {v}")).into()),
    )
    .into()
}

fn muted<'a>(line: String) -> iced::widget::Text<'a> {
    text(line).size(BODY).style(theme::text_color(|c| c.muted))
}

/// A track with a block that slides from end to end and back, for work
/// with no count to show. `at` is the block's place, from 0 at the left
/// end to 1 at the right.
fn activity_bar<'a>(at: f32) -> Element<'a, Message> {
    // The block takes a quarter of the track. The two spaces share the
    // rest in the ratio `at` gives.
    let left = (at.clamp(0.0, 1.0) * 300.0).round() as u16;
    let block = container(space())
        .width(Length::FillPortion(100))
        .height(4)
        .style(theme::bar_part(|c| c.accent));
    container(row![
        space().width(Length::FillPortion(left + 1)),
        block,
        space().width(Length::FillPortion(301 - left)),
    ])
    .width(Fill)
    .height(4)
    .style(theme::bar_part(|c| c.line))
    .into()
}

/// The buttons at the right of the head. Plan and Eject are always there
/// and are off while a task is out. Sync follows while the plan is in
/// view, and Cancel while the actions run. Plan and Sync are also off
/// while the import holds the library.
fn buttons<'a>(open: &Open, sync: &Sync) -> Vec<Element<'a, Message>> {
    let library = open.library.is_some();
    let action = |label: &'static str, message: Option<Message>| -> Element<'a, Message> {
        button(text(label).size(13))
            .on_press_maybe(message)
            .padding([5, 10])
            .style(theme::action)
            .into()
    };
    let mut buttons = vec![
        action("Plan", (library && !sync.busy()).then_some(Message::Plan)),
        action("Eject", (!sync.busy()).then_some(Message::Eject)),
    ];
    match sync.phase {
        Phase::Planned => buttons.push(
            button(text("Sync").size(13))
                .on_press_maybe(library.then_some(Message::RunSync))
                .padding([5, 12])
                .style(theme::primary)
                .into(),
        ),
        Phase::Sending => buttons.push(action(
            "Cancel",
            (!sync.cancelled).then_some(Message::CancelSync),
        )),
        _ => {}
    }
    buttons
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(action: Action, state: State) -> Step {
        Step {
            title: Some("A Book".to_string()),
            action,
            held: false,
            state,
        }
    }

    fn held(action: Action) -> Step {
        Step {
            held: true,
            ..step(action, State::Waiting)
        }
    }

    fn send_action(id: i64) -> Action {
        Action::Send { id, revision: 1 }
    }

    /// A pane with the given rows in view for a device with no volume,
    /// for the pure parts.
    fn with_steps(steps: Vec<Step>) -> Sync {
        let mut sync = Sync::new();
        sync.phase = Phase::Planned;
        sync.found = Some(Found {
            kobo: None,
            serial: "N1".to_string(),
            root: PathBuf::from("/media/KOBOeReader"),
            db_version: Some(197),
            gate: None,
        });
        sync.steps = steps;
        sync
    }

    #[test]
    fn the_plan_line_counts_every_row_that_runs() {
        let sync = with_steps(vec![
            step(send_action(1), State::Waiting),
            step(send_action(2), State::Waiting),
            step(Action::Replace { id: 3, revision: 2 }, State::Waiting),
            step(Action::Delete { id: 4 }, State::Waiting),
            held(Action::Replace { id: 5, revision: 2 }),
        ]);
        assert_eq!(plan_line(&sync), "2 to send · 1 to replace · 1 to delete");
    }

    #[test]
    fn a_plan_with_nothing_to_run_still_reads_progress_back() {
        let sync = with_steps(vec![held(Action::Replace { id: 3, revision: 2 })]);
        assert_eq!(
            plan_line(&sync),
            "Nothing to send. Sync reads progress back."
        );
    }

    #[test]
    fn the_counts_line_keeps_a_failed_action_out_of_the_tally() {
        let sync = with_steps(vec![
            step(send_action(1), State::Done),
            step(send_action(2), State::Failed("disk full".into())),
        ]);
        assert_eq!(counts_line(&sync), "1 sent · 1 failed");
    }

    #[test]
    fn the_counts_line_leaves_out_what_has_no_count() {
        let mut sync = with_steps(vec![
            step(send_action(1), State::Done),
            step(Action::Delete { id: 4 }, State::Done),
        ]);
        sync.counts = Counts {
            dot_files: 0,
            rows: 2,
            books: 12,
            changed: 3,
            words: 1,
        };
        assert_eq!(
            counts_line(&sync),
            "1 sent · 1 deleted · 2 Kobo rows written · progress for 12 books, 3 changed · 1 new word"
        );
    }

    #[test]
    fn a_sync_that_did_nothing_says_so() {
        let sync = with_steps(vec![]);
        assert_eq!(counts_line(&sync), "Nothing changed");
    }

    #[test]
    fn a_plan_that_found_no_kobo_leaves_the_pane_idle_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::init(&dir.path().join("library")).unwrap();
        let mut sync = Sync::new();
        sync.phase = Phase::Planning;
        let back = sync.planned(Some((library, Err("No Kobo found.".to_string()))));
        assert!(back.is_some());
        assert!(sync.idle());
        assert_eq!(sync.error.as_deref(), Some("No Kobo found."));
    }

    #[test]
    fn an_eject_ends_the_plan() {
        let mut sync = with_steps(vec![step(send_action(1), State::Done)]);
        sync.phase = Phase::Ejecting;
        sync.ejected(Ejected::Yes);
        assert_eq!(sync.phase, Phase::Ejected);
        assert!(sync.found.is_none());
        assert!(sync.steps.is_empty());
        assert_eq!(ejected_line(&sync), "Ejected. Unplug the Kobo.");
    }

    #[test]
    fn an_eject_that_found_a_folder_does_not_say_ejected() {
        let mut sync = with_steps(vec![]);
        sync.ejected(Ejected::NotAVolume(PathBuf::from("/tmp/KOBOeReader")));
        assert_eq!(sync.phase, Phase::Ejected);
        assert_eq!(
            ejected_line(&sync),
            "/tmp/KOBOeReader is a folder, not a mounted volume. Nothing to eject."
        );
    }

    #[test]
    fn an_eject_that_failed_leaves_the_pane_idle_with_the_error() {
        let mut sync = with_steps(vec![]);
        sync.phase = Phase::Ejecting;
        sync.ejected(Ejected::Failed("udisks2 is not running".to_string()));
        assert!(sync.idle());
        assert!(!sync.busy());
        assert_eq!(
            sync.error.as_deref(),
            Some("The eject failed: udisks2 is not running")
        );
    }

    #[test]
    fn the_activity_bar_goes_end_to_end_and_back() {
        assert_eq!(slide(Duration::ZERO), 0.0);
        assert_eq!(slide(SWEEP / 4), 0.5);
        assert_eq!(slide(SWEEP / 2), 1.0);
        assert_eq!(slide(SWEEP * 3 / 4), 0.5);
        assert_eq!(slide(SWEEP), 0.0);
    }

    #[test]
    fn only_the_phases_with_no_count_animate() {
        let mut sync = Sync::new();
        for (phase, animates) in [
            (Phase::Idle, false),
            (Phase::Planning, true),
            (Phase::Planned, false),
            (Phase::Sending, false),
            (Phase::Reading, true),
            (Phase::Done, false),
            (Phase::Ejecting, true),
            (Phase::Ejected, false),
        ] {
            sync.phase = phase;
            assert_eq!(sync.animating(), animates, "{phase:?}");
        }
    }

    #[test]
    fn the_summary_says_when_cancel_cut_the_plan_short() {
        let mut sync = with_steps(vec![]);
        sync.took = Duration::from_secs(74);
        assert_eq!(summary(&sync), "Synced in 1 min 14 s");
        sync.cancelled = true;
        assert_eq!(summary(&sync), "Cancelled after 1 min 14 s");
    }

    #[test]
    fn a_row_word_follows_the_phase() {
        let waiting = step(send_action(1), State::Waiting);
        assert_eq!(waiting.word(Phase::Planned, false), "");
        assert_eq!(waiting.word(Phase::Sending, false), "waiting");
        assert_eq!(waiting.word(Phase::Done, true), "skipped");
        let held = held(Action::Replace { id: 3, revision: 2 });
        assert_eq!(held.word(Phase::Planned, false), "held back");
        assert_eq!(held.word(Phase::Done, false), "held back");
        let done = step(Action::SendAgain { id: 5, revision: 1 }, State::Done);
        assert_eq!(done.word(Phase::Done, false), "sent again");
        let failed = step(send_action(1), State::Failed("disk full".into()));
        assert_eq!(failed.word(Phase::Done, false), "failed");
    }

    #[test]
    fn the_next_row_is_the_first_one_that_has_not_run_and_is_not_held() {
        let sync = with_steps(vec![
            step(send_action(1), State::Done),
            held(Action::Replace { id: 2, revision: 2 }),
            step(send_action(3), State::Waiting),
        ]);
        assert_eq!(sync.next(), Some(2));
        let done = with_steps(vec![step(send_action(1), State::Done)]);
        assert_eq!(done.next(), None);
    }

    #[test]
    fn a_failed_action_keeps_the_sync_going_and_lands_in_the_head() {
        let mut sync = with_steps(vec![
            step(send_action(1), State::Running),
            step(send_action(2), State::Waiting),
        ]);
        sync.phase = Phase::Sending;
        sync.current = Some(0);
        assert!(
            sync.synced(None, Err("no space left".to_string()))
                .is_none()
        );
        assert_eq!(sync.steps[0].state, State::Failed("no space left".into()));
        assert_eq!(sync.error.as_deref(), Some("send 1 failed: no space left"));
        assert_eq!(sync.next(), Some(1));
    }
}

#[cfg(test)]
pub(crate) mod library_tests {
    use super::*;
    use epubsync_core::library::ImportOutcome;
    use epubsync_epub::fixtures;
    use std::path::Path;

    /// A folder that looks like a mounted Kobo, with no device database.
    pub(crate) fn fake_kobo(parent: &Path) -> PathBuf {
        let root = parent.join("KOBOeReader");
        std::fs::create_dir_all(root.join(".kobo")).unwrap();
        std::fs::write(root.join(".kobo/version"), "N1,3.0.35,4.38.23171\n").unwrap();
        root
    }

    /// A library with one book, and a Kobo folder beside it.
    fn library_and_kobo(dir: &Path) -> (Library, PathBuf, i64) {
        let mut library = Library::init(&dir.join("library")).unwrap();
        let epub = fixtures::write_epub(dir, "lhod.epub", fixtures::EPUB2_OPF);
        let ImportOutcome::Imported { id, .. } = library.import(&epub, false).unwrap() else {
            panic!("not imported");
        };
        (library, fake_kobo(dir), id)
    }

    #[test]
    fn the_plan_holds_a_send_and_a_delete() {
        let dir = tempfile::tempdir().unwrap();
        let (library, root, id) = library_and_kobo(dir.path());
        let kobo = Kobo::at(&root).unwrap();
        // A file for a book the library does not hold.
        std::fs::create_dir_all(kobo.folder()).unwrap();
        std::fs::write(kobo.folder().join("7.kepub.epub"), b"").unwrap();

        let plan = plan(&library, kobo).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].action, Action::Send { id, revision: 1 });
        assert!(!plan.steps[0].held);
        assert!(plan.steps[0].title.is_some());
        assert_eq!(plan.steps[1].action, Action::Delete { id: 7 });
        assert!(!plan.steps[1].held);
        assert_eq!(plan.steps[1].title, None);
        assert_eq!(plan.found.gate, None);
        assert_eq!(plan.found.serial, "N1");
    }

    #[test]
    fn a_send_copies_the_book_and_the_next_plan_holds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut library, root, id) = library_and_kobo(dir.path());
        let mut kobo = Kobo::at(&root).unwrap();

        let action = Action::Send { id, revision: 1 };
        send(&mut library, &mut kobo, &action).unwrap();
        assert!(kobo.book_path(id).exists());

        let plan = plan(&library, Kobo::at(&root).unwrap()).unwrap();
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn a_delete_takes_the_file_off_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let (mut library, root, _id) = library_and_kobo(dir.path());
        let mut kobo = Kobo::at(&root).unwrap();
        std::fs::create_dir_all(kobo.folder()).unwrap();
        let stray = kobo.folder().join("7.kepub.epub");
        std::fs::write(&stray, b"").unwrap();

        send(&mut library, &mut kobo, &Action::Delete { id: 7 }).unwrap();
        assert!(!stray.exists());
    }

    /// A volume with no `.kobo/KoboReader.sqlite` gives no progress and
    /// no words, and the row updates write nothing. The read back still
    /// deletes the macOS `._` files.
    #[test]
    fn the_read_back_deletes_the_dot_underscore_files() {
        let dir = tempfile::tempdir().unwrap();
        let (mut library, root, id) = library_and_kobo(dir.path());
        let mut kobo = Kobo::at(&root).unwrap();
        send(&mut library, &mut kobo, &Action::Send { id, revision: 1 }).unwrap();
        std::fs::write(kobo.folder().join("._1.kepub.epub"), b"").unwrap();

        let counts = read_back(&mut library, &mut kobo, true).unwrap();
        assert_eq!(
            counts,
            Counts {
                dot_files: 1,
                rows: 0,
                books: 0,
                changed: 0,
                words: 0,
            }
        );
        assert!(kobo.book_path(id).exists());
    }

    /// The eject of a plain folder finds no mount to undo, and says so
    /// with the folder's path.
    #[test]
    fn an_eject_of_a_folder_says_there_is_nothing_to_eject() {
        let dir = tempfile::tempdir().unwrap();
        let root = fake_kobo(dir.path());
        let ejected = eject_one(Some(Kobo::at(&root).unwrap()));
        assert_eq!(ejected, Ejected::NotAVolume(root));
    }
}
