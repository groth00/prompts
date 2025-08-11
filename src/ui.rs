use std::{
    collections::VecDeque,
    fmt::{self, Display},
    io::Cursor,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use iced::{
    Alignment, Element, Event,
    Length::{self},
    Subscription, Task, Theme, event,
    futures::{SinkExt, Stream, channel::mpsc::Sender},
    keyboard::{
        self,
        key::{Key, Named},
    },
    stream,
    widget::{
        self, Column, Image, PaneGrid, button, center, column, combo_box, container,
        image::Handle,
        mouse_area,
        pane_grid::{self, Axis, Configuration, Direction},
        pick_list, row, scrollable,
        shader::wgpu::naga::{FastHashMap, FastIndexMap},
        text,
        text_editor::{Action, Edit},
        text_input,
    },
    window,
};
use image::{GenericImageView, ImageReader};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::{Rng, distr::Uniform, rngs::ThreadRng};
use serde_json::{Map, Value};
use tokio::{sync::Semaphore, task::JoinHandle};
use zip::ZipArchive;

use crate::{
    PROJECT_DIRS,
    db::{
        PromptKind, SqliteError, Template, delete_prompt, fetch_prompts, save_prompt,
        update_prompt, update_prompt_name,
    },
    files::{CreateEntryKind, EntryKind, FileTree, MAX_VISIBLE},
    image_metadata::extract_image_metadata,
    nai::{self, ImageGenRequest, ImageGenerationError, ImageShape, Point, Position, Requester},
};

pub struct State {
    task_state: TaskState,
    task_ids: Vec<u64>,

    pub selected_theme: Theme,
    last_key: Option<(Key, keyboard::Modifiers)>,

    message: Option<String>,

    panes: pane_grid::State<Pane>,
    focus: Option<pane_grid::Pane>,

    rng: ThreadRng,

    base_prompt: widget::text_editor::Content,
    character_prompts: [CharacterContent; 6],
    curr_char: usize,
    image_shape: ImageShape,

    pool: Pool<SqliteConnectionManager>,

    base: PromptUi<String>,
    char: PromptUi<String>,
    template: PromptUi<Template>,

    previous_seed: u64,
    current_seed: Option<u64>,
    num_generate: String,

    files: FileTree,
    files_mode: FilesMode,
    new_folder_name: String,

    images: VecDeque<Vec<u8>>,
    thumbnails: VecDeque<Handle>,
    selected_image: Option<usize>,
    image_paths: VecDeque<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        let manager = SqliteConnectionManager::file(PROJECT_DIRS.data_dir().join("prompts.db"));
        let pool = r2d2::Pool::new(manager).expect("pool");

        {
            let conn = pool.get().unwrap();
            conn.execute_batch(include_str!(
                "../migrations/20250724234734_create_tables.up.sql"
            ))
            .expect("failed to create database tables");
        }

        let (base_options, base_map, char_options, char_map, template_options, template_map) =
            fetch_prompts(pool.clone()).expect("fetch_prompts");

        let character_prompts = [
            CharacterContent::new(),
            CharacterContent::new(),
            CharacterContent::new(),
            CharacterContent::new(),
            CharacterContent::new(),
            CharacterContent::new(),
        ];

        let files_pane = Pane::new(PaneId::Files);
        let prompts_pane = Pane::new(PaneId::Prompts);
        let image_pane = Pane::new(PaneId::Image);

        let panes = pane_grid::State::with_configuration(Configuration::Split {
            axis: Axis::Vertical,
            ratio: 0.4,
            a: Box::new(Configuration::Split {
                axis: Axis::Vertical,
                ratio: 0.3,
                a: Box::new(Configuration::Pane(files_pane)),
                b: Box::new(Configuration::Pane(prompts_pane)),
            }),
            b: Box::new(Configuration::Pane(image_pane)),
        });

        let state = Self {
            task_state: TaskState {
                ready: ChannelReady::NotReady,
                status: ChannelStatus::NotReady,
            },
            task_ids: Vec::new(),

            selected_theme: Theme::CatppuccinMacchiato,
            last_key: None,

            message: None,

            panes,
            focus: None,

            rng: rand::rng(),

            base_prompt: widget::text_editor::Content::new(),
            character_prompts,
            curr_char: 0,
            image_shape: ImageShape::Portrait,

            pool,

            base: PromptUi {
                kind: PromptKind::Base,
                options: combo_box::State::new(base_options),
                map: base_map,
                selected: None,
                rename: String::new(),
            },

            char: PromptUi {
                kind: PromptKind::Character,
                options: combo_box::State::new(char_options),
                map: char_map,
                selected: None,
                rename: String::new(),
            },

            template: PromptUi {
                kind: PromptKind::Template,
                options: combo_box::State::new(template_options),
                map: template_map,
                selected: None,
                rename: String::new(),
            },

            previous_seed: 0,
            current_seed: None,
            num_generate: 1.to_string(),

            files_mode: FilesMode::Normal,
            files: FileTree::new(PROJECT_DIRS.data_dir()),
            new_folder_name: String::new(),

            images: VecDeque::new(),
            thumbnails: VecDeque::new(),
            selected_image: None,
            image_paths: VecDeque::new(),
        };
        state
    }
}

impl State {
    pub fn refresh_prompts(&mut self) {
        let (base_options, base_map, char_options, char_map, template_options, template_map) =
            fetch_prompts(self.pool.clone()).expect("fetch_prompts");

        self.base.options = combo_box::State::new(base_options);
        self.base.map = base_map;
        self.base.selected = None;

        self.char.options = combo_box::State::new(char_options);
        self.char.map = char_map;
        self.char.selected = None;

        self.template.options = combo_box::State::new(template_options);
        self.template.map = template_map;
        self.template.selected = None;
    }

