use std::{fmt, path::PathBuf, sync::Arc};

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
};

use crate::{
    files::{
        Entry, EntryKind, FilesState, entries, selected_entry, selected_entry_mut, update_visible,
    },
    nai::{ImageGenRequest, ImageGenerationError, Requester},
    prompt,
};

const FILE_EXTENSIONS: [&'static str; 7] = ["avif", "gif", "jpeg", "jpg", "qoi", "png", "webp"];
const MAX_VISIBLE: usize = 40;

pub struct State {
    message: Option<String>,
    panes: pane_grid::State<Pane>,
    focus: Option<pane_grid::Pane>,
    base_prompt: text_editor::Content,
    char1: text_editor::Content,
    char2: text_editor::Content,
    char3: text_editor::Content,
    char4: text_editor::Content,
    char5: text_editor::Content,
    char6: text_editor::Content,
    curr_char: Character,
    curr_position: Position,

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

        // TODO: set default prompt text
        let state = Self {
            message: None,
            panes,
            focus: None,
            base_prompt: text_editor::Content::new(),
            char1: text_editor::Content::new(),
            char2: text_editor::Content::new(),
            char3: text_editor::Content::new(),
            char4: text_editor::Content::new(),
            char5: text_editor::Content::new(),
            char6: text_editor::Content::new(),
            curr_char: Character::Char1,
            curr_position: Position::R2C2,

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
    CharSelected(Character),
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
}

pub fn update(state: &mut State, msg: Message) -> Task<Message> {
    use Message::*;
    match msg {
        Entries((path, mut entries)) => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                entries.sort();
                entry.children = Some(entries);
                entry.expanded = true;
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
        EditChar1(action) => state.char1.perform(action),
        EditChar2(action) => state.char2.perform(action),
        EditChar3(action) => state.char3.perform(action),
        EditChar4(action) => state.char4.perform(action),
        EditChar5(action) => state.char5.perform(action),
        EditChar6(action) => state.char6.perform(action),
        CharSelected(c) => {
            state.curr_char = c;
            return Task::done(Message::SetMessage(format!("set curr_char to {}", c)));
        }
        SetPosition(p) => {
            state.curr_position = p;
            return Task::done(Message::SetMessage(format!(
                "set curr_position of {} to {:?}",
                state.curr_char, p
            )));
        }
        SubmitRequest => {
            let mut req = ImageGenRequest::default();
            req.prompt(state.base_prompt.text());

            for c in [
                &state.char1,
                &state.char2,
                &state.char3,
                &state.char4,
                &state.char5,
                &state.char6,
            ] {
                if c.text().is_empty() {
                    continue;
                }

                let mut ch = prompt::Character::new();
                ch.prompt(c.text());
                req.add_character(&ch);
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
        ImageGenerated(res) => match res {
            Err(e) => state.message = Some(e.to_string()),
            Ok(s) => state.message = Some(s),
        },
        ToggleExpand(path) => {
            if let Some(entry) = selected_entry_mut(&mut state.files.entries, &path) {
                match entry.kind {
                    EntryKind::Folder => {
                        if !entry.visited {
                            entry.visited = true;
                            return Task::done(Message::Entries((
                                path.clone(),
                                entries(&entry.path),
                            )));
                        }
                        entry.expanded = !entry.expanded;
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
                if entry.kind == EntryKind::Folder && entry.visited {
                    if let Some(children) = entry.children.as_mut() {
                        let mut entries = entries(&entry.path);
                        entries.sort();
                        *children = entries;
                    }
                }
                if entry.expanded {
                    entry.expanded = false;
                    update_visible(&mut state.files);
                }
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
        Event::Window(ref _e) => (),
        Event::Touch(_) => (),
    }

    if let Some(focused) = state.focus {
        match state.panes.get(focused).unwrap().id {
            PaneId::Files => return handle_event_files(state, e),
            PaneId::Prompts => (),
            PaneId::Image => (),
        }
    }
    Task::none()
}

fn handle_event_files(state: &mut State, e: Event) -> Task<Message> {
    match e {
        Event::Keyboard(e) => match e {
            keyboard::Event::KeyPressed { key, modifiers, .. } => {
                let current_index = state
                    .files
                    .visible
                    .iter()
                    .position(|ve| &ve.path == &state.files.selected);

                match key.as_ref() {
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
                    Key::Named(Named::Backspace) => {
                        return Task::done(Message::NavigateUp);
                    }
                    Key::Character(".") => {
                        return Task::done(Message::SetRoot);
                    }
                    Key::Character("r") => {
                        if modifiers.shift() {
                            return Task::done(Message::Refresh);
                        }
                        return Task::done(Message::RefreshSelected);
                    }
                    _ => (),
                }

                Task::none()
            }
            _ => Task::none(),
        },
        _ => Task::none(),
    }
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

            let is_selected = state.files.selected == e.path;

            let style = if is_selected {
                text::primary
            } else {
                text::default
            };

            col.push(text(label).style(style))
        });

    scrollable(col).into()
}

fn view_prompts(state: &State) -> Element<Message> {
    let text_areas = column![
        text_editor(&state.base_prompt)
            .placeholder("base prompt")
            .on_action(Message::EditBasePrompt),
        text_editor(&state.char1)
            .placeholder("character 1")
            .on_action(Message::EditChar1),
        text_editor(&state.char2)
            .placeholder("character 2")
            .on_action(Message::EditChar2),
        text_editor(&state.char3)
            .placeholder("character 3")
            .on_action(Message::EditChar3),
        text_editor(&state.char4)
            .placeholder("character 4")
            .on_action(Message::EditChar4),
        text_editor(&state.char5)
            .placeholder("character 5")
            .on_action(Message::EditChar5),
        text_editor(&state.char6)
            .placeholder("character 6")
            .on_action(Message::EditChar6),
    ]
    .spacing(10);

    let char_dropdown = pick_list(
        [
            Character::Char1,
            Character::Char2,
            Character::Char3,
            Character::Char4,
            Character::Char5,
            Character::Char6,
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

    let submit_button = button("Submit").on_press(Message::SubmitRequest);

    let mut content = Column::with_children([
        text_areas.into(),
        char_dropdown.into(),
        position_grid.into(),
        submit_button.into(),
    ]);

    content = content.push_maybe(if let Some(message) = &state.message {
        Some(text(message))
    } else {
        None
    });

    scrollable(content).spacing(8).into()
}

fn view_image(state: &State) -> Element<Message> {
    let fallback = center(text("nothing to see here")).into();
    let notfound = center(text("no image in cache")).into();

    selected_entry(&state.files.entries, &state.files.selected).map_or(fallback, |e| {
        state
            .cache
            .get(&e.path)
            .map_or(notfound, |h| Image::new(h).into())
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

pub fn subscribe(_state: &State) -> Subscription<Message> {
    event::listen().map(Message::Event)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Character {
    Char1,
    Char2,
    Char3,
    Char4,
    Char5,
    Char6,
}

impl fmt::Display for Character {
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

#[derive(Debug, Clone, Copy)]
pub enum Position {
    R0C0,
    R0C1,
    R0C2,
    R0C3,
    R0C4,
    R1C0,
    R1C1,
    R1C2,
    R1C3,
    R1C4,
    R2C0,
    R2C1,
    R2C2,
    R2C3,
    R2C4,
    R3C0,
    R3C1,
    R3C2,
    R3C3,
    R3C4,
    R4C0,
    R4C1,
    R4C2,
    R4C3,
    R4C4,
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
