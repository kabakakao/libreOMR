mod problems;

use std::{
    collections::VecDeque,
    io::{Cursor, Write},
    path::PathBuf,
    sync::mpsc,
};

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2, emath::GuiRounding};
use egui_phosphor::regular as icons;
use image::{DynamicImage, GrayImage, ImageFormat};
use libreomr::{
    diagnostics::{self, Issue, Object, Severity},
    document_import::{DocumentPage, open_document},
    i18n,
    image_import::decode_image,
    model::{
        Articulation, Barline, BarlineKind, Clef, Duration, Dynamic, DynamicKind, KeyChange,
        KeySignature, ProjectDocument, ProjectPage, RepeatMark, RepeatMarkKind, Score, Span,
        SpanKind, Staff, TimeChange, TimeSignature, Tuplet,
    },
    musicxml, recognition,
};

#[derive(Clone, Copy)]
enum Insert {
    Note(Duration),
    Rest(Duration),
    Staff,
    Barline,
    KeyChange,
    TimeChange,
    Dynamic,
    RepeatMark(RepeatMarkKind),
    Span(SpanKind),
}

const DURATION_SHORTCUTS: [(Duration, char); 7] = [
    (Duration::Whole, '1'),
    (Duration::Half, '2'),
    (Duration::Quarter, '4'),
    (Duration::Eighth, '8'),
    (Duration::Sixteenth, '6'),
    (Duration::ThirtySecond, '3'),
    (Duration::SixtyFourth, '9'),
];

fn duration_shortcut_label(duration: Duration) -> String {
    let (_, key) = DURATION_SHORTCUTS
        .iter()
        .find(|(value, _)| *value == duration)
        .unwrap();
    format!("{}  [{key}]", duration.label())
}

#[derive(Clone, Copy)]
enum EditShortcut {
    Duration(Duration),
    Dots(u8),
    Accidental(i8),
    Delete,
}

impl EditShortcut {
    fn from_char(character: char) -> Option<Self> {
        if let Some((duration, _)) = DURATION_SHORTCUTS.iter().find(|(_, key)| *key == character) {
            return Some(Self::Duration(*duration));
        }
        match character {
            '0' => Some(Self::Dots(0)),
            '.' => Some(Self::Dots(1)),
            ':' => Some(Self::Dots(2)),
            '#' => Some(Self::Accidental(1)),
            'b' => Some(Self::Accidental(-1)),
            '=' => Some(Self::Accidental(0)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragTarget {
    Body(Object),
    Anchor(usize, bool),
}

struct ObjectDrag {
    target: DragTarget,
    origin: Vec2,
    score: Score,
}

struct LoadedPage {
    image: GrayImage,
    score: Score,
    name: String,
}

struct Loaded {
    pages: Vec<LoadedPage>,
    path: Option<PathBuf>,
}

impl Loaded {
    fn single(image: GrayImage, score: Score, name: String) -> Self {
        Self {
            pages: vec![LoadedPage { image, score, name }],
            path: None,
        }
    }
}

#[derive(Clone, Copy)]
enum JobMode {
    ReplaceDocument,
    AppendPages,
    ReplacePage,
}

enum JobEvent {
    Status(&'static str),
    Pages(Vec<DocumentPage>),
    Image(GrayImage),
    Progress(recognition::RecognitionProgress),
    Finished(Box<Result<Loaded, String>>),
}

struct PendingJob {
    mode: JobMode,
    receiver: mpsc::Receiver<JobEvent>,
    progress: Option<recognition::RecognitionProgress>,
    image_size: Option<Vec2>,
    texture: Option<egui::TextureHandle>,
}

struct JobReporter {
    sender: mpsc::Sender<JobEvent>,
    context: egui::Context,
}

impl JobReporter {
    fn send(&self, event: JobEvent) {
        let _ = self.sender.send(event);
        self.context.request_repaint();
    }

    fn recognize(&self, image: &GrayImage) -> Score {
        self.send(JobEvent::Image(image.clone()));
        recognition::recognize_with_progress(image, |progress| {
            self.send(JobEvent::Progress(progress));
        })
    }

    fn straighten(&self, image: GrayImage) -> GrayImage {
        self.send(JobEvent::Image(image.clone()));
        self.send(JobEvent::Status("Check skew and straighten image"));
        recognition::straighten(image)
    }
}

#[derive(Default)]
enum ImportState {
    #[default]
    Ready,
    Queued,
    Running {
        stage: &'static str,
        progress: Option<recognition::RecognitionProgress>,
    },
    Failed(String),
}

impl ImportState {
    fn ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    fn label(&self) -> String {
        match self {
            Self::Ready => i18n::tr("Ready").into(),
            Self::Queued => i18n::tr("Queued").into(),
            Self::Running {
                progress: Some(progress),
                ..
            } => format!("{} %", (progress.fraction() * 100.0) as u32),
            Self::Running { .. } => i18n::tr("Analysis").into(),
            Self::Failed(_) => i18n::tr("Error").into(),
        }
    }
}

#[derive(Clone)]
enum ImportSource {
    File(PathBuf),
    Page(DocumentPage),
}

impl ImportSource {
    fn name(&self) -> String {
        match self {
            Self::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Page(page) => page.name(),
        }
    }
}

struct QueuedImport {
    page_id: u64,
    source: ImportSource,
}

struct RunningDiscovery {
    page_id: u64,
    receiver: mpsc::Receiver<Result<Vec<DocumentPage>, String>>,
}

struct RunningImport {
    page_id: u64,
    receiver: mpsc::Receiver<JobEvent>,
}

#[derive(Default)]
struct PageState {
    image: GrayImage,
    texture: Option<egui::TextureHandle>,
    score: Score,
    selected: Option<usize>,
    selected_rest: Option<usize>,
    selected_object: Option<Object>,
    object_drag: Option<ObjectDrag>,
    canvas_context: Option<(Vec2, Option<Object>)>,
    selected_staff: usize,
    preview: crate::preview::Preview,
    zoom: f32,
    fit_width: bool,
    reveal_selection: bool,
    reveal_x: Option<f32>,
    show_object_list: bool,
    staff_spacing: f32,
    undo: Vec<Score>,
    redo: Vec<Score>,
    name: String,
    id: u64,
    import: ImportState,
    source: Option<ImportSource>,
}

impl PageState {
    fn loaded(loaded: LoadedPage, id: u64) -> Self {
        let mut score = loaded.score;
        score.recompute_accidentals();
        Self {
            image: loaded.image,
            score,
            name: loaded.name,
            id,
            preview: crate::preview::Preview::fitted(),
            zoom: 1.0,
            fit_width: true,
            show_object_list: true,
            staff_spacing: 16.0,
            ..Default::default()
        }
    }
}

pub struct App {
    settings: crate::settings::Settings,
    show_settings: bool,
    problems: problems::Problems,
    logo_texture: Option<egui::TextureHandle>,
    logo_dark: bool,
    page: PageState,
    // The active slot is None: its state lives in `page` while being edited.
    pages: Vec<Option<PageState>>,
    active_page: usize,
    reveal_tab: bool,
    next_page_id: u64,
    show_preview: bool,
    only_current_staff: bool,
    pending: Option<PendingJob>,
    import_queue: VecDeque<QueuedImport>,
    running_import: Option<RunningImport>,
    discovery: Option<RunningDiscovery>,
    project_path: Option<PathBuf>,
    dirty: bool,
    status: String,
    status_error: bool,
}

impl App {
    fn page_busy(&self) -> bool {
        self.pending.is_some() || !self.page.import.ready()
    }

    fn document_ready(&self) -> bool {
        self.pending.is_none()
            && (0..self.pages.len()).all(|index| self.page_at(index).import.ready())
    }

    fn require_ready_document(&self) -> Result<(), String> {
        for index in 0..self.pages.len() {
            match &self.page_at(index).import {
                ImportState::Ready => {}
                ImportState::Failed(error) => {
                    return Err(i18n::format(
                        "Page {0} ({1}): {error}",
                        &[
                            ("0", format!("{}", index + 1)),
                            ("1", self.page_at(index).name.to_string()),
                            ("error", error.to_string()),
                        ],
                    ));
                }
                _ => {
                    return Err(i18n::tr("Please wait until all pages have been analyzed.").into());
                }
            }
        }
        if self.pending.is_some() {
            return Err(i18n::tr("Processing is still running.").into());
        }
        Ok(())
    }

    fn page_by_id_mut(&mut self, id: u64) -> Option<&mut PageState> {
        if self.page.id == id {
            return Some(&mut self.page);
        }
        self.pages.iter_mut().flatten().find(|page| page.id == id)
    }

    fn queue_images(&mut self, paths: Vec<PathBuf>, mode: JobMode) {
        if paths.is_empty() {
            return;
        }
        if matches!(mode, JobMode::ReplaceDocument) {
            self.problems.reset_document();
            self.import_queue.clear();
            self.running_import = None;
            self.discovery = None;
            self.pages.clear();
            self.project_path = None;
        }
        for path in paths {
            let source = ImportSource::File(path);
            let page = self.new_queued_page(source.clone());
            let id = page.id;
            if self.pages.is_empty() {
                self.page = page;
                self.active_page = 0;
                self.pages.push(None);
                self.reveal_tab = true;
            } else {
                self.pages.push(Some(page));
            }
            self.import_queue.push_back(QueuedImport {
                page_id: id,
                source,
            });
        }
        self.dirty = true;
        self.problems.invalidate();
    }

    fn new_queued_page(&mut self, source: ImportSource) -> PageState {
        let id = self.next_page_id;
        self.next_page_id += 1;
        PageState {
            name: source.name(),
            id,
            import: ImportState::Queued,
            source: Some(source),
            zoom: 1.0,
            fit_width: true,
            show_object_list: true,
            staff_spacing: 16.0,
            preview: crate::preview::Preview::fitted(),
            ..Default::default()
        }
    }

    fn expand_document(&mut self, id: u64, pages: Vec<DocumentPage>) {
        let Some(index) = (0..self.pages.len()).find(|&index| self.page_at(index).id == id) else {
            return;
        };
        let mut pages = pages.into_iter();
        let Some(first) = pages.next() else {
            return;
        };
        let page = self.page_by_id_mut(id).expect("source page");
        page.name = first.name();
        page.source = Some(ImportSource::Page(first.clone()));
        let position = self
            .import_queue
            .iter()
            .position(|queued| queued.page_id == id);
        if let Some(position) = position {
            self.import_queue[position].source = ImportSource::Page(first);
        }
        let mut queued = Vec::new();
        for (offset, source) in pages.enumerate() {
            let source = ImportSource::Page(source);
            let page = self.new_queued_page(source.clone());
            queued.push(QueuedImport {
                page_id: page.id,
                source,
            });
            self.pages.insert(index + offset + 1, Some(page));
        }
        // Keep document pages together in the original processing queue.
        if let Some(position) = position {
            for (offset, queued) in queued.into_iter().enumerate() {
                self.import_queue.insert(position + offset + 1, queued);
            }
        } else {
            for queued in queued.into_iter().rev() {
                self.import_queue.push_front(queued);
            }
        }
        self.active_page = self
            .pages
            .iter()
            .position(Option::is_none)
            .expect("active page slot");
        self.reveal_tab = true;
        self.dirty = true;
        self.problems.invalidate();
    }

    fn needs_discovery(source: &ImportSource) -> bool {
        match source {
            ImportSource::File(path) => path.extension().is_some_and(|extension| {
                ["pdf", "tif", "tiff"]
                    .iter()
                    .any(|format| extension.eq_ignore_ascii_case(format))
            }),
            ImportSource::Page(_) => false,
        }
    }

    fn start_discovery(&mut self, context: &egui::Context) {
        if self.discovery.is_some() || self.pending.is_some() {
            return;
        }
        let Some(queued) = self
            .import_queue
            .iter()
            .find(|queued| Self::needs_discovery(&queued.source))
        else {
            return;
        };
        let ImportSource::File(path) = &queued.source else {
            return;
        };
        let path = path.clone();
        let (sender, receiver) = mpsc::channel();
        self.discovery = Some(RunningDiscovery {
            page_id: queued.page_id,
            receiver,
        });
        let context = context.clone();
        std::thread::spawn(move || {
            let result = open_document(&path);
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    fn poll_discovery(&mut self) {
        let Some(discovery) = &self.discovery else {
            return;
        };
        let result = match discovery.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(i18n::tr("Page discovery ended unexpectedly.").into())
            }
        };
        let id = discovery.page_id;
        self.discovery = None;
        self.problems.invalidate();
        match result {
            Ok(pages) => self.expand_document(id, pages),
            Err(error) => {
                if let Some(page) = self.page_by_id_mut(id) {
                    page.import = ImportState::Failed(error);
                }
                self.import_queue.retain(|queued| queued.page_id != id);
            }
        }
    }

    fn start_next_import(&mut self, context: &egui::Context) {
        self.start_discovery(context);
        if self.running_import.is_some() || self.pending.is_some() {
            return;
        }
        if self
            .import_queue
            .front()
            .is_some_and(|queued| Self::needs_discovery(&queued.source))
        {
            return;
        }
        let Some(queued) = self.import_queue.pop_front() else {
            return;
        };
        let Some(page) = self.page_by_id_mut(queued.page_id) else {
            return;
        };
        page.import = ImportState::Running {
            stage: "Load score image",
            progress: None,
        };
        let (sender, receiver) = mpsc::channel();
        let reporter = JobReporter {
            sender,
            context: context.clone(),
        };
        self.running_import = Some(RunningImport {
            page_id: queued.page_id,
            receiver,
        });
        std::thread::spawn(move || {
            let result = Self::load_source(queued.source, &reporter);
            reporter.send(JobEvent::Finished(Box::new(result)));
        });
    }

    fn poll_imports(&mut self, context: &egui::Context) {
        self.poll_discovery();
        while let Some(job) = &self.running_import {
            let id = job.page_id;
            let event = match job.receiver.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => JobEvent::Finished(Box::new(Err(
                    i18n::tr("Processing ended unexpectedly.").into(),
                ))),
            };
            if let JobEvent::Pages(pages) = event {
                self.expand_document(id, pages);
                continue;
            }
            let finished = matches!(event, JobEvent::Finished(_));
            if let Some(page) = self.page_by_id_mut(id) {
                match event {
                    JobEvent::Pages(_) => unreachable!("page discovery handled above"),
                    JobEvent::Status(stage) => {
                        page.import = ImportState::Running {
                            stage,
                            progress: None,
                        }
                    }
                    JobEvent::Progress(progress) => {
                        page.import = ImportState::Running {
                            stage: progress.stage,
                            progress: Some(progress),
                        }
                    }
                    JobEvent::Image(image) => {
                        page.image = image;
                        page.texture = None;
                    }
                    JobEvent::Finished(result) => match *result {
                        Ok(mut loaded) => {
                            let source = page.source.take();
                            *page = PageState::loaded(loaded.pages.remove(0), id);
                            page.source = source;
                        }
                        Err(error) => page.import = ImportState::Failed(error),
                    },
                }
            }
            if finished {
                self.running_import = None;
                self.dirty = true;
                self.problems.invalidate();
                if self.page.id == id && self.pending.is_none() {
                    self.status_error = matches!(self.page.import, ImportState::Failed(_));
                    self.status = match &self.page.import {
                        ImportState::Failed(error) => format!("{}: {error}", self.page.name),
                        _ => i18n::format(
                            "{0} ready: {1} staves, {2} notes",
                            &[
                                ("0", self.page.name.to_string()),
                                ("1", format!("{}", self.page.score.staves.len())),
                                ("2", format!("{}", self.page.score.notes.len())),
                            ],
                        ),
                    };
                }
                break;
            }
        }
        self.start_next_import(context);
    }

    fn retry_import(&mut self) {
        self.problems.invalidate();
        if let (ImportState::Failed(_), Some(source)) = (&self.page.import, &self.page.source) {
            self.import_queue.push_back(QueuedImport {
                page_id: self.page.id,
                source: source.clone(),
            });
            self.page.import = ImportState::Queued;
        }
    }

    fn remove_failed_page(&mut self) {
        if !matches!(self.page.import, ImportState::Failed(_)) || self.pages.len() <= 1 {
            return;
        }
        self.pages.remove(self.active_page);
        self.active_page = self.active_page.min(self.pages.len() - 1);
        self.page = self.pages[self.active_page].take().expect("inactive page");
        self.reveal_tab = true;
        self.dirty = true;
        self.problems.invalidate();
    }

    fn page_at(&self, index: usize) -> &PageState {
        self.pages[index].as_ref().unwrap_or(&self.page)
    }

    fn inherited_time_signature(&self) -> Option<TimeSignature> {
        (0..self.active_page)
            .map(|index| self.page_at(index))
            .filter(|page| page.import.ready())
            .fold(None, |meter, page| {
                page.score.ending_time_signature().or(meter)
            })
    }

    fn switch_page(&mut self, index: usize) {
        if self.pending.is_some() || index == self.active_page || index >= self.pages.len() {
            return;
        }
        let next = self.pages[index].take().expect("inactive page");
        self.pages[self.active_page] = Some(std::mem::replace(&mut self.page, next));
        self.active_page = index;
        self.reveal_tab = true;
        self.page.canvas_context = None;
        self.page.object_drag = None;
    }

    fn move_page(&mut self, from: usize, to: usize) {
        if self.pending.is_some()
            || from == to
            || from >= self.pages.len()
            || to >= self.pages.len()
        {
            return;
        }
        let page = self.pages.remove(from);
        self.pages.insert(to, page);
        self.active_page = self
            .pages
            .iter()
            .position(Option::is_none)
            .expect("active page slot");
        self.reveal_tab = true;
        self.dirty = true;
        self.problems.invalidate();
    }

    fn apply_loaded(&mut self, loaded: Loaded, mode: JobMode) {
        self.problems.invalidate();
        self.reveal_tab = true;
        let mut pages: Vec<_> = loaded
            .pages
            .into_iter()
            .map(|loaded| {
                let id = self.next_page_id;
                self.next_page_id += 1;
                PageState::loaded(loaded, id)
            })
            .collect();
        match mode {
            JobMode::ReplaceDocument => {
                self.problems.reset_document();
                self.import_queue.clear();
                self.running_import = None;
                self.discovery = None;
                self.page = pages.remove(0);
                self.pages = std::iter::once(None)
                    .chain(pages.into_iter().map(Some))
                    .collect();
                self.active_page = 0;
                self.project_path = loaded.path;
                self.dirty = self.project_path.is_none();
            }
            JobMode::AppendPages => {
                let first = self.pages.len();
                self.pages.extend(pages.into_iter().map(Some));
                self.switch_page(first);
                self.dirty = true;
                self.problems.invalidate();
            }
            JobMode::ReplacePage => {
                self.page = pages.remove(0);
                self.dirty = true;
                self.problems.invalidate();
            }
        }
    }

    fn page_tabs(&mut self, context: &egui::Context) {
        let reveal = std::mem::take(&mut self.reveal_tab);
        let mut selected = None;
        let mut moved = None;
        let mut add = false;
        egui::TopBottomPanel::top("page_tabs")
            .frame(
                egui::Frame::new()
                    .fill(context.style().visuals.window_fill)
                    .inner_margin(6.0),
            )
            .show(context, |ui| {
                ui.add_enabled_ui(self.pending.is_none(), |ui| {
                    ui.horizontal(|ui| {
                        add = ui
                            .button("+")
                            .on_hover_text(i18n::tr("Add images, PDF or TIFF as additional pages"))
                            .clicked();
                        egui::ScrollArea::horizontal()
                            .id_salt("page_tab_scroll")
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    for index in 0..self.pages.len() {
                                        let page = self.page_at(index);
                                        let name = if page.name.chars().count() > 28 {
                                            format!(
                                                "{}…{}",
                                                page.name.chars().take(14).collect::<String>(),
                                                page.name
                                                    .chars()
                                                    .skip(page.name.chars().count() - 13)
                                                    .collect::<String>()
                                            )
                                        } else {
                                            page.name.clone()
                                        };
                                        let label = if page.import.ready() {
                                            format!("{} · {name}", index + 1)
                                        } else {
                                            format!(
                                                "{} · {name} · {}",
                                                index + 1,
                                                page.import.label()
                                            )
                                        };
                                        let label = egui::RichText::new(label).color(
                                            if matches!(page.import, ImportState::Failed(_)) {
                                                ui.visuals().error_fg_color
                                            } else {
                                                ui.visuals().text_color()
                                            },
                                        );
                                        let response = ui
                                            .push_id(page.id, |ui| {
                                                ui.add(
                                                    egui::Button::new(label)
                                                        .selected(index == self.active_page)
                                                        .sense(Sense::click_and_drag()),
                                                )
                                            })
                                            .inner
                                            .on_hover_text(i18n::format(
                                                "{0}\n{1}\nDrag to change page order",
                                                &[
                                                    ("0", page.name.to_string()),
                                                    (
                                                        "1",
                                                        (match &page.import {
                                                            ImportState::Running {
                                                                stage, ..
                                                            } => i18n::tr(stage),
                                                            ImportState::Failed(error) => error,
                                                            ImportState::Queued => {
                                                                i18n::tr("Waiting for analysis")
                                                            }
                                                            ImportState::Ready => i18n::tr("Ready"),
                                                        })
                                                        .to_string(),
                                                    ),
                                                ],
                                            ));
                                        if reveal && index == self.active_page {
                                            response.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if response.clicked() {
                                            selected = Some(index);
                                        }
                                        response.dnd_set_drag_payload(index);
                                        if response.dnd_hover_payload::<usize>().is_some() {
                                            ui.painter().rect_stroke(
                                                response.rect,
                                                3.0,
                                                Stroke::new(
                                                    2.0_f32,
                                                    ui.visuals().selection.stroke.color,
                                                ),
                                                egui::StrokeKind::Inside,
                                            );
                                        }
                                        if let Some(from) = response.dnd_release_payload::<usize>()
                                        {
                                            moved = Some((*from, index));
                                        }
                                    }
                                });
                            });
                    });
                });
            });
        if let Some(index) = selected {
            self.switch_page(index);
        }
        if let Some((from, to)) = moved {
            self.move_page(from, to);
        }
        if add {
            self.add_images(context);
        }
    }

    fn ordered_objects(&self) -> Vec<Object> {
        let mut objects: Vec<_> = self
            .page
            .score
            .notes
            .iter()
            .enumerate()
            .map(|(index, note)| (note.staff, note.x, Object::Note(index)))
            .chain(
                self.page
                    .score
                    .rests
                    .iter()
                    .enumerate()
                    .map(|(index, rest)| (rest.staff, rest.x, Object::Rest(index))),
            )
            .chain(
                self.page
                    .score
                    .barlines
                    .iter()
                    .enumerate()
                    .map(|(index, bar)| (bar.staff, bar.x, Object::Barline(index))),
            )
            .chain(
                self.page
                    .score
                    .spans
                    .iter()
                    .enumerate()
                    .map(|(index, span)| (span.staff, span.start[0], Object::Span(index))),
            )
            .chain(
                self.page
                    .score
                    .key_changes
                    .iter()
                    .enumerate()
                    .map(|(index, change)| (change.staff, change.x, Object::KeyChange(index))),
            )
            .chain(
                self.page
                    .score
                    .time_changes
                    .iter()
                    .enumerate()
                    .map(|(index, change)| (change.staff, change.x, Object::TimeChange(index))),
            )
            .chain(
                self.page
                    .score
                    .dynamics
                    .iter()
                    .enumerate()
                    .map(|(index, dynamic)| (dynamic.staff, dynamic.x, Object::Dynamic(index))),
            )
            .chain(
                self.page
                    .score
                    .repeat_marks
                    .iter()
                    .enumerate()
                    .map(|(index, mark)| (mark.staff, mark.x, Object::RepeatMark(index))),
            )
            .collect();
        let priority = |object| match object {
            Object::Barline(_) => 0,
            Object::KeyChange(_) | Object::TimeChange(_) => 1,
            _ => 2,
        };
        objects.sort_by(|first, second| {
            first
                .0
                .cmp(&second.0)
                .then(first.1.total_cmp(&second.1))
                .then(priority(first.2).cmp(&priority(second.2)))
        });
        objects.into_iter().map(|(_, _, object)| object).collect()
    }

    fn select_object(&mut self, object: Option<Object>) {
        self.page.reveal_x = None;
        self.page.selected = None;
        self.page.selected_rest = None;
        self.page.selected_object = object;
        if object.is_some() {
            self.page.show_object_list = false;
        }
        match object {
            Some(Object::Note(index)) => {
                self.page.selected = Some(index);
                self.page.selected_staff = self.page.score.notes[index].staff;
            }
            Some(Object::Rest(index)) => {
                self.page.selected_rest = Some(index);
                self.page.selected_staff = self.page.score.rests[index].staff;
            }
            Some(Object::Barline(index)) => {
                self.page.selected_staff = self.page.score.barlines[index].staff
            }
            Some(Object::Span(index)) => {
                self.page.selected_staff = self.page.score.spans[index].staff
            }
            Some(Object::KeyChange(index)) => {
                self.page.selected_staff = self.page.score.key_changes[index].staff
            }
            Some(Object::TimeChange(index)) => {
                self.page.selected_staff = self.page.score.time_changes[index].staff
            }
            Some(Object::Dynamic(index)) => {
                self.page.selected_staff = self.page.score.dynamics[index].staff
            }
            Some(Object::RepeatMark(index)) => {
                self.page.selected_staff = self.page.score.repeat_marks[index].staff
            }
            None => {}
        }
    }

    fn close_details(&mut self) {
        self.select_object(None);
        self.page.object_drag = None;
        self.page.canvas_context = None;
        self.page.show_object_list = true;
        self.page.reveal_selection = true;
    }

    fn delete_object(&mut self, object: Object) {
        let valid = match object {
            Object::Note(index) => index < self.page.score.notes.len(),
            Object::Rest(index) => index < self.page.score.rests.len(),
            Object::Barline(index) => index < self.page.score.barlines.len(),
            Object::Span(index) => index < self.page.score.spans.len(),
            Object::KeyChange(index) => index < self.page.score.key_changes.len(),
            Object::TimeChange(index) => index < self.page.score.time_changes.len(),
            Object::Dynamic(index) => index < self.page.score.dynamics.len(),
            Object::RepeatMark(index) => index < self.page.score.repeat_marks.len(),
        };
        if !valid || self.page_busy() {
            return;
        }
        self.snapshot();
        match object {
            Object::Note(index) => {
                self.page.score.notes.remove(index);
            }
            Object::Rest(index) => {
                self.page.score.rests.remove(index);
            }
            Object::Barline(index) => {
                self.page.score.barlines.remove(index);
            }
            Object::Span(index) => {
                self.page.score.spans.remove(index);
            }
            Object::KeyChange(index) => {
                self.page.score.key_changes.remove(index);
            }
            Object::TimeChange(index) => {
                self.page.score.time_changes.remove(index);
            }
            Object::Dynamic(index) => {
                self.page.score.dynamics.remove(index);
            }
            Object::RepeatMark(index) => {
                self.page.score.repeat_marks.remove(index);
            }
        }
        self.page.score.recompute_accidentals();
        self.close_details();
    }

    fn insert_at(&mut self, position: Vec2, insert: Insert) {
        if self.page_busy()
            || !position.is_finite()
            || position.x < 0.0
            || position.y < 0.0
            || position.x >= self.page.image.width() as f32
            || position.y >= self.page.image.height() as f32
        {
            return;
        }
        if matches!(insert, Insert::Staff) {
            if position.y + self.page.staff_spacing * 4.0 >= self.page.image.height() as f32 {
                return;
            }
            self.snapshot();
            self.page.score.staves.push(Staff {
                lines: std::array::from_fn(|index| {
                    position.y + index as f32 * self.page.staff_spacing
                }),
                left: 0.0,
                right: self.page.image.width().saturating_sub(1) as f32,
                header: Default::default(),
            });
            self.page.selected_staff = self.page.score.staves.len() - 1;
            self.select_object(None);
            self.page.show_object_list = false;
        } else {
            let Some((staff_index, staff)) =
                self.page
                    .score
                    .staves
                    .iter()
                    .enumerate()
                    .min_by(|(_, first), (_, second)| {
                        (first.lines[2] - position.y)
                            .abs()
                            .total_cmp(&(second.lines[2] - position.y).abs())
                    })
            else {
                return;
            };
            let gap = staff.spacing();
            let end_x = (position.x + gap * 4.0).min(staff.right);
            if matches!(
                insert,
                Insert::Barline
                    | Insert::KeyChange
                    | Insert::TimeChange
                    | Insert::Dynamic
                    | Insert::RepeatMark(_)
                    | Insert::Span(_)
            ) && (position.x < staff.left
                || position.x > staff.right
                || (matches!(
                    insert,
                    Insert::Span(_) | Insert::RepeatMark(RepeatMarkKind::Ending)
                ) && end_x <= position.x))
            {
                return;
            }
            self.snapshot();
            let object = match insert {
                Insert::Note(duration) => {
                    let index = self
                        .page
                        .score
                        .add_note(position.x, position.y)
                        .expect("staff exists");
                    let note = &mut self.page.score.notes[index];
                    note.duration = duration;
                    note.y = self.page.score.staves[note.staff].y_at_pitch(note.pitch);
                    Object::Note(index)
                }
                Insert::Rest(duration) => Object::Rest(
                    self.page
                        .score
                        .add_rest(position.x, position.y, duration)
                        .expect("staff exists"),
                ),
                Insert::Barline => {
                    self.page.score.barlines.push(Barline {
                        staff: staff_index,
                        x: position.x,
                        kind: BarlineKind::Single,
                        confirmed: true,
                    });
                    Object::Barline(self.page.score.barlines.len() - 1)
                }
                Insert::KeyChange => {
                    self.page.score.key_changes.push(KeyChange {
                        staff: staff_index,
                        x: position.x,
                        key_signature: KeySignature { fifths: 0 },
                        bounds: vec![],
                        confirmed: true,
                    });
                    Object::KeyChange(self.page.score.key_changes.len() - 1)
                }
                Insert::TimeChange => {
                    self.page.score.time_changes.push(TimeChange {
                        staff: staff_index,
                        x: position.x,
                        time_signature: TimeSignature::Common,
                        bounds: vec![],
                        confirmed: true,
                    });
                    Object::TimeChange(self.page.score.time_changes.len() - 1)
                }
                Insert::RepeatMark(kind) => {
                    self.page.score.repeat_marks.push(RepeatMark {
                        staff: staff_index,
                        x: position.x,
                        y: position.y,
                        kind,
                        end_x,
                        numbers: if kind == RepeatMarkKind::Ending {
                            "1".into()
                        } else {
                            String::new()
                        },
                        closed: true,
                        bounds: None,
                        confirmed: true,
                    });
                    Object::RepeatMark(self.page.score.repeat_marks.len() - 1)
                }
                Insert::Dynamic => {
                    self.page.score.dynamics.push(Dynamic {
                        staff: staff_index,
                        x: position.x,
                        y: position.y,
                        kind: DynamicKind::Mf,
                        bounds: None,
                        confirmed: true,
                    });
                    Object::Dynamic(self.page.score.dynamics.len() - 1)
                }
                Insert::Span(kind) => {
                    self.page.score.spans.push(Span {
                        staff: staff_index,
                        kind,
                        start: position.into(),
                        end: [end_x, position.y],
                        height: if matches!(kind, SpanKind::Crescendo | SpanKind::Decrescendo) {
                            gap * 0.5
                        } else {
                            -gap
                        },
                        confirmed: true,
                    });
                    Object::Span(self.page.score.spans.len() - 1)
                }
                Insert::Staff => unreachable!(),
            };
            self.page.score.recompute_accidentals();
            self.select_object(Some(object));
        }
        self.page.object_drag = None;
        self.page.canvas_context = None;
    }

    fn canvas_context_menu(&mut self, ui: &mut egui::Ui) {
        let Some((position, object)) = self.page.canvas_context else {
            ui.close();
            return;
        };
        if let Some(object) = object {
            if ui.button(i18n::tr("Properties")).clicked() {
                self.select_object(Some(object));
                self.page.canvas_context = None;
                ui.close();
            }
            if ui.button(i18n::tr("Delete")).clicked() {
                self.delete_object(object);
                ui.close();
            }
        } else {
            ui.strong(i18n::tr("Insert object"));
            ui.separator();
            if !self.page.score.staves.is_empty() {
                if ui.button(i18n::tr("Note")).clicked() {
                    self.insert_at(position, Insert::Note(Duration::Quarter));
                    ui.close();
                }
                if ui.button(i18n::tr("Rest")).clicked() {
                    self.insert_at(position, Insert::Rest(Duration::Quarter));
                    ui.close();
                }
                if ui.button(i18n::tr("Barline")).clicked() {
                    self.insert_at(position, Insert::Barline);
                    ui.close();
                }
                if ui.button(i18n::tr("Key change")).clicked() {
                    self.insert_at(position, Insert::KeyChange);
                    ui.close();
                }
                for kind in RepeatMarkKind::ALL {
                    if ui.button(kind.label()).clicked() {
                        self.insert_at(position, Insert::RepeatMark(kind));
                        ui.close();
                    }
                }
                if ui.button(i18n::tr("Dynamics")).clicked() {
                    self.insert_at(position, Insert::Dynamic);
                    ui.close();
                }
                if ui.button(i18n::tr("Time change")).clicked() {
                    self.insert_at(position, Insert::TimeChange);
                    ui.close();
                }
                ui.separator();
                for kind in SpanKind::ALL {
                    if ui.button(kind.label()).clicked() {
                        self.insert_at(position, Insert::Span(kind));
                        ui.close();
                    }
                }
            } else {
                ui.weak(i18n::tr("No staff"));
            }
            ui.separator();
            ui.add(
                egui::DragValue::new(&mut self.page.staff_spacing)
                    .range(6.0..=80.0)
                    .prefix(i18n::tr("Line spacing: "))
                    .suffix(" px"),
            );
            let fits = position.y + self.page.staff_spacing * 4.0 < self.page.image.height() as f32;
            if ui
                .add_enabled(fits, egui::Button::new(i18n::tr("Staff")))
                .clicked()
            {
                self.insert_at(position, Insert::Staff);
                ui.close();
            }
        }
    }

    fn hit_object(&self, position: Vec2) -> Option<DragTarget> {
        if let Some(Object::Span(index)) = self.page.selected_object
            && let Some(span) = self.page.score.spans.get(index)
        {
            for (right, point) in [(false, span.start), (true, span.end)] {
                if (Vec2::from(point) - position).length() * self.page.zoom <= 10.0 {
                    return Some(DragTarget::Anchor(index, right));
                }
            }
        }
        let segment_distance = |start: [f32; 2], end: [f32; 2]| {
            let start = Vec2::from(start);
            let edge = Vec2::from(end) - start;
            let fraction =
                ((position - start).dot(edge) / edge.length_sq().max(0.001)).clamp(0.0, 1.0);
            (position - start - edge * fraction).length() * self.page.zoom
        };
        let mut hits = Vec::new();
        for (index, change) in self.page.score.key_changes.iter().enumerate() {
            let staff = &self.page.score.staves[change.staff];
            let distance = if change.bounds.is_empty() {
                Rect::from_min_max(
                    Pos2::new(change.x, staff.lines[0] - staff.spacing()),
                    Pos2::new(
                        change.x + staff.spacing() * 2.0,
                        staff.lines[4] + staff.spacing(),
                    ),
                )
                .distance_to_pos(position.to_pos2())
            } else {
                change
                    .bounds
                    .iter()
                    .map(|bounds| {
                        Rect::from_min_max(
                            Pos2::new(bounds.left, bounds.top),
                            Pos2::new(bounds.right, bounds.bottom),
                        )
                        .distance_to_pos(position.to_pos2())
                    })
                    .fold(f32::INFINITY, f32::min)
            } * self.page.zoom;
            if distance <= 10.0 {
                hits.push((distance, Object::KeyChange(index)));
            }
        }
        for (index, change) in self.page.score.time_changes.iter().enumerate() {
            let staff = &self.page.score.staves[change.staff];
            let distance = if change.bounds.is_empty() {
                Rect::from_min_max(
                    Pos2::new(change.x, staff.lines[0] - staff.spacing()),
                    Pos2::new(
                        change.x + staff.spacing() * 2.0,
                        staff.lines[4] + staff.spacing(),
                    ),
                )
                .distance_to_pos(position.to_pos2())
            } else {
                change
                    .bounds
                    .iter()
                    .map(|bounds| {
                        Rect::from_min_max(
                            Pos2::new(bounds.left, bounds.top),
                            Pos2::new(bounds.right, bounds.bottom),
                        )
                        .distance_to_pos(position.to_pos2())
                    })
                    .fold(f32::INFINITY, f32::min)
            } * self.page.zoom;
            if distance <= 10.0 {
                hits.push((distance, Object::TimeChange(index)));
            }
        }
        for (index, dynamic) in self.page.score.dynamics.iter().enumerate() {
            let bounds = dynamic.symbol_bounds(self.page.score.staves[dynamic.staff].spacing());
            let distance = Rect::from_min_max(
                Pos2::new(bounds.left, bounds.top),
                Pos2::new(bounds.right, bounds.bottom),
            )
            .distance_to_pos(position.to_pos2())
                * self.page.zoom;
            if distance <= 10.0 {
                hits.push((distance, Object::Dynamic(index)));
            }
        }
        for (index, mark) in self.page.score.repeat_marks.iter().enumerate() {
            let bounds = mark.symbol_bounds(self.page.score.staves[mark.staff].spacing());
            let distance = Rect::from_min_max(
                Pos2::new(bounds.left, bounds.top),
                Pos2::new(bounds.right, bounds.bottom),
            )
            .distance_to_pos(position.to_pos2())
                * self.page.zoom;
            if distance <= 10.0 {
                hits.push((distance, Object::RepeatMark(index)));
            }
        }
        if let Some(index) = self.nearest_note(position) {
            let note = &self.page.score.notes[index];
            hits.push((
                (Vec2::new(note.x, note.y) - position).length() * self.page.zoom,
                Object::Note(index),
            ));
        }
        if let Some(index) = self.nearest_rest(position) {
            let rest = &self.page.score.rests[index];
            let distance = rest.bounds.as_ref().map_or_else(
                || (Vec2::new(rest.x, rest.y) - position).length(),
                |bounds| {
                    Rect::from_min_max(
                        Pos2::new(bounds.left, bounds.top),
                        Pos2::new(bounds.right, bounds.bottom),
                    )
                    .distance_to_pos(position.to_pos2())
                },
            );
            hits.push((distance * self.page.zoom, Object::Rest(index)));
        }
        for (index, bar) in self.page.score.barlines.iter().enumerate() {
            let staff = &self.page.score.staves[bar.staff];
            let distance = segment_distance([bar.x, staff.lines[0]], [bar.x, staff.lines[4]]);
            if distance <= 12.0 {
                hits.push((distance, Object::Barline(index)));
            }
        }
        for (index, span) in self.page.score.spans.iter().enumerate() {
            let distance = if matches!(span.kind, SpanKind::Slur | SpanKind::Tie) {
                (0..40)
                    .map(|step| {
                        segment_distance(
                            span.curve_point(step as f32 / 40.0),
                            span.curve_point((step + 1) as f32 / 40.0),
                        )
                    })
                    .fold(f32::INFINITY, f32::min)
            } else {
                [-1.0, 1.0]
                    .into_iter()
                    .map(|side| {
                        let mut start = span.start;
                        let mut end = span.end;
                        if span.kind == SpanKind::Crescendo {
                            end[1] += side * span.height;
                        } else {
                            start[1] += side * span.height;
                        }
                        segment_distance(start, end)
                    })
                    .fold(f32::INFINITY, f32::min)
            };
            if distance <= 10.0 {
                hits.push((distance, Object::Span(index)));
            }
        }
        hits.into_iter()
            .min_by(|first, second| first.0.total_cmp(&second.0))
            .map(|(_, object)| DragTarget::Body(object))
    }

    fn move_object(&mut self, position: Vec2) {
        let Some(drag) = &self.page.object_drag else {
            return;
        };
        self.problems.invalidate();
        let delta = position - drag.origin;
        match drag.target {
            DragTarget::Body(Object::KeyChange(index)) => {
                let original = &drag.score.key_changes[index];
                let staff = &self.page.score.staves[original.staff];
                let change = &mut self.page.score.key_changes[index];
                change.x = (original.x + delta.x).clamp(staff.left, staff.right);
                change.bounds = original
                    .bounds
                    .iter()
                    .cloned()
                    .map(|mut bounds| {
                        bounds.left += change.x - original.x;
                        bounds.right += change.x - original.x;
                        bounds
                    })
                    .collect();
                change.confirmed = true;
                self.page.score.recompute_accidentals();
            }
            DragTarget::Body(Object::TimeChange(index)) => {
                let original = &drag.score.time_changes[index];
                let staff = &self.page.score.staves[original.staff];
                let change = &mut self.page.score.time_changes[index];
                change.x = (original.x + delta.x).clamp(staff.left, staff.right);
                change.bounds = original
                    .bounds
                    .iter()
                    .cloned()
                    .map(|mut bounds| {
                        bounds.left += change.x - original.x;
                        bounds.right += change.x - original.x;
                        bounds
                    })
                    .collect();
                change.confirmed = true;
            }
            DragTarget::Body(Object::Dynamic(index)) => {
                let original = &drag.score.dynamics[index];
                let staff = &self.page.score.staves[original.staff];
                let dynamic = &mut self.page.score.dynamics[index];
                dynamic.x = (original.x + delta.x).clamp(staff.left, staff.right);
                dynamic.y = (original.y + delta.y)
                    .clamp(0.0, self.page.image.height().saturating_sub(1) as f32);
                dynamic.bounds = original.bounds.clone().map(|mut bounds| {
                    bounds.left += dynamic.x - original.x;
                    bounds.right += dynamic.x - original.x;
                    bounds.top += dynamic.y - original.y;
                    bounds.bottom += dynamic.y - original.y;
                    bounds
                });
                dynamic.confirmed = true;
            }
            DragTarget::Body(Object::RepeatMark(index)) => {
                let original = &drag.score.repeat_marks[index];
                let staff = &self.page.score.staves[original.staff];
                let dx = delta.x.clamp(
                    staff.left - original.x,
                    staff.right
                        - if original.kind == RepeatMarkKind::Ending {
                            original.end_x
                        } else {
                            original.x
                        },
                );
                let mark = &mut self.page.score.repeat_marks[index];
                mark.x = original.x + dx;
                mark.end_x = original.end_x + dx;
                mark.y = (original.y + delta.y)
                    .clamp(0.0, self.page.image.height().saturating_sub(1) as f32);
                mark.bounds = None;
                mark.confirmed = true;
            }
            DragTarget::Body(Object::Note(index)) => {
                let original = &drag.score.notes[index];
                let staff = &self.page.score.staves[original.staff];
                let note = &mut self.page.score.notes[index];
                note.x = (original.x + delta.x).clamp(staff.left, staff.right);
                note.pitch = staff.pitch_at(original.y + delta.y).clamp(0, 69);
                note.y = staff.y_at_pitch(note.pitch);
                note.confirmed = true;
                self.page.score.recompute_accidentals();
            }
            DragTarget::Body(Object::Rest(index)) => {
                let original = &drag.score.rests[index];
                let staff = &self.page.score.staves[original.staff];
                let rest = &mut self.page.score.rests[index];
                rest.x = (original.x + delta.x).clamp(staff.left, staff.right);
                rest.y = (original.y + delta.y).clamp(0.0, self.page.image.height() as f32 - 1.0);
                if let Some(bounds) = &original.bounds {
                    let mut bounds = bounds.clone();
                    bounds.left += rest.x - original.x;
                    bounds.right += rest.x - original.x;
                    bounds.top += rest.y - original.y;
                    bounds.bottom += rest.y - original.y;
                    rest.bounds = Some(bounds);
                }
                rest.confirmed = true;
            }
            DragTarget::Body(Object::Barline(index)) => {
                let original = &drag.score.barlines[index];
                let staff = &self.page.score.staves[original.staff];
                self.page.score.barlines[index].x =
                    (original.x + delta.x).clamp(staff.left, staff.right);
                self.page.score.barlines[index].confirmed = true;
                self.page.score.recompute_accidentals();
            }
            DragTarget::Body(Object::Span(index)) => {
                let original = &drag.score.spans[index];
                let staff = &self.page.score.staves[original.staff];
                let shift_x = delta.x.clamp(
                    staff.left - original.start[0],
                    staff.right - original.end[0],
                );
                let shift_y = delta.y.clamp(
                    -original.start[1].min(original.end[1]),
                    self.page.image.height() as f32 - 1.0 - original.start[1].max(original.end[1]),
                );
                let span = &mut self.page.score.spans[index];
                span.start = [original.start[0] + shift_x, original.start[1] + shift_y];
                span.end = [original.end[0] + shift_x, original.end[1] + shift_y];
                span.confirmed = true;
            }
            DragTarget::Anchor(index, right) => {
                let original = &drag.score.spans[index];
                let staff = &self.page.score.staves[original.staff];
                let span = &mut self.page.score.spans[index];
                let (point, min_x, max_x) = if right {
                    (&mut span.end, original.start[0] + 1.0, staff.right)
                } else {
                    (&mut span.start, staff.left, original.end[0] - 1.0)
                };
                let source = if right { original.end } else { original.start };
                *point = [
                    (source[0] + delta.x).clamp(min_x, max_x),
                    (source[1] + delta.y).clamp(0.0, self.page.image.height() as f32 - 1.0),
                ];
                span.confirmed = true;
            }
        }
    }

    fn nearest_rest(&self, position: Vec2) -> Option<usize> {
        self.page
            .score
            .rests
            .iter()
            .enumerate()
            .filter_map(|(index, rest)| {
                let distance = (Vec2::new(rest.x, rest.y) - position).length() * self.page.zoom;
                let within = rest
                    .bounds
                    .as_ref()
                    .is_some_and(|bounds| bounds.contains(position.x, position.y));
                (within || distance < 18.0).then_some((index, distance))
            })
            .min_by(|first, second| first.1.total_cmp(&second.1))
            .map(|(index, _)| index)
    }

    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        Self::configure_style(&context.egui_ctx);
        let settings = crate::settings::Settings::load();
        settings.apply(&context.egui_ctx);
        let mut app = Self {
            settings,
            show_settings: false,
            problems: problems::Problems::default(),
            logo_texture: None,
            logo_dark: false,
            page: PageState::loaded(
                LoadedPage {
                    image: recognition::demo_image(),
                    score: Score::default(),
                    name: i18n::tr("Demonstration").into(),
                },
                0,
            ),
            pages: vec![None],
            active_page: 0,
            reveal_tab: true,
            next_page_id: 1,
            show_preview: true,
            only_current_staff: false,
            pending: None,
            import_queue: VecDeque::new(),
            running_import: None,
            discovery: None,
            project_path: None,
            dirty: false,
            status: String::new(),
            status_error: false,
        };
        app.set_texture(&context.egui_ctx);
        if cfg!(feature = "ui-screenshot") && std::env::var_os("LIBREOMR_SCREENSHOT_TO").is_some() {
            let language =
                std::env::var("LIBREOMR_SCREENSHOT_LANGUAGE").unwrap_or_else(|_| "en".into());
            i18n::set_language(i18n::Language::from_locale(&language));
            app.settings.language = if i18n::language() == i18n::Language::German {
                crate::settings::LanguagePreference::German
            } else {
                crate::settings::LanguagePreference::English
            };
            app.show_settings = std::env::var_os("LIBREOMR_SETTINGS_SCREENSHOT").is_some();
            #[cfg(feature = "ui-screenshot")]
            if let Some(path) = std::env::var_os("LIBREOMR_SCREENSHOT_IMAGE") {
                let path = PathBuf::from(path);
                let page = open_document(&path)
                    .expect("screenshot input document")
                    .into_iter()
                    .next()
                    .expect("screenshot input page");
                app.page.name = page.name();
                app.page.image =
                    recognition::straighten(page.render().expect("screenshot page rendering"));
                app.set_texture(&context.egui_ctx);
            }
            match std::env::var("LIBREOMR_SCREENSHOT_THEME").as_deref() {
                Ok("dark") => {
                    app.settings.theme = crate::settings::Theme::Dark;
                    context.egui_ctx.set_theme(egui::ThemePreference::Dark);
                }
                Ok("light") => {
                    app.settings.theme = crate::settings::Theme::Light;
                    context.egui_ctx.set_theme(egui::ThemePreference::Light);
                }
                _ => {}
            }
            app.page.score = recognition::recognize(&app.page.image);
            if std::env::var_os("LIBREOMR_PREVIEW_ACCIDENTALS_SCREENSHOT").is_some() {
                if let Some(staff) = app.page.score.staves.first_mut() {
                    staff.header.clef = Clef::Bass;
                    staff.header.key_signature = Some(KeySignature { fifths: -2 });
                    staff.header.time_signature = Some(TimeSignature::Meter {
                        beats: 4,
                        beat_unit: 4,
                    });
                }
                for (index, note) in app.page.score.notes.iter_mut().enumerate() {
                    if note.staff == 0 {
                        note.pitch -= 14;
                    }
                    note.accidental = [-1, 0, 1][index % 3];
                    note.accidental_explicit = true;
                }
            }
            if std::env::var_os("LIBREOMR_REVIEW_SCREENSHOT").is_some()
                && app.page.score.notes.len() > 1
            {
                app.page.score.notes[0].confirmed = true;
                app.page.score.notes[1].confirmed = true;
                app.select_object(Some(Object::Note(1)));
                app.page.show_object_list =
                    std::env::var("LIBREOMR_REVIEW_SCREENSHOT").as_deref() != Ok("details");
            }
            if std::env::var_os("LIBREOMR_PROBLEMS_SCREENSHOT").is_some() {
                app.problems.show = true;
            }
            app.status = i18n::tr("Demonstration").into();
            return app;
        }
        let image = app.page.image.clone();
        app.start_job(
            &context.egui_ctx,
            JobMode::ReplaceDocument,
            move |reporter| {
                let score = reporter.recognize(&image);
                Ok(Loaded::single(
                    image,
                    score,
                    i18n::tr("Demonstration").into(),
                ))
            },
        );
        app
    }

    fn configure_style(context: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        context.set_fonts(fonts);
        crate::theme::configure(context);
    }

    fn set_texture(&mut self, context: &egui::Context) {
        self.page.texture = (self.page.image.width() > 0 && self.page.image.height() > 0)
            .then(|| Self::image_texture(context, &self.page.image));
    }

    fn image_texture(context: &egui::Context, image: &GrayImage) -> egui::TextureHandle {
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_gray(size, image.as_raw());
        context.load_texture("score", pixels, egui::TextureOptions::LINEAR)
    }

    fn start_job(
        &mut self,
        context: &egui::Context,
        mode: JobMode,
        job: impl FnOnce(&JobReporter) -> Result<Loaded, String> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        let reporter = JobReporter {
            sender,
            context: context.clone(),
        };
        self.problems.invalidate();
        self.pending = Some(PendingJob {
            mode,
            receiver,
            progress: None,
            image_size: None,
            texture: None,
        });
        self.page.canvas_context = None;
        self.page.object_drag = None;
        self.status_error = false;
        self.status = i18n::tr("Processing ...").into();
        std::thread::spawn(move || {
            let result = job(&reporter);
            reporter.send(JobEvent::Finished(Box::new(result)));
        });
    }

    fn poll_job(&mut self, context: &egui::Context) {
        let result = loop {
            let Some(pending) = &mut self.pending else {
                return;
            };
            match pending.receiver.try_recv() {
                Ok(JobEvent::Pages(_)) => {}
                Ok(JobEvent::Status(status)) => {
                    self.status = i18n::tr(status).into();
                    pending.progress = None;
                }
                Ok(JobEvent::Progress(progress)) => {
                    self.status = i18n::tr(progress.stage).into();
                    pending.progress = Some(progress);
                }
                Ok(JobEvent::Image(image)) => {
                    pending.image_size =
                        Some(Vec2::new(image.width() as f32, image.height() as f32));
                    pending.texture = Some(Self::image_texture(context, &image));
                }
                Ok(JobEvent::Finished(result)) => break *result,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    break Err(i18n::tr("Processing ended unexpectedly.").into());
                }
            }
        };
        let mode = self.pending.take().expect("finished job").mode;
        match result {
            Ok(loaded) => {
                self.apply_loaded(loaded, mode);
                self.set_texture(context);
                self.status = i18n::format(
                    "{0} staves, {1} notes, {2} rests, {3} barlines",
                    &[
                        ("0", format!("{}", self.page.score.staves.len())),
                        ("1", format!("{}", self.page.score.notes.len())),
                        ("2", format!("{}", self.page.score.rests.len())),
                        ("3", format!("{}", self.page.score.barlines.len())),
                    ],
                );
            }
            Err(error) => {
                self.set_texture(context);
                self.status = error;
                self.status_error = true;
            }
        }
    }