    /// processes generated images
    fn insert_image(&mut self, bytes: Bytes, path: PathBuf) {
        self.message = Some("generated image".into());

        let reader = Cursor::new(bytes);
        let mut archive = ZipArchive::new(reader).unwrap();
        let mut file = archive.by_index(0).unwrap();

        let mut buf = Vec::with_capacity(file.size() as usize);
        std::io::copy(&mut file, &mut buf).unwrap();

        let mut reader = ImageReader::new(Cursor::new(&buf));
        reader.set_format(image::ImageFormat::Png);
        if let Ok(im) = reader.decode() {
            let dim = im.dimensions();
            let resized = image::imageops::resize(
                &im.to_rgba8(),
                dim.0 / 16,
                dim.1 / 16,
                image::imageops::FilterType::Nearest,
            );
            let dims = resized.dimensions();
            let thumb_handle = Handle::from_rgba(dims.0, dims.1, resized.into_raw());
            self.thumbnails.push_front(thumb_handle);
            self.images.push_front(buf);
            self.image_paths.push_front(path);
        }
    }

    fn rename_prompt<V>(ui: &mut PromptUi<V>, pool: Pool<SqliteConnectionManager>) -> String {
        if let Some(old_name) = &ui.selected {
            let new_name = ui.rename.clone();

            let mut new_options = ui.options.options().to_vec();
            let pos = new_options
                .iter()
                .position(|s| s == old_name)
                .unwrap_or_default();
            new_options.splice(pos..pos + 1, [new_name.clone()]);
            ui.options = combo_box::State::new(new_options);

            let old_val = ui.map.remove(old_name);
            ui.map.insert(new_name.clone(), old_val.unwrap());

            let r = update_prompt_name(pool.clone(), ui.kind, old_name.clone(), new_name);
            let message = if let Err(e) = r {
                e.to_string()
            } else {
                String::from("rename successful")
            };
            return message;
        }
        String::from("rename failed")
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    // no-op for Task::perform
    Dummy,

    Event(Event),
    FsEvent(notify::Event),

    SetMessage(String),
    SelectedTheme(Theme),

    // pane
    FocusAdjacent(pane_grid::Direction),
    Clicked(pane_grid::Pane),
    Dragged(pane_grid::DragEvent),
    Resized(pane_grid::ResizeEvent),
    Maximize(pane_grid::Pane),
    Restore,

    // edit prompt / request parameters
    EditBasePrompt(widget::text_editor::Action),
    EditCharPrompt((usize, widget::text_editor::Action)),
    CharSelected(usize),
    SetPosition(Position),
    CopySeed,
    ClearSeed,
    ImageShape(ImageShape),

    // generate
    EditNumGenerate(String),
    Generate,

    // to channel
    Pause,
    Resume,
    Cancel(u64),
    CreateImage((u64, ImageGenRequest)),

    // from channel
    Channel(ChannelEvent),

    // prompt crud
    BasePromptSelected(String),
    CharacterPromptSelected(String),
    TemplateSelected(String),
    StorePrompt,
    SavedPrompt(Result<(), SqliteError>),
    UpdatePrompt(PromptKind),
    DeletePrompt(PromptKind),
    EditRenameBasePrompt(String),
    EditRenameCharacterPrompt(String),
    EditRenameTemplate(String),
    SubmitRenameBasePrompt,
    SubmitRenameCharacterPrompt,
    SubmitRenameTemplate,

    // image pane
    ImageClicked(usize),
    MetadataFromImage(usize),
    DeleteImageHistory,

    // files pane
    ToggleExpand,
    Refresh,
    RefreshSelected,
    GotoStart,
    GotoEnd,
    NavigateUp,
    SetRoot,
    ImportPrompt(u64, String, Vec<String>),
    Delete,
    MoveBatch,
    DeleteBatch,
    FilesPaneMode(FilesMode),
    SelectEntry,
    CreatePath,
    CreatePathName(String),
}

pub fn update(state: &mut State, msg: Message) -> Task<Message> {
    use Message::*;
    match msg {
        Dummy => (),
        Event(e) => return handle_event(state, e),

        FsEvent(ev) => {
            let msg = match state.files.handle_notify(ev) {
                Err(e) => e.to_string(),
                Ok(_) => "handled notify event".into(),
            };
            return Task::done(Message::SetMessage(msg));
        }
        SetMessage(s) => state.message = Some(s),
        SelectedTheme(theme) => state.selected_theme = theme,

        // pane
        FocusAdjacent(direction) => {
            if let Some(pane) = state.focus {
                if let Some(adjacent) = state.panes.adjacent(pane, direction) {
                    state.focus = Some(adjacent);
                }
            }
        }
        Clicked(pane) => {
            state.focus = Some(pane);
        }
        Dragged(pane_grid::DragEvent::Dropped { pane, target }) => {
            state.panes.drop(pane, target);
        }
        Dragged(_) => {}
        Resized(resize) => {
            state.panes.resize(resize.split, resize.ratio);
        }
        Maximize(pane) => state.panes.maximize(pane),
        Restore => state.panes.restore(),

        // prompt edit
        EditBasePrompt(action) => state.base_prompt.perform(action),
        EditCharPrompt((i, action)) => state.character_prompts[i].content.perform(action),
        CharSelected(index) => {
            state.curr_char = index - 1;
            return Task::done(Message::SetMessage(format!(
                "set curr_char to {}",
                index - 1
            )));
        }
        SetPosition(p) => {
            state.character_prompts[state.curr_char].c.center(p);
            return Task::done(Message::SetMessage(format!(
                "set curr_position of {} to {:?}",
                state.curr_char, p
            )));
        }

        // image generation
        ClearSeed => state.current_seed = None,
        CopySeed => state.current_seed = Some(state.previous_seed),
        ImageShape(shape) => {
            state.image_shape = shape;
        }
        EditNumGenerate(s) => state.num_generate = s,
        Generate => {
            if state.base_prompt.text() == "\n" || state.base_prompt.text().is_empty() {
                return Task::done(Message::SetMessage(
                    "must have at least the base prompt".into(),
                ));
            }

            if let ChannelReady::Ready(tx) = &mut state.task_state.ready {
                if let Ok(num_generate) = state.num_generate.parse::<u64>() {
                    let mut tx = tx.clone();
                    let req = setup_request(state);

                    let between = Uniform::new(1e8 as u64, 9e9 as u64).unwrap();
                    let seeds: Vec<u64> = (&mut state.rng)
                        .sample_iter(between)
                        .take(num_generate as usize)
                        .collect();

                    for i in &seeds {
                        state.task_ids.push(*i);
                    }

                    return Task::perform(
                        async move {
                            for i in seeds {
                                let _ = tx.send(Message::CreateImage((i, req.clone()))).await;
                            }
                        },
                        // generate_many(semaphore, client, req, seeds), |results| {
                        |_r| Message::Dummy,
                    );
                }
            }
        }
        Channel(ChannelEvent::TaskReady(main_tx)) => {
            state.task_state.ready = ChannelReady::Ready(main_tx);
            state.task_state.status = ChannelStatus::Ready;
        }
        Channel(ChannelEvent::Generated(id, res)) => match res {
            Err(e) => state.message = Some(e.to_string()),
            Ok((bytes, path)) => {
                state.message = Some("generated image".into());
                state.insert_image(bytes, path);
                if let Some(index) = state.task_ids.iter().position(|i| *i == id) {
                    state.task_ids.remove(index);
                }
            }
        },
        Channel(ChannelEvent::Cancelled(id)) => {
            println!("aborted task {}", id);
        }

        Pause => {
            state.task_state.status = ChannelStatus::Paused;
            if let ChannelReady::Ready(tx) = &mut state.task_state.ready {
                let mut tx = tx.clone();
                return Task::perform(
                    async move {
                        let _ = tx.send(Message::Pause).await;
                    },
                    |_| Message::Dummy,
                );
            }
        }
        Resume => {
            state.task_state.status = ChannelStatus::Ready;
            if let ChannelReady::Ready(tx) = &mut state.task_state.ready {
                let mut tx = tx.clone();
                return Task::perform(
                    async move {
                        let _ = tx.send(Message::Resume).await;
                    },
                    |_| Message::Dummy,
                );
            }
        }
        Cancel(id) => {
            if let Some(index) = state.task_ids.iter().position(|i| *i == id) {
                state.task_ids.remove(index);
            }

            if let ChannelReady::Ready(tx) = &mut state.task_state.ready {
                let mut tx = tx.clone();
                return Task::perform(
                    async move {
                        let _ = tx.send(Message::Cancel(id)).await;
                    },
                    |_| Message::Dummy,
                );
            }
        }
        CreateImage(..) => (),

        // files
        ToggleExpand => {
            state.files.enter();
            println!("{:?}", state.files.entries[state.files.selected])
        }
        Refresh => {
            state.files = FileTree::new(PROJECT_DIRS.data_dir());
        }
        RefreshSelected => {}
        GotoStart => {
            state.files.select_start();
        }
        GotoEnd => {
            state.files.select_end();
        }
        NavigateUp => {
            state.files.cd_parent();
        }
        SetRoot => {
            state.files.cd_selected();
        }
        ImportPrompt(seed, base, characters) => {
            set_prompt_characters(state, base, characters);
            state.current_seed = Some(seed);
        }
        Delete => {
            let id = state.files.selected;
            let message = match state.files.delete(id) {
                Err(e) => e.to_string(),
                Ok(_) => "delete ok".into(),
            };
            return Task::done(Message::SetMessage(message));
        }
        MoveBatch => {
            if let Err(e) = state.files.batch_move() {
                return Task::done(Message::SetMessage(e.to_string()));
            }
            return Task::done(Message::FilesPaneMode(FilesMode::Normal));
        }
        DeleteBatch => {
            if let Err(e) = state.files.batch_delete() {
                return Task::done(Message::SetMessage(e.to_string()));
            }
            return Task::done(Message::FilesPaneMode(FilesMode::Normal));
        }
        FilesPaneMode(mode) => {
            state.files_mode = mode;

            if mode == FilesMode::Normal {
                state.files.create_flag = false;
                state.files.clear_marked();
            }
            if mode == FilesMode::Create {
                state.files.create_flag = true;
            }
        }
        SelectEntry => {
            state.files.mark();
        }
        CreatePath => {
            let newpath = state.new_folder_name.trim_end();
            println!("want to create: {:?}", &newpath);
            let create_kind = if newpath.ends_with("/") {
                CreateEntryKind::Folder(newpath.strip_suffix("/").unwrap().into())
            } else {
                CreateEntryKind::File(newpath.into())
            };
            let r = state.files.create(create_kind);

            match r {
                Err(e) => return Task::done(Message::SetMessage(e.to_string())),
                Ok(_) => return Task::done(Message::FilesPaneMode(FilesMode::Normal)),
            }
        }
        CreatePathName(s) => state.new_folder_name = s,

        // prompt storage
        BasePromptSelected(s) => {
            if let Some(prompt) = state.base.map.get(&s) {
                state.base.selected = Some(s);
                state.base_prompt.perform(Action::SelectAll);
                state.base_prompt.perform(Action::Edit(Edit::Delete));
                state
                    .base_prompt
                    .perform(Action::Edit(Edit::Paste(Arc::new(prompt.clone()))));
            }
        }
        CharacterPromptSelected(s) => {
            if let Some(prompt) = state.char.map.get(&s) {
                state.char.selected = Some(s);
                let i = state.curr_char;
                state.character_prompts[i]
                    .content
                    .perform(Action::SelectAll);
                state.character_prompts[i]
                    .content
                    .perform(Action::Edit(Edit::Delete));
                state.character_prompts[i]
                    .content
                    .perform(Action::Edit(Edit::Paste(Arc::new(prompt.clone()))));
            }
        }
        TemplateSelected(s) => {
            if let Some(template) = state.template.map.get(&s) {
                state.template.selected = Some(s);

                state.base_prompt.perform(Action::SelectAll);
                state.base_prompt.perform(Action::Edit(Edit::Delete));
                state
                    .base_prompt
                    .perform(Action::Edit(Edit::Paste(Arc::new(template.base.clone()))));

                for cc in &mut state.character_prompts {
                    cc.content.perform(Action::SelectAll);
                    cc.content.perform(Action::Edit(Edit::Delete));
                }

                for (i, ch) in template.characters.iter().enumerate() {
                    if let Some(prompt) = ch {
                        state.character_prompts[i]
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(prompt.clone()))));
                    }
                }
            }
        }
        StorePrompt => {
            let base = state.base_prompt.text().replace("\n", " ");

            let mut characters = Vec::with_capacity(6);
            for cc in &state.character_prompts {
                let prompt = cc.content.text().replace("\n", "");
                if !prompt.is_empty() {
                    characters.push(prompt.into());
                }
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("duration_since")
                .as_secs() as i64;

            let pool = state.pool.clone();
            return Task::perform(
                async move { save_prompt(pool, vec![(now, base, characters)]).await },
                |r| Message::SavedPrompt(r),
            );
        }
        SavedPrompt(r) => {
            if let Err(e) = r {
                return Task::done(Message::SetMessage(e.err));
            } else {
                state.refresh_prompts();
                return Task::done(Message::SetMessage("saved prompt".into()));
            };
        }
        UpdatePrompt(kind) => {
            let (name, content) = match kind {
                PromptKind::Base => {
                    let selected = &state.base.selected;
                    let contents = state.base_prompt.text().replace("\n", "");
                    (selected, contents)
                }
                PromptKind::Character => {
                    let selected = &state.char.selected;
                    let contents = state.character_prompts[state.curr_char]
                        .content
                        .text()
                        .replace("\n", "");
                    (selected, contents)
                }
                _ => unreachable!(),
            };
            let task = name.clone().map_or(
                Task::done(Message::SetMessage("select a prompt to update".into())),
                |name| {
                    let pool = state.pool.clone();
                    Task::perform(
                        async move { update_prompt(pool, kind, name, content).await },
                        |r| Message::SavedPrompt(r),
                    )
                },
            );
            return task;
        }
        DeletePrompt(kind) => {
            let name = match kind {
                PromptKind::Base => &state.base.selected,
                PromptKind::Character => &state.char.selected,
                PromptKind::Template => &state.template.selected,
            };
            let task = name.clone().map_or(
                Task::done(Message::SetMessage("select a prompt to delete".into())),
                |name| {
                    let pool = state.pool.clone();
                    Task::perform(async move { delete_prompt(pool, kind, name).await }, |r| {
                        Message::SavedPrompt(r)
                    })
                },
            );
            return task;
        }
        SubmitRenameBasePrompt => {
            let msg = State::rename_prompt(&mut state.base, state.pool.clone());
            state.base.rename.clear();
            state.base.selected = None;
            return Task::done(Message::SetMessage(msg));
        }
        SubmitRenameCharacterPrompt => {
            let msg = State::rename_prompt(&mut state.char, state.pool.clone());
            state.char.rename.clear();
            state.char.selected = None;
            return Task::done(Message::SetMessage(msg));
        }
        SubmitRenameTemplate => {
            let msg = State::rename_prompt(&mut state.template, state.pool.clone());
            state.template.rename.clear();
            state.template.selected = None;
            return Task::done(Message::SetMessage(msg));
        }
        EditRenameBasePrompt(s) => {
            state.base.rename = s;
        }
        EditRenameCharacterPrompt(s) => {
            state.char.rename = s;
        }
        EditRenameTemplate(s) => {
            state.template.rename = s;
        }

        // image
        ImageClicked(i) => state.selected_image = Some(i),
        MetadataFromImage(i) => {
            if let Some(bytes) = state.images.get(i) {
                let mut reader = ImageReader::new(Cursor::new(bytes));
                reader.set_format(image::ImageFormat::Png);
                if let Ok(im) = reader.decode() {
                    if let Ok(metadata) = extract_image_metadata(im) {
                        let (seed, base, characters) = get_prompt_characters(metadata);
                        set_prompt_characters(state, base, characters);
                        state.current_seed = Some(seed);
                    }
                }
            }
        }
        DeleteImageHistory => {
            if let Some(i) = state.selected_image {
                state.images.remove(i);
                state.thumbnails.remove(i);
                let path = state.image_paths.remove(i).unwrap();

                if i > 0 {
                    state.selected_image.replace(i - 1);
                }

                if let Err(e) = trash::delete(&path) {
                    return Task::done(Message::SetMessage(format!(
                        "delete {:?}: {}",
                        &path,
                        e.to_string()
                    )));
                }
            }
        }
    }
    Task::none()
}

