use std::{
    collections::HashSet,
    fmt::{self, Display},
    fs,
    path::PathBuf,
    sync::Arc,
};

use iced::{
    Element, Event,
    Length::{self},
    Subscription, Task, event,
    keyboard::{
        self,
        key::{Key, Named},
    },
    widget::{
        Column, Image, PaneGrid, button, center, column, container,
        image::Handle,
        pane_grid::{self, Axis, Configuration, Direction},
        pick_list, row, scrollable,
        shader::wgpu::naga::FastHashMap,
        text, text_editor,
    },
    window,
};

use crate::{
    files::{
        Entry, EntryKind, FilesState, entries, init_visible, selected_entry, selected_entry_mut,
        update_visible,
    },
    image_metadata::extract_image_metadata,
    nai::{ImageGenRequest, ImageGenerationError, Requester},
    prompt::{self, BASE_PROMPT, Point, Position},
};

const FILE_EXTENSIONS: [&'static str; 7] = ["avif", "gif", "jpeg", "jpg", "qoi", "png", "webp"];
const MAX_VISIBLE: usize = 40;

struct CharacterContent {
    c: prompt::Character,
    content: text_editor::Content,
}

impl CharacterContent {
    fn new() -> Self {
        Self {
            c: prompt::Character::new(),
            content: text_editor::Content::new(),
        }
    }
}

pub struct State {
    message: Option<String>,
    panes: pane_grid::State<Pane>,
    focus: Option<pane_grid::Pane>,
    base_prompt: text_editor::Content,
    c1: CharacterContent,
    c2: CharacterContent,
    c3: CharacterContent,
    c4: CharacterContent,
    c5: CharacterContent,
    c6: CharacterContent,
    curr_char: CharacterNum,

    temp: Vec<(Vec<usize>, PathBuf)>,
    files_mode: FilesMode,
    image_gen_ip: bool,
    cache: FastHashMap<PathBuf, Handle>,
    files: FilesState,
    client: Arc<Requester>,
}

impl Default for State {
    fn default() -> Self {
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
            message: None,
            panes,
            focus: None,
            base_prompt: text_editor::Content::with_text(BASE_PROMPT),
            c1: CharacterContent::new(),
            c2: CharacterContent::new(),
            c3: CharacterContent::new(),
            c4: CharacterContent::new(),
            c5: CharacterContent::new(),
            c6: CharacterContent::new(),
            curr_char: CharacterNum::Char1,

            temp: Vec::new(),
            files_mode: FilesMode::Normal,
            image_gen_ip: false,
            cache: FastHashMap::default(),
            files: FilesState::new("."),
            client: Arc::new(Requester::default()),
        };
        state
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
    EditBasePrompt(text_editor::Action),
    EditChar1(text_editor::Action),
    EditChar2(text_editor::Action),
    EditChar3(text_editor::Action),
    EditChar4(text_editor::Action),
    EditChar5(text_editor::Action),
    EditChar6(text_editor::Action),
    ImageGenerated(Result<String, ImageGenerationError>),
    CharSelected(CharacterNum),
    SetPosition(Position),
    SubmitRequest,
    ToggleExpand(Vec<usize>),
    Refresh,
    RefreshSelected,
    Entries((Vec<usize>, Vec<Entry>)),
    GotoStart,
    GotoEnd,
    NavigateUp,
    SetRoot,
    ImportPrompt(String, Vec<String>),
    Delete,
    DeleteBatch,
    MoveBatch,
    FilesMode(FilesMode),
    SelectEntry,
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

pub fn update(state: &mut State, msg: Message) -> Task<Message> {
    use Message::*;
    match msg {
        Entries((path, mut entries)) => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                entries.sort();
                entry.children = Some(entries);
                entry.set_expanded();
            }
            update_visible(&mut state.files);
        }
        Event(e) => return handle_event(state, e),
        SetMessage(s) => state.message = Some(s),
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
        EditBasePrompt(action) => state.base_prompt.perform(action),
        EditChar1(action) => state.c1.content.perform(action),
        EditChar2(action) => state.c2.content.perform(action),
        EditChar3(action) => state.c3.content.perform(action),
        EditChar4(action) => state.c4.content.perform(action),
        EditChar5(action) => state.c5.content.perform(action),
        EditChar6(action) => state.c6.content.perform(action),
        CharSelected(c) => {
            state.curr_char = c;
            return Task::done(Message::SetMessage(format!("set curr_char to {}", c)));
        }
        SetPosition(p) => {
            match state.curr_char {
                CharacterNum::Char1 => state.c1.c.center(p),
                CharacterNum::Char2 => state.c2.c.center(p),
                CharacterNum::Char3 => state.c3.c.center(p),
                CharacterNum::Char4 => state.c4.c.center(p),
                CharacterNum::Char5 => state.c5.c.center(p),
                CharacterNum::Char6 => state.c6.c.center(p),
            };
            return Task::done(Message::SetMessage(format!(
                "set curr_position of {} to {:?}",
                state.curr_char, p
            )));
        }
        SubmitRequest => {
            state.image_gen_ip = true;

            let mut req = ImageGenRequest::default();
            req.prompt(state.base_prompt.text());

            for c in [
                &mut state.c1,
                &mut state.c2,
                &mut state.c3,
                &mut state.c4,
                &mut state.c5,
                &mut state.c6,
            ]
            .iter_mut()
            {
                if c.content.text() == "\n" {
                    continue;
                }
                c.c.prompt(c.content.text());
                req.add_character(&c.c);
            }

            if [
                state.c1.c.get_center(),
                state.c2.c.get_center(),
                state.c3.c.get_center(),
                state.c4.c.get_center(),
                state.c5.c.get_center(),
                state.c6.c.get_center(),
            ]
            .iter()
            .any(|p| *p == Point { x: 0.5, y: 0.5 })
            {
                req.use_coords(true);
            }

            let r = Arc::clone(&state.client);

            return Task::perform(
                async move {
                    r.generate_image(crate::nai::ImageShape::Portrait, req)
                        .await
                },
                |res| Message::ImageGenerated(res),
            );
        }
        ImageGenerated(res) => {
            state.image_gen_ip = false;
            match res {
                Err(e) => state.message = Some(e.to_string()),
                Ok(s) => state.message = Some(s),
            }
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
        ImportPrompt(base, characters) => {
            use text_editor::{Action, Edit};
            state.base_prompt.perform(Action::SelectAll);
            state.base_prompt.perform(Action::Edit(Edit::Delete));
            state
                .base_prompt
                .perform(Action::Edit(Edit::Paste(Arc::new(base))));

            for (i, c) in characters.iter().enumerate() {
                match i {
                    0 => {
                        state.c1.content.perform(Action::SelectAll);
                        state.c1.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c1
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    1 => {
                        state.c2.content.perform(Action::SelectAll);
                        state.c2.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c2
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    2 => {
                        state.c3.content.perform(Action::SelectAll);
                        state.c3.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c3
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    3 => {
                        state.c4.content.perform(Action::SelectAll);
                        state.c4.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c4
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    4 => {
                        state.c5.content.perform(Action::SelectAll);
                        state.c5.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c5
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    5 => {
                        state.c6.content.perform(Action::SelectAll);
                        state.c6.content.perform(Action::Edit(Edit::Delete));
                        state
                            .c6
                            .content
                            .perform(Action::Edit(Edit::Paste(Arc::new(c.clone()))));
                    }
                    _ => (),
                }
            }
        }
        Delete => return delete_file(state),
        DeleteBatch => (),
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
    }
    Task::none()
}

fn handle_event(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(ref e) => match e {
            keyboard::Event::KeyPressed { key, .. } => match key.as_ref() {
                Key::Named(Named::ArrowUp) => {
                    return Task::done(Message::FocusAdjacent(Direction::Up));
                }
                Key::Named(Named::ArrowDown) => {
                    return Task::done(Message::FocusAdjacent(Direction::Down));
                }
                Key::Named(Named::ArrowLeft) => {
                    return Task::done(Message::FocusAdjacent(Direction::Left));
                }
                Key::Named(Named::ArrowRight) => {
                    return Task::done(Message::FocusAdjacent(Direction::Right));
                }
                _ => (),
            },
            _ => (),
        },
        Event::Mouse(ref _e) => (),
        Event::Window(ref e) => match e {
            window::Event::FileDropped(path) => return get_prompt_metadata(path),
            _ => (),
        },
        Event::Touch(_) => (),
    }

    if let Some(focused) = state.focus {
        if let Some(pane) = state.panes.get(focused) {
            match pane.id {
                PaneId::Files => return handle_event_files(state, e),
                PaneId::Prompts => (),
                PaneId::Image => (),
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
                                    return get_prompt_metadata(&entry.path);
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
                        Key::Character("d") => {
                            return Task::done(Message::DeleteBatch);
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
    let text_areas = column![
        text_editor(&state.base_prompt)
            .placeholder("base prompt")
            .on_action(Message::EditBasePrompt),
        text_editor(&state.c1.content)
            .placeholder("character 1")
            .on_action(Message::EditChar1),
        text_editor(&state.c2.content)
            .placeholder("character 2")
            .on_action(Message::EditChar2),
        text_editor(&state.c3.content)
            .placeholder("character 3")
            .on_action(Message::EditChar3),
        text_editor(&state.c4.content)
            .placeholder("character 4")
            .on_action(Message::EditChar4),
        text_editor(&state.c5.content)
            .placeholder("character 5")
            .on_action(Message::EditChar5),
        text_editor(&state.c6.content)
            .placeholder("character 6")
            .on_action(Message::EditChar6),
    ]
    .spacing(10);

    let char_dropdown = pick_list(
        [
            CharacterNum::Char1,
            CharacterNum::Char2,
            CharacterNum::Char3,
            CharacterNum::Char4,
            CharacterNum::Char5,
            CharacterNum::Char6,
        ],
        Some(state.curr_char),
        Message::CharSelected,
    );

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

    let mut content = Column::with_children([
        text_areas.into(),
        char_dropdown.into(),
        position_grid.into(),
    ]);

    content = content.push_maybe(if !state.image_gen_ip {
        Some(button("Submit").on_press(Message::SubmitRequest))
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
    let fallback = center(text("nothing to see here")).into();
    let notfound = center(text("no image")).into();

    selected_entry(&state.files.entries, &state.files.selected).map_or(fallback, |e| {
        state
            .cache
            .get(&e.path)
            .map_or(notfound, |h| center(Image::new(h)).into())
    })
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

fn get_prompt_metadata(path: &PathBuf) -> Task<Message> {
    if path
        .extension()
        .map_or(false, |s| s.to_string_lossy() == "png")
    {
        if let Ok(meta) = extract_image_metadata(&path) {
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
            return Task::done(Message::ImportPrompt(prompt, character_prompts));
        }
    }
    Task::none()
}

pub fn subscribe(_state: &State) -> Subscription<Message> {
    event::listen().map(Message::Event)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterNum {
    Char1,
    Char2,
    Char3,
    Char4,
    Char5,
    Char6,
}

impl fmt::Display for CharacterNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char1 => write!(f, "char1"),
            Self::Char2 => write!(f, "char2"),
            Self::Char3 => write!(f, "char3"),
            Self::Char4 => write!(f, "char4"),
            Self::Char5 => write!(f, "char5"),
            Self::Char6 => write!(f, "char6"),
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