    fn confirm_discard(&self) -> bool {
        !self.dirty
            || rfd::MessageDialog::new()
                .set_title(i18n::tr("Unsaved changes"))
                .set_description(i18n::tr("Discard changes?"))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    fn snapshot(&mut self) {
        if self.page.undo.len() >= 100 {
            self.page.undo.remove(0);
        }
        self.page.undo.push(self.page.score.clone());
        self.page.redo.clear();
        self.dirty = true;
        self.problems.invalidate();
    }

    fn undo(&mut self) {
        self.page.canvas_context = None;
        if let Some(previous) = self.page.undo.pop() {
            self.page
                .redo
                .push(std::mem::replace(&mut self.page.score, previous));
            self.page.selected = None;
            self.page.selected_rest = None;
            self.page.selected_object = None;
            self.page.object_drag = None;
            self.dirty = true;
            self.problems.invalidate();
        }
    }

    fn redo(&mut self) {
        self.page.canvas_context = None;
        if let Some(next) = self.page.redo.pop() {
            self.page
                .undo
                .push(std::mem::replace(&mut self.page.score, next));
            self.page.selected = None;
            self.page.selected_rest = None;
            self.page.selected_object = None;
            self.page.object_drag = None;
            self.dirty = true;
            self.problems.invalidate();
        }
    }

    fn open_image(&mut self, context: &egui::Context) {
        if self.confirm_discard() {
            self.pick_images(context, JobMode::ReplaceDocument);
        }
    }

    fn add_images(&mut self, context: &egui::Context) {
        self.pick_images(context, JobMode::AppendPages);
    }

    fn pick_images(&mut self, context: &egui::Context, mode: JobMode) {
        let Some(paths) = rfd::FileDialog::new()
            .set_title(if matches!(mode, JobMode::AppendPages) {
                i18n::tr("Add pages")
            } else {
                i18n::tr("Open images and documents")
            })
            .add_filter(
                i18n::tr("Score images and documents"),
                &["pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp"],
            )
            .pick_files()
        else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        self.queue_images(paths, mode);
        self.start_next_import(context);
    }

    #[cfg(test)]
    fn load_image(path: PathBuf, reporter: &JobReporter) -> Result<Loaded, String> {
        Self::load_source(ImportSource::File(path), reporter)
    }

    fn load_source(source: ImportSource, reporter: &JobReporter) -> Result<Loaded, String> {
        let page = match source {
            ImportSource::File(path) => {
                reporter.send(JobEvent::Status(i18n::tr("Discover document pages")));
                let pages = open_document(&path)?;
                let first = pages[0].clone();
                reporter.send(JobEvent::Pages(pages));
                first
            }
            ImportSource::Page(page) => page,
        };
        reporter.send(JobEvent::Status(i18n::tr("Load and rasterize page")));
        let image = page.render()?;
        let image = reporter.straighten(image);
        let score = reporter.recognize(&image);
        Ok(Loaded::single(image, score, page.name()))
    }

    fn open_project(&mut self, context: &egui::Context) {
        if !self.confirm_discard() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter(i18n::tr("libreOMR project"), &["json"])
            .pick_file()
        else {
            return;
        };
        self.start_job(context, JobMode::ReplaceDocument, move |reporter| {
            reporter.send(JobEvent::Status(i18n::tr("Load project")));
            Self::read_project(&path)
        });
    }

    fn read_project(path: &std::path::Path) -> Result<Loaded, String> {
        if std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .len()
            > 256 * 1024 * 1024
        {
            return Err(i18n::tr("Project is larger than 256 MiB.").into());
        }
        let project =
            ProjectDocument::from_json(&std::fs::read(path).map_err(|error| error.to_string())?)?;
        let mut pages = Vec::with_capacity(project.pages.len());
        for (index, page) in project.pages.into_iter().enumerate() {
            let image = decode_image(
                image::ImageReader::new(Cursor::new(page.image_png))
                    .with_guessed_format()
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| {
                i18n::format(
                    "Page {0}: {error}",
                    &[
                        ("0", format!("{}", index + 1)),
                        ("error", error.to_string()),
                    ],
                )
            })?;
            pages.push(LoadedPage {
                image,
                score: page.score,
                name: page.image_name,
            });
        }
        Ok(Loaded {
            pages,
            path: Some(path.to_path_buf()),
        })
    }

    fn project_document(&self) -> Result<ProjectDocument, String> {
        self.require_ready_document()?;
        let mut pages = Vec::with_capacity(self.pages.len());
        for index in 0..self.pages.len() {
            let page = self.page_at(index);
            page.score.validate().map_err(|error| {
                i18n::format(
                    "Page {0}: {error}",
                    &[
                        ("0", format!("{}", index + 1)),
                        ("error", error.to_string()),
                    ],
                )
            })?;
            let mut png = Cursor::new(Vec::new());
            DynamicImage::ImageLuma8(page.image.clone())
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|error| error.to_string())?;
            pages.push(ProjectPage {
                image_name: page.name.clone(),
                image_png: png.into_inner(),
                score: page.score.clone(),
            });
        }
        Ok(ProjectDocument { version: 2, pages })
    }