fn handle_event(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(ref e) => match e {
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                state.last_key = Some((key.clone(), *modifiers));
                if key.as_ref() == Key::Named(Named::ArrowUp) && modifiers.command() {
                    return Task::done(Message::FocusAdjacent(Direction::Up));
                }
                if key.as_ref() == Key::Named(Named::ArrowDown) && modifiers.command() {
                    return Task::done(Message::FocusAdjacent(Direction::Down));
                }
                if key.as_ref() == Key::Named(Named::ArrowLeft) && modifiers.command() {
                    return Task::done(Message::FocusAdjacent(Direction::Left));
                }
                if key.as_ref() == Key::Named(Named::ArrowRight) && modifiers.command() {
                    return Task::done(Message::FocusAdjacent(Direction::Right));
                }
            }
            _ => (),
        },
        Event::Mouse(ref _e) => (),
        Event::Window(ref e) => match e {
            window::Event::FileDropped(path) => {
                if let Some((seed, prompt, characters)) = get_prompt_metadata(path) {
                    return Task::done(Message::ImportPrompt(seed, prompt, characters));
                }
            }
            _ => (),
        },
        Event::Touch(_) => (),
    }

    if let Some(focused) = state.focus {
        if let Some(pane) = state.panes.get(focused) {
            match pane.id {
                PaneId::Files => return handle_event_files(state, e),
                PaneId::Prompts => (),
                PaneId::Image => return handle_event_image(state, e),
            }
        }
    }
    Task::none()
}

