use std::{
    collections::VecDeque,
    fmt::{self, Display},
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use iced::{
    Alignment, Element, Event,
    Length::{self},
    Subscription, Task, event,
    keyboard::{
        self,
        key::{Key, Named},
    },
    widget::{
        self, Column, Image, PaneGrid, button, center, column, combo_box, container,
        image::Handle,
        mouse_area,
        pane_grid::{self, Axis, Configuration, Direction},
        pick_list, row, scrollable,
        shader::wgpu::naga::FastHashMap,
        text,
        text_editor::{Action, Edit},
        text_input,
    },
    window,
};
use image::ImageReader;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::{Rng, distr::Uniform, rngs::ThreadRng};
use serde_json::{Map, Value};
use tokio::sync::Semaphore;
use zip::ZipArchive;

use crate::{
    db::{PromptKind, SqliteError, Template, fetch_prompts, save_prompt, update_prompt_name},
    files::{
        Entry, EntryKind, FilesState, entries, init_visible, selected_entry, selected_entry_mut,
        update_visible,
    },
    image_metadata::extract_image_metadata,
    nai::{self, ImageGenRequest, ImageGenerationError, ImageShape, Point, Position, Requester},
};

const FILE_EXTENSIONS: [&'static str; 3] = ["jpeg", "jpg", "png"];
const MAX_VISIBLE: usize = 40;

pub struct State {
    semaphore: Arc<Semaphore>,
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
    num_generate_temp: String,
    num_generate: u8,

    temp: Vec<(Vec<usize>, PathBuf)>,
    files_mode: FilesMode,
    image_gen_ip: bool,
    cache: FastHashMap<PathBuf, Handle>,
    files: FilesState,
    client: Arc<Requester>,
    images: VecDeque<Vec<u8>>,
    selected_image: Option<usize>,
    thumbnails: VecDeque<Handle>,
    image_paths: VecDeque<PathBuf>,
    trash: PathBuf,
}

impl Default for State {
    fn default() -> Self {
        let sqlite_url = std::env::var("SQLITE_URL").expect("missing SQLITE_URL env var");
        let manager = SqliteConnectionManager::file(sqlite_url);
        let pool = r2d2::Pool::new(manager).expect("pool");

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

        let trash = if let Some(home) = std::env::home_dir() {
            home.join(".Trash")
        } else if let Err(_e) = fs::exists("trash") {
            fs::create_dir("trash").expect("create trash dir");
            PathBuf::from("trash")
        } else {
            PathBuf::from("trash")
        };

        let state = Self {
            semaphore: Arc::new(Semaphore::new(1)),
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
            num_generate_temp: String::new(),
            num_generate: 1,

            // history: Vec::new(),
            temp: Vec::new(),
            files_mode: FilesMode::Normal,
            image_gen_ip: false,
            cache: FastHashMap::default(),
            files: FilesState::new("."),
            client: Arc::new(Requester::default()),

            images: VecDeque::new(),
            selected_image: None,
            thumbnails: VecDeque::new(),
            image_paths: VecDeque::new(),
            trash,
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
            let resized = image::imageops::resize(
                &im.to_rgba8(),
                64,
                64,
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
            new_options.remove(new_options.iter().position(|s| s == old_name).unwrap());
            new_options.push(new_name.clone());
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
    Event(Event),
    SetMessage(String),

    FocusAdjacent(pane_grid::Direction),
    Clicked(pane_grid::Pane),
    Dragged(pane_grid::DragEvent),
    Resized(pane_grid::ResizeEvent),
    Maximize(pane_grid::Pane),
    Restore,

    EditBasePrompt(widget::text_editor::Action),
    EditCharPrompt((usize, widget::text_editor::Action)),
    CharSelected(usize),
    SetPosition(Position),

    CopySeed,
    ClearSeed,
    EditNumGenerate(String),
    NumGenerate,
    ImageShape(ImageShape),
    GenerateOne,
    GenerateMany,
    ImageGenerated(Result<(Bytes, PathBuf), ImageGenerationError>),
    ImagesGenerated(Vec<Result<(Bytes, PathBuf), ImageGenerationError>>),

    Entries((Vec<usize>, Vec<Entry>)),
    ToggleExpand(Vec<usize>),
    Refresh,
    RefreshSelected,
    GotoStart,
    GotoEnd,
    NavigateUp,
    SetRoot,
    ImportPrompt(u64, String, Vec<String>),
    Delete,
    MoveBatch,
    FilesMode(FilesMode),
    SelectEntry,

    BasePromptSelected(String),
    CharacterPromptSelected(String),
    TemplateSelected(String),
    SavePrompt,
    SavedPrompt(Result<(), SqliteError>),
    EditRenameBasePrompt(String),
    EditRenameCharacterPrompt(String),
    EditRenameTemplate(String),
    SubmitRenameBasePrompt,
    SubmitRenameCharacterPrompt,
    SubmitRenameTemplate,

    ImageClicked(usize),
    MetadataFromImage(usize),
    DeleteImageHistory,
}

pub fn update(state: &mut State, msg: Message) -> Task<Message> {
    use Message::*;
    match msg {
        Event(e) => return handle_event(state, e),
        SetMessage(s) => state.message = Some(s),

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
        EditNumGenerate(s) => state.num_generate_temp = s,
        NumGenerate => match state.num_generate_temp.parse::<u8>() {
            Ok(num) => {
                state.num_generate = num;
                return Task::done(Message::SetMessage("set num_generate".into()));
            }
            Err(e) => return Task::done(Message::SetMessage(e.to_string())),
        },
        GenerateOne => {
            state.image_gen_ip = true;

            let req = setup_request(state);
            let r = Arc::clone(&state.client);

            return Task::perform(async move { r.generate_image(req).await }, |r| {
                Message::ImageGenerated(r)
            });
        }
        GenerateMany => {
            state.image_gen_ip = true;

            let req = setup_request(state);
            let num_generate = state.num_generate;
            let semaphore = Arc::clone(&state.semaphore);
            let client = Arc::clone(&state.client);

            let between = Uniform::new(1e8 as u64, 9e9 as u64).unwrap();
            let seeds: Vec<u64> = (&mut state.rng)
                .sample_iter(between)
                .take(num_generate as usize)
                .collect();

            return Task::perform(generate_many(semaphore, client, req, seeds), |results| {
                Message::ImagesGenerated(results)
            });
        }
        ImageGenerated(res) => {
            state.image_gen_ip = false;

            match res {
                Err(e) => state.message = Some(e.to_string()),
                Ok((bytes, path)) => {
                    state.message = Some("generated image".into());
                    state.insert_image(bytes, path);
                }
            }
        }
        ImagesGenerated(results) => {
            state.image_gen_ip = false;

            for r in results {
                match r {
                    Err(e) => state.message = Some(e.to_string()),
                    Ok((bytes, path)) => {
                        state.message = Some("generated image".into());
                        state.insert_image(bytes, path);
                    }
                }
            }
        }

        // files
        Entries((path, mut entries)) => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                entries.sort();
                entry.children = Some(entries);
                entry.set_expanded();
            }
            update_visible(&mut state.files);
        }
        ToggleExpand(path) => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                match entry.kind {
                    EntryKind::Folder => {
                        if !entry.visited() {
                            entry.set_visited();
                            return Task::done(Message::Entries((
                                path.clone(),
                                entries(&entry.path),
                            )));
                        }
                        entry.toggle_expanded();
                        update_visible(&mut state.files);
                    }
                    EntryKind::File => {
                        if let Some(_) = entry
                            .path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .filter(|ext| FILE_EXTENSIONS.contains(&ext))
                        {
                            if !state.cache.contains_key(&entry.path) {
                                let handle = Handle::from_path(&entry.path);
                                state.cache.insert(entry.path.clone(), handle);
                            } else {
                                state.cache.remove(&entry.path);
                            }
                        }
                    }
                }
            }
            return Task::none();
        }
        Refresh => {
            state.files = FilesState::new(".");
        }
        RefreshSelected => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &state.files.selected)
            {
                if entry.kind == EntryKind::Folder && entry.visited() {
                    if let Some(children) = entry.children.as_mut() {
                        let mut entries = entries(&entry.path);
                        entries.sort();
                        *children = entries;
                    }
                } else if entry.kind == EntryKind::File {
                    if state.files.selected.len() > 1 {
                        state.files.selected.pop();
                        return Task::done(Message::RefreshSelected);
                    }
                }
                update_visible(&mut state.files);
            }
        }
        GotoStart => {
            state.files.selected = vec![0];
        }
        GotoEnd => {
            state.files.selected = vec![state.files.entries.len() - 1];
        }
        NavigateUp => {
            state.files = FilesState::new("../");
        }
        SetRoot => {
            if let Some(entry) = selected_entry(&state.files.entries, &state.files.selected) {
                if entry.kind == EntryKind::Folder {
                    state.files = FilesState::new(&entry.path);
                }
            }
        }
        ImportPrompt(seed, base, characters) => {
            set_prompt_characters(state, base, characters);
            state.current_seed = Some(seed);
        }
        Delete => return delete_file(state),
        MoveBatch => {
            if state.files.selected.len() == 1 {
                return Task::done(Message::SetMessage(String::from(
                    "moving files into the current directory is not supported",
                )));
            }

            // NOTE: don't screw around with the fs while this happens or they'll really be zombies
            let mut zombies: Vec<Entry> = vec![];
            for (parent_path, entry_pathbuf) in &mut state.temp {
                if let Some(parent) = selected_entry_mut(&mut state.files.entries, parent_path) {
                    if let Some(children) = &mut parent.children {
                        if let Some(pos) = children.iter().position(|e| e.path == *entry_pathbuf) {
                            zombies.push(children.remove(pos));
                        }
                    }
                }
            }

            if let Some(new_parent) =
                selected_entry_mut(&mut state.files.entries, &state.files.selected)
            {
                state.temp.clear();
                state
                    .temp
                    .push((state.files.selected.clone(), new_parent.path.clone()));

                for zo in &zombies {
                    if let Err(e) = fs::rename(
                        &zo.path,
                        new_parent.path.join(&zo.path.file_name().unwrap()),
                    ) {
                        eprintln!("move: {}", e);
                    }
                }
                match &mut new_parent.children {
                    Some(children) => children.extend_from_slice(&zombies),
                    None => new_parent.children = Some(zombies),
                }
                new_parent.children.as_mut().unwrap().sort();
            }
            state.files.visible.clear();
            init_visible(&state.files.entries, &mut state.files.visible, 0, vec![]);
        }
        FilesMode(mode) => {
            state.files_mode = mode;

            for (path, _) in &state.temp {
                if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                    if let Some(children) = &mut entry.children {
                        for c in children {
                            c.clear_selected();
                        }
                    } else {
                        entry.clear_selected();
                    }
                }
            }
        }
        SelectEntry => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &state.files.selected)
            {
                entry.toggle_selected();
                state.temp.push((
                    state.files.selected[..state.files.selected.len() - 1].to_vec(),
                    entry.path.clone(),
                ));
            }
        }

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
        SavePrompt => {
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
        SubmitRenameBasePrompt => {
            let msg = State::rename_prompt(&mut state.base, state.pool.clone());
            return Task::done(Message::SetMessage(msg));
        }
        SubmitRenameCharacterPrompt => {
            let msg = State::rename_prompt(&mut state.char, state.pool.clone());
            return Task::done(Message::SetMessage(msg));
        }
        SubmitRenameTemplate => {
            let msg = State::rename_prompt(&mut state.template, state.pool.clone());
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

                if let Err(e) = fs::rename(&path, state.trash.join(&path.file_name().unwrap())) {
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

async fn generate_many(
    semaphore: Arc<Semaphore>,
    r: Arc<Requester>,
    req: ImageGenRequest,
    seeds: Vec<u64>,
) -> Vec<Result<(Bytes, PathBuf), ImageGenerationError>> {
    let mut jhs = Vec::new();
    for seed in seeds {
        let semaphore = semaphore.clone();
        let r = r.clone();

        let mut req = req.clone();
        req.seed(seed);

        let jh = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            let resp = r.generate_image(req).await;
            drop(_permit);
            resp
        });
        jhs.push(jh);

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    }

    let mut ret = Vec::new();
    for jh in jhs {
        let resp = jh.await.unwrap();
        ret.push(resp);
    }
    ret
}

fn handle_event(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(ref e) => match e {
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
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
            let current_index = state
                .files
                .visible
                .iter()
                .position(|ve| &ve.path == &state.files.selected);

            match state.files_mode {
                FilesMode::Normal => match e {
                    keyboard::Event::KeyPressed { ref key, .. } => match key.as_ref() {
                        Key::Named(Named::Backspace) => {
                            return Task::done(Message::NavigateUp);
                        }
                        Key::Character(".") => {
                            return Task::done(Message::SetRoot);
                        }
                        Key::Character("i") => {
                            if let Some(entry) =
                                selected_entry(&state.files.entries, &state.files.selected)
                            {
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
                        }
                        Key::Character("d") => {
                            return Task::done(Message::Delete);
                        }
                        Key::Character("b") => {
                            return Task::done(Message::FilesMode(FilesMode::Batch));
                        }
                        _ => (),
                    },
                    _ => (),
                },
                FilesMode::Batch => match e {
                    keyboard::Event::KeyPressed {
                        ref key,
                        modifiers: _,
                        ..
                    } => match key.as_ref() {
                        Key::Character("s") => {
                            return Task::done(Message::SelectEntry);
                        }
                        Key::Named(Named::Escape) => {
                            return Task::done(Message::FilesMode(FilesMode::Normal));
                        }
                        Key::Character("m") => {
                            return Task::done(Message::MoveBatch);
                        }
                        _ => (),
                    },
                    _ => (),
                },
            }

            match e {
                keyboard::Event::KeyPressed { key, modifiers, .. } => match key.as_ref() {
                    Key::Named(Named::ArrowUp) | Key::Character("k") => {
                        if let Some(i) = current_index {
                            if i > 0 {
                                state.files.selected = state.files.visible[i - 1].path.clone();
                                state.files.view_offset = state.files.view_offset.saturating_sub(1);
                            }
                        }
                    }
                    Key::Named(Named::ArrowDown) | Key::Character("j") => {
                        if let Some(i) = current_index {
                            if i + 1 < state.files.visible.len() {
                                state.files.selected = state.files.visible[i + 1].path.clone();
                                if i + 1 >= state.files.view_offset + MAX_VISIBLE {
                                    state.files.view_offset += 1;
                                }
                            }
                        } else if !state.files.visible.is_empty() {
                            state.files.selected = state.files.visible[0].path.clone();
                        }
                    }
                    Key::Named(Named::Enter) => {
                        if let Some(i) = current_index {
                            return Task::done(Message::ToggleExpand(
                                state.files.visible[i].path.clone(),
                            ));
                        }
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
                        }
                        return Task::done(Message::RefreshSelected);
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
                text("foobar"),
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

    let col = slice
        .iter()
        .enumerate()
        .fold(Column::new(), |col, (_i, e)| {
            let entry = selected_entry(&state.files.entries, &e.path).unwrap();

            let label = match entry.kind {
                EntryKind::Folder => {
                    format!(
                        "{}D  {}",
                        "  ".repeat(e.depth),
                        entry.path.file_name().unwrap().to_string_lossy()
                    )
                }
                EntryKind::File => {
                    format!(
                        "{}F  {}",
                        "  ".repeat(e.depth),
                        entry.path.file_name().unwrap().to_string_lossy()
                    )
                }
            };

            let is_current = state.files.selected == e.path;
            let is_selected = entry.selected();

            let style = if is_current {
                text::primary
            } else if is_selected {
                text::success
            } else {
                text::default
            };

            col.push(text(label).style(style))
        });

    let mode = text(state.files_mode.to_string());

    scrollable(column![col, mode]).into()
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

    let save_prompt = button(text("Save Prompt")).on_press(Message::SavePrompt);
    let base_rename = text_input("rename base_prompt", &state.base.rename)
        .on_input(Message::EditRenameBasePrompt)
        .on_submit(Message::SubmitRenameBasePrompt);
    let char_rename = text_input("rename character_prompt", &state.char.rename)
        .on_input(Message::EditRenameCharacterPrompt)
        .on_submit(Message::SubmitRenameCharacterPrompt);
    let template_rename = text_input("rename template", &state.template.rename)
        .on_input(Message::EditRenameTemplate)
        .on_submit(Message::SubmitRenameTemplate);
    let save_rename =
        row![save_prompt, base_rename, char_rename, template_rename].align_y(Alignment::Center);

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
    let seed = row![text("Seed: "), current_seed, copy_seed, clear_seed];

    let orientation = pick_list(
        [
            ImageShape::Portrait,
            ImageShape::Landscape,
            ImageShape::Square,
        ],
        Some(state.image_shape),
        Message::ImageShape,
    );

    let mut content = Column::with_children([
        select_prompt.into(),
        save_rename.into(),
        text_areas.into(),
        position_grid.into(),
        seed.into(),
        orientation.into(),
    ]);

    content = content.push_maybe(if !state.image_gen_ip && state.base_prompt.text() != "\n" {
        Some(column![
            row![button("Submit").on_press(Message::GenerateOne)],
            row![
                text_input("1", &state.num_generate_temp)
                    .on_input(Message::EditNumGenerate)
                    .on_submit(Message::NumGenerate),
                button("Generate Many").on_press(Message::GenerateMany),
            ]
        ])
    } else {
        None
    });

    content = content.push_maybe(if let Some(message) = &state.message {
        Some(text(message))
    } else {
        None
    });

    scrollable(content).spacing(8).into()
}

fn view_image(state: &State) -> Element<Message> {
    let file_pane_image: Option<Element<Message>> =
        selected_entry(&state.files.entries, &state.files.selected).map_or(None, |e| {
            state
                .cache
                .get(&e.path)
                .map_or(None, |h| Some(Image::new(h).into()))
        });

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

fn delete_file(state: &mut State) -> Task<Message> {
    if let Some(entry) = selected_entry(&state.files.entries, &state.files.selected) {
        if entry.kind == EntryKind::File {
            let remove_path = entry.path.clone();

            // remove from fs
            if let Err(e) = fs::remove_file(&remove_path) {
                return Task::done(Message::SetMessage(e.to_string()));
            }

            // evict image from memory (if it was opened)
            state.cache.remove(&remove_path);

            // remove from internal entries & update visible entries
            if state.files.selected.len() == 1 {
                state.files.entries.remove(state.files.selected[0]);
                state.files.visible.clear();
                init_visible(&state.files.entries, &mut state.files.visible, 0, vec![]);
            } else {
                // entries don't have a reference to their parents; pop state.selected to go up
                state.files.selected.pop();
                if let Some(entry) =
                    selected_entry_mut(&mut state.files.entries, &state.files.selected)
                {
                    if let Some(children) = &mut entry.children {
                        children
                            .iter()
                            .position(|e| e.path == remove_path)
                            .and_then(|i| Some(children.remove(i)));
                    }
                }
                update_visible(&mut state.files);
            }
            return Task::done(Message::SetMessage(format!("deleted {:?}", remove_path)));
        }
    }
    Task::none()
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

pub fn subscribe(_state: &State) -> Subscription<Message> {
    event::listen().map(Message::Event)
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
}

impl Display for FilesMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use FilesMode::*;
        match self {
            Normal => write!(f, "Normal"),
            Batch => write!(f, "Batch"),
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

struct PromptHistoryEntry {
    timestamp: u64,
    prompt: String,
    characters: Vec<String>,
}

impl PartialOrd for PromptHistoryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for PromptHistoryEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp.cmp(&other.timestamp)
    }
}

impl PartialEq for PromptHistoryEntry {
    fn eq(&self, other: &PromptHistoryEntry) -> bool {
        self.prompt == other.prompt && self.characters == other.characters
    }
}

impl Eq for PromptHistoryEntry {}

impl Hash for PromptHistoryEntry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.prompt.as_bytes());
        state.write(
            &self
                .characters
                .iter()
                .flat_map(|s| s.as_bytes())
                .copied()
                .collect::<Vec<u8>>(),
        );
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

#[cfg(test)]
mod test {
    use super::*;
    use iced::widget::shader::wgpu::naga::FastHashSet;

    #[test]
    fn history1() {
        let h1 = PromptHistoryEntry {
            timestamp: 1,
            prompt: "foobar".into(),
            characters: vec!["foo".into()],
        };
        let h2 = PromptHistoryEntry {
            timestamp: 2,
            prompt: "foobar".into(),
            characters: vec!["foo".into()],
        };
        let h3 = PromptHistoryEntry {
            timestamp: 10,
            prompt: "this is the newest prompt".into(),
            characters: vec!["foo".into(), "bar".into()],
        };

        let mut set = FastHashSet::default();
        set.insert(h1);
        set.insert(h2);
        set.insert(h3);

        assert_eq!(set.len(), 2);
    }
}