    fn save_project(&mut self) {
        if let Err(error) = self.require_ready_document() {
            self.status = error;
            self.status_error = true;
            return;
        }
        let path = self.project_path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter(i18n::tr("libreOMR project"), &["json"])
                .set_file_name("score.libreomr.json")
                .save_file()
        });
        let Some(path) = path else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let project = self.project_document()?;
            let bytes = serde_json::to_vec(&project).map_err(|error| error.to_string())?;
            if bytes.len() > 256 * 1024 * 1024 {
                return Err(i18n::tr("Project is larger than 256 MiB.").into());
            }
            let directory = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let mut temporary =
                tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
            temporary
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|error| error.to_string())?;
            temporary
                .persist(&path)
                .map_err(|error| error.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.project_path = Some(path);
                self.dirty = false;
                self.status = i18n::tr("Project saved.").into();
                self.status_error = false;
            }
            Err(error) => {
                self.status =
                    i18n::format("Saving failed: {error}", &[("error", error.to_string())]);
                self.status_error = true;
            }
        }
    }

    fn write_musicxml_to(&self, path: &std::path::Path) -> Result<Vec<String>, String> {
        self.require_ready_document()?;
        let name = &self.page_at(0).name;
        let title = std::path::Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name);
        let scores: Vec<_> = (0..self.pages.len())
            .map(|index| &self.page_at(index).score)
            .collect();
        let export = musicxml::export_pages(&scores, title)?;
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        let mut temporary =
            tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
        temporary
            .write_all(export.xml.as_bytes())
            .map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        temporary.persist(path).map_err(|error| error.to_string())?;
        Ok(export.warnings)
    }

    fn export_musicxml(&mut self) {
        let filename = std::path::Path::new(&self.page_at(0).name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("score");
        let Some(path) = rfd::FileDialog::new()
            .add_filter("MusicXML", &["musicxml", "xml"])
            .set_file_name(format!("{filename}.musicxml"))
            .save_file()
        else {
            return;
        };
        match self.write_musicxml_to(&path) {
            Ok(warnings) => {
                self.status_error = false;
                self.status = if warnings.is_empty() {
                    i18n::format(
                        "MusicXML exported: {0}",
                        &[("0", format!("{}", path.display()))],
                    )
                } else {
                    i18n::format(
                        "MusicXML exported ({0} notices): {1}",
                        &[
                            ("0", format!("{}", warnings.len())),
                            ("1", format!("{}", path.display())),
                        ],
                    )
                };
                if !warnings.is_empty() {
                    self.show_export_problems();
                }
            }
            Err(error) => {
                self.show_export_problems();
                self.status = i18n::format(
                    "MusicXML export failed: {error}",
                    &[("error", error.to_string())],
                );
                self.status_error = true;
            }
        }
    }

    fn is_confirmed(&self, object: Object) -> bool {
        match object {
            Object::Note(index) => self.page.score.notes[index].confirmed,
            Object::Rest(index) => self.page.score.rests[index].confirmed,
            Object::Span(index) => self.page.score.spans[index].confirmed,
            Object::KeyChange(index) => self.page.score.key_changes[index].confirmed,
            Object::TimeChange(index) => self.page.score.time_changes[index].confirmed,
            Object::Dynamic(index) => self.page.score.dynamics[index].confirmed,
            Object::RepeatMark(index) => self.page.score.repeat_marks[index].confirmed,
            Object::Barline(index) => self.page.score.barlines[index].confirmed,
        }
    }

    fn review_progress(&self) -> (usize, usize) {
        let objects = self.ordered_objects();
        let reviewable: Vec<_> = objects.into_iter().collect();
        (
            reviewable
                .iter()
                .filter(|object| self.is_confirmed(**object))
                .count(),
            reviewable.len(),
        )
    }

    fn next_unchecked(&mut self) {
        let objects = self.ordered_objects();
        let start = self
            .page
            .selected_object
            .and_then(|selected| objects.iter().position(|object| *object == selected))
            .map_or(0, |index| index + 1);
        if let Some(object) = objects
            .iter()
            .skip(start)
            .chain(objects.iter().take(start))
            .find(|object| !self.is_confirmed(**object))
            .copied()
        {
            self.select_object(Some(object));
            self.page.reveal_selection = true;
        }
    }

    fn shortcuts(&mut self, context: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let shortcut = |key, modifiers| {
            context.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
            })
        };
        if shortcut(egui::Key::S, egui::Modifiers::COMMAND) {
            self.save_project();
        }
        if shortcut(
            egui::Key::O,
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        ) {
            self.open_project(context);
        } else if shortcut(egui::Key::O, egui::Modifiers::COMMAND) {
            self.open_image(context);
        }
        if !self.page_busy() && !context.wants_keyboard_input() {
            if shortcut(
                egui::Key::Z,
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
            ) || shortcut(egui::Key::Y, egui::Modifiers::COMMAND)
            {
                self.redo();
            } else if shortcut(egui::Key::Z, egui::Modifiers::COMMAND) {
                self.undo();
            }
            if shortcut(egui::Key::F6, egui::Modifiers::NONE) {
                self.next_unchecked();
            }
            if shortcut(egui::Key::Escape, egui::Modifiers::NONE) {
                self.close_details();
            }
            if !egui::Popup::is_any_open(context)
                && self.page.canvas_context.is_none()
                && self.page.object_drag.is_none()
            {
                self.edit_shortcuts(context);
            }
        }
    }

    fn edit_shortcuts(&mut self, context: &egui::Context) {
        let commands = context.input_mut(|input| {
            let mut commands = Vec::new();
            // Use the produced character for symbols such as #, : and =. Their
            // physical key and modifiers differ between keyboard layouts.
            let has_text = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Text(_)));
            let modifiers = input.modifiers;
            input.events.retain(|event| {
                let command = match event {
                    egui::Event::Text(text)
                        if !modifiers.command && !modifiers.ctrl && !modifiers.mac_cmd =>
                    {
                        let mut characters = text.chars();
                        characters.next().and_then(|character| {
                            characters
                                .next()
                                .is_none()
                                .then(|| EditShortcut::from_char(character))
                                .flatten()
                        })
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none() => {
                        match key {
                            egui::Key::Delete | egui::Key::Backspace => Some(EditShortcut::Delete),
                            // Some integrations deliver logical key events only.
                            _ if !has_text => {
                                let name = key.name().to_lowercase();
                                (name.len() == 1)
                                    .then(|| name.chars().next().and_then(EditShortcut::from_char))
                                    .flatten()
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(command) = command {
                    commands.push(command);
                    false
                } else {
                    true
                }
            });
            commands
        });
        for command in commands {
            self.apply_edit_shortcut(command);
        }
    }

    fn apply_edit_shortcut(&mut self, command: EditShortcut) {
        let Some(object) = self.page.selected_object else {
            return;
        };
        match (object, command) {
            (_, EditShortcut::Delete) => self.delete_object(object),
            (Object::Note(index), command) => {
                let Some(original) = self.page.score.notes.get(index) else {
                    return;
                };
                let mut note = original.clone();
                match command {
                    EditShortcut::Duration(duration) => note.duration = duration,
                    EditShortcut::Dots(dots) => note.dots = dots,
                    EditShortcut::Accidental(accidental) => {
                        note.accidental = accidental;
                        note.accidental_explicit = true;
                    }
                    EditShortcut::Delete => unreachable!(),
                }
                if (
                    note.duration,
                    note.dots,
                    note.accidental,
                    note.accidental_explicit,
                ) != (
                    original.duration,
                    original.dots,
                    original.accidental,
                    original.accidental_explicit,
                ) {
                    self.snapshot();
                    self.page.score.notes[index] = note;
                    self.page.score.recompute_accidentals();
                }
            }
            (Object::Rest(index), command) => {
                let Some(original) = self.page.score.rests.get(index) else {
                    return;
                };
                if original.measures.is_some() {
                    return;
                }
                let mut rest = original.clone();
                match command {
                    EditShortcut::Duration(duration) => rest.duration = duration,
                    EditShortcut::Dots(dots) => rest.dots = dots,
                    _ => return,
                }
                if (rest.duration, rest.dots) != (original.duration, original.dots) {
                    self.snapshot();
                    self.page.score.rests[index] = rest;
                }
            }
            _ => {}
        }
    }

    fn shortcut_help(ui: &mut egui::Ui) {
        ui.menu_button("?", |ui| {
            ui.strong(i18n::tr("Keyboard shortcuts"));
            ui.label(i18n::tr("First select an object in the scan or preview."));
            ui.separator();
            egui::Grid::new("edit_shortcut_help").show(ui, |ui| {
                for (duration, key) in DURATION_SHORTCUTS {
                    ui.monospace(key.to_string());
                    ui.label(i18n::format(
                        "{0} note / rest",
                        &[("0", duration.label().to_string())],
                    ));
                    ui.end_row();
                }
                for (key, label) in [
                    (".", i18n::tr("Single-dotted note / rest")),
                    (":", i18n::tr("Double-dotted note / rest")),
                    ("0", i18n::tr("No dots")),
                    ("#", i18n::tr("Sharp (note)")),
                    ("b", i18n::tr("Flat (note)")),
                    ("=", i18n::tr("Natural (note)")),
                    (i18n::tr("Delete / Backspace"), i18n::tr("Delete object")),
                    ("F6", i18n::tr("Next unreviewed object")),
                    ("Escape", i18n::tr("Clear selection")),
                ] {
                    ui.monospace(key);
                    ui.label(label);
                    ui.end_row();
                }
            });
            ui.separator();
            ui.label(i18n::tr(
                "Editing shortcuts are inactive while entering text.",
            ));
        })
        .response
        .on_hover_text(i18n::tr("Show keyboard shortcuts"));
    }

    fn show_logo(&mut self, ui: &mut egui::Ui) {
        static LOGO: std::sync::OnceLock<image::RgbaImage> = std::sync::OnceLock::new();
        let source = LOGO.get_or_init(|| {
            let image = image::load_from_memory(include_bytes!("../assets/libreomr-logo.png"))
                .expect("embedded project logo must be a valid PNG")
                .into_rgba8();
            // Trim transparent padding for a legible wordmark in the 30px toolbar.
            let (mut left, mut top, mut right, mut bottom) = (image.width(), image.height(), 0, 0);
            for (x, y, pixel) in image.enumerate_pixels() {
                if pixel[3] > 16 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
            image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image()
        });
        let dark = ui.visuals().dark_mode;
        let density = ui.ctx().pixels_per_point();
        let scale =
            (136.0 * density / source.width() as f32).min(30.0 * density / source.height() as f32);
        let width = (source.width() as f32 * scale).round().max(1.0) as u32;
        let height = (source.height() as f32 * scale).round().max(1.0) as u32;
        let pixel_size = [width as usize, height as usize];
        if self.logo_dark != dark
            || self
                .logo_texture
                .as_ref()
                .is_none_or(|texture| texture.size() != pixel_size)
        {
            self.logo_dark = dark;
            let mut foreground = source.clone();
            if dark {
                for pixel in foreground.pixels_mut() {
                    let teal = pixel[1].saturating_sub(pixel[0]) > 12;
                    let color = if teal {
                        [85, 194, 208]
                    } else {
                        [229, 235, 241]
                    };
                    pixel.0[..3].copy_from_slice(&color);
                }
            }
            // Composite on the active toolbar color before filtering to avoid
            // alpha fringes, preserving the sharp display-density-aware logo.
            let mut background = image::RgbaImage::from_pixel(
                source.width(),
                source.height(),
                image::Rgba(ui.visuals().window_fill.to_array()),
            );
            image::imageops::overlay(&mut background, &foreground, 0, 0);
            // Prefilter to the actual framebuffer size; a single bilinear GPU
            // sample of the large original loses thin strokes at toolbar size.
            let image = image::imageops::resize(
                &background,
                width,
                height,
                image::imageops::FilterType::Lanczos3,
            );
            let pixels = egui::ColorImage::from_rgba_unmultiplied(pixel_size, image.as_raw());
            if let Some(texture) = &mut self.logo_texture {
                texture.set(pixels, egui::TextureOptions::NEAREST);
            } else {
                self.logo_texture = Some(ui.ctx().load_texture(
                    "libreomr-logo",
                    pixels,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        let texture = self.logo_texture.as_ref().unwrap();
        let size = texture.size_vec2() / density;
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        let rect = Rect::from_min_size(rect.min.round_to_pixels(density), size);
        // One texture pixel per framebuffer pixel, without another resampling.
        egui::Image::new(texture).paint_at(ui, rect);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Image, ui.is_enabled(), "libreOMR")
        });
    }

    fn toolbar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .min_height(54.0)
            .frame(
                egui::Frame::new()
                    .fill(context.style().visuals.window_fill)
                    .inner_margin(12.0),
            )
            .show(context, |ui| {
                ui.set_min_height(30.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.show_logo(ui);
                    ui.separator();
                    ui.add_enabled_ui(self.pending.is_none(), |ui| {
                        ui.menu_button(i18n::tr("File"), |ui| {
                            if ui.button(i18n::tr("Open image")).clicked() {
                                ui.close();
                                self.open_image(context);
                            }
                            if ui.button(i18n::tr("Add pages")).clicked() {
                                ui.close();
                                self.add_images(context);
                            }
                            if ui.button(i18n::tr("Open project")).clicked() {
                                ui.close();
                                self.open_project(context);
                            }
                            if ui
                                .add_enabled(
                                    self.document_ready()
                                        && (self.dirty || self.project_path.is_none()),
                                    egui::Button::new(i18n::tr("Save")),
                                )
                                .on_disabled_hover_text(
                                    self.require_ready_document()
                                        .err()
                                        .unwrap_or_else(|| i18n::tr("No unsaved changes.").into()),
                                )
                                .clicked()
                            {
                                ui.close();
                                self.save_project();
                            }
                            if ui
                                .add_enabled(
                                    self.document_ready()
                                        && (0..self.pages.len()).any(|index| {
                                            let score = &self.page_at(index).score;
                                            !score.staves.is_empty()
                                                && (!score.notes.is_empty()
                                                    || !score.rests.is_empty())
                                        }),
                                    egui::Button::new(i18n::tr("Export MusicXML")),
                                )
                                .on_disabled_hover_text(
                                    self.require_ready_document().err().unwrap_or_else(|| {
                                        i18n::tr("No notes or rests available.").into()
                                    }),
                                )
                                .clicked()
                            {
                                ui.close();
                                self.export_musicxml();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    !self.page_busy(),
                                    egui::Button::new(i18n::tr("Recognize again")),
                                )
                                .clicked()
                                && self.confirm_discard()
                            {
                                ui.close();
                                let image = self.page.image.clone();
                                let name = self.page.name.clone();
                                self.start_job(context, JobMode::ReplacePage, move |reporter| {
                                    let image = reporter.straighten(image);
                                    let score = reporter.recognize(&image);
                                    Ok(Loaded::single(image, score, name))
                                });
                            }
                            if ui.button(i18n::tr("Load demo")).clicked() && self.confirm_discard()
                            {
                                ui.close();
                                self.start_job(context, JobMode::ReplaceDocument, |reporter| {
                                    let image = recognition::demo_image();
                                    let score = reporter.recognize(&image);
                                    Ok(Loaded::single(
                                        image,
                                        score,
                                        i18n::tr("Demonstration").into(),
                                    ))
                                });
                            }
                        });
                    });
                    if self.pending.is_some() {
                        ui.spinner();
                    }
                    ui.separator();
                    ui.add_enabled_ui(!self.page_busy(), |ui| {
                        if ui
                            .add_enabled(!self.page.undo.is_empty(), egui::Button::new("<"))
                            .on_hover_text(i18n::tr("Undo"))
                            .clicked()
                        {
                            self.undo();
                        }
                        if ui
                            .add_enabled(!self.page.redo.is_empty(), egui::Button::new(">"))
                            .on_hover_text(i18n::tr("Redo"))
                            .clicked()
                        {
                            self.redo();
                        }
                    });
                    ui.separator();
                    if ui.button("-").on_hover_text(i18n::tr("Zoom out")).clicked() {
                        self.page.zoom = (self.page.zoom / 1.25).max(0.02);
                        self.page.fit_width = false;
                    }
                    let mut percent = self.page.zoom * 100.0;
                    if ui
                        .add(
                            egui::DragValue::new(&mut percent)
                                .range(2.0..=300.0)
                                .suffix(" %")
                                .speed(1.0),
                        )
                        .changed()
                    {
                        self.page.zoom = percent / 100.0;
                        self.page.fit_width = false;
                    }
                    if ui.button("+").on_hover_text(i18n::tr("Zoom in")).clicked() {
                        self.page.zoom = (self.page.zoom * 1.25).min(3.0);
                        self.page.fit_width = false;
                    }
                    if ui
                        .selectable_label(self.page.fit_width, i18n::tr("Fit"))
                        .on_hover_text(i18n::tr("Fit page width"))
                        .clicked()
                    {
                        self.page.fit_width = true;
                    }
                    ui.checkbox(&mut self.show_preview, i18n::tr("Preview"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(icons::GEAR)
                            .on_hover_text(i18n::tr("Settings"))
                            .clicked()
                        {
                            self.show_settings = !self.show_settings;
                        }
                        ui.separator();
                        ui.add(egui::Label::new(&self.page.name).truncate())
                            .on_hover_text(&self.page.name);
                    });
                });
            });
    }

    fn preview(&mut self, context: &egui::Context) {
        if !self.show_preview || !self.page.import.ready() {
            return;
        }
        let inherited_meter = self.inherited_time_signature();
        egui::TopBottomPanel::bottom("recognition_preview")
            .resizable(true)
            .default_height(160.0)
            .min_height(110.0)
            .max_height(
                (context.content_rect().height() * if self.problems.show { 0.25 } else { 0.5 })
                    .max(110.0),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(i18n::format(
                        "Preview - Staff {0}",
                        &[("0", format!("{}", self.page.selected_staff + 1))],
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("\u{00d7}")
                            .on_hover_text(i18n::tr("Hide preview"))
                            .clicked()
                        {
                            self.show_preview = false;
                        }
                        ui.toggle_value(&mut self.page.preview.fit_width, icons::ARROWS_HORIZONTAL)
                            .on_hover_text(i18n::tr(
                                "Fit preview horizontally (off: original size)",
                            ));
                    });
                });
                let selected = self
                    .page
                    .selected
                    .map(crate::preview::Selection::Note)
                    .or_else(|| self.page.selected_rest.map(crate::preview::Selection::Rest))
                    .or(match self.page.selected_object {
                        Some(Object::KeyChange(index)) => {
                            Some(crate::preview::Selection::KeyChange(index))
                        }
                        Some(Object::TimeChange(index)) => {
                            Some(crate::preview::Selection::TimeChange(index))
                        }
                        Some(Object::Dynamic(index)) => {
                            Some(crate::preview::Selection::Dynamic(index))
                        }
                        Some(Object::RepeatMark(index)) => {
                            Some(crate::preview::Selection::RepeatMark(index))
                        }
                        Some(Object::Span(index)) => Some(crate::preview::Selection::Span(index)),
                        Some(Object::Barline(index)) => {
                            Some(crate::preview::Selection::Barline(index))
                        }
                        _ => None,
                    });
                if let Some(selection) = self.page.preview.show_with_inherited(
                    ui,
                    &self.page.score,
                    self.page.selected_staff,
                    selected,
                    inherited_meter,
                ) {
                    match selection {
                        crate::preview::Selection::Note(index) => {
                            self.select_object(Some(Object::Note(index)));
                        }
                        crate::preview::Selection::Rest(index) => {
                            self.select_object(Some(Object::Rest(index)));
                        }
                        crate::preview::Selection::KeyChange(index) => {
                            self.select_object(Some(Object::KeyChange(index)))
                        }
                        crate::preview::Selection::TimeChange(index) => {
                            self.select_object(Some(Object::TimeChange(index)))
                        }
                        crate::preview::Selection::Dynamic(index) => {
                            self.select_object(Some(Object::Dynamic(index)))
                        }
                        crate::preview::Selection::RepeatMark(index) => {
                            self.select_object(Some(Object::RepeatMark(index)))
                        }
                        crate::preview::Selection::Barline(index) => {
                            self.select_object(Some(Object::Barline(index)))
                        }
                        crate::preview::Selection::Span(index) => {
                            self.select_object(Some(Object::Span(index)))
                        }
                    }
                }
                if let Some(edit) = self.page.preview.take_anchor_edit() {
                    self.apply_preview_anchor_edit(edit);
                }
            });
    }

    fn apply_preview_anchor_edit(&mut self, edit: crate::preview::AnchorEdit) {
        if edit.started {
            let Some(span) = self.page.score.spans.get(edit.index) else {
                return;
            };
            let origin = Vec2::from(if edit.right { span.end } else { span.start });
            self.select_object(Some(Object::Span(edit.index)));
            self.snapshot();
            self.page.object_drag = Some(ObjectDrag {
                target: DragTarget::Anchor(edit.index, edit.right),
                origin,
                score: self.page.score.clone(),
            });
        }
        self.move_object(Vec2::from(edit.position));
        if edit.stopped {
            self.page.object_drag = None;
        }
    }

    fn repeat_mark_editor(&mut self, ui: &mut egui::Ui) {
        let Some(Object::RepeatMark(index)) = self.page.selected_object else {
            return;
        };
        let mut mark = self.page.score.repeat_marks[index].clone();
        ui.strong(mark.kind.label());
        let mut changed = false;
        if mark.kind != RepeatMarkKind::Ending {
            egui::ComboBox::from_id_salt("repeat_sign_kind")
                .selected_text(mark.kind.label())
                .show_ui(ui, |ui| {
                    for kind in [RepeatMarkKind::Segno, RepeatMarkKind::Coda] {
                        changed |= ui
                            .selectable_value(&mut mark.kind, kind, kind.label())
                            .changed();
                    }
                });
        }
        if mark.kind == RepeatMarkKind::Ending {
            ui.label(i18n::tr("Endings (e.g. 1 or 1,2)"));
            let draft_id = ui.id().with(("ending_numbers", index, &mark.numbers));
            let mut numbers = ui
                .data(|data| data.get_temp::<String>(draft_id))
                .unwrap_or_else(|| mark.numbers.clone());
            if ui
                .add(egui::TextEdit::singleline(&mut numbers).desired_width(ui.available_width()))
                .changed()
            {
                mark.numbers = numbers.clone();
                changed = mark.valid_numbers();
                ui.data_mut(|data| {
                    if changed {
                        data.remove::<String>(draft_id);
                    } else {
                        data.insert_temp(draft_id, numbers);
                    }
                });
            }
            if mark.numbers.is_empty() {
                ui.weak(i18n::tr("Please check and complete the number."));
            }
            changed |= ui
                .checkbox(&mut mark.closed, i18n::tr("Closed right end"))
                .changed();
            let staff = &self.page.score.staves[mark.staff];
            ui.label(i18n::tr("Start / end"));
            changed |= ui
                .add(egui::DragValue::new(&mut mark.x).range(staff.left..=mark.end_x - 1.0))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut mark.end_x).range(mark.x + 1.0..=staff.right))
                .changed();
        }
        changed |= ui
            .checkbox(&mut mark.confirmed, i18n::tr("Reviewed"))
            .changed();
        if changed && mark.valid_numbers() {
            self.snapshot();
            mark.bounds = None;
            self.page.score.repeat_marks[index] = mark;
        }
        if ui.button(i18n::tr("Delete")).clicked() {
            self.delete_object(Object::RepeatMark(index));
        }
    }

    fn dynamic_editor(&mut self, ui: &mut egui::Ui) {
        let Some(Object::Dynamic(index)) = self.page.selected_object else {
            return;
        };
        let Some(mut dynamic) = self.page.score.dynamics.get(index).cloned() else {
            return;
        };
        ui.strong(i18n::tr("Dynamics"));
        let previous = dynamic.kind;
        egui::ComboBox::from_id_salt("dynamic_kind")
            .selected_text(dynamic.kind.label())
            .show_ui(ui, |ui| {
                for kind in DynamicKind::ALL {
                    ui.selectable_value(&mut dynamic.kind, kind, kind.label());
                }
            });
        let mut changed = previous != dynamic.kind;
        if changed {
            dynamic.bounds = None;
            dynamic.confirmed = true;
        }
        changed |= ui
            .checkbox(&mut dynamic.confirmed, i18n::tr("Reviewed"))
            .changed();
        if changed {
            self.snapshot();
            self.page.score.dynamics[index] = dynamic;
        }
        if ui.button(i18n::tr("Delete dynamic")).clicked() {
            self.delete_object(Object::Dynamic(index));
        }
    }

    fn time_change_editor(&mut self, ui: &mut egui::Ui) {
        let Some(Object::TimeChange(index)) = self.page.selected_object else {
            return;
        };
        let Some(mut change) = self.page.score.time_changes.get(index).cloned() else {
            return;
        };
        ui.strong(i18n::tr("Time change"));
        let previous = change.time_signature;
        egui::ComboBox::from_id_salt("time_change_signature")
            .selected_text(change.time_signature.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut change.time_signature, TimeSignature::Common, "C (4/4)");
                ui.selectable_value(
                    &mut change.time_signature,
                    TimeSignature::Cut,
                    i18n::tr("Alla breve (2/2)"),
                );
                if ui
                    .selectable_label(
                        matches!(change.time_signature, TimeSignature::Meter { .. }),
                        i18n::tr("Numerator / denominator"),
                    )
                    .clicked()
                    && !matches!(change.time_signature, TimeSignature::Meter { .. })
                {
                    change.time_signature = TimeSignature::Meter {
                        beats: 4,
                        beat_unit: 4,
                    };
                }
            });
        if let TimeSignature::Meter { beats, beat_unit } = &mut change.time_signature {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(beats).range(1..=32));
                ui.label("/");
                egui::ComboBox::from_id_salt("time_change_unit")
                    .width(64.0)
                    .selected_text(beat_unit.to_string())
                    .show_ui(ui, |ui| {
                        for unit in [1, 2, 4, 8, 16, 32] {
                            ui.selectable_value(beat_unit, unit, unit.to_string());
                        }
                    });
            });
        }
        if previous != change.time_signature {
            change.confirmed = true;
        }
        let confirmed_changed = ui
            .checkbox(&mut change.confirmed, i18n::tr("Time change reviewed"))
            .changed();
        if previous != change.time_signature || confirmed_changed {
            self.snapshot();
            self.page.score.time_changes[index] = change;
        }
        if ui.button(i18n::tr("Delete time change")).clicked() {
            self.delete_object(Object::TimeChange(index));
        }
    }

    fn key_change_editor(&mut self, ui: &mut egui::Ui) {
        let mut changes = self.page.score.key_changes.clone();
        let mut changed = false;
        if let Some(Object::KeyChange(index)) = self.page.selected_object
            && index < changes.len()
        {
            ui.horizontal(|ui| {
                ui.strong(i18n::tr("Key change"));
                if ui
                    .button("\u{00d7}")
                    .on_hover_text(i18n::tr("Delete key change"))
                    .clicked()
                {
                    changes.remove(index);
                    self.page.selected_object = None;
                    changed = true;
                }
            });
            if self.page.selected_object.is_some() {
                let previous_key = changes[index].key_signature;
                let response = egui::ComboBox::from_id_salt("key_change_signature")
                    .width(ui.available_width())
                    .wrap_mode(egui::TextWrapMode::Wrap)
                    .selected_text(changes[index].key_signature.label())
                    .show_ui(ui, |ui| {
                        for fifths in -7..=7 {
                            let key = KeySignature { fifths };
                            changed |= ui
                                .selectable_value(
                                    &mut changes[index].key_signature,
                                    key,
                                    key.label(),
                                )
                                .changed();
                        }
                    });
                if previous_key != changes[index].key_signature {
                    response.response.scroll_to_me(None);
                }
                changed |= ui
                    .checkbox(
                        &mut changes[index].confirmed,
                        i18n::tr("Key change reviewed"),
                    )
                    .changed();
            }
        }
        if changed {
            self.snapshot();
            self.page.score.key_changes = changes;
            self.page.score.recompute_accidentals();
        }
    }

    fn span_editor(&mut self, ui: &mut egui::Ui) {
        let Some(staff) = self.page.score.staves.get(self.page.selected_staff) else {
            return;
        };
        let mut spans = self.page.score.spans.clone();
        let mut changed = false;
        let gap = staff.spacing();
        ui.push_id(("spans", self.page.selected_staff), |ui| {
            let mut remove = None;
            for (index, span) in spans
                .iter_mut()
                .enumerate()
                .filter(|(index, _)| self.page.selected_object == Some(Object::Span(*index)))
            {
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("kind")
                            .width((ui.available_width() - 32.0).min(145.0))
                            .wrap_mode(egui::TextWrapMode::Wrap)
                            .selected_text(span.kind.label())
                            .show_ui(ui, |ui| {
                                for kind in SpanKind::ALL {
                                    if ui
                                        .selectable_value(&mut span.kind, kind, kind.label())
                                        .changed()
                                    {
                                        if matches!(
                                            kind,
                                            SpanKind::Crescendo | SpanKind::Decrescendo
                                        ) {
                                            span.height = span.height.abs().max(gap * 0.25);
                                        }
                                        changed = true;
                                    }
                                }
                            });
                        if ui
                            .button("\u{00d7}")
                            .on_hover_text(i18n::tr("Delete slur, tie or wedge"))
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                    let gabel = matches!(span.kind, SpanKind::Crescendo | SpanKind::Decrescendo);
                    ui.label(if gabel {
                        i18n::tr("Opening")
                    } else {
                        i18n::tr("Curvature")
                    });
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut span.height)
                                .range(if gabel {
                                    gap * 0.1..=gap * 6.0
                                } else {
                                    -gap * 6.0..=gap * 6.0
                                })
                                .speed(0.5),
                        )
                        .changed();
                    ui.strong(i18n::tr("Anchor"));
                    ui.weak(i18n::tr("Drag endpoints in the scan or preview."));
                    let mut anchors_changed = false;
                    for right in [false, true] {
                        ui.label(if right {
                            i18n::tr("End")
                        } else {
                            i18n::tr("Start")
                        });
                        let (minimum, maximum) = if right {
                            (span.start[0] + 1.0, staff.right)
                        } else {
                            (staff.left, span.end[0] - 1.0)
                        };
                        let point = if right {
                            &mut span.end
                        } else {
                            &mut span.start
                        };
                        ui.horizontal(|ui| {
                            anchors_changed |= ui
                                .add(
                                    egui::DragValue::new(&mut point[0])
                                        .prefix("x: ")
                                        .range(minimum..=maximum)
                                        .speed(0.5),
                                )
                                .changed();
                            anchors_changed |= ui
                                .add(
                                    egui::DragValue::new(&mut point[1])
                                        .prefix("y: ")
                                        .range(
                                            0.0..=self.page.image.height().saturating_sub(1) as f32,
                                        )
                                        .speed(0.5),
                                )
                                .changed();
                        });
                    }
                    if anchors_changed {
                        span.confirmed = true;
                        changed = true;
                    }
                    changed |= ui
                        .checkbox(&mut span.confirmed, i18n::tr("Reviewed"))
                        .changed();
                    span.start[0] = span.start[0].min(staff.right - 1.0);
                    span.end[0] = span.end[0].max(span.start[0] + 1.0);
                    ui.separator();
                });
            }
            if let Some(index) = remove {
                spans.remove(index);
                self.page.selected_object = None;
                changed = true;
            }
        });
        if changed {
            self.snapshot();
            self.page.score.spans = spans;
        }
    }

    fn add_span(&mut self, kind: SpanKind) {
        let Some(staff) = self.page.score.staves.get(self.page.selected_staff) else {
            return;
        };
        let gap = staff.spacing();
        let mut notes: Vec<_> = self
            .page
            .score
            .notes
            .iter()
            .filter(|note| note.staff == self.page.selected_staff)
            .collect();
        notes.sort_by(|first, second| first.x.total_cmp(&second.x));
        let first = self
            .page
            .selected
            .and_then(|index| self.page.score.notes.get(index))
            .filter(|note| note.staff == self.page.selected_staff)
            .or_else(|| notes.first().copied());
        let last = first.and_then(|first| {
            notes
                .iter()
                .find(|note| note.x > first.x + gap * 0.3)
                .copied()
        });
        let start_x = first.map_or(staff.left + (staff.right - staff.left) * 0.25, |note| {
            note.x
        });
        let end_x = last.map_or((start_x + gap * 4.0).min(staff.right), |note| note.x);
        if end_x <= start_x {
            return;
        }
        let gabel = matches!(kind, SpanKind::Crescendo | SpanKind::Decrescendo);
        let span = Span {
            staff: self.page.selected_staff,
            kind,
            start: [
                start_x,
                if gabel {
                    staff.lines[4] + gap * 2.0
                } else {
                    first.map_or(staff.lines[0], |note| note.y) - gap
                },
            ],
            end: [
                end_x,
                if gabel {
                    staff.lines[4] + gap * 2.0
                } else {
                    last.map_or(staff.lines[0], |note| note.y) - gap
                },
            ],
            height: if gabel { gap * 0.5 } else { -gap },
            confirmed: true,
        };
        self.snapshot();
        self.page.score.spans.push(span);
        self.select_object(Some(Object::Span(self.page.score.spans.len() - 1)));
    }

    fn add_menu(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(
            !self.page_busy() && !self.page.score.staves.is_empty(),
            |ui| {
                ui.menu_button(icons::PLUS, |ui| {
                    if ui.button(i18n::tr("Barline")).clicked() {
                        let staff = &self.page.score.staves[self.page.selected_staff];
                        let bar = Barline {
                            staff: self.page.selected_staff,
                            x: (staff.left + staff.right) / 2.0,
                            kind: BarlineKind::Single,
                            confirmed: true,
                        };
                        self.snapshot();
                        self.page.score.barlines.push(bar);
                        self.page.score.recompute_accidentals();
                        self.select_object(Some(Object::Barline(
                            self.page.score.barlines.len() - 1,
                        )));
                        ui.close();
                    }
                    if ui.button(i18n::tr("Key change")).clicked() {
                        let staff = &self.page.score.staves[self.page.selected_staff];
                        let x = self
                            .page
                            .selected
                            .and_then(|index| self.page.score.notes.get(index))
                            .filter(|note| note.staff == self.page.selected_staff)
                            .map_or((staff.left + staff.right) / 2.0, |note| note.x);
                        self.snapshot();
                        self.page.score.key_changes.push(KeyChange {
                            staff: self.page.selected_staff,
                            x,
                            key_signature: KeySignature { fifths: 0 },
                            bounds: vec![],
                            confirmed: true,
                        });
                        self.page.score.recompute_accidentals();
                        self.select_object(Some(Object::KeyChange(
                            self.page.score.key_changes.len() - 1,
                        )));
                        ui.close();
                    }
                    for kind in RepeatMarkKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            let staff = &self.page.score.staves[self.page.selected_staff];
                            self.insert_at(
                                Vec2::new(
                                    (staff.left + staff.right) * 0.5,
                                    (staff.lines[0] - staff.spacing() * 3.0).max(0.0),
                                ),
                                Insert::RepeatMark(kind),
                            );
                            ui.close();
                        }
                    }
                    if ui.button(i18n::tr("Dynamics")).clicked() {
                        let staff = &self.page.score.staves[self.page.selected_staff];
                        let x = self
                            .page
                            .selected
                            .and_then(|index| self.page.score.notes.get(index))
                            .filter(|note| note.staff == self.page.selected_staff)
                            .map_or((staff.left + staff.right) / 2.0, |note| note.x);
                        self.insert_at(
                            Vec2::new(x, staff.lines[4] + staff.spacing() * 2.5),
                            Insert::Dynamic,
                        );
                        ui.close();
                    }
                    if ui.button(i18n::tr("Time change")).clicked() {
                        let staff = &self.page.score.staves[self.page.selected_staff];
                        let x = self
                            .page
                            .selected
                            .and_then(|index| self.page.score.notes.get(index))
                            .filter(|note| note.staff == self.page.selected_staff)
                            .map_or((staff.left + staff.right) / 2.0, |note| note.x);
                        self.snapshot();
                        self.page.score.time_changes.push(TimeChange {
                            staff: self.page.selected_staff,
                            x,
                            time_signature: TimeSignature::Common,
                            bounds: vec![],
                            confirmed: true,
                        });
                        self.select_object(Some(Object::TimeChange(
                            self.page.score.time_changes.len() - 1,
                        )));
                        ui.close();
                    }
                    ui.separator();
                    for kind in SpanKind::ALL {
                        if ui.button(kind.label()).clicked() {
                            self.add_span(kind);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(i18n::tr("Add object"));
            },
        );
    }

    fn object_row(&mut self, ui: &mut egui::Ui, object: Object) {
        let (icon, label, tooltip) = match object {
            Object::Note(index) => {
                let note = &self.page.score.notes[index];
                (
                    icons::MUSIC_NOTE,
                    format!("{}   {}", note.pitch_label(), note.duration_label()),
                    i18n::format(
                        "Note {0} - {1}",
                        &[
                            ("0", note.pitch_label().to_string()),
                            ("1", note.duration_label().to_string()),
                        ],
                    ),
                )
            }
            Object::Rest(index) => {
                let value = self.page.score.rests[index].duration_label();
                (
                    icons::PAUSE,
                    value.clone(),
                    i18n::format("Rest {value}", &[("value", value.to_string())]),
                )
            }
            Object::Barline(index) => {
                let kind = self.page.score.barlines[index].kind.label();
                (
                    icons::LINE_VERTICAL,
                    kind.to_owned(),
                    i18n::format("Barline - {kind}", &[("kind", kind.to_string())]),
                )
            }
            Object::Span(index) => {
                let kind = self.page.score.spans[index].kind.label();
                (icons::BEZIER_CURVE, kind.to_owned(), kind.to_owned())
            }
            Object::KeyChange(index) => {
                let key = self.page.score.key_changes[index].key_signature.label();
                (
                    icons::HASH,
                    key.clone(),
                    i18n::format("Key change {key}", &[("key", key.to_string())]),
                )
            }
            Object::Dynamic(index) => {
                let label = self.page.score.dynamics[index].kind.label();
                (
                    icons::MUSIC_NOTE,
                    label.to_owned(),
                    i18n::format("Dynamic {label}", &[("label", label.to_string())]),
                )
            }
            Object::RepeatMark(index) => {
                let label = self.page.score.repeat_marks[index].kind.label();
                (icons::MUSIC_NOTE, label.to_owned(), label.to_owned())
            }
            Object::TimeChange(index) => {
                let meter = self.page.score.time_changes[index].time_signature.label();
                (
                    icons::CLOCK,
                    meter.clone(),
                    i18n::format("Time change {meter}", &[("meter", meter.to_string())]),
                )
            }
        };
        let confirmed = self.is_confirmed(object);
        let selected = self.page.selected_object == Some(object);
        let palette = crate::theme::palette(ui.visuals().dark_mode);
        ui.push_id(("object", object), |ui| {
            ui.horizontal(|ui| {
                let color = if confirmed {
                    palette.confirmed
                } else {
                    ui.visuals().text_color()
                };
                let mut button = egui::Button::selectable(
                    selected,
                    egui::RichText::new(format!("{icon}  {label}")).color(color),
                )
                .min_size(Vec2::new((ui.available_width() - 32.0).max(40.0), 30.0))
                .wrap();
                if confirmed && !selected {
                    button = button.fill(palette.confirmed_bg);
                }
                if ui.add(button).on_hover_text(tooltip).clicked() {
                    self.select_object(Some(object));
                    self.page.reveal_selection = true;
                }
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                let rect = rect.shrink(4.0);
                if response.hovered() {
                    ui.painter().rect_filled(
                        rect.expand(4.0),
                        3.0_f32,
                        ui.visuals().widgets.hovered.weak_bg_fill,
                    );
                }
                if confirmed {
                    ui.painter().add(egui::Shape::line(
                        vec![
                            rect.min + Vec2::new(2.0, 8.0),
                            rect.min + Vec2::new(6.0, 12.0),
                            rect.min + Vec2::new(14.0, 3.0),
                        ],
                        Stroke::new(2.0_f32, color),
                    ));
                } else {
                    ui.painter().circle_stroke(
                        rect.center(),
                        4.0,
                        Stroke::new(1.5_f32, palette.pending),
                    );
                }
                if response
                    .on_hover_text(if confirmed {
                        i18n::tr("Reviewed - mark as unreviewed")
                    } else {
                        i18n::tr("Mark as reviewed")
                    })
                    .clicked()
                {
                    self.snapshot();
                    match object {
                        Object::Note(index) => self.page.score.notes[index].confirmed = !confirmed,
                        Object::Rest(index) => self.page.score.rests[index].confirmed = !confirmed,
                        Object::Barline(index) => {
                            self.page.score.barlines[index].confirmed = !confirmed
                        }
                        Object::Span(index) => self.page.score.spans[index].confirmed = !confirmed,
                        Object::KeyChange(index) => {
                            self.page.score.key_changes[index].confirmed = !confirmed
                        }
                        Object::TimeChange(index) => {
                            self.page.score.time_changes[index].confirmed = !confirmed
                        }
                        Object::Dynamic(index) => {
                            self.page.score.dynamics[index].confirmed = !confirmed
                        }
                        Object::RepeatMark(index) => {
                            self.page.score.repeat_marks[index].confirmed = !confirmed
                        }
                    }
                }
            });
        });
    }

    fn object_list(&mut self, ui: &mut egui::Ui) {
        let objects = self.ordered_objects();
        for staff in 0..self.page.score.staves.len() {
            if self.only_current_staff && staff != self.page.selected_staff {
                continue;
            }
            let row: Vec<_> = objects
                .iter()
                .copied()
                .filter(|object| match *object {
                    Object::Note(index) => self.page.score.notes[index].staff == staff,
                    Object::Rest(index) => self.page.score.rests[index].staff == staff,
                    Object::Barline(index) => self.page.score.barlines[index].staff == staff,
                    Object::Span(index) => self.page.score.spans[index].staff == staff,
                    Object::KeyChange(index) => self.page.score.key_changes[index].staff == staff,
                    Object::TimeChange(index) => self.page.score.time_changes[index].staff == staff,
                    Object::Dynamic(index) => self.page.score.dynamics[index].staff == staff,
                    Object::RepeatMark(index) => self.page.score.repeat_marks[index].staff == staff,
                })
                .collect();
            egui::CollapsingHeader::new(i18n::format(
                "Staff {0} ({1})",
                &[
                    ("0", format!("{}", staff + 1)),
                    ("1", format!("{}", row.len())),
                ],
            ))
            .id_salt(("objects_staff", staff))
            .default_open(staff == self.page.selected_staff)
            .open((self.page.reveal_selection && staff == self.page.selected_staff).then_some(true))
            .show(ui, |ui| {
                for object in row {
                    self.object_row(ui, object);
                }
            });
        }
    }

    fn object_preview(&mut self, ui: &mut egui::Ui) {
        let Some(object) = self.page.selected_object else {
            return;
        };
        let selected = match object {
            Object::Note(index) => crate::preview::Selection::Note(index),
            Object::Rest(index) => crate::preview::Selection::Rest(index),
            Object::Barline(index) => crate::preview::Selection::Barline(index),
            Object::Span(index) => crate::preview::Selection::Span(index),
            Object::KeyChange(index) => crate::preview::Selection::KeyChange(index),
            Object::TimeChange(index) => crate::preview::Selection::TimeChange(index),
            Object::Dynamic(index) => crate::preview::Selection::Dynamic(index),
            Object::RepeatMark(index) => crate::preview::Selection::RepeatMark(index),
        };
        ui.horizontal_wrapped(|ui| {
            ui.weak(i18n::tr("Object preview"));
            ui.weak(if self.is_confirmed(object) {
                i18n::tr("Reviewed")
            } else {
                i18n::tr("Unreviewed")
            });
        });
        let height = (ui.available_height() * 0.28).clamp(100.0, 160.0);
        ui.push_id("selected_object_preview", |ui| {
            self.page
                .preview
                .show_object(ui, &self.page.score, selected, height);
        });
        ui.separator();
    }

    fn inspector_combo(ui: &mut egui::Ui, label: &str) -> egui::ComboBox {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .width(ui.available_width())
            .wrap_mode(egui::TextWrapMode::Wrap)
    }

    fn dot_editor(ui: &mut egui::Ui, dots: &mut u8) -> bool {
        let changed = ui
            .add(
                egui::DragValue::new(dots)
                    .range(0..=2)
                    .prefix(i18n::tr("Dots: ")),
            )
            .on_hover_text(i18n::tr("Dots: 0 = none, . = single, : = double"))
            .changed();
        ui.small(i18n::tr("Keys: . single · : double · 0 none"));
        changed
    }

    fn inspector(&mut self, context: &egui::Context) {
        egui::SidePanel::right("inspector")
            .default_width(240.0)
            .width_range(200.0..=380.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(context.style().visuals.panel_fill)
                    .inner_margin(8.0),
            )
            .show(context, |ui| {
                // Keep widgets within the resized panel instead of letting their
                // preferred widths silently increase its effective minimum.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                ui.spacing_mut().button_padding = Vec2::new(6.0, 4.0);
                ui.spacing_mut().interact_size.y = 26.0;
                ui.spacing_mut().combo_width = 140.0_f32.min(ui.available_width());
                ui.spacing_mut().text_edit_width = ui.available_width();
                ui.horizontal(|ui| {
                    ui.heading(i18n::tr("Correction"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !self.page.show_object_list
                            && ui
                                .button(icons::X)
                                .on_hover_text(
                                    i18n::tr("Close details and return to overview (Escape)"),
                                )
                                .clicked()
                        {
                            self.close_details();
                        }
                        self.add_menu(ui);
                        Self::shortcut_help(ui);
                    });
                });
                let (confirmed, total) = self.review_progress();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::ProgressBar::new(if total == 0 {
                            0.0
                        } else {
                            confirmed as f32 / total as f32
                        })
                        .desired_width((ui.available_width() - 40.0).max(40.0))
                        .text(format!("{confirmed} / {total}")),
                    )
                    .on_hover_text(i18n::tr("Reviewed symbols"));
                    if ui
                        .add_enabled(
                            !self.page_busy() && confirmed < total,
                            egui::Button::new(icons::ARROW_RIGHT).min_size(Vec2::splat(30.0)),
                        )
                        .on_hover_text(i18n::tr("Next open correction (F6)"))
                        .clicked()
                    {
                        self.next_unchecked();
                    }
                });
                ui.separator();
                let compact_tabs = ui.available_width() < 210.0;
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.page.show_object_list,
                        true,
                        if compact_tabs {
                            icons::LIST_BULLETS.to_owned()
                        } else {
                            i18n::format("{0}  Objects", &[("0", icons::LIST_BULLETS.to_string())])
                        },
                    )
                    .on_hover_text(i18n::tr("Objects"));
                    ui.selectable_value(
                        &mut self.page.show_object_list,
                        false,
                        if compact_tabs {
                            icons::SLIDERS_HORIZONTAL.to_owned()
                        } else {
                            i18n::format("{0}  Details", &[("0", icons::SLIDERS_HORIZONTAL.to_string())])
                        },
                    )
                    .on_hover_text(i18n::tr("Details"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.page.show_object_list {
                            ui.toggle_value(&mut self.only_current_staff, icons::FUNNEL)
                                .on_hover_text(i18n::tr("Current staff only"));
                        }
                    });
                });
                if !self.page.show_object_list {
                    self.object_preview(ui);
                }
                egui::ScrollArea::vertical()
                    .id_salt(("correction_scroll", self.page.show_object_list))
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!self.page_busy(), |ui| {
                            if self.page.show_object_list {
                                self.object_list(ui);
                                return;
                            }
                            if !self.page.score.staves.is_empty() {
                                self.page.selected_staff =
                                    self.page.selected_staff.min(self.page.score.staves.len() - 1);
                                let previous_staff = self.page.selected_staff;
                                egui::ComboBox::from_id_salt("selected_staff")
                                    .selected_text(i18n::format("Staff {0}", &[("0", format!("{}", self.page.selected_staff + 1))]))
                                    .show_ui(ui, |ui| {
                                        for index in 0..self.page.score.staves.len() {
                                            ui.selectable_value(
                                                &mut self.page.selected_staff,
                                                index,
                                                i18n::format("Staff {0}", &[("0", format!("{}", index + 1))]),
                                            );
                                        }
                                    });
                                if previous_staff != self.page.selected_staff {
                                    self.select_object(None);
                                    self.page.reveal_selection = true;
                                }
                                egui::CollapsingHeader::new(i18n::tr("Staff information"))
                                    .default_open(false)
                                    .open(self.problems.open_system.then_some(true))
                                    .show(ui, |ui| {
                                        let mut header =
                                            self.page.score.staves[self.page.selected_staff].header.clone();
                                        let previous_header = header.clone();
                                        Self::inspector_combo(ui, i18n::tr("Clef"))
                                            .selected_text(header.clef.label())
                                            .show_ui(ui, |ui| {
                                                for clef in Clef::ALL {
                                                    ui.selectable_value(
                                                        &mut header.clef,
                                                        clef,
                                                        clef.label(),
                                                    );
                                                }
                                            });
                                        if header.clef == Clef::Unknown {
                                            ui.label(i18n::tr("Pitch reference: treble (assumed)"));
                                        }
                                        ui.label(i18n::tr("Key signature"));
                                        egui::ComboBox::from_id_salt("key_signature")
                                            .width(ui.available_width())
                                            .selected_text(
                                                header
                                                    .key_signature
                                                    .map(KeySignature::label)
                                                    .unwrap_or_else(|| i18n::tr("Not recognized").into()),
                                            )
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut header.key_signature,
                                                    None,
                                                    i18n::tr("Not recognized"),
                                                );
                                                for fifths in -7..=7 {
                                                    let key = KeySignature { fifths };
                                                    ui.selectable_value(
                                                        &mut header.key_signature,
                                                        Some(key),
                                                        key.label(),
                                                    );
                                                }
                                            });
                                        Self::inspector_combo(ui, i18n::tr("Time signature"))
                                            .selected_text(
                                                header
                                                    .time_signature
                                                    .map(TimeSignature::label)
                                                    .unwrap_or_else(|| i18n::tr("Not recognized").into()),
                                            )
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut header.time_signature,
                                                    None,
                                                    i18n::tr("Not recognized"),
                                                );
                                                for meter in
                                                    [TimeSignature::Common, TimeSignature::Cut]
                                                {
                                                    ui.selectable_value(
                                                        &mut header.time_signature,
                                                        Some(meter),
                                                        meter.label(),
                                                    );
                                                }
                                                if ui
                                                    .selectable_label(
                                                        matches!(
                                                            header.time_signature,
                                                            Some(TimeSignature::Meter { .. })
                                                        ),
                                                        i18n::tr("Numeric"),
                                                    )
                                                    .clicked()
                                                    && !matches!(
                                                        header.time_signature,
                                                        Some(TimeSignature::Meter { .. })
                                                    )
                                                {
                                                    header.time_signature =
                                                        Some(TimeSignature::Meter {
                                                            beats: 4,
                                                            beat_unit: 4,
                                                        });
                                                }
                                            });
                                        if let Some(TimeSignature::Meter { beats, beat_unit }) =
                                            &mut header.time_signature
                                        {
                                            ui.horizontal(|ui| {
                                                ui.add(egui::DragValue::new(beats).range(1..=32));
                                                ui.label("/");
                                                egui::ComboBox::from_id_salt("beat_unit")
                                                    .width(64.0)
                                                    .selected_text(beat_unit.to_string())
                                                    .show_ui(ui, |ui| {
                                                        for value in [1, 2, 4, 8, 16, 32] {
                                                            ui.selectable_value(
                                                                beat_unit,
                                                                value,
                                                                value.to_string(),
                                                            );
                                                        }
                                                    });
                                            });
                                        }
                                        ui.checkbox(
                                            &mut header.confirmed,
                                            i18n::tr("Staff information reviewed"),
                                        );
                                        if header != previous_header {
                                            self.snapshot();
                                            self.page.score
                                                .set_staff_header(self.page.selected_staff, header);
                                        }
                                    });
                                ui.separator();
                                let mut bars = self.page.score.barlines.clone();
                                let mut changed = false;
                                ui.push_id(("barlines", self.page.selected_staff), |ui| {
                                    let mut indices: Vec<_> = bars
                                        .iter()
                                        .enumerate()
                                        .filter(|(index, _)| {
                                            self.page.selected_object == Some(Object::Barline(*index))
                                        })
                                        .map(|(index, _)| index)
                                        .collect();
                                    indices.sort_by(|&first, &second| {
                                        bars[first].x.total_cmp(&bars[second].x)
                                    });
                                    let mut remove = None;
                                    for index in indices {
                                        ui.push_id(index, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.strong(i18n::tr("Barline"));
                                                if ui
                                                    .button("\u{00d7}")
                                                    .on_hover_text(i18n::tr("Delete barline"))
                                                    .clicked()
                                                {
                                                    remove = Some(index);
                                                }
                                            });
                                            let previous_kind = bars[index].kind;
                                            let response =
                                                egui::ComboBox::from_id_salt("barline_kind")
                                                    .width(ui.available_width())
                                                    .wrap_mode(egui::TextWrapMode::Wrap)
                                                    .selected_text(bars[index].kind.label())
                                                    .show_ui(ui, |ui| {
                                                        for kind in BarlineKind::ALL {
                                                            changed |= ui
                                                                .selectable_value(
                                                                    &mut bars[index].kind,
                                                                    kind,
                                                                    kind.label(),
                                                                )
                                                                .changed();
                                                        }
                                                    });
                                            if previous_kind != bars[index].kind {
                                                response.response.scroll_to_me(None);
                                            }
                                            changed |= ui
                                                .checkbox(&mut bars[index].confirmed, i18n::tr("Reviewed"))
                                                .changed();
                                        });
                                    }
                                    if let Some(index) = remove {
                                        bars.remove(index);
                                        self.page.selected_object = None;
                                        changed = true;
                                    }
                                });
                                if changed {
                                    self.snapshot();
                                    self.page.score.barlines = bars;
                                    self.page.score.recompute_accidentals();
                                }
                                self.key_change_editor(ui);
                                self.time_change_editor(ui);
                                self.dynamic_editor(ui);
                                self.repeat_mark_editor(ui);
                                self.span_editor(ui);
                                if matches!(
                                    self.page.selected_object,
                                    Some(
                                        Object::Barline(_)
                                            | Object::KeyChange(_)
                                            | Object::TimeChange(_)
                                            | Object::Dynamic(_)
                                            | Object::RepeatMark(_)
                                            | Object::Span(_)
                                    )
                                ) {
                                    ui.separator();
                                }
                            }
                            if let Some(index) = self.page
                                .selected
                                .filter(|&index| index < self.page.score.notes.len())
                            {
                                let mut note = self.page.score.notes[index].clone();
                                ui.heading(note.pitch_label());
                                let mut changed = false;
                                ui.horizontal(|ui| {
                                    let names = ["C", "D", "E", "F", "G", "A", "B / H"];
                                    let mut degree = note.pitch.rem_euclid(7);
                                    let mut octave = note.pitch.div_euclid(7);
                                    egui::ComboBox::from_id_salt("pitch_name")
                                        .width(65.0)
                                        .selected_text(names[degree as usize])
                                        .show_ui(ui, |ui| {
                                            for (index, name) in names.iter().enumerate() {
                                                ui.selectable_value(
                                                    &mut degree,
                                                    index as i32,
                                                    *name,
                                                );
                                            }
                                        });
                                    ui.add(
                                        egui::DragValue::new(&mut octave)
                                            .range(0..=9)
                                            .prefix(i18n::tr("Octave: ")),
                                    );
                                    let pitch = octave * 7 + degree;
                                    changed |= pitch != note.pitch;
                                    note.pitch = pitch;
                                });
                                Self::inspector_combo(ui, i18n::tr("Accidental"))
                                    .selected_text(if !note.accidental_explicit {
                                        i18n::tr("Automatic")
                                    } else {
                                        match note.accidental {
                                            -1 => "b",
                                            1 => "#",
                                            _ => i18n::tr("Natural"),
                                        }
                                    })
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                !note.accidental_explicit,
                                                i18n::tr("Automatic"),
                                            )
                                            .clicked()
                                        {
                                            note.accidental_explicit = false;
                                            changed = true;
                                        }
                                        for (value, label) in
                                            [(-1, "b  [b]"), (0, i18n::tr("Natural  [=]")), (1, "#  [#]")]
                                        {
                                            if ui
                                                .selectable_label(
                                                    note.accidental_explicit
                                                        && note.accidental == value,
                                                    label,
                                                )
                                                .clicked()
                                            {
                                                note.accidental = value;
                                                note.accidental_explicit = true;
                                                changed = true;
                                            }
                                        }
                                    });
                                Self::inspector_combo(ui, i18n::tr("Note value"))
                                    .selected_text(duration_shortcut_label(note.duration))
                                    .show_ui(ui, |ui| {
                                        for duration in Duration::ALL {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut note.duration,
                                                    duration,
                                                    duration_shortcut_label(duration),
                                                )
                                                .changed();
                                        }
                                    });
                                changed |= Self::dot_editor(ui, &mut note.dots);
                                let mut use_tuplet = note.tuplet_ratio().is_some();
                                if ui.checkbox(&mut use_tuplet, i18n::tr("Tuplet")).changed() {
                                    note.tuplet = use_tuplet.then(Tuplet::default);
                                    note.triplet = false;
                                    changed = true;
                                }
                                if use_tuplet {
                                    let mut ratio = note.tuplet_ratio().unwrap_or_default();
                                    ui.horizontal_wrapped(|ui| {
                                        let actual_changed = ui.add(egui::DragValue::new(&mut ratio.actual)
                                            .range(2..=64).prefix(i18n::tr("Notes: "))).changed();
                                        ui.label(i18n::tr("in the time of"));
                                        let normal_changed = ui.add(egui::DragValue::new(&mut ratio.normal)
                                            .range(1..=64)).changed();
                                        if actual_changed || normal_changed {
                                            note.tuplet = Some(ratio);
                                            note.triplet = false;
                                            changed = true;
                                        }
                                    });
                                    ui.small(i18n::tr("e.g. triplet 3:2, quintuplet 5:4, septuplet 7:4"));
                                }
                                if ui.checkbox(&mut note.grace, i18n::tr("Grace note")).on_hover_text(
                                    i18n::tr("Does not count towards measure duration. The note value determines its appearance.")
                                ).changed() {
                                    if note.grace { note.small = true; }
                                    changed = true;
                                }
                                changed |= ui.checkbox(&mut note.small, i18n::tr("Small noteheads")).changed();
                                Self::inspector_combo(ui, i18n::tr("Articulation"))
                                    .selected_text(note.articulation.label())
                                    .show_ui(ui, |ui| {
                                        for articulation in Articulation::ALL {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut note.articulation,
                                                    articulation,
                                                    articulation.label(),
                                                )
                                                .changed();
                                        }
                                    });
                                changed |= ui.checkbox(&mut note.confirmed, i18n::tr("Reviewed")).changed();
                                if changed {
                                    self.snapshot();
                                    if note.pitch != self.page.score.notes[index].pitch {
                                        let staff = &self.page.score.staves[note.staff];
                                        note.y = staff.y_at_pitch(note.pitch);
                                    }
                                    self.page.score.notes[index] = note;
                                    self.page.score.recompute_accidentals();
                                }
                                if ui
                                    .button("\u{00d7}")
                                    .on_hover_text(i18n::tr("Delete note (Delete / Backspace)"))
                                    .clicked()
                                {
                                    self.snapshot();
                                    self.page.score.notes.remove(index);
                                    self.page.score.recompute_accidentals();
                                    self.select_object(None);
                                }
                                ui.separator();
                            }
                            if let Some(index) = self.page
                                .selected_rest
                                .filter(|&index| index < self.page.score.rests.len())
                            {
                                let mut rest = self.page.score.rests[index].clone();
                                ui.heading(i18n::tr("Rest"));
                                let mut changed = false;
                                let mut multiple = rest.measures.is_some();
                                if ui.checkbox(&mut multiple, i18n::tr("Multimeasure rest")).changed() {
                                    rest.measures = multiple.then_some(2);
                                    if multiple {
                                        rest.duration = Duration::Whole;
                                        rest.dots = 0;
                                    }
                                    changed = true;
                                }
                                if let Some(count) = &mut rest.measures {
                                    ui.horizontal(|ui| {
                                        ui.label(i18n::tr("Measures"));
                                        changed |= ui
                                            .add(egui::DragValue::new(count).range(2..=99))
                                            .changed();
                                    });
                                } else {
                                    Self::inspector_combo(ui, i18n::tr("Rest value"))
                                        .selected_text(duration_shortcut_label(rest.duration))
                                        .show_ui(ui, |ui| {
                                            for duration in Duration::ALL {
                                                changed |= ui
                                                    .selectable_value(
                                                        &mut rest.duration,
                                                        duration,
                                                        duration_shortcut_label(duration),
                                                    )
                                                    .changed();
                                            }
                                        });
                                    changed |= Self::dot_editor(ui, &mut rest.dots);
                                }
                                changed |= ui.checkbox(&mut rest.confirmed, i18n::tr("Reviewed")).changed();
                                if changed {
                                    self.snapshot();
                                    self.page.score.rests[index] = rest;
                                }
                                if ui
                                    .button("\u{00d7}")
                                    .on_hover_text(i18n::tr("Delete rest (Delete / Backspace)"))
                                    .clicked()
                                {
                                    self.snapshot();
                                    self.page.score.rests.remove(index);
                                    self.select_object(None);
                                }
                                ui.separator();
                            }
                        });
                    });
            });
    }

    fn processing_overlay(&self, ui: &mut egui::Ui, viewport: Rect) {
        let (status, progress) = if let Some(pending) = &self.pending {
            (self.status.as_str(), pending.progress)
        } else {
            match &self.page.import {
                ImportState::Queued => (i18n::tr("Waiting for analysis"), None),
                ImportState::Running { stage, progress } => (*stage, *progress),
                _ => return,
            }
        };
        let width = (viewport.width() - 24.0).clamp(1.0, 360.0);
        let rect = Rect::from_min_size(
            Pos2::new(viewport.center().x - width / 2.0, viewport.top() + 24.0),
            Vec2::new(width, 160.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::Frame::new()
                    .fill(ui.visuals().window_fill)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(10.0)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.set_width((width - 32.0).max(1.0));
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.strong(if progress.is_some() {
                                i18n::tr("Recognition in progress")
                            } else {
                                i18n::tr("Processing")
                            });
                        });
                        ui.add(egui::Label::new(i18n::tr(status)).wrap());
                        if let Some(progress) = progress {
                            if let Some((current, total)) = progress.staff {
                                ui.weak(i18n::format(
                                    "Staff {current} of {total}",
                                    &[
                                        ("current", format!("{}", current)),
                                        ("total", format!("{}", total)),
                                    ],
                                ));
                            }
                            ui.add(
                                egui::ProgressBar::new(progress.fraction())
                                    .desired_width(ui.available_width())
                                    .text(i18n::format(
                                        "Step {0} of {1}",
                                        &[
                                            (
                                                "0",
                                                format!(
                                                    "{}",
                                                    (progress.completed_steps + 1)
                                                        .min(progress.total_steps)
                                                ),
                                            ),
                                            ("1", format!("{}", progress.total_steps)),
                                        ],
                                    )),
                            );
                        }
                    });
            },
        );
    }

    fn canvas(&mut self, context: &egui::Context) {
        let inherited_meter = self.inherited_time_signature();
        if self.page.texture.is_none() && self.pending.is_none() && self.page.image.width() > 0 {
            self.set_texture(context);
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::palette(context.style().visuals.dark_mode).canvas)
                    .inner_margin(20.0),
            )
            .show(context, |ui| {
                let viewport = ui.available_rect_before_wrap();
                if let ImportState::Failed(error) = &self.page.import {
                    let error = error.clone();
                    ui.vertical_centered(|ui| {
                        ui.heading(i18n::tr("Could not load page"));
                        ui.add(egui::Label::new(error).wrap());
                        if ui.button(i18n::tr("Try again")).clicked() {
                            self.retry_import();
                        }
                        if ui
                            .add_enabled(
                                self.pages.len() > 1,
                                egui::Button::new(i18n::tr("Remove page")),
                            )
                            .clicked()
                        {
                            self.remove_failed_page();
                        }
                    });
                    return;
                }
                if self.page.image.width() == 0 && self.pending.is_none() {
                    self.processing_overlay(ui, viewport);
                    return;
                }
                let image_size = self
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.image_size)
                    .unwrap_or_else(|| {
                        Vec2::new(
                            self.page.image.width() as f32,
                            self.page.image.height() as f32,
                        )
                    });
                let fitted_zoom = ((ui.available_width() - 12.0).max(1.0) / image_size.x.max(1.0))
                    .clamp(0.02, 3.0);
                let zoom = if self.page_busy() {
                    fitted_zoom
                } else {
                    if self.page.fit_width {
                        self.page.zoom = fitted_zoom;
                    }
                    self.page.zoom
                };
                egui::ScrollArea::both()
                    .id_salt(("page_canvas", self.page.id, self.page_busy()))
                    .scroll_source(egui::scroll_area::ScrollSource {
                        drag: false,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        let size = image_size * zoom;
                        let (rect, response) =
                            ui.allocate_exact_size(size, Sense::click_and_drag());
                        if self.page.reveal_selection && !self.page_busy() {
                            if let Some(staff) =
                                self.page.score.staves.get(self.page.selected_staff)
                            {
                                let x = match self.page.selected_object {
                                    Some(Object::Note(index)) => self.page.score.notes[index].x,
                                    Some(Object::Rest(index)) => self.page.score.rests[index].x,
                                    Some(Object::Barline(index)) => {
                                        self.page.score.barlines[index].x
                                    }
                                    Some(Object::Span(index)) => {
                                        self.page.score.spans[index].start[0]
                                    }
                                    Some(Object::KeyChange(index)) => {
                                        self.page.score.key_changes[index].x
                                    }
                                    Some(Object::TimeChange(index)) => {
                                        self.page.score.time_changes[index].x
                                    }
                                    Some(Object::Dynamic(index)) => {
                                        self.page.score.dynamics[index].x
                                    }
                                    Some(Object::RepeatMark(index)) => {
                                        self.page.score.repeat_marks[index].x
                                    }
                                    None => self
                                        .page
                                        .reveal_x
                                        .take()
                                        .unwrap_or((staff.left + staff.right) / 2.0),
                                };
                                ui.scroll_to_rect(
                                    Rect::from_center_size(
                                        rect.min
                                            + Vec2::new(x, (staff.lines[0] + staff.lines[4]) / 2.0)
                                                * self.page.zoom,
                                        Vec2::new(80.0, staff.spacing() * self.page.zoom * 8.0),
                                    ),
                                    Some(egui::Align::Center),
                                );
                            }
                            self.page.reveal_selection = false;
                        }
                        if let Some(texture) = self
                            .pending
                            .as_ref()
                            .and_then(|pending| pending.texture.as_ref())
                            .or(self.page.texture.as_ref())
                        {
                            ui.painter().image(
                                texture.id(),
                                rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        if self.page_busy() {
                            return;
                        }
                        let map = |x: f32, y: f32| rect.min + Vec2::new(x, y) * self.page.zoom;
                        for (staff_index, staff) in self.page.score.staves.iter().enumerate() {
                            for (index, measure) in self
                                .page
                                .score
                                .measure_validities_with_inherited(staff_index, inherited_meter)
                                .iter()
                                .enumerate()
                            {
                                crate::preview::paint_measure_validity(
                                    ui,
                                    measure,
                                    Rect::from_min_max(
                                        map(
                                            (measure.right - staff.spacing() * 1.8)
                                                .max(measure.left),
                                            staff.lines[0] - staff.spacing() * 0.3,
                                        ),
                                        map(measure.right, staff.lines[4] + staff.spacing() * 0.3),
                                    ),
                                    ui.id().with(("measure_validity", staff_index, index)),
                                );
                            }
                        }
                        for staff in &self.page.score.staves {
                            for &line in &staff.lines {
                                ui.painter().line_segment(
                                    [map(staff.left, line), map(staff.right, line)],
                                    Stroke::new(
                                        1.0_f32,
                                        Color32::from_rgba_unmultiplied(30, 125, 115, 90),
                                    ),
                                );
                            }
                            let color = if staff.header.confirmed {
                                Color32::from_rgb(0, 135, 85)
                            } else {
                                Color32::from_rgb(0, 105, 125)
                            };
                            let clef_label = match staff.header.clef {
                                Clef::Bass => i18n::tr("Bass"),
                                Clef::Treble => i18n::tr("Treble"),
                                Clef::Unknown => "?",
                            };
                            let time_label = staff
                                .header
                                .time_signature
                                .map(TimeSignature::label)
                                .unwrap_or_else(|| "?".into());
                            for (bounds, label) in [
                                (&staff.header.clef_bounds, clef_label),
                                (&staff.header.time_bounds, time_label.as_str()),
                            ] {
                                if let Some(bounds) = bounds {
                                    let symbol_rect = Rect::from_min_max(
                                        map(bounds.left, bounds.top),
                                        map(bounds.right, bounds.bottom),
                                    );
                                    ui.painter().rect_stroke(
                                        symbol_rect,
                                        2.0_f32,
                                        Stroke::new(2.0_f32, color),
                                        egui::StrokeKind::Outside,
                                    );
                                    ui.painter().text(
                                        symbol_rect.left_top() - Vec2::new(0.0, 3.0),
                                        egui::Align2::LEFT_BOTTOM,
                                        label,
                                        egui::FontId::proportional(12.0),
                                        color,
                                    );
                                }
                            }
                            let key_rect = staff
                                .header
                                .key_bounds
                                .iter()
                                .map(|bounds| {
                                    Rect::from_min_max(
                                        map(bounds.left, bounds.top),
                                        map(bounds.right, bounds.bottom),
                                    )
                                })
                                .reduce(Rect::union);
                            if let Some(symbol_rect) = key_rect {
                                ui.painter().rect_stroke(
                                    symbol_rect,
                                    2.0_f32,
                                    Stroke::new(2.0_f32, color),
                                    egui::StrokeKind::Outside,
                                );
                                if let Some(key) = staff.header.key_signature {
                                    ui.painter().text(
                                        symbol_rect.left_top() - Vec2::new(0.0, 3.0),
                                        egui::Align2::LEFT_BOTTOM,
                                        key.label(),
                                        egui::FontId::proportional(11.0),
                                        color,
                                    );
                                }
                            }
                        }
                        for (index, change) in self.page.score.key_changes.iter().enumerate() {
                            let staff = &self.page.score.staves[change.staff];
                            let color =
                                if self.page.selected_object == Some(Object::KeyChange(index)) {
                                    Color32::from_rgb(0, 95, 180)
                                } else if change.confirmed {
                                    Color32::from_rgb(0, 135, 85)
                                } else {
                                    Color32::from_rgb(155, 90, 0)
                                };
                            let mut rect = Rect::from_min_max(
                                map(change.x, staff.lines[0] - staff.spacing()),
                                map(change.x + staff.spacing(), staff.lines[4] + staff.spacing()),
                            );
                            for bounds in &change.bounds {
                                rect = rect.union(Rect::from_min_max(
                                    map(bounds.left, bounds.top),
                                    map(bounds.right, bounds.bottom),
                                ));
                            }
                            if change.confirmed {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0_f32,
                                    Color32::from_rgba_unmultiplied(0, 135, 85, 24),
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                2.0_f32,
                                Stroke::new(1.8_f32, color),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                rect.left_top() - Vec2::new(0.0, 3.0),
                                egui::Align2::LEFT_BOTTOM,
                                change.key_signature.label(),
                                egui::FontId::proportional(11.0),
                                color,
                            );
                        }
                        for (index, change) in self.page.score.time_changes.iter().enumerate() {
                            let staff = &self.page.score.staves[change.staff];
                            let color =
                                if self.page.selected_object == Some(Object::TimeChange(index)) {
                                    Color32::from_rgb(0, 95, 180)
                                } else if change.confirmed {
                                    Color32::from_rgb(0, 135, 85)
                                } else {
                                    Color32::from_rgb(155, 90, 0)
                                };
                            let mut rect = Rect::from_min_max(
                                map(change.x, staff.lines[0] - staff.spacing()),
                                map(change.x + staff.spacing(), staff.lines[4] + staff.spacing()),
                            );
                            for bounds in &change.bounds {
                                rect = rect.union(Rect::from_min_max(
                                    map(bounds.left, bounds.top),
                                    map(bounds.right, bounds.bottom),
                                ));
                            }
                            if change.confirmed {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0_f32,
                                    Color32::from_rgba_unmultiplied(0, 135, 85, 24),
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                2.0_f32,
                                Stroke::new(1.8_f32, color),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                rect.left_top() - Vec2::new(0.0, 3.0),
                                egui::Align2::LEFT_BOTTOM,
                                change.time_signature.label(),
                                egui::FontId::proportional(11.0),
                                color,
                            );
                        }
                        for (index, dynamic) in self.page.score.dynamics.iter().enumerate() {
                            let bounds = dynamic
                                .symbol_bounds(self.page.score.staves[dynamic.staff].spacing());
                            let rect = Rect::from_min_max(
                                map(bounds.left, bounds.top),
                                map(bounds.right, bounds.bottom),
                            );
                            let color = if self.page.selected_object == Some(Object::Dynamic(index))
                            {
                                Color32::from_rgb(0, 95, 180)
                            } else if dynamic.confirmed {
                                Color32::from_rgb(0, 135, 85)
                            } else {
                                Color32::from_rgb(155, 90, 0)
                            };
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                Stroke::new(1.8_f32, color),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                rect.left_top() - Vec2::new(0.0, 3.0),
                                egui::Align2::LEFT_BOTTOM,
                                dynamic.kind.label(),
                                egui::FontId::proportional(12.0),
                                color,
                            );
                        }
                        for (index, mark) in self.page.score.repeat_marks.iter().enumerate() {
                            let bounds =
                                mark.symbol_bounds(self.page.score.staves[mark.staff].spacing());
                            let rect = Rect::from_min_max(
                                map(bounds.left, bounds.top),
                                map(bounds.right, bounds.bottom),
                            );
                            let color =
                                if self.page.selected_object == Some(Object::RepeatMark(index)) {
                                    Color32::from_rgb(0, 95, 180)
                                } else if mark.confirmed {
                                    Color32::from_rgb(0, 135, 85)
                                } else {
                                    Color32::from_rgb(155, 90, 0)
                                };
                            if mark.kind == RepeatMarkKind::Ending {
                                crate::preview::paint_ending(
                                    ui.painter(),
                                    mark,
                                    self.page.score.staves[mark.staff].spacing(),
                                    |p| map(p[0], p[1]),
                                    color,
                                    self.page.zoom,
                                );
                            }
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                Stroke::new(1.8_f32, color),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                rect.left_top() - Vec2::new(0.0, 3.0),
                                egui::Align2::LEFT_BOTTOM,
                                mark.kind.label(),
                                egui::FontId::proportional(12.0),
                                color,
                            );
                        }
                        for (index, bar) in self.page.score.barlines.iter().enumerate() {
                            let staff = &self.page.score.staves[bar.staff];
                            let gap = (staff.spacing() * self.page.zoom).max(10.0);
                            let color = if self.page.selected_object == Some(Object::Barline(index))
                            {
                                Color32::from_rgb(0, 95, 180)
                            } else if bar.confirmed {
                                Color32::from_rgb(0, 135, 85)
                            } else {
                                Color32::from_rgb(185, 115, 30)
                            };
                            let (strokes, dots_left, dots_right): (&[(f32, bool)], bool, bool) =
                                match bar.kind {
                                    BarlineKind::Single => (&[(0.0, false)], false, false),
                                    BarlineKind::Double => {
                                        (&[(-0.35, false), (0.35, false)], false, false)
                                    }
                                    BarlineKind::Final => {
                                        (&[(-0.35, false), (0.35, true)], false, false)
                                    }
                                    BarlineKind::RepeatStart => {
                                        (&[(-0.35, true), (0.35, false)], false, true)
                                    }
                                    BarlineKind::RepeatEnd => {
                                        (&[(-0.35, false), (0.35, true)], true, false)
                                    }
                                    BarlineKind::RepeatBoth => {
                                        (&[(-0.55, false), (0.0, true), (0.55, false)], true, true)
                                    }
                                };
                            for &(offset, thick) in strokes {
                                let top = map(bar.x, staff.lines[0] - staff.spacing() * 0.3)
                                    + Vec2::new(offset * gap, 0.0);
                                let bottom = map(bar.x, staff.lines[4] + staff.spacing() * 0.3)
                                    + Vec2::new(offset * gap, 0.0);
                                ui.painter().line_segment(
                                    [top, bottom],
                                    Stroke::new(
                                        if thick {
                                            (gap * 0.18).max(3.0)
                                        } else {
                                            1.5_f32
                                        },
                                        color,
                                    ),
                                );
                            }
                            for (direction, present) in [(-1.0, dots_left), (1.0, dots_right)] {
                                if present {
                                    for position in [1.5, 2.5] {
                                        let center =
                                            map(bar.x, staff.lines[0] + staff.spacing() * position)
                                                + Vec2::new(direction * gap * 1.2, 0.0);
                                        ui.painter().circle_filled(
                                            center,
                                            (gap * 0.12).clamp(1.5, 3.5),
                                            color,
                                        );
                                    }
                                }
                            }
                        }
                        for (index, span) in self.page.score.spans.iter().enumerate() {
                            let selected = self.page.selected_object == Some(Object::Span(index));
                            crate::preview::paint_span(
                                ui.painter(),
                                span,
                                |point| map(point[0], point[1]),
                                if selected {
                                    Color32::from_rgb(0, 95, 180)
                                } else if span.confirmed {
                                    Color32::from_rgb(0, 135, 85)
                                } else {
                                    Color32::from_rgb(165, 65, 100)
                                },
                                1.8,
                            );
                            if selected {
                                for (right, point) in [(false, span.start), (true, span.end)] {
                                    let center = map(point[0], point[1]);
                                    ui.painter().circle_filled(center, 6.0, Color32::WHITE);
                                    ui.painter().circle_stroke(
                                        center,
                                        6.0,
                                        Stroke::new(2.0_f32, Color32::from_rgb(0, 95, 180)),
                                    );
                                    ui.interact(
                                        Rect::from_center_size(center, Vec2::splat(20.0)),
                                        ui.id().with(("anchor_hint", index, right)),
                                        Sense::hover(),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::Grab)
                                    .on_hover_text(if right {
                                        i18n::tr("Right anchor")
                                    } else {
                                        i18n::tr("Left anchor")
                                    });
                                }
                            }
                        }
                        for (index, rest) in self.page.score.rests.iter().enumerate() {
                            let color = if self.page.selected_rest == Some(index) {
                                Color32::from_rgb(0, 95, 180)
                            } else if rest.confirmed {
                                Color32::from_rgb(0, 135, 85)
                            } else {
                                Color32::from_rgb(165, 65, 100)
                            };
                            let symbol_rect = if let Some(bounds) = &rest.bounds {
                                Rect::from_min_max(
                                    map(bounds.left, bounds.top),
                                    map(bounds.right, bounds.bottom),
                                )
                            } else {
                                Rect::from_center_size(
                                    map(rest.x, rest.y),
                                    Vec2::new(1.3, 2.0)
                                        * (self.page.score.staves[rest.staff].spacing()
                                            * self.page.zoom)
                                            .max(8.0),
                                )
                            };
                            if rest.confirmed {
                                ui.painter().rect_filled(
                                    symbol_rect,
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(0, 135, 85, 24),
                                );
                            }
                            ui.painter().rect_stroke(
                                symbol_rect,
                                1.0_f32,
                                Stroke::new(2.0_f32, color),
                                egui::StrokeKind::Outside,
                            );
                            ui.painter().text(
                                symbol_rect.center_top() - Vec2::new(0.0, 3.0),
                                egui::Align2::CENTER_BOTTOM,
                                format!("{} {}", i18n::tr("R"), rest.duration_label()),
                                egui::FontId::proportional(12.0),
                                color,
                            );
                        }
                        for (index, note) in self.page.score.notes.iter().enumerate() {
                            let color = if self.page.selected == Some(index) {
                                Color32::from_rgb(0, 95, 180)
                            } else if note.confirmed {
                                Color32::from_rgb(0, 135, 85)
                            } else {
                                Color32::from_rgb(200, 100, 0)
                            };
                            let center = map(note.x, note.y);
                            if note.confirmed {
                                ui.painter().circle_filled(
                                    center,
                                    (self.page.score.staves[note.staff].spacing()
                                        * self.page.zoom
                                        * 0.75)
                                        .max(6.0),
                                    Color32::from_rgba_unmultiplied(0, 135, 85, 30),
                                );
                            }
                            ui.painter().circle_stroke(
                                center,
                                (self.page.score.staves[note.staff].spacing()
                                    * self.page.zoom
                                    * 0.75)
                                    .max(6.0),
                                Stroke::new(2.0_f32, color),
                            );
                            if self.page.selected == Some(index) {
                                ui.painter().text(
                                    center + Vec2::new(0.0, -20.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    note.pitch_label(),
                                    egui::FontId::proportional(14.0),
                                    color,
                                );
                            }
                        }
                        if response.secondary_clicked()
                            && let Some(pointer) = response.interact_pointer_pos()
                        {
                            let position = (pointer - rect.min) / self.page.zoom;
                            let object = self.hit_object(position).map(|target| match target {
                                DragTarget::Body(object) => object,
                                DragTarget::Anchor(index, _) => Object::Span(index),
                            });
                            let show_object_list = self.page.show_object_list;
                            self.select_object(object);
                            self.page.show_object_list = show_object_list;
                            self.page.object_drag = None;
                            self.page.canvas_context = Some((position, object));
                        }
                        egui::Popup::context_menu(&response)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| self.canvas_context_menu(ui));
                        if let Some(pointer) = response.hover_pos()
                            && self
                                .hit_object((pointer - rect.min) / self.page.zoom)
                                .is_some()
                        {
                            context.set_cursor_icon(if self.page.object_drag.is_some() {
                                egui::CursorIcon::Grabbing
                            } else {
                                egui::CursorIcon::Grab
                            });
                        }
                        if let Some(pointer) = response.interact_pointer_pos() {
                            let position = (pointer - rect.min) / self.page.zoom;
                            if response.clicked() {
                                let target = self.hit_object(position);
                                self.select_object(target.map(|target| match target {
                                    DragTarget::Body(object) => object,
                                    DragTarget::Anchor(index, _) => Object::Span(index),
                                }));
                                if target.is_none()
                                    && let Some((index, _)) =
                                        self.page.score.staves.iter().enumerate().min_by(
                                            |(_, first), (_, second)| {
                                                (first.lines[2] - position.y).abs().total_cmp(
                                                    &(second.lines[2] - position.y).abs(),
                                                )
                                            },
                                        )
                                {
                                    self.page.selected_staff = index;
                                }
                            }
                            if response.drag_started() {
                                let origin = context
                                    .input(|input| input.pointer.press_origin())
                                    .map_or(position, |origin| {
                                        (origin - rect.min) / self.page.zoom
                                    });
                                if let Some(target) = self.hit_object(origin) {
                                    self.select_object(Some(match target {
                                        DragTarget::Body(object) => object,
                                        DragTarget::Anchor(index, _) => Object::Span(index),
                                    }));
                                    self.snapshot();
                                    self.page.object_drag = Some(ObjectDrag {
                                        target,
                                        origin,
                                        score: self.page.score.clone(),
                                    });
                                }
                            }
                            if response.dragged() || response.drag_stopped() {
                                self.move_object(position);
                            }
                        }
                        if !context.input(|input| input.pointer.primary_down()) {
                            self.page.object_drag = None;
                        }
                    });
                self.processing_overlay(ui, viewport);
            });
    }

    fn nearest_note(&self, position: Vec2) -> Option<usize> {
        self.page
            .score
            .notes
            .iter()
            .enumerate()
            .map(|(index, note)| (index, (Vec2::new(note.x, note.y) - position).length()))
            .filter(|(_, distance)| *distance * self.page.zoom < 18.0)
            .min_by(|first, second| first.1.total_cmp(&second.1))
            .map(|(index, _)| index)
    }
}