fn handle_event_files(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(e) => {
            match state.files_mode {
                FilesMode::Normal => match e {
                    keyboard::Event::KeyPressed { ref key, .. } => {
                        if key.as_ref() == Key::Named(Named::Backspace) {
                            return Task::done(Message::NavigateUp);
                        }
                        if key.as_ref() == Key::Character(".") {
                            return Task::done(Message::SetRoot);
                        }
                        if key.as_ref() == Key::Character("i") {
                            let id = state.files.selected;
                            let entry = &state.files.entries[id];
                            if entry.kind == EntryKind::File
                                && entry
                                    .path
                                    .extension()
                                    .is_some_and(|s| s.to_string_lossy().ends_with("png"))
                            {
                                if let Some((seed, prompt, characters)) =
                                    get_prompt_metadata(&entry.path)
                                {
                                    return Task::done(Message::ImportPrompt(
                                        seed, prompt, characters,
                                    ));
                                }
                            }
                        }
                        if key.as_ref() == Key::Character("d") {
                            return Task::done(Message::Delete);
                        }
                        if key.as_ref() == Key::Character("b") {
                            return Task::done(Message::FilesPaneMode(FilesMode::Batch));
                        }
                        if key.as_ref() == Key::Character("a") {
                            return Task::done(Message::FilesPaneMode(FilesMode::Create));
                        }
                    }
                    _ => (),
                },
                FilesMode::Batch => match e {
                    keyboard::Event::KeyPressed {
                        ref key, modifiers, ..
                    } => {
                        if key.as_ref() == Key::Character("s") {
                            return Task::done(Message::SelectEntry);
                        }
                        if key.as_ref() == Key::Character("m") {
                            return Task::done(Message::MoveBatch);
                        }
                        if key.as_ref() == Key::Character("d") && modifiers.shift() {
                            return Task::done(Message::DeleteBatch);
                        }
                    }
                    _ => (),
                },
                _ => (),
            }

            match e {
                keyboard::Event::KeyPressed { key, modifiers, .. } => match key.as_ref() {
                    Key::Named(Named::Escape) => {
                        return Task::done(Message::FilesPaneMode(FilesMode::Normal));
                    }
                    Key::Named(Named::ArrowUp) | Key::Character("k") => {
                        state.files.move_up();
                    }
                    Key::Named(Named::ArrowDown) | Key::Character("j") => {
                        state.files.move_down();
                    }
                    Key::Named(Named::Enter) => {
                        return Task::done(Message::ToggleExpand);
                    }
                    Key::Character("g") => {
                        if modifiers.shift() {
                            return Task::done(Message::GotoEnd);
                        } else {
                            return Task::done(Message::GotoStart);
                        }
                    }
                    Key::Character("r") => {
                        if modifiers.shift() {
                            return Task::done(Message::Refresh);
                        } else {
                            return Task::done(Message::RefreshSelected);
                        }
                    }

                    _ => (),
                },
                _ => (),
            }
        }
        _ => (),
    }
    Task::none()
}