impl eframe::App for App {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_job(context);
        self.poll_imports(context);
        self.shortcuts(context);
        if context.input(|input| input.viewport().close_requested()) && !self.confirm_discard() {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.toolbar(context);
        self.page_tabs(context);
        self.problem_status(context);
        self.inspector(context);
        self.problems.open_system = false;
        self.problem_panel(context);
        self.preview(context);
        self.canvas(context);
        let previous_language = i18n::language();
        if self.settings.show(context, &mut self.show_settings) {
            self.settings.apply(context);
            if i18n::language() != previous_language {
                self.problems.invalidate();
                if !self.status_error {
                    self.status = self
                        .pending
                        .as_ref()
                        .map(|pending| {
                            i18n::tr(
                                pending
                                    .progress
                                    .map_or("Processing ...", |progress| progress.stage),
                            )
                            .to_owned()
                        })
                        .unwrap_or_default();
                }
            }
            if let Err(error) = self.settings.save() {
                self.status = i18n::format("Could not save settings: {error}", &[("error", error)]);
                self.status_error = true;
            }
            context.request_repaint();
        }
        if self.problems.needs_refresh() && self.pending.is_none() {
            context.request_repaint();
        }
        #[cfg(feature = "ui-screenshot")]
        crate::screenshot::capture(context);
    }
}