fn handle_event_image(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(e) => {
            let current_index = state.selected_image;

            match e {
                keyboard::Event::KeyPressed { key, modifiers, .. } => {
                    if key.as_ref() == Key::Character("d") && modifiers.shift() {
                        return Task::done(Message::DeleteImageHistory);
                    }
                    if key.as_ref() == Key::Named(Named::ArrowUp) {
                        if let Some(i) = current_index {
                            if i > 0 {
                                state.selected_image.replace(i - 1);
                            }
                        }
                    }
                    if key.as_ref() == Key::Named(Named::ArrowDown) {
                        if let Some(i) = current_index {
                            if i + 1 < state.images.len() {
                                state.selected_image.replace(i + 1);
                            }
                        }
                    }
                }
                _ => (),
            }
        }
        _ => (),
    }
    Task::none()
}

pub fn view(state: &State) -> Element<Message> {
    let focus = state.focus;
    let pane_grid = PaneGrid::new(&state.panes, |id, pane, is_maximized| {
        let is_focused = focus == Some(id);

        let title = row![
            "Pane",
            text(pane.id.to_string()).color(if is_focused {
                style::PANE_ID_COLOR_FOCUSED
            } else {
                style::PANE_ID_COLOR_UNFOCUSED
            }),
        ]
        .spacing(5);

        let title_bar = pane_grid::TitleBar::new(title)
            .controls(pane_grid::Controls::dynamic(
                view_controls(id, is_maximized),
                text("Controls"),
            ))
            .padding(10)
            .style(if is_focused {
                style::title_bar_focused
            } else {
                style::title_bar_active
            });

        let content = match pane {
            Pane { id: PaneId::Files } => view_files(state).into(),
            Pane {
                id: PaneId::Prompts,
            } => view_prompts(state),
            Pane { id: PaneId::Image } => view_image(state).into(),
        };

        pane_grid::Content::new(content)
            .title_bar(title_bar)
            .style(if is_focused {
                style::pane_focused
            } else {
                style::pane_active
            })
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .spacing(10)
    .on_click(Message::Clicked)
    .on_drag(Message::Dragged)
    .on_resize(10, Message::Resized);

    container(pane_grid).padding(10).into()
}

fn view_files(state: &State) -> Element<Message> {
    let end = (state.files.view_offset + MAX_VISIBLE).min(state.files.visible.len());
    let slice = &state.files.visible[state.files.view_offset..end];

    let theme_selector = pick_list(
        [
            Theme::CatppuccinLatte,
            Theme::CatppuccinFrappe,
            Theme::CatppuccinMacchiato,
            Theme::CatppuccinMocha,
            Theme::TokyoNight,
            Theme::TokyoNightStorm,
            Theme::TokyoNightLight,
            Theme::KanagawaWave,
            Theme::KanagawaDragon,
            Theme::KanagawaLotus,
        ],
        Some(&state.selected_theme),
        Message::SelectedTheme,
    );

    let mut col = slice
        .iter()
        .enumerate()
        .fold(Column::new(), |col, (_idx, v)| {
            let entry = &state.files.entries[v.id];
            let label = match entry.kind {
                EntryKind::Folder => {
                    format!(
                        "{}D  {}",
                        "  ".repeat(v.depth),
                        entry.path.file_name().unwrap().to_string_lossy()
                    )
                }
                _ => {
                    format!(
                        "{}F  {}",
                        "  ".repeat(v.depth),
                        entry.path.file_name().unwrap().to_string_lossy()
                    )
                }
            };

            let is_current = state.files.selected == v.id;
            let is_selected = entry.marked;

            let style = if is_current {
                text::primary
            } else if is_selected {
                text::success
            } else {
                text::default
            };

            col.push(text(label).style(style))
        })
        .padding(4)
        .spacing(2);

    col = col.push_maybe(if state.files.create_flag {
        Some(
            text_input("new folder name", &state.new_folder_name)
                .on_input(Message::CreatePathName)
                .on_submit(Message::CreatePath),
        )
    } else {
        None
    });

    let mode = text(state.files_mode.to_string());

    let mut all = column![theme_selector, col, mode];
    all = all.push_maybe(if let Some(k) = &state.last_key {
        Some(text(format!("{:?} {:?}", k.0, k.1)))
    } else {
        None
    });

    scrollable(all.padding(2).spacing(4)).into()
}

fn view_prompts(state: &State) -> Element<Message> {
    let base_select = combo_box(
        &state.base.options,
        "base",
        state.base.selected.as_ref(),
        Message::BasePromptSelected,
    );
    let char_select = combo_box(
        &state.char.options,
        "character",
        state.char.selected.as_ref(),
        Message::CharacterPromptSelected,
    );
    let template_select = combo_box(
        &state.template.options,
        "template",
        state.template.selected.as_ref(),
        Message::TemplateSelected,
    );
    let char_dropdown = pick_list(
        [1, 2, 3, 4, 5, 6],
        Some(state.curr_char + 1),
        Message::CharSelected,
    );
    let select_prompt = row![char_dropdown, base_select, char_select, template_select];

    let base_rename = text_input("rename base_prompt", &state.base.rename)
        .on_input(Message::EditRenameBasePrompt)
        .on_submit(Message::SubmitRenameBasePrompt);
    let char_rename = text_input("rename character_prompt", &state.char.rename)
        .on_input(Message::EditRenameCharacterPrompt)
        .on_submit(Message::SubmitRenameCharacterPrompt);
    let template_rename = text_input("rename template", &state.template.rename)
        .on_input(Message::EditRenameTemplate)
        .on_submit(Message::SubmitRenameTemplate);
    let rename = row![base_rename, char_rename, template_rename].align_y(Alignment::Center);

    let save_prompt = button(text("Save New")).on_press(Message::StorePrompt);
    let update_base = button(text("Base Prompt")).on_press(Message::UpdatePrompt(PromptKind::Base));
    let update_char =
        button(text("Character")).on_press(Message::UpdatePrompt(PromptKind::Character));
    let update_template =
        button(text("Template")).on_press(Message::UpdatePrompt(PromptKind::Template));
    let delete_base = button(text("Base Prompt")).on_press(Message::DeletePrompt(PromptKind::Base));
    let delete_char =
        button(text("Character")).on_press(Message::DeletePrompt(PromptKind::Character));
    let delete_template =
        button(text("Template")).on_press(Message::DeletePrompt(PromptKind::Template));
    let crud_prompt = column![
        row![save_prompt].align_y(Alignment::Center),
        row![text("Update"), update_base, update_char, update_template]
            .align_y(Alignment::Center)
            .spacing(4),
        row![text("Delete"), delete_base, delete_char, delete_template]
            .align_y(Alignment::Center)
            .spacing(4),
    ]
    .align_x(Alignment::Start);

    let mut text_areas = Column::with_capacity(7).spacing(10);
    text_areas = text_areas.push(
        widget::text_editor(&state.base_prompt)
            .placeholder("base prompt")
            .on_action(Message::EditBasePrompt),
    );
    for (i, cc) in state.character_prompts.iter().enumerate() {
        text_areas = text_areas.push(
            widget::text_editor(&cc.content)
                .placeholder("")
                .on_action(move |action| Message::EditCharPrompt((i, action))),
        );
    }

    use Position::*;
    let position_grid = column![
        row![
            button("*").on_press(Message::SetPosition(R0C0)),
            button("*").on_press(Message::SetPosition(R0C1)),
            button("*").on_press(Message::SetPosition(R0C2)),
            button("*").on_press(Message::SetPosition(R0C3)),
            button("*").on_press(Message::SetPosition(R0C4))
        ],
        row![
            button("*").on_press(Message::SetPosition(R1C0)),
            button("*").on_press(Message::SetPosition(R1C1)),
            button("*").on_press(Message::SetPosition(R1C2)),
            button("*").on_press(Message::SetPosition(R1C3)),
            button("*").on_press(Message::SetPosition(R1C4))
        ],
        row![
            button("*").on_press(Message::SetPosition(R2C0)),
            button("*").on_press(Message::SetPosition(R2C1)),
            button("*").on_press(Message::SetPosition(R2C2)),
            button("*").on_press(Message::SetPosition(R2C3)),
            button("*").on_press(Message::SetPosition(R2C4))
        ],
        row![
            button("*").on_press(Message::SetPosition(R3C0)),
            button("*").on_press(Message::SetPosition(R3C1)),
            button("*").on_press(Message::SetPosition(R3C2)),
            button("*").on_press(Message::SetPosition(R3C3)),
            button("*").on_press(Message::SetPosition(R3C4))
        ],
        row![
            button("*").on_press(Message::SetPosition(R4C0)),
            button("*").on_press(Message::SetPosition(R4C1)),
            button("*").on_press(Message::SetPosition(R4C2)),
            button("*").on_press(Message::SetPosition(R4C3)),
            button("*").on_press(Message::SetPosition(R4C4))
        ],
    ];

    let current_seed = text(state.current_seed.unwrap_or_default());
    let copy_seed = button(text("Use Previous Seed")).on_press(Message::CopySeed);
    let clear_seed = button(text("Clear Seed")).on_press(Message::ClearSeed);
    let seed = row![text("Seed: "), current_seed, copy_seed, clear_seed]
        .align_y(Alignment::Center)
        .spacing(4);

    let orientation = pick_list(
        [
            ImageShape::Portrait,
            ImageShape::Landscape,
            ImageShape::Square,
        ],
        Some(state.image_shape),
        Message::ImageShape,
    );

    let num_images = column![
        text_input("1", &state.num_generate).on_input(Message::EditNumGenerate),
        button("Generate").on_press(Message::Generate),
    ];

    let mut ids = Column::with_capacity(state.task_ids.len());
    for i in &state.task_ids {
        ids = ids.push(row![
            text(i),
            button("Cancel").on_press(Message::Cancel(*i))
        ]);
    }
    let task_ids = scrollable(ids);

    let generate_controls = column![
        num_images,
        text(format!("Status: {}", state.task_state.status)),
        button("Pause").on_press(Message::Pause),
        button("Resume").on_press(Message::Resume),
        task_ids,
    ]
    .spacing(4);

    let mut content = Column::with_children([
        select_prompt.into(),
        rename.into(),
        crud_prompt.into(),
        text_areas.into(),
        position_grid.into(),
        seed.into(),
        orientation.into(),
        generate_controls.into(),
    ])
    .padding(2)
    .spacing(4);

    content = content.push_maybe(if let Some(message) = &state.message {
        Some(text(message))
    } else {
        None
    });

    scrollable(content).spacing(8).into()
}

fn view_image(state: &State) -> Element<Message> {
    let file_pane_image: Option<Element<Message>> = {
        let entry = &state.files.entries[state.files.selected];
        state
            .files
            .cache
            .get(&entry.path)
            .map_or(None, |h| Some(Image::new(h).into()))
    };

    let mut thumbs = Column::with_capacity(state.thumbnails.len()).align_x(Alignment::Center);
    for (index, handle) in state.thumbnails.iter().enumerate() {
        let style = if let Some(i) = state.selected_image {
            if i == index {
                container::bordered_box
            } else {
                container::rounded_box
            }
        } else {
            container::rounded_box
        };

        let im = Image::new(handle);
        let border = container(im).style(style);
        let clickable = mouse_area(border)
            .on_press(Message::ImageClicked(index))
            .on_right_press(Message::MetadataFromImage(index));
        thumbs = thumbs.push(clickable);
    }

    let final_image: Element<Message> = if let Some(image) = file_pane_image {
        image
    } else if !state.images.is_empty() {
        if let Some(i) = state.selected_image {
            Image::new(Handle::from_bytes(state.images[i].clone())).into()
        } else {
            text("invalid selected image").into()
        }
    } else {
        text("nothing to see here").into()
    };

    let image_history = scrollable(thumbs);
    row![center(final_image), image_history].into()
}

fn view_controls<'a>(pane: pane_grid::Pane, is_maximized: bool) -> Element<'a, Message> {
    let (content, message) = if is_maximized {
        ("Restore", Message::Restore)
    } else {
        ("Maximize", Message::Maximize(pane))
    };

    let row = row![
        button(text(content).size(14))
            .style(button::secondary)
            .padding(3)
            .on_press(message),
    ]
    .spacing(5);

    row.into()
}

fn setup_request(state: &mut State) -> ImageGenRequest {
    let mut req = ImageGenRequest::default();

    req.prompt(state.base_prompt.text());

    let new_seed = state.rng.random_range(1e9..9e9) as u64;
    req.seed(state.current_seed.unwrap_or(new_seed));
    state.previous_seed = new_seed;
    state.current_seed = None;

    req.height_width(state.image_shape);

    for cc in &mut state.character_prompts {
        if cc.content.text() == "\n" {
            continue;
        }
        cc.c.prompt(cc.content.text());
        req.add_character(&cc.c);
    }

    if state
        .character_prompts
        .iter()
        .any(|ch| ch.c.get_center() == Point { x: 0.5, y: 0.5 })
    {
        req.use_coords(true);
    } else {
        req.use_coords(false);
    }
    req
}

fn set_prompt_characters(state: &mut State, base: String, characters: Vec<String>) {
    state.base_prompt.perform(Action::SelectAll);
    state.base_prompt.perform(Action::Edit(Edit::Delete));
    state
        .base_prompt
        .perform(Action::Edit(Edit::Paste(Arc::new(base))));

    for (i, c) in characters.iter().enumerate() {
        state.character_prompts[i]
            .content
            .perform(Action::SelectAll);
        state.character_prompts[i]
            .content
            .perform(Action::Edit(Edit::Delete));
        state.character_prompts[i]
            .content
            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
    }
}

fn get_prompt_characters(meta: Map<String, Value>) -> (u64, String, Vec<String>) {
    let seed = meta
        .get("Comment")
        .unwrap()
        .get("seed")
        .unwrap()
        .as_u64()
        .unwrap();
    let caption = meta
        .get("Comment")
        .unwrap()
        .get("v4_prompt")
        .unwrap()
        .get("caption")
        .unwrap();
    let prompt = caption
        .get("base_caption")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let characters = caption.get("char_captions").unwrap().as_array().unwrap();

    let mut character_prompts = Vec::with_capacity(6);
    for c in characters {
        character_prompts.push(c.get("char_caption").unwrap().as_str().unwrap().to_owned());
    }
    (seed, prompt, character_prompts)
}

pub fn get_prompt_metadata<P: AsRef<std::path::Path>>(
    path: P,
) -> Option<(u64, String, Vec<String>)> {
    if path
        .as_ref()
        .extension()
        .map_or(false, |s| s.to_string_lossy() == "png")
    {
        if let Ok(im) = image::open(path) {
            if let Ok(meta) = extract_image_metadata(im) {
                return Some(get_prompt_characters(meta));
            }
        }
    }
    None
}

pub fn event_subscribe(_state: &State) -> Subscription<Message> {
    event::listen().map(Message::Event)
}

pub fn run_channel_subscription() -> Subscription<Message> {
    Subscription::run(channel_subscribe).map(Message::Channel)
}

fn channel_subscribe() -> impl Stream<Item = ChannelEvent> {
    use iced::futures::{FutureExt, StreamExt, channel::mpsc, pin_mut, select};
    use tokio::time;

    stream::channel(200, |mut output| async move {
        let (main_tx, main_rx) = mpsc::channel(200);
        let mut rx = main_rx.fuse();
        let mut interval = time::interval(Duration::from_millis(1000));

        let mut paused = false;
        let mut buf: VecDeque<(u64, ImageGenRequest)> = VecDeque::with_capacity(64);
        let mut in_flight: FastIndexMap<
            u64,
            JoinHandle<Result<(Bytes, PathBuf), ImageGenerationError>>,
        > = FastIndexMap::default();
        let client = Arc::new(Requester::default());
        let semaphore = Arc::new(Semaphore::new(1));

        let _ = output.send(ChannelEvent::TaskReady(main_tx)).await;
        println!("sent TaskReady");

        loop {
            let tick = interval.tick().fuse();

            pin_mut!(tick);

            select! {
                input = rx.select_next_some() => {
                    match input {
                        Message::Cancel(id) => {
                            println!("rcv cancel");
                            if let Some(handle) = in_flight.shift_remove(&id) {
                                handle.abort();
                                let _ = output.send(ChannelEvent::Cancelled(id)).await;
                            }
                        }
                        Message::Pause => {
                            println!("rcv pause");
                            paused = true;
                        }
                        Message::Resume => {
                            println!("rcv resume");
                            paused = false;

                            while let Some((seed, mut req)) = buf.pop_front() {
                                println!("resumed creating task {}", seed);

                                let client = Arc::clone(&client);
                                let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
                                let jh = tokio::spawn(async move {
                                    let _permit = permit;
                                    req.seed(seed);

                                    let result = client.generate_image(req).await;

                                    time::sleep(Duration::from_millis(1250)).await;

                                    result
                                });
                                in_flight.insert(seed, jh);
                            }
                        }
                        Message::CreateImage((seed, mut req)) => {
                            if paused {
                                buf.push_back((seed, req));
                            } else {
                                println!("creating task {}", seed);

                                let client = Arc::clone(&client);
                                let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
                                let jh = tokio::spawn(async move {
                                    let _permit = permit;
                                    req.seed(seed);

                                    let result = client.generate_image(req).await;
                                    result
                                });
                                in_flight.insert(seed, jh);
                            }
                        }
                        _ => (),
                    }
                }

                _ = tick => {
                     let done: Vec<u64> = in_flight
                         .iter()
                         .filter(|(_id, handle)| handle.is_finished())
                         .map(|(&id, _handle)| id)
                         .collect();

                     for id in done {
                         if let Some(handle) = in_flight.shift_remove(&id) {
                             match handle.await {
                                 Ok(res) => {
                                     let _ = output.send(ChannelEvent::Generated(id, res)).await;
                                 }
                                 Err(e) if e.is_cancelled() => {
                                     let _ = output.send(ChannelEvent::Cancelled(id)).await;
                                 }
                                 Err(e) => eprintln!("task {} failed: {}", id, e),
                             }
                         }
                     }

                }
            }
        }
    })
}

pub fn run_fsevent_subscription() -> Subscription<Message> {
    Subscription::run(channel_fsevent).map(Message::FsEvent)
}

fn channel_fsevent() -> impl Stream<Item = notify::Event> {
    use iced::futures::{StreamExt, channel::mpsc, executor};
    use notify::Watcher;

    stream::channel(64, |mut output| async move {
        let (mut tx, mut rx) = mpsc::channel(64);

        let mut watcher = notify::RecommendedWatcher::new(
            move |res| {
                executor::block_on(async {
                    tx.send(res).await.unwrap();
                })
            },
            notify::Config::default(),
        )
        .expect("failed to init watcher");

        watcher
            .watch(PROJECT_DIRS.data_dir(), notify::RecursiveMode::Recursive)
            .expect("failed to watch project data_dir");

        while let Some(res) = rx.next().await {
            match res {
                Ok(e) => {
                    let _ = output.send(e).await.unwrap();
                }
                Err(e) => eprintln!("{}", e),
            }
        }
    })
}

struct TaskState {
    // sender
    ready: ChannelReady,

    // for ui only
    status: ChannelStatus,
}

enum ChannelStatus {
    NotReady,
    Ready,
    Paused,
}

impl Display for ChannelStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => write!(f, "Not Ready"),
            Self::Ready => write!(f, "Ready"),
            Self::Paused => write!(f, "Paused"),
        }
    }
}