#[cfg(test)]
fn read_image(path: &std::path::Path) -> Result<GrayImage, String> {
    decode_image(image::ImageReader::open(path).map_err(|error| {
        i18n::format(
            "Cannot read image: {error}",
            &[("error", format!("{}", error))],
        )
    })?)
    .map(recognition::straighten)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libreomr::model::Project;

    #[test]
    #[ignore = "requires LIBREOMR_TEST_PDF"]
    fn external_pdf_first_staff_accidentals_remain_transparent_after_switching() {
        let path = PathBuf::from(std::env::var_os("LIBREOMR_TEST_PDF").unwrap());
        let page = open_document(&path).unwrap().remove(0);
        let image = recognition::straighten(page.render().unwrap());
        let score = recognition::recognize(&image);
        let context = egui::Context::default();
        App::configure_style(&context);
        context.set_theme(egui::Theme::Light);
        let mut app = demo_app();
        app.page.image = image;
        app.page.score = score;
        app.page.preview = crate::preview::Preview::fitted();
        let mut textures = std::collections::HashMap::new();
        for staff in [0, 0, 1, 0] {
            app.page.selected_staff = staff;
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 820.0))),
                    ..Default::default()
                },
                |context| {
                    app.toolbar(context);
                    app.page_tabs(context);
                    app.problem_status(context);
                    app.inspector(context);
                    app.preview(context);
                    app.canvas(context);
                },
            );
            for (id, delta) in &output.textures_delta.set {
                if delta.pos.is_none() {
                    let egui::ImageData::Color(image) = &delta.image;
                    textures.insert(*id, image.clone());
                }
            }
            let mut checked = 0;
            for shape in &output.shapes {
                if let egui::Shape::Mesh(mesh) = &shape.shape {
                    let is_flat = context
                        .tex_manager()
                        .read()
                        .meta(mesh.texture_id)
                        .is_some_and(|meta| meta.name.starts_with("preview-\u{e260}-"));
                    if !is_flat {
                        continue;
                    }
                    let image = &textures[&mesh.texture_id];
                    assert!(
                        image.pixels.iter().any(|pixel| pixel.a() == 0),
                        "flat on staff {staff} must have transparent surroundings"
                    );
                    assert!(
                        image.pixels.iter().any(|pixel| pixel.a() >= 200),
                        "flat on staff {staff} must have clear ink strokes"
                    );
                    checked += 1;
                }
            }
            assert!(
                checked >= 3,
                "the source PDF must render its three-flat signature"
            );
        }
    }

    #[test]
    #[ignore = "requires local original scan"]
    fn external_repeat_marks() {
        let path = std::env::var("LIBREOMR_HOLY_NIGHT_TEST_IMAGE").unwrap();
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(
            score
                .repeat_marks
                .iter()
                .map(|mark| (mark.staff, mark.kind))
                .collect::<Vec<_>>(),
            [
                (3, RepeatMarkKind::Segno),
                (7, RepeatMarkKind::Coda),
                (9, RepeatMarkKind::Segno),
                (10, RepeatMarkKind::Coda)
            ]
        );
        for (mark, x) in score
            .repeat_marks
            .iter()
            .zip([465.0, 3433.5, 4452.5, 355.0])
        {
            assert!((mark.x - x).abs() < 8.0);
            assert!(!mark.confirmed);
            let bounds = mark.bounds.as_ref().unwrap();
            assert!(
                !score
                    .notes
                    .iter()
                    .any(|note| note.staff == mark.staff && bounds.contains(note.x, note.y))
            );
        }
        let restored: Score =
            serde_json::from_value(serde_json::to_value(&score).unwrap()).unwrap();
        assert!(restored.validate().is_ok());
        assert_eq!(restored.repeat_marks.len(), 4);
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE and LIBREOMR_SUITE_TEST_IMAGE reference scans"]
    fn external_repeat_marks_reject_rehearsal_boxes() {
        for variable in ["LIBREOMR_TEST_IMAGE", "LIBREOMR_SUITE_TEST_IMAGE"] {
            let path = std::env::var(variable).unwrap();
            let image = read_image(std::path::Path::new(&path)).unwrap();
            let score = recognition::recognize(&image);
            assert!(
                score.repeat_marks.is_empty(),
                "{variable}: {:?}",
                score.repeat_marks
            );
        }
    }

    #[test]
    fn import_alignment_keeps_project_pixels_unchanged() {
        let mut image = GrayImage::from_pixel(1000, 400, image::Luma([255]));
        for row in [100, 120, 140, 160, 180] {
            for column in 50..950 {
                image.put_pixel(column, row, image::Luma([0]));
            }
        }
        let tilted = imageproc::geometric_transformations::rotate_about_center(
            &image,
            0.008,
            imageproc::geometric_transformations::Interpolation::Bilinear,
            image::Luma([255]),
        );
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(tilted.clone())
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let mut file = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        file.write_all(png.get_ref()).unwrap();
        let decoded = decode_image(image::ImageReader::new(Cursor::new(png.into_inner()))).unwrap();
        assert_eq!(
            decoded, tilted,
            "saved project pixels must not be transformed"
        );
        let imported = read_image(file.path()).unwrap();
        assert_ne!(imported, tilted);
        assert_eq!(imported.dimensions(), tilted.dimensions());
        assert_eq!(recognition::recognize(&imported).staves.len(), 1);
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to a bass-clef score in common time"]
    fn recognizes_external_bass_common_time_header() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        for (index, staff) in score.staves.iter().enumerate() {
            println!("Staff {}: {:?}", index + 1, staff.header);
        }
        let first = score.staves.first().expect("staff detected");
        assert_eq!(first.header.clef, libreomr::model::Clef::Bass);
        assert_eq!(
            first.header.time_signature,
            Some(libreomr::model::TimeSignature::Common)
        );
        let clef = first.header.clef_bounds.as_ref().expect("clef bounds");
        let meter = first.header.time_bounds.as_ref().expect("time bounds");
        assert!(
            score
                .notes
                .iter()
                .filter(|note| note.staff == 0)
                .all(|note| !clef.contains(note.x, note.y) && !meter.contains(note.x, note.y))
        );
        assert!(
            score
                .staves
                .iter()
                .all(|staff| staff.header.clef == libreomr::model::Clef::Bass)
        );
        assert!(score.validate().is_ok());
        println!(
            "{} staves; {} note candidates",
            score.staves.len(),
            score.notes.len()
        );
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia baritone scan"]
    fn external_dynamics_and_marcato() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let actual: Vec<_> = score
            .dynamics
            .iter()
            .map(|dynamic| (dynamic.staff, dynamic.kind))
            .collect();
        assert_eq!(
            actual,
            [
                (0, DynamicKind::Pp),
                (1, DynamicKind::F),
                (3, DynamicKind::P),
                (3, DynamicKind::Mf),
                (4, DynamicKind::Mf),
                (6, DynamicKind::Mf),
                (7, DynamicKind::F),
                (8, DynamicKind::F),
                (9, DynamicKind::Mf)
            ]
        );
        for (dynamic, x) in score.dynamics.iter().zip([
            661.0, 675.0, 961.0, 1535.5, 1109.0, 1314.5, 1377.5, 1917.0, 2886.0,
        ]) {
            assert!((dynamic.x - x).abs() < 12.0, "{dynamic:?}");
            assert!(!dynamic.confirmed);
            let bounds = dynamic.bounds.as_ref().unwrap();
            assert!(
                !score
                    .notes
                    .iter()
                    .any(|note| note.staff == dynamic.staff && bounds.contains(note.x, note.y))
            );
        }
        assert!(score.notes.iter().any(|note| note.staff == 8
            && (note.x - 1095.5).abs() < 15.0
            && note.articulation == Articulation::Marcato));
        let restored: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&score).unwrap()
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_HOLY_NIGHT_TEST_IMAGE pointing to the Holy Night bass scan"]
    fn external_holy_night_time_changes_and_staff_start_bars() {
        let path = std::env::var_os("LIBREOMR_HOLY_NIGHT_TEST_IMAGE")
            .expect("set LIBREOMR_HOLY_NIGHT_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.staves.len(), 12);
        for (index, staff) in score.staves.iter().enumerate() {
            assert_eq!(staff.header.clef, Clef::Bass, "staff {index}");
            let clef = staff
                .header
                .clef_bounds
                .as_ref()
                .unwrap_or_else(|| panic!("staff {index}: {staff:?}"));
            assert!(
                score
                    .barlines
                    .iter()
                    .filter(|bar| bar.staff == index)
                    .all(|bar| bar.x > clef.right),
                "staff {index}: bars before the clef"
            );
        }
        let actual: Vec<_> = score
            .time_changes
            .iter()
            .map(|change| (change.staff, change.time_signature))
            .collect();
        let expected: Vec<_> = [0, 0, 2, 2, 3, 3, 9, 9, 11, 11]
            .into_iter()
            .enumerate()
            .map(|(index, staff)| {
                (
                    staff,
                    TimeSignature::Meter {
                        beats: if index % 2 == 0 { 2 } else { 4 },
                        beat_unit: 4,
                    },
                )
            })
            .collect();
        assert_eq!(actual, expected);
        for (change, x) in score.time_changes.iter().zip([
            3755.0, 4409.0, 2634.0, 3459.0, 3727.0, 4399.0, 1880.0, 2944.0, 2632.0, 3233.0,
        ]) {
            assert!((change.x - x).abs() < 30.0, "{change:?}");
            assert!(!change.confirmed);
            let gap = score.staves[change.staff].spacing();
            assert!(score.barlines.iter().any(|bar| bar.staff == change.staff
                && bar.kind == BarlineKind::Double
                && bar.x < change.x
                && change.x - bar.x < gap));
            assert!(
                !score.notes.iter().any(|note| note.staff == change.staff
                    && change
                        .bounds
                        .iter()
                        .any(|bounds| bounds.contains(note.x, note.y))),
                "meter digits became notes: {change:?}"
            );
        }
        assert_eq!(
            score
                .dynamics
                .iter()
                .map(|dynamic| (dynamic.staff, dynamic.kind))
                .collect::<Vec<_>>(),
            [
                (0, DynamicKind::Mf),
                (5, DynamicKind::F),
                (8, DynamicKind::Mf),
                (10, DynamicKind::Ff)
            ]
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_SAX_TEST_IMAGE pointing to the Atlantic Avenue alto sax scan"]
    fn external_atlantic_sax_staves() {
        let path =
            std::env::var_os("LIBREOMR_SAX_TEST_IMAGE").expect("set LIBREOMR_SAX_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.staves.len(), 7, "{:?}", score.staves);
        assert!(score.staves[0].lines[0] < image.height() as f32 * 0.16);
        assert!(score.staves[1].lines[0] < image.height() as f32 * 0.3);
        assert_eq!(score.staves[0].header.clef, Clef::Treble);
        assert_eq!(score.staves[1].header.clef, Clef::Treble);
        assert_eq!(
            score.staves[0].header.time_signature,
            Some(TimeSignature::Meter {
                beats: 12,
                beat_unit: 8
            })
        );
        assert_eq!(
            score.staves[0].header.key_signature,
            Some(KeySignature { fifths: 1 })
        );
        for staff in 0..7 {
            assert!(
                score.notes.iter().any(|note| note.staff == staff),
                "staff {staff}"
            );
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_SUITE_TEST_IMAGE pointing to the Little Concert Suite scan"]
    fn external_concert_suite_staves() {
        let path =
            std::env::var_os("LIBREOMR_SUITE_TEST_IMAGE").expect("set LIBREOMR_SUITE_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.staves.len(), 11, "{:?}", score.staves);
        assert!(score.staves[0].lines[0] < image.height() as f32 * 0.18);
        assert!(score.staves[10].lines[0] > image.height() as f32 * 0.85);
        assert_eq!(score.staves[0].header.clef, Clef::Treble);
        assert_eq!(
            score.staves[0].header.time_signature,
            Some(TimeSignature::Meter {
                beats: 4,
                beat_unit: 4
            })
        );
        assert_eq!(
            score.staves[0].header.key_signature,
            Some(KeySignature { fifths: -1 })
        );
        for staff in 0..11 {
            assert!(
                score.notes.iter().any(|note| note.staff == staff),
                "staff {staff}"
            );
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_OBOE_TEST_IMAGE pointing to the Atlantic Avenue scan"]
    fn external_atlantic_header_and_symbols() {
        let path =
            std::env::var_os("LIBREOMR_OBOE_TEST_IMAGE").expect("set LIBREOMR_OBOE_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.staves.len(), 10);
        assert_eq!(score.staves[0].header.clef, Clef::Treble);
        assert_eq!(
            score.staves[0].header.time_signature,
            Some(TimeSignature::Meter {
                beats: 12,
                beat_unit: 8
            })
        );
        assert!(
            score
                .staves
                .iter()
                .all(|staff| staff.header.clef == Clef::Treble)
        );
        assert_eq!(
            score.staves[0].header.key_signature,
            Some(KeySignature { fifths: -2 })
        );
        assert_eq!(
            score.notes.iter().filter(|note| note.staff == 0).count(),
            17,
            "{:?}: {:?}",
            score.staves[0],
            score
                .notes
                .iter()
                .filter(|note| note.staff == 0)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            score.rests.iter().filter(|rest| rest.staff == 0).count(),
            14
        );
        assert_eq!(
            score
                .rests
                .iter()
                .filter_map(|rest| rest.measures)
                .collect::<Vec<_>>(),
            vec![4, 8, 8, 8, 8]
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_OBOE_TEST_IMAGE pointing to the Atlantic Avenue scan"]
    fn external_atlantic_beams_are_not_notes() {
        let path =
            std::env::var_os("LIBREOMR_OBOE_TEST_IMAGE").expect("set LIBREOMR_OBOE_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let mut false_heads = Vec::new();
        for (staff, column, row) in [
            (3, 1680.0, 2495.0),
            (3, 3880.0, 2670.0),
            (3, 4785.0, 2470.0),
        ] {
            false_heads.extend(score.notes.iter().filter(|note| {
                note.staff == staff && (note.x - column).abs() < 45.0 && (note.y - row).abs() < 30.0
            }));
        }
        assert!(
            false_heads.is_empty(),
            "beam ends must not become notes: {false_heads:?}"
        );
        for (column, row) in [(1645.0, 2627.0), (3925.0, 2551.0), (4754.0, 2589.0)] {
            let actual: Vec<_> = score
                .notes
                .iter()
                .filter(|note| {
                    note.staff == 3 && (note.x - column).abs() < 30.0 && (note.y - row).abs() < 20.0
                })
                .collect();
            assert_eq!(actual.len(), 1, "real head {column},{row}: {actual:?}");
            assert_eq!(actual[0].duration, Duration::Eighth, "{:?}", actual[0]);
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_OBOE_TEST_IMAGE pointing to the Atlantic Avenue scan"]
    fn external_atlantic_inline_key_changes() {
        let path =
            std::env::var_os("LIBREOMR_OBOE_TEST_IMAGE").expect("set LIBREOMR_OBOE_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.key_changes.len(), 4, "{:?}", score.key_changes);
        for (staff, column, fifths) in [
            (2, 3938.0, -4),
            (4, 4126.0, -1),
            (5, 1171.0, -4),
            (7, 1717.0, -2),
        ] {
            let change = score
                .key_changes
                .iter()
                .find(|change| change.staff == staff)
                .unwrap();
            assert_eq!(change.key_signature.fifths, fifths);
            assert!((change.x - column).abs() < score.staves[staff].spacing() * 3.0);
            assert!(
                !score
                    .notes
                    .iter()
                    .filter(|note| note.staff == staff)
                    .any(|note| change
                        .bounds
                        .iter()
                        .any(|bounds| bounds.contains(note.x, note.y)))
            );
            let following: Vec<_> = score
                .notes
                .iter()
                .filter(|note| {
                    note.staff == staff && note.x > change.x && !note.accidental_explicit
                })
                .collect();
            if staff == 4 {
                assert!(following.is_empty());
                assert!(score.rests.iter().any(|rest| rest.staff == staff
                    && rest.x > change.x
                    && rest.measures.is_some()));
            } else {
                assert!(!following.is_empty(), "staff {staff}");
            }
            for note in following {
                let explicit = score
                    .notes
                    .iter()
                    .filter(|other| {
                        other.staff == staff
                            && other.accidental_explicit
                            && other.pitch == note.pitch
                            && other.x >= change.x
                            && other.x <= note.x
                    })
                    .any(|other| {
                        !score
                            .barlines
                            .iter()
                            .any(|bar| bar.staff == staff && bar.x > other.x && bar.x <= note.x)
                    });
                if !explicit {
                    assert_eq!(
                        note.accidental,
                        change.key_signature.accidental_at(note.pitch),
                        "{note:?}"
                    );
                }
            }
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires both Cassiopeia reference scans"]
    fn external_cassiopeia_has_no_inline_key_changes() {
        for variable in ["LIBREOMR_TEST_IMAGE", "LIBREOMR_BASS_TEST_IMAGE"] {
            let path =
                std::env::var_os(variable).expect("set reference image environment variables");
            let score = recognition::recognize(&read_image(std::path::Path::new(&path)).unwrap());
            assert!(
                score.key_changes.is_empty(),
                "{variable}: {:?}",
                score.key_changes
            );
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_rests_and_signature() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(score.staves.len(), 11);
        assert!(
            score
                .staves
                .iter()
                .all(|staff| staff.header.key_signature == Some(KeySignature { fifths: 1 }))
        );
        for rest in &score.rests {
            println!(
                "Staff {} rest {:?}: {}, {}",
                rest.staff + 1,
                rest.duration,
                rest.x,
                rest.y
            );
        }
        assert!(score.rests.iter().any(|rest| rest.staff == 3
            && rest.duration == Duration::Quarter
            && (700.0..850.0).contains(&rest.x)));
        assert!(score.rests.iter().any(|rest| rest.staff == 3
            && rest.duration == Duration::Eighth
            && (600.0..750.0).contains(&rest.x)));
        assert!(score.rests.iter().any(|rest| rest.staff == 4
            && rest.duration == Duration::Half
            && (700.0..900.0).contains(&rest.x)));
        for rest in &score.rests {
            let bounds = rest.bounds.as_ref().unwrap();
            assert!(
                !score
                    .notes
                    .iter()
                    .any(|note| note.staff == rest.staff && bounds.contains(note.x, note.y))
            );
        }
        assert!(score.validate().is_ok());
        println!(
            "{} notes; {} rest candidates",
            score.notes.len(),
            score.rests.len()
        );
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_note_values() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        for note in score.notes.iter().filter(|note| note.staff < 2) {
            println!(
                "Staff {}: {} {:?} at {}, {}",
                note.staff + 1,
                note.pitch_label(),
                note.duration,
                note.x,
                note.y
            );
        }
        for (staff, x, expected) in [
            (0, 665.0, Duration::Half),
            (0, 1738.0, Duration::Whole),
            (0, 2567.0, Duration::Quarter),
            (1, 1110.0, Duration::Eighth),
            (1, 1305.0, Duration::Sixteenth),
        ] {
            let nearby: Vec<_> = score
                .notes
                .iter()
                .filter(|note| note.staff == staff && (note.x - x).abs() < 35.0)
                .collect();
            assert_eq!(nearby.len(), 1, "staff {}, x {x}: {nearby:?}", staff + 1);
            assert_eq!(
                nearby[0].duration,
                expected,
                "staff {}, x {x}: {:?}",
                staff + 1,
                nearby[0]
            );
        }
        println!("{} note candidates", score.notes.len());
        assert_eq!(
            score.notes.iter().filter(|note| note.staff == 0).count(),
            12
        );
        assert_eq!(
            score
                .notes
                .iter()
                .find(|note| note.staff == 0 && (note.x - 2262.0).abs() < 30.0)
                .unwrap()
                .dots,
            1
        );
        assert_eq!(
            score
                .notes
                .iter()
                .find(|note| note.staff == 1 && (note.x - 1110.0).abs() < 30.0)
                .unwrap()
                .dots,
            1
        );
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_missing_nested_and_continued_bows() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        for (measure, staff, start, end) in [
            (17, 3, 973.0, 1254.0),
            (49, 8, 3785.0, 4069.0),
            (55, 9, 3852.0, 3980.0),
            (59, 10, 2609.0, 2737.0),
            (61, 10, 3569.0, 3721.0),
        ] {
            assert!(
                score.spans.iter().any(|span| span.staff == staff
                    && matches!(span.kind, SpanKind::Slur | SpanKind::Tie)
                    && (span.start[0] - start).abs() < 55.0
                    && (span.end[0] - end).abs() < 55.0),
                "missing bow in measure {measure}"
            );
        }
        let nested: Vec<_> = score
            .spans
            .iter()
            .filter(|span| {
                span.staff == 8
                    && span.start[0] > 3230.0
                    && span.start[0] < 3340.0
                    && matches!(span.kind, SpanKind::Slur | SpanKind::Tie)
            })
            .collect();
        assert!(
            nested
                .iter()
                .any(|span| (span.end[0] - 3383.0).abs() < 40.0),
            "missing inner bow in measure 48: {nested:?}"
        );
        assert!(
            nested
                .iter()
                .any(|span| (span.end[0] - 3500.0).abs() < 40.0),
            "missing outer bow in measure 48: {nested:?}"
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_short_notes_do_not_create_half_notes() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let notes: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.staff == 3 && (1450.0..2380.0).contains(&note.x))
            .collect();
        for note in &notes {
            println!(
                "{} {:?}: {}, {}",
                note.pitch_label(),
                note.duration,
                note.x,
                note.y
            );
        }
        assert!(!notes.is_empty());
        assert!(
            notes
                .iter()
                .all(|note| !matches!(note.duration, Duration::Half | Duration::Whole)),
            "{notes:?}"
        );
        assert_eq!(notes.len(), 8);
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_note_accidentals() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let score: Score = serde_json::from_slice(&serde_json::to_vec(&score).unwrap()).unwrap();
        let explicit: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.accidental_explicit)
            .collect();
        for note in &explicit {
            println!(
                "Staff {} {} {:?} at {}, {}",
                note.staff + 1,
                note.pitch_label(),
                note.duration,
                note.x,
                note.y
            );
        }
        for (staff, center, accidental) in [
            (9, 2893.0, -1),
            (9, 3646.0, 0),
            (10, 396.0, -1),
            (10, 1464.0, -1),
            (10, 1737.0, -1),
            (10, 2423.0, 0),
        ] {
            let nearby: Vec<_> = score
                .notes
                .iter()
                .filter(|note| note.staff == staff && (note.x - center).abs() < 25.0)
                .collect();
            assert_eq!(
                nearby.len(),
                1,
                "staff {}, x {center}: {nearby:?}",
                staff + 1
            );
            assert!(nearby[0].accidental_explicit, "{:?}", nearby[0]);
            assert_eq!(nearby[0].accidental, accidental, "{:?}", nearby[0]);
        }
        assert!(explicit.iter().all(|note| note.staff >= 9), "{explicit:?}");
        assert_eq!(explicit.len(), 6, "{explicit:?}");
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_eighth_in_tenth_staff_is_not_duplicated() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let notes: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.staff == 9 && (2800.0..3500.0).contains(&note.x))
            .collect();
        for note in &notes {
            println!(
                "{} {:?} {}, {}",
                note.pitch_label(),
                note.duration,
                note.x,
                note.y
            );
        }
        assert_eq!(notes.len(), 4, "{notes:?}");
        for (note, (center, duration)) in notes.iter().zip([
            (2893.0, Duration::Quarter),
            (3106.0, Duration::Eighth),
            (3231.0, Duration::Quarter),
            (3399.0, Duration::Quarter),
        ]) {
            assert!((note.x - center).abs() < 25.0, "{note:?}");
            assert_eq!(note.duration, duration, "{note:?}");
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_hairpins_and_bows() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert!(!score.spans.is_empty());
        for (staff, kind, start) in [
            (1, libreomr::model::SpanKind::Crescendo, 785.0),
            (3, libreomr::model::SpanKind::Decrescendo, 309.0),
            (8, libreomr::model::SpanKind::Decrescendo, 342.0),
            (0, libreomr::model::SpanKind::Tie, 908.0),
            (9, libreomr::model::SpanKind::Slur, 1582.0),
            (10, libreomr::model::SpanKind::Tie, 596.0),
        ] {
            assert!(
                score.spans.iter().any(|span| span.staff == staff
                    && span.kind == kind
                    && (span.start[0] - start).abs() < 30.0),
                "staff {staff}, {kind:?}, x {start}: {:?}",
                score.spans
            );
        }
        assert!(
            !score.spans.iter().any(|span| (4..=7).contains(&span.staff)),
            "bogenfreie Zeilen: {:?}",
            score.spans
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_BASS_TEST_IMAGE pointing to the BassCf reference scan"]
    fn external_bass_eighth_rest_in_30() {
        let path =
            std::env::var_os("LIBREOMR_BASS_TEST_IMAGE").expect("set LIBREOMR_BASS_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let rests: Vec<_> = score
            .rests
            .iter()
            .filter(|rest| rest.staff == 5 && (rest.x - 2660.0).abs() < 65.0)
            .collect();
        assert_eq!(rests.len(), 1, "{rests:?}");
        assert_eq!(rests[0].duration, Duration::Eighth);
        assert_eq!(rests[0].measures, None);
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_BASS_TEST_IMAGE pointing to the BassCf reference scan"]
    fn external_bass_decrescendo_in_37() {
        let path =
            std::env::var_os("LIBREOMR_BASS_TEST_IMAGE").expect("set LIBREOMR_BASS_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert!(
            score.spans.iter().any(|span| span.staff == 7
                && span.kind == SpanKind::Decrescendo
                && (span.start[0] - 1340.0).abs() < 65.0),
            "{:?}",
            score.spans
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_BASS_TEST_IMAGE pointing to the BassCf reference scan"]
    fn external_bass_multimeasure_rest_in_62() {
        let path =
            std::env::var_os("LIBREOMR_BASS_TEST_IMAGE").expect("set LIBREOMR_BASS_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let rests: Vec<_> = score
            .rests
            .iter()
            .filter(|rest| rest.staff == 10 && (rest.x - 1965.0).abs() < 100.0)
            .collect();
        assert_eq!(rests.len(), 1, "{rests:?}");
        assert_eq!(rests[0].measures, Some(3));
        assert!(
            !score
                .notes
                .iter()
                .any(|note| note.staff == 10 && (note.x - 1965.0).abs() < 180.0)
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_multimeasure_rest_at_38_is_not_a_note() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let nearby: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.staff == 8 && (1300.0..1800.0).contains(&note.x))
            .collect();
        assert!(nearby.is_empty(), "{nearby:?}");
        let rests: Vec<_> = score
            .rests
            .iter()
            .filter(|rest| rest.staff == 8 && (rest.x - 1545.0).abs() < 30.0)
            .collect();
        assert_eq!(rests.len(), 1, "{:?}", score.rests);
        assert_eq!(rests[0].measures, Some(8), "{:?}", rests[0]);
        assert_eq!(rests[0].duration_label(), "8 Takte");
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_double_before_38_and_both_repeats() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        for (staff, center, expected) in [
            (8, 1238.5, BarlineKind::Double),
            (9, 2740.5, BarlineKind::RepeatStart),
            (10, 4277.0, BarlineKind::RepeatEnd),
        ] {
            let nearby: Vec<_> = score
                .barlines
                .iter()
                .filter(|bar| bar.staff == staff && (bar.x - center).abs() < 3.0)
                .collect();
            assert_eq!(
                nearby.len(),
                1,
                "staff {}, x {center}: {nearby:?}",
                staff + 1
            );
            assert_eq!(nearby[0].kind, expected, "{:?}", nearby[0]);
        }
        assert!(
            !score
                .notes
                .iter()
                .any(|note| note.staff == 8 && (note.x - 1230.0).abs() < 50.0)
        );
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_barlines_and_accidental_carry() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        assert_eq!(
            score.barlines.last().unwrap().kind,
            BarlineKind::RepeatEnd,
            "{:?}",
            score.barlines.last()
        );
        assert_eq!(
            score
                .barlines
                .iter()
                .find(|bar| bar.staff == 9 && (bar.x - 2740.5).abs() < 3.0)
                .unwrap()
                .kind,
            BarlineKind::RepeatStart
        );
        for staff in 0..score.staves.len() {
            let positions: Vec<_> = score
                .barlines
                .iter()
                .filter(|bar| bar.staff == staff)
                .map(|bar| bar.x)
                .collect();
            println!("Staff {}: {positions:?}", staff + 1);
            assert_eq!(
                positions.len(),
                [7, 4, 4, 5, 4, 4, 4, 4, 6, 6, 6][staff],
                "staff {}: {positions:?}",
                staff + 1
            );
        }
        for center in [2740.5, 3541.5] {
            assert_eq!(
                score
                    .barlines
                    .iter()
                    .filter(|bar| bar.staff == 9 && (bar.x - center).abs() < 3.0)
                    .count(),
                1
            );
        }
        for (center, expected) in [
            (3106.0, -1),
            (3231.0, -1),
            (3399.0, -1),
            (3857.0, 0),
            (3981.0, 0),
        ] {
            let note = score
                .notes
                .iter()
                .find(|note| note.staff == 9 && (note.x - center).abs() < 25.0)
                .unwrap();
            assert!(!note.accidental_explicit, "{note:?}");
            assert_eq!(note.accidental, expected, "{note:?}");
        }
        assert!(score.validate().is_ok());
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_eighth_notes_in_sixth_staff() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        let notes: Vec<_> = score
            .notes
            .iter()
            .filter(|note| note.staff == 5 && note.x < 1700.0)
            .collect();
        for note in &notes {
            println!(
                "{} {:?} {}, {}",
                note.pitch_label(),
                note.duration,
                note.x,
                note.y
            );
        }
        assert!(!notes.is_empty());
        assert!(
            notes
                .iter()
                .all(|note| !matches!(note.duration, Duration::Half | Duration::Whole)),
            "{notes:?}"
        );
        for center in [748.0, 977.0] {
            let nearby: Vec<_> = notes
                .iter()
                .filter(|note| (note.x - center).abs() < 25.0)
                .collect();
            assert_eq!(nearby.len(), 1, "x {center}: {nearby:?}");
            assert_eq!(
                nearby[0].duration,
                Duration::Eighth,
                "x {center}: {:?}",
                nearby[0]
            );
        }
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to the Cassiopeia reference scan"]
    fn external_triplets_in_second_staff() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let image = read_image(std::path::Path::new(&path)).unwrap();
        let score = recognition::recognize(&image);
        for center in [
            1600.0, 1722.0, 1840.0, 1959.0, 2081.0, 2203.0, 3577.0, 3699.0, 3818.0,
        ] {
            let nearby: Vec<_> = score
                .notes
                .iter()
                .filter(|note| note.staff == 1 && (note.x - center).abs() < 25.0)
                .collect();
            assert_eq!(nearby.len(), 1, "x {center}: {nearby:?}");
            assert!(nearby[0].triplet, "x {center}: {:?}", nearby[0]);
            assert_eq!(nearby[0].duration, Duration::Eighth);
        }
        assert_eq!(
            score
                .notes
                .iter()
                .filter(|note| note.staff == 1 && note.triplet)
                .count(),
            9
        );
        assert!(
            score
                .notes
                .iter()
                .filter(|note| note.staff == 0)
                .all(|note| !note.triplet)
        );
    }

    #[test]
    #[ignore = "requires LIBREOMR_TEST_IMAGE pointing to a local TIFF"]
    fn imports_external_tiff() {
        let path = std::env::var_os("LIBREOMR_TEST_IMAGE").expect("set LIBREOMR_TEST_IMAGE");
        let path = std::path::Path::new(&path);
        let expected = tiff::decoder::Decoder::new(std::io::BufReader::new(
            std::fs::File::open(path).unwrap(),
        ))
        .unwrap()
        .dimensions()
        .unwrap();
        let image = read_image(path).unwrap();
        assert_eq!(image.dimensions(), expected);
        assert!(image.pixels().any(|pixel| pixel[0] == 0));
        assert!(image.pixels().any(|pixel| pixel[0] == 255));
        let score = recognition::recognize(&image);
        assert!(score.validate().is_ok());
        println!(
            "Imported {} x {} pixels; {} staves, {} note candidates",
            image.width(),
            image.height(),
            score.staves.len(),
            score.notes.len()
        );
    }

    fn fax3_tiff(options: u32, fill_order: u32, photometric: u32, rows_per_strip: u32) -> Vec<u8> {
        let mut strips = Vec::new();
        for first_row in (0..20).step_by(rows_per_strip as usize) {
            let mut bits = String::new();
            for row in first_row..(first_row + rows_per_strip).min(20) {
                if options & 4 != 0 {
                    while !(bits.len() + 12).is_multiple_of(8) {
                        bits.push('0');
                    }
                }
                bits.push_str("000000000001");
                let two_dimensional = options & 1 != 0 && (row - first_row) % 2 == 1;
                if options & 1 != 0 {
                    bits.push(if two_dimensional { '0' } else { '1' });
                }
                if two_dimensional {
                    bits.push_str("001");
                }
                if row % 2 == 0 {
                    bits.push_str("0011010100001101000");
                } else {
                    bits.push_str("0001000");
                    if two_dimensional {
                        bits.push_str("0000110111");
                    }
                }
            }
            bits.push_str(&"000000000001".repeat(6));
            while !bits.len().is_multiple_of(8) {
                bits.push('0');
            }
            let mut data: Vec<u8> = bits
                .as_bytes()
                .chunks(8)
                .map(|byte| {
                    byte.iter()
                        .fold(0, |value, bit| (value << 1) | (bit - b'0'))
                })
                .collect();
            if fill_order == 2 {
                data.iter_mut().for_each(|byte| *byte = byte.reverse_bits());
            }
            strips.push(data);
        }
        let strip_count = strips.len() as u32;
        let directory_end = 146;
        let data_start = directory_end + if strip_count > 1 { strip_count * 8 } else { 0 };
        let tags = [
            (256_u16, 4_u16, 1_u32, 20_u32),
            (257, 4, 1, 20),
            (258, 3, 1, 1),
            (259, 3, 1, 3),
            (262, 3, 1, photometric),
            (266, 3, 1, fill_order),
            (
                273,
                4,
                strip_count,
                if strip_count == 1 {
                    data_start
                } else {
                    directory_end
                },
            ),
            (277, 3, 1, 1),
            (278, 4, 1, rows_per_strip),
            (
                279,
                4,
                strip_count,
                if strip_count == 1 {
                    strips[0].len() as u32
                } else {
                    directory_end + strip_count * 4
                },
            ),
            (292, 4, 1, options),
        ];
        let mut tiff = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        tiff.extend_from_slice(&11_u16.to_le_bytes());
        for (tag, kind, count, value) in tags {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&kind.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        }
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        if strip_count > 1 {
            let mut offset = data_start;
            for strip in &strips {
                tiff.extend_from_slice(&offset.to_le_bytes());
                offset += strip.len() as u32;
            }
            for strip in &strips {
                tiff.extend_from_slice(&(strip.len() as u32).to_le_bytes());
            }
        }
        for strip in strips {
            tiff.extend_from_slice(&strip);
        }
        tiff
    }

    #[test]
    fn document_import_multipage_fax3_keeps_both_pages_and_page_local_errors() {
        let mut bytes = fax3_tiff(0, 1, 0, 20);
        let mut second = fax3_tiff(1, 2, 1, 20);
        let base = bytes.len() as u32;
        bytes[142..146].copy_from_slice(&(base + 8).to_le_bytes());
        // One strip: relocate its offset while keeping the second IFD's tag values.
        let strip_value = 8 + 2 + 6 * 12 + 8;
        second[strip_value..strip_value + 4].copy_from_slice(&(base + 146).to_le_bytes());
        bytes.extend(second);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fax.tif");
        std::fs::write(&path, &bytes).unwrap();
        let pages = open_document(&path).unwrap();
        assert_eq!(pages.len(), 2);
        for (index, page) in pages.iter().enumerate() {
            let image = page.render().unwrap();
            assert_eq!(image.dimensions(), (20, 20));
            for (x, y, pixel) in image.enumerate_pixels() {
                let black = (y % 2 == 0) != (index == 1);
                assert_eq!(
                    pixel[0],
                    if black { 0 } else { 255 },
                    "page {index}, pixel {x},{y}"
                );
            }
        }
        // A broken second strip must not prevent rendering the first page.
        let corrupt_offset = base as usize + strip_value;
        bytes[corrupt_offset..corrupt_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let pages = open_document(&path).unwrap();
        assert!(pages[0].render().is_ok());
        assert!(pages[1].render().unwrap_err().contains("Seite 2 von 2"));
    }

    #[test]
    fn imports_fax3_tiff() {
        let reader =
            image::ImageReader::with_format(Cursor::new(fax3_tiff(0, 1, 0, 20)), ImageFormat::Tiff);
        let decoded = decode_image(reader).unwrap();
        assert_eq!(decoded.dimensions(), (20, 20));
        for (x, y, pixel) in decoded.enumerate_pixels() {
            assert_eq!(pixel[0], if y % 2 == 0 { 0 } else { 255 }, "pixel {x},{y}");
        }
    }

    #[test]
    fn imports_fax3_tiff_variants() {
        for options in [0, 1, 4, 5] {
            for fill_order in [1, 2] {
                for photometric in [0, 1] {
                    for rows_per_strip in [7, 20] {
                        let bytes = fax3_tiff(options, fill_order, photometric, rows_per_strip);
                        let decoded =
                            decode_image(image::ImageReader::new(Cursor::new(bytes))).unwrap();
                        for (x, y, pixel) in decoded.enumerate_pixels() {
                            let black = (y % 2 == 0) ^ (photometric == 1);
                            assert_eq!(
                                pixel[0],
                                if black { 0 } else { 255 },
                                "pixel {x},{y}, options {options}, fill {fill_order}, photo {photometric}, rows {rows_per_strip}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_truncated_fax3_tiff() {
        let mut bytes = fax3_tiff(0, 1, 0, 20);
        bytes.truncate(bytes.len() - 20);
        assert!(decode_image(image::ImageReader::new(Cursor::new(bytes))).is_err());
    }

    #[test]
    fn imports_uncompressed_tiff_through_standard_decoder() {
        let expected = GrayImage::from_fn(20, 20, |_, y| {
            image::Luma([if y % 2 == 0 { 0 } else { 255 }])
        });
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(expected.clone())
            .write_to(&mut bytes, ImageFormat::Tiff)
            .unwrap();
        bytes.set_position(0);
        assert_eq!(
            decode_image(image::ImageReader::new(bytes)).unwrap(),
            expected
        );
    }

    #[test]
    fn transparent_png_is_composited_on_white() {
        let image = image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 0, 0]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        png.set_position(0);
        let reader = image::ImageReader::with_format(png, ImageFormat::Png);
        let decoded = decode_image(reader).unwrap();
        assert!(decoded.pixels().all(|pixel| pixel[0] == 255));
    }

    #[test]
    fn project_embeds_a_decodable_source_image() {
        let image = recognition::demo_image();
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image.clone())
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let project = Project {
            version: 1,
            image_name: "demo.png".into(),
            image_png: png.into_inner(),
            score: recognition::recognize(&image),
        };
        let restored: Project =
            serde_json::from_slice(&serde_json::to_vec(&project).unwrap()).unwrap();
        let decoded = decode_image(image::ImageReader::with_format(
            Cursor::new(restored.image_png),
            ImageFormat::Png,
        ))
        .unwrap();
        assert_eq!(decoded, image);
        assert_eq!(restored.score.notes.len(), 16);
        assert_eq!(
            restored.score.staves[0].header,
            project.score.staves[0].header
        );
        assert_eq!(
            restored.score.staves[1].header,
            project.score.staves[1].header
        );
    }

    #[test]
    fn rest_editor_controls_fit_and_delete_is_undoable() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.selected_rest = app.page.score.add_rest(450.0, 130.0, Duration::Eighth);
            app.page.score.rests[0].dots = 2;
            let mut header = app.page.score.staves[0].header.clone();
            header.key_signature = Some(KeySignature { fifths: 7 });
            app.page.score.set_staff_header(0, header);
            let mut delete = None;
            for _pass in 0..3 {
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                );
                for expected in ["Pause", "Pausenwert", "Systemangaben"] {
                    let text = output.shapes.iter().find(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == expected)).expect(expected);
                    let bounds = text.shape.visual_bounding_rect();
                    assert!(
                        bounds.min.x >= 0.0 && bounds.max.x <= size.x,
                        "{expected}: {bounds:?}"
                    );
                    assert!(
                        bounds.min.y >= 0.0 && bounds.max.y <= size.y,
                        "{expected}: {bounds:?}"
                    );
                }
                delete = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "\u{00d7}" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                });
            }
            let position = delete.expect("rest delete button visible");
            for pressed in [true, false] {
                let _output = context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events: vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                );
            }
            assert!(app.page.score.rests.is_empty());
            app.undo();
            assert_eq!(app.page.score.rests[0].duration, Duration::Eighth);
            assert_eq!(app.page.score.rests[0].dots, 2);
        }
    }

    #[test]
    fn multimeasure_rest_control_is_editable_and_undoable() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.selected_rest = app.page.score.add_rest(450.0, 130.0, Duration::Eighth);
            app.page.score.rests[0].dots = 2;
            let render = |app: &mut App, events: Vec<egui::Event>| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, Vec::new());
            }
            let output = render(&mut app, Vec::new());
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Mehrtaktpause" => {
                        let bounds = shape.shape.visual_bounding_rect();
                        assert!(shape.clip_rect.contains_rect(bounds));
                        Some(bounds.center())
                    }
                    _ => None,
                })
                .expect("multimeasure checkbox visible");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
            }
            assert_eq!(app.page.score.rests[0].measures, Some(2));
            assert_eq!(app.page.score.rests[0].duration, Duration::Whole);
            assert_eq!(app.page.score.rests[0].dots, 0);
            let output = render(&mut app, Vec::new());
            for label in ["Takte", "P 2 Takte"] {
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)), "{label}");
            }
            let restored: Score =
                serde_json::from_slice(&serde_json::to_vec(&app.page.score).unwrap()).unwrap();
            assert_eq!(restored.rests[0].measures, Some(2));
            app.undo();
            assert_eq!(app.page.score.rests[0].measures, None);
            assert_eq!(app.page.score.rests[0].duration, Duration::Eighth);
            assert_eq!(app.page.score.rests[0].dots, 2);
            app.redo();
            assert_eq!(app.page.score.rests[0].measures, Some(2));
            assert_eq!(app.page.score.rests[0].dots, 0);
        }
    }

    #[test]
    fn barline_editor_updates_accidentals_and_undo() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            let mut header = app.page.score.staves[0].header.clone();
            header.key_signature = Some(KeySignature { fifths: 1 });
            app.page.score.set_staff_header(0, header);
            for index in 0..3 {
                app.page.score.notes[index].pitch = 31;
                app.page.score.notes[index].y = app.page.score.staves[0].y_at_pitch(31);
            }
            app.page.score.notes[0].accidental_explicit = true;
            app.page.score.notes[0].accidental = 0;
            app.page.score.barlines.push(Barline {
                staff: 0,
                x: 375.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
            app.page.score.recompute_accidentals();
            assert_eq!(app.page.score.notes[1].accidental, 0);
            assert_eq!(app.page.score.notes[2].accidental, 1);
            app.select_object(Some(Object::Barline(0)));
            let render = |app: &mut App, events: Vec<egui::Event>| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            for label in ["\u{00d7}"] {
                let mut position = None;
                for _pass in 0..12 {
                    let output = render(&mut app, vec![]);
                    position = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = shape.shape.visual_bounding_rect();
                            assert!(
                                bounds.min.x >= 0.0
                                    && bounds.max.x <= size.x
                                    && bounds.min.y >= 0.0
                                    && bounds.max.y <= size.y,
                                "{label}: {bounds:?}"
                            );
                            Some(bounds.center())
                        }
                        _ => None,
                    });
                }
                let position = position.expect(label);
                for pressed in [true, false] {
                    let _output = render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::default(),
                            },
                        ],
                    );
                }
            }
            assert!(app.page.score.barlines.is_empty());
            assert_eq!(app.page.score.notes[2].accidental, 0);
            app.undo();
            assert_eq!(app.page.score.barlines.len(), 1);
            assert_eq!(app.page.score.notes[2].accidental, 1);
            app.redo();
            assert!(app.page.score.barlines.is_empty());
            assert_eq!(app.page.score.notes[2].accidental, 0);
        }
    }

    #[test]
    fn barline_types_are_editable_and_undoable() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.score.barlines.push(Barline {
                staff: 0,
                x: 375.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
            let render = |app: &mut App, events: Vec<egui::Event>| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            let click = |app: &mut App, label: &str| {
                let mut position = None;
                let mut visible = Vec::new();
                for _pass in 0..12 {
                    let output = render(app, vec![]);
                    visible = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    position = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = shape.shape.visual_bounding_rect();
                            if !shape.clip_rect.contains(bounds.center()) {
                                return None;
                            }
                            assert!(
                                bounds.min.x >= 0.0
                                    && bounds.max.x <= size.x
                                    && bounds.min.y >= 0.0
                                    && bounds.max.y <= size.y,
                                "{label}: {bounds:?}"
                            );
                            Some(bounds.center())
                        }
                        _ => None,
                    });
                }
                let position = position.unwrap_or_else(|| {
                    panic!(
                        "missing {label} at {size:?}, selected {:?}, visible {visible:?}",
                        app.page.score.barlines[0].kind
                    )
                });
                for pressed in [true, false] {
                    let _output = render(
                        app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::default(),
                            },
                        ],
                    );
                }
            };
            app.select_object(Some(Object::Barline(0)));
            for kind in BarlineKind::ALL
                .into_iter()
                .skip(1)
                .chain([BarlineKind::Single])
            {
                let previous = app.page.score.barlines[0].kind;
                click(&mut app, previous.label());
                click(&mut app, kind.label());
                assert_eq!(app.page.score.barlines[0].kind, kind);
                let restored: Score =
                    serde_json::from_slice(&serde_json::to_vec(&app.page.score).unwrap()).unwrap();
                assert_eq!(restored.barlines[0].kind, kind);
                app.undo();
                assert_eq!(app.page.score.barlines[0].kind, previous);
                app.redo();
                assert_eq!(app.page.score.barlines[0].kind, kind);
                app.select_object(Some(Object::Barline(0)));
            }
        }
    }

    #[test]
    fn triplet_control_is_editable_and_undoable() {
        let context = egui::Context::default();
        let mut app = demo_app();
        app.page.selected = Some(0);
        let size = Vec2::new(1280.0, 820.0);
        let mut control = None;
        for _pass in 0..3 {
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    ..Default::default()
                },
                |context| {
                    app.toolbar(context);
                    app.inspector(context);
                    app.canvas(context);
                },
            );
            control = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "n-Tole" => {
                    Some(shape.shape.visual_bounding_rect().center())
                }
                _ => None,
            });
        }
        let position = control.expect("triplet checkbox rendered");
        assert!(position.x < size.x && position.y < size.y);
        for pressed in [true, false] {
            let _output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                    ..Default::default()
                },
                |context| {
                    app.toolbar(context);
                    app.inspector(context);
                    app.canvas(context);
                },
            );
        }
        assert_eq!(
            app.page.score.notes[0].tuplet_ratio(),
            Some(Tuplet::default())
        );
        assert_eq!(app.page.score.notes[0].duration_label(), "1/4 (3:2)");
        app.undo();
        assert_eq!(app.page.score.notes[0].tuplet_ratio(), None);
        app.redo();
        assert_eq!(
            app.page.score.notes[0].tuplet_ratio(),
            Some(Tuplet::default())
        );
    }

    #[test]
    fn grace_and_small_note_controls_preserve_independent_values_and_history() {
        for label in ["Vorschlagsnote", "Kleine Notenköpfe"] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.selected = Some(0);
            let size = Vec2::new(1280.0, 1000.0);
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            let mut position = None;
            for _ in 0..3 {
                let output = render(&mut app, vec![]);
                position = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        let center = shape.shape.visual_bounding_rect().center();
                        shape.clip_rect.contains(center).then_some(center)
                    }
                    _ => None,
                });
            }
            let position = position.expect("note control visible");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
            }
            let is_grace = label == "Vorschlagsnote";
            assert_eq!(app.page.score.notes[0].grace, is_grace);
            assert!(app.page.score.notes[0].small);
            let saved = serde_json::to_string(&app.page.score).unwrap();
            let restored: Score = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored.notes[0].grace, is_grace);
            assert!(restored.notes[0].small);
            app.undo();
            assert!(!app.page.score.notes[0].grace);
            assert!(!app.page.score.notes[0].small);
            app.redo();
            assert_eq!(app.page.score.notes[0].grace, is_grace);
            assert!(app.page.score.notes[0].small);
        }
    }

    #[test]
    fn system_controls_fit_supported_window_sizes() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.selected_staff = 1;
            app.page.score.staves[1].header.key_signature = Some(KeySignature { fifths: 7 });
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let header = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Systemangaben" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .expect("system settings can be expanded");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(header),
                        egui::Event::PointerButton {
                            pos: header,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            for _pass in 0..3 {
                let output = render(&mut app, vec![]);
                for expected in ["Bassschluessel", "Cis-Dur / ais-Moll"] {
                    let clef = output
                        .shapes
                        .iter()
                        .find(|shape| match &shape.shape {
                            egui::Shape::Text(text) => text.galley.text() == expected,
                            _ => false,
                        })
                        .expect("selected clef is rendered");
                    let bounds = clef.shape.visual_bounding_rect();
                    assert!(bounds.min.x >= 0.0 && bounds.max.x <= size.x);
                    assert!(bounds.min.y >= 0.0 && bounds.max.y <= size.y);
                }
            }
        }
    }

    pub(super) fn demo_app() -> App {
        let image = recognition::demo_image();
        App {
            settings: crate::settings::Settings::default(),
            show_settings: false,
            problems: problems::Problems::default(),
            logo_texture: None,
            logo_dark: false,
            page: PageState {
                score: recognition::recognize(&image),
                image,
                texture: None,
                selected: None,
                selected_rest: None,
                selected_object: None,
                object_drag: None,
                canvas_context: None,
                selected_staff: 0,
                preview: crate::preview::Preview::default(),
                zoom: 1.0,
                fit_width: false,
                reveal_selection: false,
                reveal_x: None,
                show_object_list: false,
                staff_spacing: 16.0,
                undo: vec![],
                redo: vec![],
                name: "test".into(),
                id: 0,
                import: ImportState::Ready,
                source: None,
            },
            pages: vec![None],
            active_page: 0,
            reveal_tab: true,
            next_page_id: 1,
            show_preview: true,
            only_current_staff: false,
            pending: None,
            import_queue: VecDeque::new(),
            running_import: None,
            discovery: None,
            project_path: None,
            dirty: false,
            status: String::new(),
            status_error: false,
        }
    }

    fn sample_pages(names: &[&str], score: &Score) -> Loaded {
        Loaded {
            pages: names
                .iter()
                .enumerate()
                .map(|(index, name)| LoadedPage {
                    image: GrayImage::from_pixel(
                        40 + index as u32 * 10,
                        50,
                        image::Luma([240 - index as u8]),
                    ),
                    score: score.clone(),
                    name: (*name).into(),
                })
                .collect(),
            path: None,
        }
    }

    fn controlled_import(app: &mut App, index: usize) -> mpsc::Sender<JobEvent> {
        let id = app.page_at(index).id;
        app.import_queue.retain(|queued| queued.page_id != id);
        let (sender, receiver) = mpsc::channel();
        app.running_import = Some(RunningImport {
            page_id: id,
            receiver,
        });
        app.page_by_id_mut(id).unwrap().import = ImportState::Running {
            stage: "Notenbild laden",
            progress: None,
        };
        sender
    }

    fn two_page_sources(directory: &std::path::Path) -> (PathBuf, PathBuf) {
        let pdf = directory.join("score.pdf");
        std::fs::write(&pdf, include_bytes!("../tests/fixtures/two-pages.pdf")).unwrap();
        let tiff = directory.join("scan.tif");
        let mut data = Cursor::new(Vec::new());
        {
            let mut encoder = tiff::encoder::TiffEncoder::new(&mut data).unwrap();
            encoder
                .write_image::<tiff::encoder::colortype::Gray8>(30, 40, &vec![255; 30 * 40])
                .unwrap();
            encoder
                .write_image::<tiff::encoder::colortype::Gray8>(50, 60, &vec![220; 50 * 60])
                .unwrap();
        }
        std::fs::write(&tiff, data.into_inner()).unwrap();
        (pdf, tiff)
    }

    // Wait for a real worker deterministically, then let the normal UI polling
    // consume its events. No timing-dependent sleeps or repeated busy polling.
    fn finish_discovery(app: &mut App, context: &egui::Context) {
        let discovery = app.discovery.take().expect("metadata worker");
        let result = discovery
            .receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        sender.send(result).unwrap();
        app.discovery = Some(RunningDiscovery {
            page_id: discovery.page_id,
            receiver,
        });
        app.poll_imports(context);
    }

    fn finish_page_worker(app: &mut App, context: &egui::Context) {
        let job = app.running_import.take().expect("page worker");
        let (sender, receiver) = mpsc::channel();
        loop {
            let event = job
                .receiver
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            let finished = matches!(event, JobEvent::Finished(_));
            sender.send(event).unwrap();
            if finished {
                break;
            }
        }
        app.running_import = Some(RunningImport {
            page_id: job.page_id,
            receiver,
        });
        app.poll_imports(context);
    }

    #[test]
    fn document_import_discovers_pdf_and_tiff_tabs_during_another_pages_analysis() {
        let directory = tempfile::tempdir().unwrap();
        let (pdf, tiff) = two_page_sources(directory.path());
        let context = egui::Context::default();
        let mut app = demo_app();
        let original_id = app.page.id;
        app.snapshot();
        app.page.score.notes[0].dots = 2;
        app.queue_images(vec!["slow.png".into(), pdf, tiff], JobMode::AppendPages);
        let _busy_page = controlled_import(&mut app, 1);
        app.start_next_import(&context);
        assert!(
            app.discovery.is_some(),
            "metadata discovery must run alongside page analysis"
        );
        app.move_page(2, 0); // Move the PDF placeholder before its metadata arrives.
        finish_discovery(&mut app, &context);
        assert_eq!(app.pages.len(), 5);
        assert_eq!(app.active_page, 2);
        assert_eq!(app.page.id, original_id);
        assert_eq!(app.page_at(0).name, "score.pdf – Seite 1 von 2");
        assert_eq!(app.page_at(1).name, "score.pdf – Seite 2 von 2");
        assert!(
            app.discovery.is_some(),
            "the TIFF should be discovered next without waiting for the busy page"
        );
        finish_discovery(&mut app, &context);
        assert_eq!(app.pages.len(), 6);
        assert_eq!(app.page_at(4).name, "scan.tif – Seite 1 von 2");
        assert_eq!(app.page_at(5).name, "scan.tif – Seite 2 von 2");
        assert_eq!(
            app.import_queue
                .iter()
                .map(|queued| queued.source.name())
                .collect::<Vec<_>>(),
            vec![
                "score.pdf – Seite 1 von 2",
                "score.pdf – Seite 2 von 2",
                "scan.tif – Seite 1 von 2",
                "scan.tif – Seite 2 von 2"
            ]
        );
        assert!(!app.page_busy());
        assert_eq!(app.page.score.notes[0].dots, 2);
        app.undo();
        assert_eq!(app.page.score.notes[0].dots, 0);
        app.redo();
        assert_eq!(app.page.score.notes[0].dots, 2);
    }

    #[test]
    fn document_import_all_pages_complete_save_reload_and_export_in_tab_order() {
        let directory = tempfile::tempdir().unwrap();
        let (pdf, tiff) = two_page_sources(directory.path());
        let context = egui::Context::default();
        let mut app = demo_app();
        let score = app.page.score.clone();
        app.queue_images(vec![pdf, tiff], JobMode::ReplaceDocument);
        app.start_next_import(&context);
        while app.discovery.is_some()
            || app.running_import.is_some()
            || !app.import_queue.is_empty()
        {
            if app.discovery.is_some() {
                finish_discovery(&mut app, &context);
            }
            if app.running_import.is_some() {
                finish_page_worker(&mut app, &context);
            }
        }
        assert_eq!(app.pages.len(), 4);
        assert!(app.document_ready());
        for (index, size) in [(300, 400), (250, 300), (30, 40), (50, 60)]
            .into_iter()
            .enumerate()
        {
            let id = app.page_at(index).id;
            let page = app.page_by_id_mut(id).unwrap();
            assert_eq!(page.image.dimensions(), size);
            page.score = score.clone(); // Export integration uses a known, valid corrected score.
        }
        app.page_by_id_mut(app.page_at(3).id).unwrap().score.notes[0].dots = 2;
        app.move_page(3, 0);
        let project = app.project_document().unwrap();
        assert_eq!(project.pages[0].image_name, "scan.tif – Seite 2 von 2");
        let path = directory.path().join("project.libreomr.json");
        std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
        let reloaded = App::read_project(&path).unwrap();
        assert_eq!(reloaded.pages.len(), 4);
        assert_eq!(reloaded.pages[0].image.dimensions(), (50, 60));
        assert_eq!(reloaded.pages[0].score.notes[0].dots, 2);
        let path = directory.path().join("score.musicxml");
        app.write_musicxml_to(&path).unwrap();
        let xml = std::fs::read_to_string(path).unwrap();
        assert!(xml.contains("<work-title>scan</work-title>"));
        assert_eq!(xml.matches("<print new-page=\"yes\"/>").count(), 3);
        assert_eq!(xml.matches("<note>").count(), score.notes.len() * 4);
        let first_note = xml
            .split("<note>")
            .nth(1)
            .unwrap()
            .split("</note>")
            .next()
            .unwrap();
        assert_eq!(first_note.matches("<dot/>").count(), 2);
    }

    #[test]
    fn document_import_retry_only_requeues_the_failed_page_and_bad_metadata_does_not_stop_other_files()
     {
        let directory = tempfile::tempdir().unwrap();
        let (pdf, _) = two_page_sources(directory.path());
        let context = egui::Context::default();
        let mut app = demo_app();
        app.queue_images(vec![pdf], JobMode::AppendPages);
        app.start_discovery(&context);
        finish_discovery(&mut app, &context);
        finish_page_worker(&mut app, &context);
        // Replace the second page's worker with a controlled failure.
        let sender = controlled_import(&mut app, 2);
        sender
            .send(JobEvent::Finished(Box::new(Err("render failed".into()))))
            .unwrap();
        app.poll_imports(&context);
        app.switch_page(2);
        app.retry_import();
        assert_eq!(
            app.pages.len(),
            3,
            "retry must not expand the whole PDF again"
        );
        assert!(matches!(
            app.import_queue.back().unwrap().source,
            ImportSource::Page(_)
        ));
        let bad = directory.path().join("broken.pdf");
        std::fs::write(&bad, b"broken").unwrap();
        app.queue_images(vec![bad], JobMode::AppendPages);
        app.start_discovery(&context);
        finish_discovery(&mut app, &context);
        assert!(matches!(app.page_at(3).import, ImportState::Failed(_)));
        assert!(
            !app.import_queue
                .iter()
                .any(|queued| queued.page_id == app.page_at(3).id)
        );
    }

    #[test]
    fn background_imports_create_tabs_immediately_and_unlock_the_first_ready_page() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            let score = app.page.score.clone();
            app.queue_images(
                vec!["one.png".into(), "two.png".into(), "three.png".into()],
                JobMode::ReplaceDocument,
            );
            assert_eq!(app.pages.len(), 3);
            assert_eq!(app.import_queue.len(), 3);
            assert!(app.pending.is_none());
            let sender = controlled_import(&mut app, 0);
            sender
                .send(JobEvent::Progress(recognition::RecognitionProgress {
                    stage: "Noten suchen",
                    completed_steps: 1,
                    total_steps: 4,
                    staff: None,
                }))
                .unwrap();
            app.poll_imports(&context);
            let render = |app: &mut App| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.page_tabs(context);
                        app.inspector(context);
                        app.preview(context);
                        app.canvas(context);
                    },
                )
            };
            for _ in 0..3 {
                render(&mut app);
            }
            let output = render(&mut app);
            for label in [
                "1 · one.png · 25 %",
                "2 · two.png · Wartet",
                "3 · three.png · Wartet",
            ] {
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label)),
                    "{label}"
                );
            }
            app.insert_at(Vec2::new(50.0, 50.0), Insert::Staff);
            assert!(
                app.page.score.staves.is_empty(),
                "queued and running pages cannot be edited"
            );
            app.switch_page(1);
            assert_eq!(app.active_page, 1);
            render(&mut app);
            app.switch_page(0);
            sender
                .send(JobEvent::Finished(Box::new(Ok(sample_pages(
                    &["one.png"],
                    &score,
                )))))
                .unwrap();
            app.poll_imports(&context);
            assert!(!app.page_busy());
            assert!(app.page.import.ready());
            assert_eq!(
                app.running_import.as_ref().unwrap().page_id,
                app.page_at(1).id
            );
            assert!(matches!(app.page_at(1).import, ImportState::Running { .. }));
            assert!(matches!(app.page_at(2).import, ImportState::Queued));
            app.snapshot();
            app.page.score.notes[0].dots = 2;
            app.undo();
            assert_eq!(app.page.score.notes[0].dots, 0);
            app.redo();
            assert_eq!(app.page.score.notes[0].dots, 2);
            assert!(
                app.project_document().is_err(),
                "pending pages must not be silently omitted when saving"
            );
            let path = tempfile::tempdir().unwrap().path().join("pending.musicxml");
            assert!(app.write_musicxml_to(&path).is_err());
            assert!(!path.exists());
        }
    }

    #[test]
    fn background_results_follow_page_ids_without_overwriting_edits_or_changing_the_active_tab() {
        let context = egui::Context::default();
        let mut app = demo_app();
        let score = app.page.score.clone();
        let original_id = app.page.id;
        app.snapshot();
        app.page.score.notes[0].dots = 1;
        app.page.zoom = 1.7;
        app.page.fit_width = false;
        app.select_object(Some(Object::Note(0)));
        let original = serde_json::to_value(&app.page.score).unwrap();
        let image = app.page.image.clone();
        app.project_path = Some("existing.libreomr.json".into());
        app.queue_images(
            vec!["extra.png".into(), "later.png".into()],
            JobMode::AppendPages,
        );
        assert_eq!(app.page.id, original_id);
        assert!(!app.page_busy());
        assert!(app.project_path.is_some());
        let sender = controlled_import(&mut app, 1);
        let imported_id = app.page_at(1).id;
        sender
            .send(JobEvent::Image(GrayImage::new(100, 200)))
            .unwrap();
        app.poll_imports(&context);
        assert_eq!(app.page.image, image);
        app.switch_page(1);
        assert_eq!(app.page.image.dimensions(), (100, 200));
        app.move_page(1, 0);
        app.switch_page(1);
        assert_eq!(app.page.id, original_id);
        sender
            .send(JobEvent::Finished(Box::new(Ok(sample_pages(
                &["extra.png"],
                &score,
            )))))
            .unwrap();
        app.poll_imports(&context);
        assert_eq!(app.page_at(0).id, imported_id);
        assert!(app.page_at(0).import.ready());
        assert_eq!(app.page.id, original_id);
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
        assert_eq!(app.page.selected_object, Some(Object::Note(0)));
        assert_eq!(app.page.zoom, 1.7);
        assert!(!app.page.fit_width);
        app.undo();
        assert_eq!(app.page.score.notes[0].dots, 0);
    }

    #[test]
    fn background_failed_pages_can_be_retried_and_removed_without_losing_ready_pages() {
        let context = egui::Context::default();
        let mut app = demo_app();
        app.snapshot();
        app.page.score.notes[0].dots = 2;
        let original_id = app.page.id;
        app.queue_images(vec!["broken.png".into()], JobMode::AppendPages);
        let sender = controlled_import(&mut app, 1);
        sender
            .send(JobEvent::Finished(Box::new(
                Err("Bild nicht lesbar".into()),
            )))
            .unwrap();
        app.poll_imports(&context);
        assert!(matches!(app.page_at(1).import, ImportState::Failed(_)));
        assert!(!app.page_busy());
        assert!(!app.document_ready());
        assert!(
            app.project_document()
                .err()
                .unwrap()
                .contains("Seite 2 (broken.png)")
        );
        app.switch_page(1);
        let _ = context.run(Default::default(), |context| app.canvas(context));
        assert!(app.page.texture.is_none());
        app.retry_import();
        assert!(matches!(app.page.import, ImportState::Queued));
        assert_eq!(app.import_queue.len(), 1);
        assert!(
            matches!(&app.import_queue[0].source, ImportSource::File(path) if path == &PathBuf::from("broken.png"))
        );
        let sender = controlled_import(&mut app, 1);
        drop(sender); // Unexpected termination must also become a visible page error.
        app.poll_imports(&context);
        assert!(matches!(app.page.import, ImportState::Failed(_)));
        app.remove_failed_page();
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.page.id, original_id);
        assert_eq!(app.page.score.notes[0].dots, 2);
        assert!(app.document_ready());
        assert!(app.project_document().is_ok());
    }

    #[test]
    fn background_failures_do_not_stop_the_queue_and_replacement_discards_old_jobs() {
        let context = egui::Context::default();
        let mut app = demo_app();
        let score = app.page.score.clone();
        app.queue_images(
            vec!["broken.png".into(), "next.png".into()],
            JobMode::ReplaceDocument,
        );
        let sender = controlled_import(&mut app, 0);
        sender
            .send(JobEvent::Finished(Box::new(Err("kaputt".into()))))
            .unwrap();
        app.poll_imports(&context);
        assert!(matches!(app.page.import, ImportState::Failed(_)));
        assert_eq!(
            app.running_import.as_ref().unwrap().page_id,
            app.page_at(1).id
        );
        let old = controlled_import(&mut app, 1);
        app.apply_loaded(sample_pages(&["new.png"], &score), JobMode::ReplaceDocument);
        assert!(app.running_import.is_none());
        assert!(app.import_queue.is_empty());
        assert!(
            old.send(JobEvent::Finished(Box::new(Ok(sample_pages(
                &["stale.png"],
                &score
            )))))
            .is_err()
        );
        app.poll_imports(&context);
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.page.name, "new.png");
    }

    #[test]
    fn multipage_measure_colors_follow_inherited_meter_edits_and_tab_order() {
        let context = egui::Context::default();
        let mut app = demo_app();
        let mut score = Score {
            staves: vec![Staff {
                lines: [60.0, 72.0, 84.0, 96.0, 108.0],
                left: 20.0,
                right: 500.0,
                header: Default::default(),
            }],
            ..Default::default()
        };
        score.add_rest(180.0, 84.0, Duration::Half);
        score.rests[0].dots = 1;
        score.barlines.push(Barline {
            staff: 0,
            x: 400.0,
            kind: BarlineKind::Final,
            confirmed: false,
        });
        let mut loaded = sample_pages(&["first", "middle", "last"], &score);
        for page in &mut loaded.pages {
            page.image = GrayImage::from_pixel(550, 160, image::Luma([255]));
        }
        let three_four = TimeSignature::Meter {
            beats: 3,
            beat_unit: 4,
        };
        loaded.pages[0].score.staves[0].header.time_signature = Some(three_four);
        app.apply_loaded(loaded, JobMode::ReplaceDocument);
        app.switch_page(2);
        let unchanged = serde_json::to_value(&app.page.score).unwrap();
        let assert_colors = |app: &mut App, expected: Option<bool>| {
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                    ..Default::default()
                },
                |context| {
                    app.preview(context);
                    app.canvas(context);
                },
            );
            let colors: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh)
                        if mesh.vertices.len() == 4
                            && mesh.vertices[0].color == Color32::TRANSPARENT
                            && mesh.vertices[1].color.a() == 90 =>
                    {
                        Some(mesh.vertices[1].color)
                    }
                    _ => None,
                })
                .collect();
            if let Some(valid) = expected {
                let color = if valid {
                    Color32::from_rgba_unmultiplied(0, 155, 85, 90)
                } else {
                    Color32::from_rgba_unmultiplied(220, 45, 45, 90)
                };
                assert_eq!(
                    colors,
                    vec![color; 2],
                    "scan and preview both show the meter check"
                );
            } else {
                assert!(colors.is_empty());
            }
        };
        assert_colors(&mut app, Some(true));
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), unchanged);

        app.switch_page(0);
        app.snapshot();
        app.page.score.staves[0].header.time_signature = Some(TimeSignature::Common);
        app.switch_page(2);
        assert_colors(&mut app, Some(false));
        app.switch_page(0);
        app.undo();
        app.switch_page(2);
        assert_colors(&mut app, Some(true));
        app.switch_page(0);
        app.redo();
        app.switch_page(2);
        assert_colors(&mut app, Some(false));

        // A meter printed on the active page overrides preceding pages.
        app.page.score.staves[0].header.time_signature = Some(three_four);
        assert_colors(&mut app, Some(true));
        app.page.score.staves[0].header.time_signature = None;
        app.move_page(0, 2);
        assert_eq!(app.active_page, 1);
        assert_colors(&mut app, None);
        app.move_page(2, 0);
        assert_colors(&mut app, Some(false));
    }

    #[test]
    fn multipage_switching_preserves_edits_selection_and_independent_history() {
        let mut app = demo_app();
        let score = app.page.score.clone();
        app.apply_loaded(
            sample_pages(&["one.png", "two.png"], &score),
            JobMode::ReplaceDocument,
        );
        app.snapshot();
        app.page.score.notes[0].dots = 2;
        app.select_object(Some(Object::Note(0)));
        app.page.zoom = 1.75;
        app.page.fit_width = false;
        let first_id = app.page.id;
        app.switch_page(1);
        assert_eq!(app.page.name, "two.png");
        assert!(app.page.undo.is_empty());
        assert!(app.page.selected_object.is_none());
        app.snapshot();
        app.page.score.notes[0].articulation = Articulation::Accent;
        app.undo();
        assert_eq!(app.page.score.notes[0].articulation, Articulation::None);
        app.switch_page(0);
        assert_eq!(app.page.id, first_id);
        assert_eq!(app.page.selected_object, Some(Object::Note(0)));
        assert_eq!(app.page.zoom, 1.75);
        assert!(!app.page.fit_width);
        assert_eq!(app.page.score.notes[0].dots, 2);
        app.undo();
        assert_eq!(app.page.score.notes[0].dots, 0);
        app.redo();
        app.switch_page(1);
        app.redo();
        assert_eq!(app.page.score.notes[0].articulation, Articulation::Accent);
        assert_eq!(app.page.score.notes[0].dots, 0);
        let context = egui::Context::default();
        let _ = context.run(Default::default(), |context| app.canvas(context));
        assert!(
            app.page.texture.is_some(),
            "new pages acquire their image texture on first display"
        );
    }

    #[test]
    fn multipage_append_and_recognize_preserve_other_pages_and_project_path() {
        let mut app = demo_app();
        let score = app.page.score.clone();
        let path = PathBuf::from("saved.libreomr.json");
        let mut initial = sample_pages(&["one.png", "two.png"], &score);
        initial.path = Some(path.clone());
        app.apply_loaded(initial, JobMode::ReplaceDocument);
        assert!(!app.dirty);
        app.snapshot();
        app.page.score.notes[0].dots = 1;
        let id = app.page.id;
        app.apply_loaded(
            sample_pages(&["three.png", "four.png"], &score),
            JobMode::AppendPages,
        );
        assert_eq!(app.pages.len(), 4);
        assert_eq!(app.active_page, 2);
        assert_eq!(app.page.name, "three.png");
        assert_eq!(app.project_path, Some(path.clone()));
        assert!(app.dirty);
        app.apply_loaded(sample_pages(&["three.png"], &score), JobMode::ReplacePage);
        assert_eq!(app.pages.len(), 4);
        assert_eq!(app.project_path, Some(path));
        app.switch_page(0);
        assert_eq!(app.page.id, id);
        assert_eq!(app.page.score.notes[0].dots, 1);
        assert_eq!(app.page.undo.len(), 1);
        app.move_page(0, 3);
        assert_eq!(app.active_page, 3);
        assert_eq!(app.page.id, id);
        app.move_page(0, 2);
        assert_eq!(app.active_page, 3);
        assert_eq!(app.page_at(2).name, "two.png");
        assert_eq!(app.pages.iter().filter(|page| page.is_none()).count(), 1);
    }

    #[test]
    fn multipage_project_and_export_follow_tabs_even_when_another_page_is_active() {
        let mut app = demo_app();
        let score = app.page.score.clone();
        app.apply_loaded(
            sample_pages(&["one.png", "two.png", "three.png"], &score),
            JobMode::ReplaceDocument,
        );
        app.move_page(2, 0);
        app.switch_page(2);
        app.page.score.notes[0].dots = 2;
        let document = app.project_document().unwrap();
        assert_eq!(
            document
                .pages
                .iter()
                .map(|page| page.image_name.as_str())
                .collect::<Vec<_>>(),
            vec!["three.png", "one.png", "two.png"]
        );
        assert_eq!(document.pages[2].score.notes[0].dots, 2);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("score.libreomr.json");
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let loaded = App::read_project(&path).unwrap();
        assert_eq!(loaded.path, Some(path));
        for (index, page) in loaded.pages.iter().enumerate() {
            assert_eq!(page.name, document.pages[index].image_name);
            assert_eq!(page.image.width(), [60, 40, 50][index]);
            assert_eq!(
                serde_json::to_value(&page.score).unwrap(),
                serde_json::to_value(&document.pages[index].score).unwrap()
            );
        }
        let export_path = directory.path().join("score.musicxml");
        app.write_musicxml_to(&export_path).unwrap();
        let xml = std::fs::read_to_string(export_path).unwrap();
        let expected = musicxml::export_pages(
            &document
                .pages
                .iter()
                .map(|page| &page.score)
                .collect::<Vec<_>>(),
            "three",
        )
        .unwrap();
        assert_eq!(xml, expected.xml);
        assert_eq!(xml.matches("<print new-page=\"yes\"/>").count(), 2);
        app.apply_loaded(loaded, JobMode::ReplaceDocument);
        assert!(!app.dirty);
        assert_eq!(app.page.name, "three.png");
        assert_eq!(app.page_at(2).score.notes[0].dots, 2);
    }

    #[test]
    fn multipage_tabs_can_be_clicked_and_reordered_by_dragging() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            let score = app.page.score.clone();
            app.apply_loaded(
                sample_pages(&["one.png", "two.png", "three.png"], &score),
                JobMode::ReplaceDocument,
            );
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.page_tabs(context);
                        app.inspector(context);
                        app.preview(context);
                        app.canvas(context);
                    },
                )
            };
            for _ in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let tab = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = shape.shape.visual_bounding_rect();
                            assert!(shape.clip_rect.contains_rect(bounds));
                            Some(bounds.center())
                        }
                        _ => None,
                    })
                    .expect("page tab rendered")
            };
            let first = tab("1 · one.png");
            let second = tab("2 · two.png");
            let third = tab("3 · three.png");
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![egui::Event::PointerMoved(second), button(second, pressed)],
                );
            }
            assert_eq!(app.active_page, 1);
            render(
                &mut app,
                vec![egui::Event::PointerMoved(first), button(first, true)],
            );
            render(
                &mut app,
                vec![egui::Event::PointerMoved(first + Vec2::new(20.0, 0.0))],
            );
            render(&mut app, vec![egui::Event::PointerMoved(third)]);
            render(&mut app, vec![button(third, false)]);
            assert_eq!(
                (0..3)
                    .map(|index| app.page_at(index).name.as_str())
                    .collect::<Vec<_>>(),
                vec!["two.png", "three.png", "one.png"]
            );
            assert_eq!(app.active_page, 0);
            assert_eq!(app.page.name, "two.png");
            assert!(app.dirty);
        }
    }

    #[test]
    fn multipage_image_import_preserves_order_and_reports_the_failed_file() {
        let directory = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["two.png", "one.png"]
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let path = directory.path().join(name);
                GrayImage::from_pixel(40 + index as u32 * 10, 50, image::Luma([255]))
                    .save(&path)
                    .unwrap();
                path
            })
            .collect();
        let (sender, _receiver) = mpsc::channel();
        let reporter = JobReporter {
            sender,
            context: egui::Context::default(),
        };
        let loaded = App::load_image(paths[0].clone(), &reporter).unwrap();
        assert_eq!(loaded.pages[0].name, "two.png");
        let second = App::load_image(paths[1].clone(), &reporter).unwrap();
        assert_eq!(second.pages[0].name, "one.png");
        assert_eq!(second.pages[0].image.width(), 50);
        let bad = directory.path().join("broken.png");
        std::fs::write(&bad, b"not an image").unwrap();
        let error = App::load_image(bad, &reporter).err().unwrap();
        assert!(error.contains("Bild"), "{error}");
    }

    #[test]
    fn multipage_failed_import_preserves_current_page_and_its_view() {
        let context = egui::Context::default();
        let mut app = demo_app();
        app.page.zoom = 1.5;
        app.page.fit_width = false;
        let image = app.page.image.clone();
        let score = serde_json::to_value(&app.page.score).unwrap();
        let (sender, receiver) = mpsc::channel();
        app.pending = Some(PendingJob {
            mode: JobMode::AppendPages,
            receiver,
            progress: None,
            image_size: None,
            texture: None,
        });
        sender
            .send(JobEvent::Image(GrayImage::new(300, 400)))
            .unwrap();
        app.poll_job(&context);
        let _ = context.run(Default::default(), |context| app.canvas(context));
        assert_eq!(app.page.zoom, 1.5);
        app.switch_page(0);
        app.move_page(0, 0);
        sender
            .send(JobEvent::Finished(Box::new(Err("broken.png".into()))))
            .unwrap();
        app.poll_job(&context);
        assert!(app.pending.is_none());
        assert!(app.status_error);
        assert_eq!(app.pages.len(), 1);
        assert_eq!(app.page.image, image);
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), score);
        assert_eq!(app.page.zoom, 1.5);
        assert!(!app.page.fit_width);
        assert!(!app.dirty);
    }

    #[test]
    fn objects_follow_staff_and_position_and_anchors_stay_ordered() {
        let mut app = demo_app();
        app.page.score.spans = vec![Span {
            staff: 0,
            kind: SpanKind::Slur,
            start: [250.0, 90.0],
            end: [400.0, 100.0],
            height: -20.0,
            confirmed: false,
        }];
        let rest = app
            .page
            .score
            .add_rest(300.0, 130.0, Duration::Quarter)
            .unwrap();
        app.page.score.barlines.push(Barline {
            staff: 0,
            x: 350.0,
            kind: BarlineKind::Single,
            confirmed: false,
        });
        let objects = app.ordered_objects();
        let span_order = objects
            .iter()
            .position(|object| *object == Object::Span(0))
            .unwrap();
        let rest_order = objects
            .iter()
            .position(|object| *object == Object::Rest(rest))
            .unwrap();
        let bar_order = objects
            .iter()
            .position(|object| *object == Object::Barline(0))
            .unwrap();
        assert!(span_order < rest_order && rest_order < bar_order);
        app.select_object(Some(Object::Span(0)));
        assert_eq!(app.page.selected_staff, 0);
        app.snapshot();
        app.page.object_drag = Some(ObjectDrag {
            target: DragTarget::Anchor(0, false),
            origin: Vec2::new(250.0, 90.0),
            score: app.page.score.clone(),
        });
        app.move_object(Vec2::new(450.0, 110.0));
        assert_eq!(app.page.score.spans[0].start, [399.0, 110.0]);
        assert_eq!(app.page.score.spans[0].end, [400.0, 100.0]);
        assert!(app.page.score.validate().is_ok());
        app.undo();
        assert_eq!(app.page.score.spans[0].start, [250.0, 90.0]);
        app.redo();
        assert_eq!(app.page.score.spans[0].start, [399.0, 110.0]);
        app.select_object(None);
        app.page.score.spans[0].start = [250.0, 90.0];
        for kind in [SpanKind::Crescendo, SpanKind::Decrescendo] {
            app.page.score.spans[0].kind = kind;
            app.page.score.spans[0].height = 20.0;
            let position = Vec2::new(325.0, 105.0);
            assert_eq!(
                app.hit_object(position),
                Some(DragTarget::Body(Object::Span(0)))
            );
        }
        app.page.score.notes[0].staff = 1;
        app.page.score.notes[0].x = 1.0;
        let objects = app.ordered_objects();
        assert!(
            objects
                .iter()
                .position(|object| *object == Object::Note(0))
                .unwrap()
                > objects
                    .iter()
                    .position(|object| *object == Object::Barline(0))
                    .unwrap()
        );
    }

    #[test]
    fn context_insertion_keeps_the_clicked_position_and_supports_undo() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            for zoom in [0.4, 1.0] {
                for menu in [
                    "Taktstrich",
                    "Note",
                    "Pause",
                    "Tonartwechsel",
                    "Crescendo",
                    "Notensystem",
                ] {
                    let context = egui::Context::default();
                    App::configure_style(&context);
                    context.style_mut(|style| style.animation_time = 0.0);
                    let mut app = demo_app();
                    app.page.zoom = zoom;
                    app.set_texture(&context);
                    app.page.score.notes.clear();
                    app.page.score.rests.clear();
                    app.page.score.barlines.clear();
                    app.page.score.spans.clear();
                    app.page.score.key_changes.clear();
                    if menu == "Notensystem" {
                        app.page.score.staves.clear();
                    }
                    let before = serde_json::to_value(&app.page.score).unwrap();
                    let source = Vec2::new(320.0, 140.0);
                    let render = |app: &mut App, events| {
                        context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                events,
                                ..Default::default()
                            },
                            |context| {
                                app.canvas(context);
                            },
                        )
                    };
                    for _pass in 0..3 {
                        render(&mut app, vec![]);
                    }
                    let output = render(&mut app, vec![]);
                    let texture = app.page.texture.as_ref().unwrap().id();
                    let origin = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                                Some(mesh.calc_bounds().min)
                            }
                            _ => None,
                        })
                        .unwrap();
                    let position = origin + source * zoom;
                    for pressed in [true, false] {
                        render(
                            &mut app,
                            vec![
                                egui::Event::PointerMoved(position),
                                egui::Event::PointerButton {
                                    pos: position,
                                    button: egui::PointerButton::Secondary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                    }
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    let label = menu;
                    for _pass in 0..3 {
                        render(&mut app, vec![]);
                    }
                    let output = render(&mut app, vec![]);
                    let heading = output
                        .shapes
                        .iter()
                        .find(|shape| {
                            matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == "Objekt einfuegen")
                        })
                        .expect("insertion menu heading");
                    assert!(
                        heading
                            .clip_rect
                            .contains_rect(heading.shape.visual_bounding_rect())
                    );
                    let position = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) if text.galley.text() == label => {
                                let bounds = shape.shape.visual_bounding_rect();
                                shape
                                    .clip_rect
                                    .contains_rect(bounds)
                                    .then_some(bounds.center())
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| {
                            let visible: Vec<_> = output
                                .shapes
                                .iter()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                                    _ => None,
                                })
                                .collect();
                            panic!(
                                "visible insertion choice {label} at {size:?}/{zoom}: {visible:?}"
                            )
                        });
                    for pressed in [true, false] {
                        render(
                            &mut app,
                            vec![
                                egui::Event::PointerMoved(position),
                                egui::Event::PointerButton {
                                    pos: position,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                    }
                    match menu {
                        "Note" => {
                            let note = &app.page.score.notes[0];
                            assert!((note.x - source.x).abs() < 0.01);
                            assert_eq!(
                                note.y,
                                app.page.score.staves[note.staff].y_at_pitch(note.pitch)
                            );
                            assert_eq!(note.duration, Duration::Quarter);
                        }
                        "Pause" => {
                            assert_eq!(app.page.score.rests[0].duration, Duration::Quarter);
                            assert!(
                                (Vec2::new(app.page.score.rests[0].x, app.page.score.rests[0].y)
                                    - source)
                                    .length()
                                    < 0.01
                            );
                        }
                        "Taktstrich" => {
                            assert!((app.page.score.barlines[0].x - source.x).abs() < 0.01)
                        }
                        "Tonartwechsel" => {
                            assert!((app.page.score.key_changes[0].x - source.x).abs() < 0.01)
                        }
                        "Crescendo" => {
                            assert!(
                                (Vec2::from(app.page.score.spans[0].start) - source).length()
                                    < 0.01
                            )
                        }
                        "Notensystem" => assert!(
                            (app.page.score.staves.last().unwrap().lines[0] - source.y).abs()
                                < 0.01
                        ),
                        _ => unreachable!(),
                    }
                    assert_eq!(app.page.undo.len(), 1);
                    assert!(app.page.canvas_context.is_none());
                    assert!(app.page.score.validate().is_ok());
                    let after = serde_json::to_value(&app.page.score).unwrap();
                    app.undo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    app.redo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), after);
                }
            }
        }
    }

    #[test]
    fn context_menu_cancellation_does_not_change_the_score() {
        for escape in [false, true] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.set_texture(&context);
            let before = serde_json::to_value(&app.page.score).unwrap();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.shortcuts(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let position = Pos2::new(200.0, 200.0);
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            assert!(egui::Popup::is_any_open(&context));
            if escape {
                render(
                    &mut app,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            } else {
                let outside = Pos2::new(755.0, 515.0);
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(outside),
                            egui::Event::PointerButton {
                                pos: outside,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
            }
            render(&mut app, vec![]);
            assert!(!egui::Popup::is_any_open(&context));
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
            assert!(app.page.undo.is_empty());
            assert!(!app.dirty);
        }
    }

    #[test]
    fn context_actions_cover_all_object_types_and_properties_do_not_edit() {
        for object in [
            Object::Note(0),
            Object::Rest(0),
            Object::Barline(0),
            Object::Span(0),
            Object::KeyChange(0),
        ] {
            let context = egui::Context::default();
            context.style_mut(|style| style.animation_time = 0.0);
            let mut app = demo_app();
            app.page.score.notes.truncate(1);
            app.page.score.notes[0].x = 200.0;
            app.page
                .score
                .add_rest(300.0, 130.0, Duration::Quarter)
                .unwrap();
            app.page.score.barlines.push(Barline {
                staff: 0,
                x: 350.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
            app.page.score.spans.push(Span {
                staff: 0,
                kind: SpanKind::Slur,
                start: [250.0, 70.0],
                end: [400.0, 80.0],
                height: -20.0,
                confirmed: false,
            });
            app.page.score.key_changes.push(KeyChange {
                staff: 0,
                x: 450.0,
                key_signature: KeySignature { fifths: -2 },
                bounds: vec![],
                confirmed: false,
            });
            app.page.score.recompute_accidentals();
            let before = serde_json::to_value(&app.page.score).unwrap();
            app.page.canvas_context = Some((Vec2::new(300.0, 130.0), Some(object)));
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            app.canvas_context_menu(ui);
                        });
                    },
                )
            };
            for action in ["Eigenschaften", "Loeschen"] {
                app.page.canvas_context = Some((Vec2::new(300.0, 130.0), Some(object)));
                let output = render(&mut app, vec![]);
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == action => {
                            Some(shape.shape.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .unwrap();
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                if action == "Eigenschaften" {
                    assert_eq!(app.page.selected_object, Some(object));
                    assert!(!app.page.show_object_list);
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    assert!(app.page.undo.is_empty());
                }
            }
            assert_eq!(app.page.undo.len(), 1);
            assert!(app.page.selected_object.is_none());
            assert!(app.page.show_object_list);
            let after = serde_json::to_value(&app.page.score).unwrap();
            assert_ne!(after, before);
            app.undo();
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
            app.redo();
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), after);
        }
    }

    #[test]
    fn right_click_on_a_note_offers_properties_and_undoable_delete() {
        for zoom in [0.4, 1.0] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.page.zoom = zoom;
            app.page.show_object_list = true;
            let original = serde_json::to_value(&app.page.score).unwrap();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.stroke.color == Color32::from_rgb(200, 100, 0) =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("visible note overlay");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let output = render(&mut app, vec![]);
            for expected in ["Eigenschaften", "Loeschen"] {
                assert!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == expected)),
                    "context action {expected}"
                );
            }
            assert_eq!(app.page.selected_object, Some(Object::Note(0)));
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
            assert!(app.page.show_object_list);
            for action in ["Eigenschaften", "Loeschen"] {
                if action == "Loeschen" {
                    for _pass in 0..3 {
                        render(&mut app, vec![]);
                    }
                    for pressed in [true, false] {
                        render(
                            &mut app,
                            vec![
                                egui::Event::PointerMoved(position),
                                egui::Event::PointerButton {
                                    pos: position,
                                    button: egui::PointerButton::Secondary,
                                    pressed,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                        );
                    }
                }
                for _pass in 0..3 {
                    render(&mut app, vec![]);
                }
                let output = render(&mut app, vec![]);
                let button = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == action => {
                        let bounds = shape.shape.visual_bounding_rect();
                        shape.clip_rect.contains_rect(bounds).then_some(bounds.center())
                    }, _ => None,
                }).unwrap_or_else(|| panic!("visible context action {action}, selected {:?}, context {:?}, popup {}, texts {:?}",
                    app.page.selected_object, app.page.canvas_context, egui::Popup::is_any_open(&context),
                    output.shapes.iter().filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text().to_owned()), _ => None,
                    }).collect::<Vec<_>>()));
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(button),
                            egui::Event::PointerButton {
                                pos: button,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                if action == "Eigenschaften" {
                    assert!(!app.page.show_object_list);
                    assert_eq!(app.page.selected_object, Some(Object::Note(0)));
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
                    assert!(app.page.undo.is_empty());
                }
            }
            assert!(app.page.selected_object.is_none());
            assert!(app.page.show_object_list);
            assert_eq!(app.page.undo.len(), 1);
            let after = serde_json::to_value(&app.page.score).unwrap();
            assert_ne!(after, original);
            app.undo();
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
            app.redo();
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), after);
        }
    }

    #[test]
    fn correction_object_preview_stays_visible_and_tracks_settings_and_history() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.show_preview = false;
            app.set_texture(&context);
            let scan_texture = app.page.texture.as_ref().unwrap().id();
            app.page.score.add_rest(300.0, 130.0, Duration::Quarter);
            app.page.score.barlines.push(Barline {
                staff: 0,
                x: 350.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
            app.page.score.spans.push(Span {
                staff: 0,
                kind: SpanKind::Slur,
                start: [250.0, 70.0],
                end: [400.0, 80.0],
                height: -20.0,
                confirmed: false,
            });
            app.page.score.key_changes.push(KeyChange {
                staff: 0,
                x: 450.0,
                key_signature: KeySignature { fifths: -2 },
                bounds: vec![],
                confirmed: false,
            });
            app.page.score.recompute_accidentals();
            let before = serde_json::to_value(&app.page.score).unwrap();
            let render = |app: &mut App| {
                let mut output = None;
                for _pass in 0..4 {
                    output = Some(context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |context| {
                            app.toolbar(context);
                            app.inspector(context);
                            app.canvas(context);
                        },
                    ));
                }
                output.unwrap()
            };
            let meshes = |output: &egui::FullOutput| {
                output
                    .shapes
                    .iter()
                    .filter(|shape| {
                        matches!(&shape.shape, egui::Shape::Mesh(mesh)
                        if mesh.texture_id != egui::TextureId::default() && mesh.texture_id != scan_texture)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let fingerprint = |output: &egui::FullOutput| {
                let glyphs = meshes(output);
                let origin = glyphs[0].shape.visual_bounding_rect().min;
                format!(
                    "{:?}",
                    glyphs
                        .iter()
                        .filter_map(|glyph| match &glyph.shape {
                            egui::Shape::Mesh(mesh) => Some((
                                mesh.texture_id,
                                mesh.vertices
                                    .iter()
                                    .map(|vertex| (vertex.pos - origin, vertex.uv, vertex.color))
                                    .collect::<Vec<_>>()
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                )
            };
            for object in [
                Object::Note(0),
                Object::Rest(0),
                Object::Barline(0),
                Object::Span(0),
                Object::KeyChange(0),
            ] {
                app.select_object(Some(object));
                let output = render(&mut app);
                let heading = output
                    .shapes
                    .iter()
                    .find(|shape| {
                        matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "Objektvorschau")
                    })
                    .unwrap();
                assert!(
                    heading
                        .clip_rect
                        .contains_rect(heading.shape.visual_bounding_rect())
                );
                let glyphs = meshes(&output);
                assert!(
                    !glyphs.is_empty(),
                    "{object:?} must render without the row preview"
                );
                for glyph in glyphs {
                    let bounds = glyph.shape.visual_bounding_rect();
                    assert!(
                        glyph.clip_rect.contains_rect(bounds),
                        "{object:?} clipped at {size:?}"
                    );
                    assert!(
                        bounds.min.x >= size.x - 380.0
                            && bounds.min.y > heading.shape.visual_bounding_rect().max.y
                    );
                }
            }
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
            assert!(app.page.undo.is_empty());
            app.select_object(Some(Object::Note(0)));
            let original = fingerprint(&render(&mut app));
            app.snapshot();
            let note = &mut app.page.score.notes[0];
            note.duration = Duration::Sixteenth;
            note.dots = 2;
            note.triplet = true;
            note.articulation = Articulation::Accent;
            note.accidental = 1;
            note.accidental_explicit = true;
            note.pitch += 7;
            note.y = app.page.score.staves[note.staff].y_at_pitch(note.pitch);
            let changed = fingerprint(&render(&mut app));
            assert_ne!(changed, original);
            assert_eq!(app.page.undo.len(), 1);
            app.undo();
            app.select_object(Some(Object::Note(0)));
            assert_eq!(fingerprint(&render(&mut app)), original);
            app.redo();
            app.select_object(Some(Object::Note(0)));
            assert_eq!(fingerprint(&render(&mut app)), changed);
            app.close_details();
            let output = render(&mut app);
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Objektvorschau")));
        }
    }

    #[test]
    fn correction_details_close_to_overview_with_escape_and_close_button() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.page.preview = crate::preview::Preview::fitted();
            let original = serde_json::to_value(&app.page.score).unwrap();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.shortcuts(context);
                        app.toolbar(context);
                        app.inspector(context);
                        app.preview(context);
                        app.canvas(context);
                    },
                )
            };
            app.select_object(Some(Object::Note(0)));
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            render(
                &mut app,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(
                app.page.show_object_list,
                "Escape must return to the overview"
            );
            app.select_object(Some(Object::Note(0)));
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == icons::X => {
                        let bounds = shape.shape.visual_bounding_rect();
                        shape
                            .clip_rect
                            .contains_rect(bounds)
                            .then_some(bounds.center())
                    }
                    _ => None,
                })
                .expect("visible close button in correction details");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            for _pass in 0..3 {
                render(&mut app, vec![]);
                assert!(
                    app.page.show_object_list,
                    "overview must remain open with the preview visible"
                );
                assert!(app.page.selected_object.is_none());
            }
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
            assert!(app.page.undo.is_empty());
            assert!(!app.dirty);
        }
    }

    #[test]
    fn mixed_object_list_is_ordered_and_selectable_after_scrolling() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.page.show_object_list = true;
            app.page.score.notes.truncate(1);
            app.page.score.notes[0].x = 200.0;
            app.page.score.rests.clear();
            app.page
                .score
                .add_rest(300.0, 130.0, Duration::Quarter)
                .unwrap();
            app.page.score.barlines = vec![Barline {
                staff: 0,
                x: 350.0,
                kind: BarlineKind::Single,
                confirmed: false,
            }];
            app.page.score.spans = vec![Span {
                staff: 0,
                kind: SpanKind::Slur,
                start: [250.0, 70.0],
                end: [400.0, 80.0],
                height: -20.0,
                confirmed: false,
            }];
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            render(
                &mut app,
                vec![
                    egui::Event::PointerMoved(Pos2::new(size.x - 50.0, size.y - 50.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -1000.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            let mut output = render(&mut app, vec![]);
            for _pass in 0..20 {
                output = render(&mut app, vec![]);
            }
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if shape
                            .clip_rect
                            .contains_rect(shape.shape.visual_bounding_rect()) =>
                    {
                        let label = text.galley.text();
                        (label.starts_with(icons::MUSIC_NOTE)
                            || label.starts_with(icons::BEZIER_CURVE)
                            || label.starts_with(icons::PAUSE)
                            || label.starts_with(icons::LINE_VERTICAL))
                        .then_some((
                            label.to_owned(),
                            shape.shape.visual_bounding_rect().center(),
                        ))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(labels.len(), 4, "{size:?}: {labels:?}");
            assert!(labels[0].0.starts_with(icons::MUSIC_NOTE));
            assert_eq!(labels[1].0, format!("{}  Bindebogen", icons::BEZIER_CURVE));
            assert!(labels[2].0.starts_with(icons::PAUSE));
            assert_eq!(labels[3].0, format!("{}  Einfach", icons::LINE_VERTICAL));
            for (index, object) in [
                Object::Note(0),
                Object::Span(0),
                Object::Rest(0),
                Object::Barline(0),
            ]
            .into_iter()
            .enumerate()
            {
                assert_eq!(app.ordered_objects()[index], object);
            }
            let position = labels[1].1;
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            assert_eq!(app.page.selected_object, Some(Object::Span(0)));
            assert!(app.page.selected.is_none() && app.page.selected_rest.is_none());
            assert!(!app.page.show_object_list);
            let output = render(&mut app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == format!("{}  Objekte", icons::LIST_BULLETS) =>
                    {
                        let bounds = shape.shape.visual_bounding_rect();
                        shape
                            .clip_rect
                            .contains_rect(bounds)
                            .then_some(bounds.center())
                    }
                    _ => None,
                })
                .expect("list tab remains available from details");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(app.page.show_object_list);
            assert_eq!(app.page.selected_object, Some(Object::Span(0)));
        }
    }

    #[test]
    fn canvas_groups_and_names_initial_key_signatures() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            for zoom in [0.4, 1.0] {
                for fifths in [-4_i8, 3] {
                    for confirmed in [false, true] {
                        let context = egui::Context::default();
                        let mut app = demo_app();
                        app.page.zoom = zoom;
                        app.page.score.notes.clear();
                        app.page.score.rests.clear();
                        app.page.score.barlines.clear();
                        app.page.score.spans.clear();
                        app.page.score.key_changes.clear();
                        app.page.score.staves.truncate(1);
                        let header = &mut app.page.score.staves[0].header;
                        header.clef_bounds = None;
                        header.time_bounds = None;
                        header.confirmed = confirmed;
                        let key = KeySignature { fifths };
                        header.key_signature = Some(key);
                        header.key_bounds = (0..fifths.unsigned_abs())
                            .map(|index| libreomr::model::SymbolBounds {
                                left: 120.0 + f32::from(index) * 18.0,
                                right: 132.0 + f32::from(index) * 18.0,
                                top: 110.0 - f32::from(index) * 6.0,
                                bottom: 150.0 - f32::from(index) * 6.0,
                            })
                            .collect();
                        let before = serde_json::to_value(&app.page.score).unwrap();
                        let output = context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |context| app.canvas(context),
                        );
                        let color = if confirmed {
                            Color32::from_rgb(0, 135, 85)
                        } else {
                            Color32::from_rgb(0, 105, 125)
                        };
                        let frames: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Rect(rect) if rect.stroke.color == color => {
                                    Some(rect.rect)
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(frames.len(), 1, "one frame for {fifths} at zoom {zoom}");
                        assert!(
                            (frames[0].width()
                                - (12.0 + f32::from(fifths.unsigned_abs() - 1) * 18.0) * zoom)
                                .abs()
                                < 0.01
                        );
                        let labels: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) if text.galley.text() == key.label() => {
                                    Some((shape, text))
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(labels.len(), 1);
                        assert_eq!(labels[0].1.fallback_color, color);
                        assert!(
                            labels[0]
                                .0
                                .clip_rect
                                .contains_rect(labels[0].0.shape.visual_bounding_rect())
                        );
                        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    }
                }
            }
        }
    }

    #[test]
    fn canvas_drags_all_objects_and_both_anchors_with_one_undo_step() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            for zoom in [0.4, 1.0] {
                for target in [
                    DragTarget::Body(Object::Note(0)),
                    DragTarget::Body(Object::Rest(0)),
                    DragTarget::Body(Object::Barline(0)),
                    DragTarget::Body(Object::Span(0)),
                    DragTarget::Body(Object::KeyChange(0)),
                    DragTarget::Body(Object::TimeChange(0)),
                    DragTarget::Body(Object::Dynamic(0)),
                    DragTarget::Body(Object::RepeatMark(0)),
                    DragTarget::Anchor(0, false),
                    DragTarget::Anchor(0, true),
                ] {
                    let context = egui::Context::default();
                    let mut app = demo_app();
                    app.page.zoom = zoom;
                    app.page.score.notes.truncate(1);
                    app.page.score.notes[0].x = 200.0;
                    app.page.score.notes[0].y =
                        app.page.score.staves[0].y_at_pitch(app.page.score.notes[0].pitch);
                    app.page.score.rests.clear();
                    app.page
                        .score
                        .add_rest(300.0, 130.0, Duration::Quarter)
                        .unwrap();
                    app.page.score.barlines = vec![Barline {
                        staff: 0,
                        x: 350.0,
                        kind: BarlineKind::Single,
                        confirmed: false,
                    }];
                    app.page.score.spans = vec![Span {
                        staff: 0,
                        kind: SpanKind::Slur,
                        start: [250.0, 70.0],
                        end: [400.0, 80.0],
                        height: -20.0,
                        confirmed: false,
                    }];
                    app.page.score.key_changes = vec![KeyChange {
                        staff: 0,
                        x: 450.0,
                        key_signature: KeySignature { fifths: -4 },
                        bounds: vec![],
                        confirmed: false,
                    }];
                    app.page.score.time_changes = vec![TimeChange {
                        staff: 0,
                        x: 550.0,
                        time_signature: TimeSignature::Common,
                        bounds: vec![],
                        confirmed: false,
                    }];
                    app.page.score.dynamics = vec![Dynamic {
                        staff: 0,
                        x: 650.0,
                        y: 210.0,
                        kind: DynamicKind::Mf,
                        bounds: None,
                        confirmed: false,
                    }];
                    app.page.score.repeat_marks = vec![RepeatMark {
                        staff: 0,
                        x: 650.0,
                        y: 55.0,
                        kind: RepeatMarkKind::Ending,
                        end_x: 750.0,
                        numbers: "1".into(),
                        closed: true,
                        bounds: None,
                        confirmed: false,
                    }];
                    let source = match target {
                        DragTarget::Body(Object::Note(_)) => {
                            [app.page.score.notes[0].x, app.page.score.notes[0].y]
                        }
                        DragTarget::Body(Object::Rest(_)) => [300.0, 130.0],
                        DragTarget::Body(Object::Barline(_)) => {
                            [350.0, app.page.score.staves[0].lines[2]]
                        }
                        DragTarget::Body(Object::Span(_)) => {
                            app.page.score.spans[0].curve_point(0.5)
                        }
                        DragTarget::Body(Object::KeyChange(_)) => {
                            [450.0, app.page.score.staves[0].lines[2]]
                        }
                        DragTarget::Body(Object::Dynamic(_)) => [650.0, 210.0],
                        DragTarget::Body(Object::RepeatMark(_)) => [650.0, 55.0],
                        DragTarget::Body(Object::TimeChange(_)) => {
                            [550.0, app.page.score.staves[0].lines[2]]
                        }
                        DragTarget::Anchor(_, right) => {
                            if right {
                                app.page.score.spans[0].end
                            } else {
                                app.page.score.spans[0].start
                            }
                        }
                    };
                    if matches!(target, DragTarget::Anchor(..)) {
                        app.select_object(Some(Object::Span(0)));
                    }
                    let before = serde_json::to_value(&app.page.score).unwrap();
                    let render = |app: &mut App, events| {
                        context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                                events,
                                ..Default::default()
                            },
                            |context| app.canvas(context),
                        )
                    };
                    for _pass in 0..3 {
                        render(&mut app, vec![]);
                    }
                    let start = Pos2::new(16.0, 16.0) + Vec2::from(source) * zoom;
                    let end = start + Vec2::new(30.0, 10.0) * zoom;
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(start),
                            egui::Event::PointerButton {
                                pos: start,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    render(
                        &mut app,
                        vec![egui::Event::PointerMoved(start.lerp(end, 0.7))],
                    );
                    assert_eq!(
                        app.page.object_drag.as_ref().map(|drag| drag.target),
                        Some(target),
                        "{target:?}, zoom {zoom}"
                    );
                    render(&mut app, vec![egui::Event::PointerMoved(end)]);
                    render(
                        &mut app,
                        vec![egui::Event::PointerButton {
                            pos: end,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        }],
                    );
                    assert!(app.page.object_drag.is_none());
                    assert_eq!(app.page.undo.len(), 1, "{target:?}");
                    match target {
                        DragTarget::Body(Object::Note(_)) => {
                            assert!((app.page.score.notes[0].x - 230.0).abs() < 0.01);
                            assert_eq!(
                                app.page.score.notes[0].pitch,
                                app.page.score.staves[0].pitch_at(source[1] + 10.0)
                            );
                        }
                        DragTarget::Body(Object::Rest(_)) => {
                            assert!((app.page.score.rests[0].x - 330.0).abs() < 0.01);
                            assert!((app.page.score.rests[0].y - 140.0).abs() < 0.01);
                        }
                        DragTarget::Body(Object::Barline(_)) => {
                            assert!((app.page.score.barlines[0].x - 380.0).abs() < 0.01)
                        }
                        DragTarget::Body(Object::Span(_)) => {
                            assert!((app.page.score.spans[0].start[0] - 280.0).abs() < 0.01);
                            assert!((app.page.score.spans[0].end[0] - 430.0).abs() < 0.01);
                        }
                        DragTarget::Body(Object::RepeatMark(_)) => {
                            let mark = &app.page.score.repeat_marks[0];
                            assert!((mark.x - 680.0).abs() < 0.01);
                            assert!((mark.end_x - 780.0).abs() < 0.01);
                            assert!((mark.y - 65.0).abs() < 0.01);
                            assert!(mark.confirmed);
                        }
                        DragTarget::Body(Object::Dynamic(_)) => {
                            assert!((app.page.score.dynamics[0].x - 680.0).abs() < 0.01);
                            assert!((app.page.score.dynamics[0].y - 220.0).abs() < 0.01);
                            assert!(app.page.score.dynamics[0].confirmed);
                        }
                        DragTarget::Body(Object::TimeChange(_)) => {
                            assert!((app.page.score.time_changes[0].x - 580.0).abs() < 0.01);
                            assert!(app.page.score.time_changes[0].confirmed);
                        }
                        DragTarget::Body(Object::KeyChange(_)) => {
                            assert!((app.page.score.key_changes[0].x - 480.0).abs() < 0.01);
                            assert!(app.page.score.key_changes[0].confirmed);
                        }
                        DragTarget::Anchor(_, right) => {
                            let span = &app.page.score.spans[0];
                            let (moved, fixed, expected_fixed) = if right {
                                (span.end, span.start, [250.0, 70.0])
                            } else {
                                (span.start, span.end, [400.0, 80.0])
                            };
                            assert!((moved[0] - source[0] - 30.0).abs() < 0.01);
                            assert!((moved[1] - source[1] - 10.0).abs() < 0.01);
                            assert_eq!(fixed, expected_fixed);
                        }
                    }
                    assert!(app.page.score.validate().is_ok());
                    let after = serde_json::to_value(&app.page.score).unwrap();
                    app.undo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    app.redo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), after);
                }
            }
        }
    }

    #[test]
    fn preview_drags_both_span_anchors_with_one_undo_step_at_both_scales() {
        for kind in SpanKind::ALL {
            for fit in [false, true] {
                for right in [false, true] {
                    let context = egui::Context::default();
                    let mut app = demo_app();
                    app.page.image = GrayImage::new(1200, 400);
                    app.page.score = Score::default();
                    app.page.score.staves.push(Staff {
                        lines: [60.0, 72.0, 84.0, 96.0, 108.0],
                        left: 20.0,
                        right: 1100.0,
                        header: Default::default(),
                    });
                    for x in [160.0, 300.0, 480.0, 900.0] {
                        app.page.score.add_note(x, 108.0).unwrap();
                    }
                    app.page.score.spans.push(Span {
                        staff: 0,
                        kind,
                        start: [160.0, 138.0],
                        end: [480.0, 138.0],
                        height: 8.0,
                        confirmed: false,
                    });
                    app.select_object(Some(Object::Span(0)));
                    app.page.preview.fit_width = fit;
                    let before = serde_json::to_value(&app.page.score).unwrap();
                    let render = |app: &mut App, events| {
                        context.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(
                                    Pos2::ZERO,
                                    Vec2::new(760.0, 520.0),
                                )),
                                events,
                                ..Default::default()
                            },
                            |context| app.preview(context),
                        )
                    };
                    for _ in 0..3 {
                        render(&mut app, vec![]);
                    }
                    let output = render(&mut app, vec![]);
                    let handles: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Circle(circle)
                                if circle.radius == 6.0
                                    && circle.stroke.color == Color32::from_rgb(0, 95, 180) =>
                            {
                                Some(circle.center)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(handles.len(), 2, "{kind:?}, fit {fit}");
                    let start = handles[usize::from(right)];
                    let end = start + Vec2::new(20.0, 8.0);
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(start),
                            egui::Event::PointerButton {
                                pos: start,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: Default::default(),
                            },
                        ],
                    );
                    render(
                        &mut app,
                        vec![egui::Event::PointerMoved(start.lerp(end, 0.6))],
                    );
                    render(&mut app, vec![egui::Event::PointerMoved(end)]);
                    render(
                        &mut app,
                        vec![egui::Event::PointerButton {
                            pos: end,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: Default::default(),
                        }],
                    );
                    assert_eq!(app.page.undo.len(), 1, "{kind:?}, fit {fit}, right {right}");
                    let span = &app.page.score.spans[0];
                    let (moved, fixed, original, expected_fixed) = if right {
                        (span.end, span.start, [480.0, 138.0], [160.0, 138.0])
                    } else {
                        (span.start, span.end, [160.0, 138.0], [480.0, 138.0])
                    };
                    assert!(
                        moved[0] > original[0] && moved[1] > original[1],
                        "{kind:?}, fit {fit}, right {right}: {moved:?}"
                    );
                    assert_eq!(fixed, expected_fixed);
                    assert!(span.confirmed);
                    assert!(app.page.object_drag.is_none());
                    assert!(app.page.score.validate().is_ok());
                    let after = serde_json::to_value(&app.page.score).unwrap();
                    app.undo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
                    app.redo();
                    assert_eq!(serde_json::to_value(&app.page.score).unwrap(), after);
                }
            }
        }
    }

    #[test]
    fn span_and_key_change_editors_add_changes_and_delete_with_undo() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            context.style_mut(|style| style.animation_time = 0.0);
            let mut app = demo_app();
            app.page.score.spans.clear();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            ui.set_max_width(230.0);
                            app.add_menu(ui);
                            app.key_change_editor(ui);
                            app.span_editor(ui);
                        });
                    },
                )
            };
            let click = |app: &mut App, label: &str| {
                let output = render(app, Vec::new());
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = shape.shape.visual_bounding_rect();
                            shape
                                .clip_rect
                                .contains_rect(bounds)
                                .then_some(bounds.center())
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("visible control {label}"));
                for pressed in [true, false] {
                    render(
                        app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::default(),
                            },
                        ],
                    );
                }
            };
            for _pass in 0..3 {
                render(&mut app, Vec::new());
            }
            for (index, kind) in SpanKind::ALL.into_iter().enumerate() {
                click(&mut app, icons::PLUS);
                click(&mut app, kind.label());
                assert_eq!(app.page.score.spans.len(), 1);
                assert_eq!(app.page.score.spans[0].kind, kind);
                assert_eq!(app.page.score.spans[0].staff, app.page.selected_staff);
                assert!(app.page.score.validate().is_ok());
                let next = SpanKind::ALL[(index + 1) % SpanKind::ALL.len()];
                click(&mut app, kind.label());
                click(&mut app, next.label());
                assert_eq!(app.page.score.spans[0].kind, next);
                app.undo();
                assert_eq!(app.page.score.spans[0].kind, kind);
                app.redo();
                assert_eq!(app.page.score.spans[0].kind, next);
                app.select_object(Some(Object::Span(0)));
                click(&mut app, "Geprueft");
                assert!(!app.page.score.spans[0].confirmed);
                let restored: Score =
                    serde_json::from_slice(&serde_json::to_vec(&app.page.score).unwrap()).unwrap();
                assert_eq!(restored.spans[0].kind, next);
                click(&mut app, "\u{00d7}");
                assert!(app.page.score.spans.is_empty());
                app.undo();
                assert_eq!(app.page.score.spans[0].kind, next);
                app.redo();
                assert!(app.page.score.spans.is_empty());
            }
            click(&mut app, icons::PLUS);
            click(&mut app, "Tonartwechsel");
            assert_eq!(app.page.score.key_changes.len(), 1);
            assert_eq!(app.page.selected_object, Some(Object::KeyChange(0)));
            let mut previous = KeySignature { fifths: 0 };
            for fifths in [-7, -4, -1, 1, 4, 7, 0] {
                let next = KeySignature { fifths };
                click(&mut app, &previous.label());
                let menu = render(&mut app, Vec::new());
                let pointer = menu
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if (-7..=7).any(|fifths| {
                                KeySignature { fifths }.label() == text.galley.text()
                            }) && shape
                                .clip_rect
                                .contains_rect(shape.shape.visual_bounding_rect()) =>
                        {
                            Some(shape.shape.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .expect("visible key menu");
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pointer),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: Vec2::new(0.0, if fifths >= 4 { -900.0 } else { 900.0 }),
                            modifiers: Default::default(),
                        },
                    ],
                );
                click(&mut app, &next.label());
                assert_eq!(app.page.score.key_changes[0].key_signature, next);
                app.undo();
                assert_eq!(app.page.score.key_changes[0].key_signature, previous);
                app.redo();
                assert_eq!(app.page.score.key_changes[0].key_signature, next);
                app.select_object(Some(Object::KeyChange(0)));
                previous = next;
            }
            click(&mut app, "Tonartwechsel geprueft");
            assert!(!app.page.score.key_changes[0].confirmed);
            assert!(app.page.score.validate().is_ok());
            let restored: Score =
                serde_json::from_slice(&serde_json::to_vec(&app.page.score).unwrap()).unwrap();
            assert_eq!(restored.key_changes[0].key_signature, previous);
            click(&mut app, "\u{00d7}");
            assert!(app.page.score.key_changes.is_empty());
            app.undo();
            assert_eq!(app.page.score.key_changes[0].key_signature, previous);
            app.redo();
            assert!(app.page.score.key_changes.is_empty());
        }
    }

    #[test]
    fn preview_follows_active_staff_and_edits_and_can_be_hidden() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            let rest = app
                .page
                .score
                .add_rest(450.0, app.page.score.staves[0].lines[2], Duration::Whole)
                .unwrap();
            app.page.score.rests[rest].measures = Some(8);
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.preview(context);
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            let has_text = |output: &egui::FullOutput, expected: &str| {
                output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == expected))
            };
            for _pass in 0..3 {
                render(&mut app, Vec::new());
            }
            let output = render(&mut app, Vec::new());
            assert!(has_text(&output, "Vorschau - Notensystem 1"));
            assert!(has_text(&output, "8"));
            app.page.selected_staff = 1;
            let output = render(&mut app, Vec::new());
            assert!(has_text(&output, "Vorschau - Notensystem 2"));
            assert!(!has_text(&output, "8"));
            app.page.selected_staff = 0;
            app.snapshot();
            app.page.score.rests[rest].measures = Some(12);
            assert!(has_text(&render(&mut app, Vec::new()), "12"));
            app.undo();
            assert!(has_text(&render(&mut app, Vec::new()), "8"));
            app.redo();
            assert!(has_text(&render(&mut app, Vec::new()), "12"));
            let output = render(&mut app, Vec::new());
            let close = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "\u{00d7}" => {
                        let bounds = shape.shape.visual_bounding_rect();
                        shape
                            .clip_rect
                            .contains_rect(bounds)
                            .then_some(bounds.center())
                    }
                    _ => None,
                })
                .expect("preview close button visible");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(close),
                        egui::Event::PointerButton {
                            pos: close,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
            }
            assert!(!app.show_preview);
            let output = render(&mut app, Vec::new());
            assert!(!has_text(&output, "Vorschau - Notensystem 1"));
            let toggle = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Vorschau" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .expect("preview checkbox remains available");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(toggle),
                        egui::Event::PointerButton {
                            pos: toggle,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                );
            }
            assert!(app.show_preview);
            assert!(has_text(
                &render(&mut app, Vec::new()),
                "Vorschau - Notensystem 1"
            ));
            app.page.score = Score::default();
            assert!(has_text(&render(&mut app, Vec::new()), "Kein Notensystem"));
        }
    }

    #[test]
    fn system_groups_can_be_collapsed_and_reopened_without_score_changes() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            context.style_mut(|style| style.animation_time = 0.0);
            let mut app = demo_app();
            let original = serde_json::to_value(&app.page.score).unwrap();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            ui.set_max_width(300.0);
                            app.object_list(ui);
                        });
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            for should_be_visible in [false, true] {
                let output = render(&mut app, vec![]);
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if text.galley.text().starts_with("Notensystem 1 (") =>
                        {
                            Some(shape.shape.visual_bounding_rect().center())
                        }
                        _ => None,
                    })
                    .unwrap();
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                let output = render(&mut app, vec![]);
                assert_eq!(
                    output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text().starts_with(icons::MUSIC_NOTE))),
                    should_be_visible
                );
            }
            assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
            assert!(app.page.undo.is_empty());
        }
    }

    #[test]
    fn review_badges_confirm_all_object_kinds_and_restore_with_undo() {
        for object in [
            Object::Note(0),
            Object::Rest(0),
            Object::Barline(0),
            Object::Span(0),
            Object::KeyChange(0),
        ] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.page.score.notes[0].confirmed = false;
            app.page
                .score
                .add_rest(300.0, 130.0, Duration::Quarter)
                .unwrap();
            app.page.score.rests[0].confirmed = false;
            app.page.score.barlines.push(Barline {
                staff: 0,
                x: 350.0,
                kind: BarlineKind::Single,
                confirmed: false,
            });
            app.page.score.spans.push(Span {
                staff: 0,
                kind: SpanKind::Slur,
                start: [250.0, 70.0],
                end: [400.0, 80.0],
                height: -20.0,
                confirmed: false,
            });
            app.page.score.key_changes.push(KeyChange {
                staff: 0,
                x: 450.0,
                key_signature: KeySignature { fifths: -2 },
                bounds: vec![],
                confirmed: false,
            });
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            ui.set_max_width(300.0);
                            app.object_row(ui, object);
                        });
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.stroke.color == Color32::from_rgb(185, 115, 30) =>
                    {
                        Some(circle.center)
                    }
                    _ => None,
                })
                .expect("unchecked badge visible");
            let before = app.review_progress().0;
            for expected in [true, false] {
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert_eq!(app.is_confirmed(object), expected);
                assert_eq!(app.review_progress().0, before + usize::from(expected));
                assert_eq!(app.page.selected_object, None);
                let restored: Score =
                    serde_json::from_value(serde_json::to_value(&app.page.score).unwrap()).unwrap();
                assert_eq!(
                    restored.barlines[0].confirmed,
                    app.page.score.barlines[0].confirmed
                );
            }
            app.undo();
            assert!(app.is_confirmed(object));
            app.redo();
            assert!(!app.is_confirmed(object));
        }
    }

    #[test]
    fn review_navigation_wraps_and_never_mutates_the_score() {
        let mut app = demo_app();
        let rest = app
            .page
            .score
            .add_rest(450.0, 130.0, Duration::Eighth)
            .unwrap();
        app.page.score.rests[rest].confirmed = false;
        let original = serde_json::to_value(&app.page.score).unwrap();
        let (_, total) = app.review_progress();
        assert_eq!(total, app.page.score.notes.len() + 1);
        let unchecked: Vec<_> = app
            .ordered_objects()
            .into_iter()
            .filter(|object| !app.is_confirmed(*object))
            .collect();
        for expected in unchecked.iter().chain(unchecked.iter().take(1)) {
            app.next_unchecked();
            assert_eq!(app.page.selected_object, Some(*expected));
            assert!(app.page.reveal_selection);
            assert!(app.page.undo.is_empty());
        }
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
        for note in &mut app.page.score.notes {
            note.confirmed = true;
        }
        app.select_object(None);
        app.next_unchecked();
        assert_eq!(app.page.selected_object, Some(Object::Rest(rest)));
        app.page.score.rests[rest].confirmed = true;
        app.select_object(None);
        app.next_unchecked();
        assert_eq!(app.page.selected_object, None);
        assert_eq!(app.review_progress(), (total, total));
        app.page.score = Score::default();
        app.next_unchecked();
        assert_eq!(app.review_progress(), (0, 0));
    }

    #[test]
    fn musicxml_file_export_is_atomic_and_preserves_project_state() {
        let mut app = demo_app();
        app.page.name = "Test & Stimme.tif".into();
        app.dirty = true;
        app.project_path = Some(PathBuf::from("original.libreomr.json"));
        app.snapshot();
        let score = serde_json::to_string(&app.page.score).unwrap();
        let history = app.page.undo.len();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("export.musicxml");
        app.write_musicxml_to(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let document = String::from_utf8(bytes.clone()).unwrap();
        assert!(document.contains("<score-partwise version=\"4.0\">"));
        assert!(document.contains("<work-title>Test &amp; Stimme</work-title>"));
        assert_eq!(serde_json::to_string(&app.page.score).unwrap(), score);
        assert!(app.dirty);
        assert_eq!(
            app.project_path,
            Some(PathBuf::from("original.libreomr.json"))
        );
        assert_eq!(app.page.undo.len(), history);
        app.write_musicxml_to(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(
            app.write_musicxml_to(&directory.path().join("missing/export.musicxml"))
                .is_err()
        );
        app.page.score.notes[0].pitch = -1;
        assert!(app.write_musicxml_to(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn dynamics_can_be_inserted_reviewed_deleted_and_undone() {
        let mut app = demo_app();
        for note in &mut app.page.score.notes {
            note.confirmed = true;
        }
        let before = serde_json::to_value(&app.page.score).unwrap();
        app.insert_at(Vec2::new(450.0, 210.0), Insert::Dynamic);
        assert_eq!(app.page.selected_object, Some(Object::Dynamic(0)));
        assert_eq!(app.page.score.dynamics.len(), 1);
        assert_eq!(app.page.score.dynamics[0].kind, DynamicKind::Mf);
        assert!(app.is_confirmed(Object::Dynamic(0)));
        assert!(app.ordered_objects().contains(&Object::Dynamic(0)));
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        app.redo();
        app.page.score.dynamics[0].confirmed = false;
        app.next_unchecked();
        assert_eq!(app.page.selected_object, Some(Object::Dynamic(0)));
        let restored: Score =
            serde_json::from_value(serde_json::to_value(&app.page.score).unwrap()).unwrap();
        assert_eq!(restored.dynamics[0].kind, DynamicKind::Mf);
        app.delete_object(Object::Dynamic(0));
        assert!(app.page.score.dynamics.is_empty());
        app.undo();
        assert_eq!(app.page.score.dynamics.len(), 1);
        assert!(!app.page.score.dynamics[0].confirmed);
    }

    #[test]
    fn ending_editor_accepts_comma_separated_numbers_and_undo() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            let mut app = demo_app();
            app.insert_at(
                Vec2::new(450.0, 60.0),
                Insert::RepeatMark(RepeatMarkKind::Ending),
            );
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| app.inspector(context),
                )
            };
            for _ in 0..3 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let position = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "1" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .max_by(|a, b| a.y.total_cmp(&b.y))
                .expect("ending number input visible");
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
            render(
                &mut app,
                vec![
                    egui::Event::Key {
                        key: egui::Key::End,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    },
                    egui::Event::Text(",".into()),
                ],
            );
            assert_eq!(app.page.score.repeat_marks[0].numbers, "1");
            render(&mut app, vec![egui::Event::Text("2".into())]);
            assert_eq!(app.page.score.repeat_marks[0].numbers, "1,2");
            assert!(app.page.score.validate().is_ok());
            app.undo();
            assert_eq!(app.page.score.repeat_marks[0].numbers, "1");
            app.redo();
            assert_eq!(app.page.score.repeat_marks[0].numbers, "1,2");
        }
    }

    #[test]
    fn repeat_marks_can_be_inserted_reviewed_deleted_and_undone() {
        let mut app = demo_app();
        for note in &mut app.page.score.notes {
            note.confirmed = true;
        }
        let before = serde_json::to_value(&app.page.score).unwrap();
        app.insert_at(
            Vec2::new(450.0, 210.0),
            Insert::RepeatMark(RepeatMarkKind::Ending),
        );
        assert_eq!(app.page.selected_object, Some(Object::RepeatMark(0)));
        assert_eq!(app.page.score.repeat_marks.len(), 1);
        assert_eq!(app.page.score.repeat_marks[0].kind, RepeatMarkKind::Ending);
        assert!(app.is_confirmed(Object::RepeatMark(0)));
        assert!(app.ordered_objects().contains(&Object::RepeatMark(0)));
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        app.redo();
        app.page.score.repeat_marks[0].confirmed = false;
        app.next_unchecked();
        assert_eq!(app.page.selected_object, Some(Object::RepeatMark(0)));
        let restored: Score =
            serde_json::from_value(serde_json::to_value(&app.page.score).unwrap()).unwrap();
        assert_eq!(restored.repeat_marks[0].kind, RepeatMarkKind::Ending);
        app.delete_object(Object::RepeatMark(0));
        assert!(app.page.score.repeat_marks.is_empty());
        app.undo();
        assert_eq!(app.page.score.repeat_marks.len(), 1);
        assert!(!app.page.score.repeat_marks[0].confirmed);
    }

    #[test]
    fn time_changes_can_be_inserted_reviewed_deleted_and_undone() {
        let mut app = demo_app();
        for note in &mut app.page.score.notes {
            note.confirmed = true;
        }
        let before = serde_json::to_value(&app.page.score).unwrap();
        app.insert_at(Vec2::new(450.0, 130.0), Insert::TimeChange);
        assert_eq!(app.page.selected_object, Some(Object::TimeChange(0)));
        assert_eq!(app.page.score.time_changes.len(), 1);
        assert_eq!(app.page.score.time_changes[0].x, 450.0);
        assert!(app.is_confirmed(Object::TimeChange(0)));
        assert!(app.ordered_objects().contains(&Object::TimeChange(0)));
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        app.redo();
        assert_eq!(app.page.score.time_changes.len(), 1);
        app.page.score.time_changes[0].confirmed = false;
        app.next_unchecked();
        assert_eq!(app.page.selected_object, Some(Object::TimeChange(0)));
        app.delete_object(Object::TimeChange(0));
        assert!(app.page.score.time_changes.is_empty());
        app.undo();
        assert_eq!(app.page.score.time_changes.len(), 1);
        assert!(!app.page.score.time_changes[0].confirmed);
    }

    #[test]
    fn inspector_respects_compact_width_for_overview_and_all_editors() {
        let size = Vec2::new(760.0, 520.0);
        let mut app = demo_app();
        app.page.score.add_rest(300.0, 130.0, Duration::SixtyFourth);
        for insert in [
            Insert::Barline,
            Insert::KeyChange,
            Insert::TimeChange,
            Insert::Dynamic,
            Insert::RepeatMark(RepeatMarkKind::Ending),
            Insert::Span(SpanKind::Decrescendo),
        ] {
            app.insert_at(Vec2::new(450.0, 130.0), insert);
        }
        app.page.score.key_changes[0].key_signature = KeySignature { fifths: -7 };
        app.page.score.barlines[0].kind = BarlineKind::RepeatBoth;
        app.page.score.time_changes[0].time_signature = TimeSignature::Meter {
            beats: 12,
            beat_unit: 16,
        };
        app.page.score.staves[0].header.time_signature = Some(TimeSignature::Meter {
            beats: 12,
            beat_unit: 16,
        });
        app.page.score.staves[0].header.key_signature = Some(KeySignature { fifths: -7 });
        let objects = app.ordered_objects();
        for width in [200.0, 240.0, 380.0] {
            let context = egui::Context::default();
            App::configure_style(&context);
            context.data_mut(|data| {
                data.insert_persisted(
                    egui::Id::new("inspector"),
                    egui::containers::panel::PanelState {
                        rect: Rect::from_min_size(
                            Pos2::new(size.x - width, 0.0),
                            Vec2::new(width, size.y),
                        ),
                    },
                );
            });
            // Include the overview, each editor and expanded system settings.
            for object in std::iter::once(None).chain(objects.iter().copied().map(Some)) {
                app.select_object(object);
                app.page.show_object_list = object.is_none();
                let mut header_position = None;
                for pass in 0..8 {
                    let events = if let (Some(pos), 4 | 5) = (header_position, pass) {
                        vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: pass == 4,
                                modifiers: Default::default(),
                            },
                        ]
                    } else {
                        vec![]
                    };
                    let output = context.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |context| app.inspector(context),
                    );
                    let panel = egui::containers::panel::PanelState::load(
                        &context,
                        egui::Id::new("inspector"),
                    )
                    .unwrap();
                    assert!(
                        (panel.rect.width() - width).abs() <= 1.0,
                        "{object:?}: requested {width}, got {:?}",
                        panel.rect
                    );
                    for shape in &output.shapes {
                        if let egui::Shape::Text(text) = &shape.shape {
                            let bounds = shape.shape.visual_bounding_rect();
                            if pass == 3 && text.galley.text() == "Systemangaben" {
                                header_position = Some(bounds.center());
                            }
                            if bounds.intersects(shape.clip_rect) {
                                assert!(
                                    bounds.left() >= shape.clip_rect.left() - 1.0
                                        && bounds.right() <= shape.clip_rect.right() + 1.0,
                                    "{object:?} at {width}: {:?} clipped: {bounds:?} / {:?}",
                                    text.galley.text(),
                                    shape.clip_rect
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn toolbar_logo_tracks_display_density_and_pixel_grid() {
        let context = egui::Context::default();
        App::configure_style(&context);
        let mut app = demo_app();
        let mut previous_key = None;
        for (density, theme) in [
            (1.0, egui::Theme::Light),
            (1.5, egui::Theme::Light),
            (2.0, egui::Theme::Light),
            (2.0, egui::Theme::Dark),
            (2.0, egui::Theme::Light),
            (1.0, egui::Theme::Light),
        ] {
            context.set_pixels_per_point(density);
            context.set_theme(theme);
            let mut render = || {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 520.0))),
                        ..Default::default()
                    },
                    |context| {
                        egui::CentralPanel::default().show(context, |ui| {
                            // Fractional layout positions must still paint on
                            // whole physical pixels, including at 150% scaling.
                            ui.add_space(0.3);
                            app.show_logo(ui);
                        });
                    },
                )
            };
            for _ in 0..3 {
                render();
            }
            let output = render();
            let texture = app.logo_texture.as_ref().unwrap();
            let pixel_size = texture.size();
            assert_ne!(previous_key, Some((pixel_size, theme)));
            previous_key = Some((pixel_size, theme));
            assert_eq!(app.logo_dark, theme == egui::Theme::Dark);
            assert!(pixel_size[0] <= (136.0 * density).round() as usize);
            assert!(pixel_size[1] <= (30.0 * density).round() as usize);
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill_texture_id() == texture.id() => {
                        Some(rect.rect)
                    }
                    _ => None,
                })
                .expect("logo must be painted");
            for (points, pixels) in [
                (rect.width(), pixel_size[0]),
                (rect.height(), pixel_size[1]),
            ] {
                assert!((points * density - pixels as f32).abs() < 0.001);
            }
            for coordinate in [rect.min.x, rect.min.y, rect.max.x, rect.max.y] {
                let pixels = coordinate * density;
                assert!((pixels - pixels.round()).abs() < 0.001);
            }
            assert!(
                output
                    .textures_delta
                    .set
                    .iter()
                    .all(|(id, _)| *id != texture.id()),
                "unchanged display density must reuse the prepared logo"
            );
        }
    }

    #[test]
    fn styled_workspace_fits_large_scans_and_small_windows() {
        for (size, theme) in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)]
            .into_iter()
            .flat_map(|size| [egui::Theme::Light, egui::Theme::Dark].map(|theme| (size, theme)))
        {
            let context = egui::Context::default();
            App::configure_style(&context);
            context.set_theme(theme);
            let mut app = demo_app();
            app.page.image = GrayImage::new(5318, 6908);
            app.page.fit_width = true;
            app.only_current_staff = true;
            app.page.name =
                "Ein_sehr_langer_Dateiname_fuer_eine_gescannte_Orchesterstimme.tif".into();
            let original = app.page.score.clone();
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        // Simulate a renderer that supports this 5318x6908 scan.
                        max_texture_side: Some(8192),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.toolbar(context);
                        app.inspector(context);
                        app.preview(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..4 {
                render(&mut app, vec![]);
            }
            let output = render(&mut app, vec![]);
            let logo_id = app.logo_texture.as_ref().unwrap().id();
            let logo = output
                .shapes
                .iter()
                .find(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Rect(rect) if rect.fill_texture_id() == logo_id)
                })
                .expect("project logo must be visible in the toolbar");
            let logo_bounds = logo.shape.visual_bounding_rect();
            assert!(logo.clip_rect.contains_rect(logo_bounds));
            assert!(logo_bounds.max.y <= 54.0);
            assert!(logo_bounds.width() > 4.0 * logo_bounds.height());
            for expected in [
                icons::GEAR,
                "Datei",
                "Einpassen",
                "Vorschau",
                app.page.name.as_str(),
                "Korrektur",
                icons::ARROW_RIGHT,
            ] {
                let shape = output
                    .shapes
                    .iter()
                    .find(|shape| {
                        matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == expected)
                    })
                    .expect(expected);
                let bounds = shape.shape.visual_bounding_rect();
                assert!(
                    shape.clip_rect.contains_rect(bounds),
                    "{size:?}: {expected} clipped: {bounds:?} / {:?}",
                    shape.clip_rect
                );
                assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(bounds));
                if matches!(expected, "Datei" | "Einpassen" | "Vorschau") || expected == icons::GEAR
                {
                    assert!(
                        bounds.max.y <= 54.0,
                        "{expected} must stay in the single toolbar row"
                    );
                }
            }
            for hidden in ["Bild oeffnen", "Speichern", "Demonstration"] {
                assert!(
                    !output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == hidden)),
                    "{hidden} must not appear in the toolbar"
                );
            }
            let file_menu = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "Datei" => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap();
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(file_menu),
                        egui::Event::PointerButton {
                            pos: file_menu,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            let menu_output = render(&mut app, vec![]);
            for action in [
                "Bild oeffnen",
                "Projekt oeffnen",
                "Speichern",
                "MusicXML exportieren",
                "Neu erkennen",
            ] {
                let shape = menu_output
                    .shapes
                    .iter()
                    .find(|shape| {
                        matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == action)
                    })
                    .expect(action);
                assert!(
                    shape
                        .clip_rect
                        .contains_rect(shape.shape.visual_bounding_rect()),
                    "{action} clipped in Datei menu"
                );
            }
            render(
                &mut app,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            assert!(
                app.page.zoom < 0.2,
                "large scans must fit below the old minimum"
            );
            assert!(app.page.image.width() as f32 * app.page.zoom < size.x - 260.0);
            let button = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == icons::ARROW_RIGHT => {
                        Some(shape.shape.visual_bounding_rect().center())
                    }
                    _ => None,
                })
                .unwrap();
            for pressed in [true, false] {
                render(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(button),
                        egui::Event::PointerButton {
                            pos: button,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(app.page.selected_object.is_some());
            assert_eq!(
                serde_json::to_value(&app.page.score).unwrap(),
                serde_json::to_value(original).unwrap()
            );
        }
    }

    #[test]
    fn musical_pitch_selector_updates_position_and_is_undoable() {
        for size in [Vec2::new(760.0, 520.0), Vec2::new(1280.0, 820.0)] {
            let context = egui::Context::default();
            App::configure_style(&context);
            let mut app = demo_app();
            app.page.score.notes[0].pitch = 28;
            app.page.score.notes[0].y = app.page.score.staves[0].y_at_pitch(28);
            app.select_object(Some(Object::Note(0)));
            let render = |app: &mut App, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| {
                        app.inspector(context);
                        app.canvas(context);
                    },
                )
            };
            for _pass in 0..3 {
                render(&mut app, vec![]);
            }
            for label in ["C", "D"] {
                let output = render(&mut app, vec![]);
                let position = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = shape.shape.visual_bounding_rect();
                            shape
                                .clip_rect
                                .contains_rect(bounds)
                                .then_some(bounds.center())
                        }
                        _ => None,
                    })
                    .expect("pitch selector option is visible");
                for pressed in [true, false] {
                    render(
                        &mut app,
                        vec![
                            egui::Event::PointerMoved(position),
                            egui::Event::PointerButton {
                                pos: position,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
            }
            assert_eq!(app.page.score.notes[0].pitch, 29);
            assert_eq!(
                app.page.score.notes[0].y,
                app.page.score.staves[0].y_at_pitch(29)
            );
            app.undo();
            assert_eq!(app.page.score.notes[0].pitch, 28);
            app.redo();
            assert_eq!(app.page.score.notes[0].pitch, 29);
        }
    }

    fn edit_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn send_edit_events(app: &mut App, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        let context = egui::Context::default();
        let _ = context.run(
            egui::RawInput {
                events,
                modifiers,
                ..Default::default()
            },
            |context| app.shortcuts(context),
        );
    }

    #[test]
    fn capella_shortcuts_edit_values_and_set_dots_idempotently() {
        let mut app = demo_app();
        app.select_object(Some(Object::Note(0)));
        for (key, duration) in [
            (egui::Key::Num1, Duration::Whole),
            (egui::Key::Num2, Duration::Half),
            (egui::Key::Num4, Duration::Quarter),
            (egui::Key::Num8, Duration::Eighth),
            (egui::Key::Num6, Duration::Sixteenth),
            (egui::Key::Num3, Duration::ThirtySecond),
            (egui::Key::Num9, Duration::SixtyFourth),
        ] {
            let count = app.page.undo.len();
            send_edit_events(
                &mut app,
                vec![
                    edit_key(key, egui::Modifiers::NONE),
                    egui::Event::Text(key.name().into()),
                ],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.page.score.notes[0].duration, duration);
            assert_eq!(
                app.page.undo.len(),
                count + 1,
                "one key/text pair is one edit"
            );
        }
        for (text, dots) in [(".", 1), (".", 1), (":", 2), ("0", 0)] {
            send_edit_events(
                &mut app,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.page.score.notes[0].dots, dots);
        }
        assert_eq!(
            app.page.undo.len(),
            10,
            "repeating . must not create undo entries"
        );
        assert!(app.dirty);
        app.undo();
        assert_eq!(app.page.score.notes[0].dots, 2);
        app.redo();
        assert_eq!(app.page.score.notes[0].dots, 0);
        assert!(app.page.score.validate().is_ok());
    }

    #[test]
    fn capella_shortcuts_use_layout_characters_and_recompute_accidentals() {
        let mut app = demo_app();
        app.page.score.notes.truncate(1);
        let mut following = app.page.score.notes[0].clone();
        following.x += 15.0;
        following.accidental_explicit = false;
        following.confirmed = true;
        app.page.score.notes.push(following);
        app.select_object(Some(Object::Note(0)));
        // US Shift+3 produces #, not the 32nd-note command bound to 3.
        // Option+3 produces # on other layouts.
        for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::ALT] {
            let duration = app.page.score.notes[0].duration;
            send_edit_events(
                &mut app,
                vec![
                    edit_key(egui::Key::Num3, modifiers),
                    egui::Event::Text("#".into()),
                ],
                modifiers,
            );
            assert_eq!(app.page.score.notes[0].duration, duration);
            assert_eq!(app.page.score.notes[0].accidental, 1);
            assert!(app.page.score.notes[0].accidental_explicit);
            assert_eq!(app.page.score.notes[1].accidental, 1);
            assert!(!app.page.score.notes[1].confirmed);
        }
        for (text, accidental) in [("b", -1), ("=", 0)] {
            send_edit_events(
                &mut app,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.page.score.notes[0].accidental, accidental);
            assert_eq!(app.page.score.notes[1].accidental, accidental);
        }
        app.undo();
        assert_eq!(app.page.score.notes[1].accidental, -1);
        app.redo();
        assert_eq!(app.page.score.notes[1].accidental, 0);
    }

    #[test]
    fn capella_shortcuts_handle_rests_other_objects_and_deletion() {
        let mut app = demo_app();
        app.page.score.add_rest(300.0, 130.0, Duration::Quarter);
        app.select_object(Some(Object::Rest(0)));
        send_edit_events(
            &mut app,
            vec![edit_key(egui::Key::Num8, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.page.score.rests[0].duration, Duration::Eighth);
        let count = app.page.undo.len();
        for text in ["#", "b", "="] {
            send_edit_events(
                &mut app,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
        }
        assert_eq!(app.page.undo.len(), count);
        app.page.score.rests[0].measures = Some(4);
        send_edit_events(
            &mut app,
            vec![egui::Event::Text("4".into())],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.page.score.rests[0].measures, Some(4));
        assert_eq!(app.page.undo.len(), count);
        send_edit_events(
            &mut app,
            vec![edit_key(egui::Key::Delete, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.page.score.rests.is_empty());
        app.undo();
        assert_eq!(app.page.score.rests.len(), 1);
        app.select_object(None);
        send_edit_events(
            &mut app,
            vec![egui::Event::Text("4".into())],
            egui::Modifiers::NONE,
        );
        assert_eq!(app.page.undo.len(), count);
        app.insert_at(Vec2::new(450.0, 130.0), Insert::Barline);
        let object = app.page.selected_object.unwrap();
        let before = serde_json::to_value(&app.page.score).unwrap();
        send_edit_events(
            &mut app,
            vec![egui::Event::Text("4".into())],
            egui::Modifiers::NONE,
        );
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        send_edit_events(
            &mut app,
            vec![edit_key(egui::Key::Backspace, egui::Modifiers::NONE)],
            egui::Modifiers::NONE,
        );
        assert!(app.page.selected_object.is_none());
        app.undo();
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), before);
        assert!(matches!(object, Object::Barline(_)));
    }

    #[test]
    fn capella_shortcuts_set_rest_dots_with_history_and_persistence() {
        let mut app = demo_app();
        app.page.score.add_rest(300.0, 130.0, Duration::Eighth);
        app.select_object(Some(Object::Rest(0)));
        for (text, dots, undo_count) in [(".", 1, 1), (".", 1, 1), (":", 2, 2), ("0", 0, 3)] {
            send_edit_events(
                &mut app,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.page.score.rests[0].dots, dots);
            assert_eq!(app.page.undo.len(), undo_count);
            assert_eq!(app.page.score.rests[0].duration, Duration::Eighth);
        }
        app.undo();
        assert_eq!(app.page.score.rests[0].dots, 2);
        let document = app.project_document().unwrap();
        assert_eq!(document.pages[0].score.rests[0].dots, 2);
        app.redo();
        assert_eq!(app.page.score.rests[0].dots, 0);
        app.select_object(Some(Object::Rest(0)));
        app.page.score.rests[0].measures = Some(4);
        for text in [".", ":", "0"] {
            send_edit_events(
                &mut app,
                vec![egui::Event::Text(text.into())],
                egui::Modifiers::NONE,
            );
            assert_eq!(app.page.score.rests[0].dots, 0);
        }
        assert!(app.page.score.validate().is_ok());
    }

    #[test]
    fn capella_shortcuts_leave_text_input_menus_and_busy_pages_alone() {
        let mut app = demo_app();
        app.select_object(Some(Object::Note(0)));
        let original = serde_json::to_value(&app.page.score).unwrap();
        for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::CTRL] {
            send_edit_events(
                &mut app,
                vec![
                    edit_key(egui::Key::Num8, modifiers),
                    egui::Event::Text("8".into()),
                ],
                modifiers,
            );
        }
        for import in [
            ImportState::Queued,
            ImportState::Running {
                stage: "test",
                progress: None,
            },
        ] {
            app.page.import = import;
            send_edit_events(
                &mut app,
                vec![egui::Event::Text("8".into())],
                egui::Modifiers::NONE,
            );
        }
        app.page.import = ImportState::Ready;
        app.page.canvas_context = Some((Vec2::ZERO, None));
        send_edit_events(
            &mut app,
            vec![egui::Event::Text("8".into())],
            egui::Modifiers::NONE,
        );
        app.page.canvas_context = None;
        let context = egui::Context::default();
        let mut text = String::new();
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            });
        });
        let _ = context.run(
            egui::RawInput {
                events: vec![egui::Event::Text("8".into())],
                ..Default::default()
            },
            |context| {
                assert!(context.wants_keyboard_input());
                app.shortcuts(context);
                egui::CentralPanel::default().show(context, |ui| {
                    ui.text_edit_singleline(&mut text);
                });
            },
        );
        assert_eq!(text, "8");
        context.memory_mut(|memory| memory.surrender_focus(memory.focused().unwrap()));
        egui::Popup::open_id(&context, egui::Id::new("test_menu"));
        let _ = context.run(
            egui::RawInput {
                events: vec![egui::Event::Text("8".into())],
                ..Default::default()
            },
            |context| app.shortcuts(context),
        );
        assert_eq!(serde_json::to_value(&app.page.score).unwrap(), original);
        assert!(app.page.undo.is_empty());
        assert!(!app.dirty);
    }

    #[test]
    fn keyboard_review_and_undo_obey_busy_state() {
        let context = egui::Context::default();
        let mut app = demo_app();
        let send_key = |app: &mut App, key, modifiers| {
            context.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |context| {
                    app.shortcuts(context);
                },
            )
        };
        app.snapshot();
        app.page.score.notes[0].dots = 2;
        send_key(&mut app, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(app.page.score.notes[0].dots, 0);
        send_key(
            &mut app,
            egui::Key::Z,
            egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        );
        assert_eq!(app.page.score.notes[0].dots, 2);
        send_key(&mut app, egui::Key::F6, egui::Modifiers::NONE);
        assert!(app.page.selected_object.is_some());
        send_key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.page.selected_object.is_none());
        let (_sender, receiver) = mpsc::channel();
        app.pending = Some(PendingJob {
            mode: JobMode::ReplaceDocument,
            receiver,
            progress: None,
            image_size: None,
            texture: None,
        });
        send_key(&mut app, egui::Key::F6, egui::Modifiers::NONE);
        assert!(app.page.selected_object.is_none());
    }

    #[test]
    fn undo_and_redo_restore_manual_changes() {
        let mut app = demo_app();
        app.snapshot();
        app.page.score.notes[0].articulation = Articulation::Accent;
        app.page.score.notes[0].dots = 1;
        app.page.score.notes[0].triplet = true;
        app.undo();
        assert_eq!(app.page.score.notes[0].articulation, Articulation::None);
        assert_eq!(app.page.score.notes[0].dots, 0);
        assert!(!app.page.score.notes[0].triplet);
        app.redo();
        assert_eq!(app.page.score.notes[0].articulation, Articulation::Accent);
        assert_eq!(app.page.score.notes[0].dots, 1);
        assert!(app.page.score.notes[0].triplet);
        app.undo();
        app.snapshot();
        app.page.score.notes.remove(0);
        assert!(app.page.redo.is_empty());
        assert_eq!(app.page.score.notes.len(), 15);
        app.snapshot();
        let header = libreomr::model::StaffHeader {
            clef: Clef::Bass,
            time_signature: Some(TimeSignature::Common),
            ..Default::default()
        };
        app.page.score.set_staff_header(0, header);
        let pitch = app.page.score.notes[0].pitch;
        app.undo();
        assert_eq!(app.page.score.staves[0].header.clef, Clef::Treble);
        assert_eq!(app.page.score.notes[0].pitch, pitch + 12);
        app.redo();
        assert_eq!(app.page.score.staves[0].header.clef, Clef::Bass);
        assert_eq!(app.page.score.notes[0].pitch, pitch);
        app.snapshot();
        let rest = app
            .page
            .score
            .add_rest(450.0, 130.0, Duration::Eighth)
            .unwrap();
        app.page.selected_rest = Some(rest);
        assert_eq!(app.nearest_rest(Vec2::new(450.0, 130.0)), Some(rest));
        app.undo();
        assert!(app.page.score.rests.is_empty());
        assert!(app.page.selected_rest.is_none());
        app.redo();
        assert_eq!(app.page.score.rests[rest].duration, Duration::Eighth);
        app.snapshot();
        let mut header = app.page.score.staves[0].header.clone();
        header.key_signature = Some(KeySignature { fifths: 2 });
        app.page.score.set_staff_header(0, header);
        app.undo();
        assert!(app.page.score.staves[0].header.key_signature.is_none());
        app.redo();
        assert_eq!(
            app.page.score.staves[0].header.key_signature,
            Some(KeySignature { fifths: 2 })
        );
    }
}