enum ChannelReady {
    NotReady,
    Ready(Sender<Message>),
}

#[derive(Debug, Clone)]
pub enum ChannelEvent {
    Generated(u64, Result<(Bytes, PathBuf), ImageGenerationError>),
    Cancelled(u64),
    TaskReady(Sender<Message>),
}

#[derive(Debug, Clone)]
struct PromptUi<V> {
    kind: PromptKind,
    options: combo_box::State<String>,
    map: FastHashMap<String, V>,
    selected: Option<String>,
    rename: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum FilesMode {
    #[default]
    Normal,
    Batch,
    Create,
}

impl Display for FilesMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FilesMode::*;
        match self {
            Normal => write!(f, "Normal"),
            Batch => write!(f, "Batch"),
            Create => write!(f, "CreateFolder"),
        }
    }
}

struct CharacterContent {
    c: nai::Character,
    content: widget::text_editor::Content,
}

impl CharacterContent {
    fn new() -> Self {
        Self {
            c: nai::Character::new(),
            content: widget::text_editor::Content::new(),
        }
    }
}

enum PaneId {
    Files,
    Prompts,
    Image,
}

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Files => write!(f, "files"),
            Self::Prompts => write!(f, "prompts"),
            Self::Image => write!(f, "image"),
        }
    }
}

struct Pane {
    id: PaneId,
}

impl Pane {
    fn new(id: PaneId) -> Self {
        Self { id }
    }
}

mod style {
    use iced::{Border, Color, Theme, widget::container};

    pub const PANE_ID_COLOR_UNFOCUSED: Color = Color::from_rgb(
        0xFF as f32 / 255.0,
        0xC7 as f32 / 255.0,
        0xC7 as f32 / 255.0,
    );
    pub const PANE_ID_COLOR_FOCUSED: Color = Color::from_rgb(
        0xFF as f32 / 255.0,
        0x47 as f32 / 255.0,
        0x47 as f32 / 255.0,
    );

    pub fn title_bar_active(theme: &Theme) -> container::Style {
        let palette = theme.extended_palette();

        container::Style {
            text_color: Some(palette.background.strong.text),
            background: Some(palette.background.strong.color.into()),
            ..Default::default()
        }
    }

    pub fn title_bar_focused(theme: &Theme) -> container::Style {
        let palette = theme.extended_palette();

        container::Style {
            text_color: Some(palette.primary.strong.text),
            background: Some(palette.primary.strong.color.into()),
            ..Default::default()
        }
    }

    pub fn pane_active(theme: &Theme) -> container::Style {
        let palette = theme.extended_palette();

        container::Style {
            background: Some(palette.background.weak.color.into()),
            border: Border {
                width: 2.0,
                color: palette.background.strong.color,
                ..Border::default()
            },
            ..Default::default()
        }
    }

    pub fn pane_focused(theme: &Theme) -> container::Style {
        let palette = theme.extended_palette();

        container::Style {
            background: Some(palette.background.weak.color.into()),
            border: Border {
                width: 2.0,
                color: palette.primary.strong.color,
                ..Border::default()
            },
            ..Default::default()
        }
    }
}
