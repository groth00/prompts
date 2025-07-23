use std::{
    cmp::Ordering,
    fs::{self},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub expanded: bool,
    pub visited: bool,
    pub children: Option<Vec<Entry>>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.kind == EntryKind::Folder && other.kind == EntryKind::Folder
            || self.kind == EntryKind::File && other.kind == EntryKind::File
        {
            self.path.cmp(&other.path)
        } else if self.kind == EntryKind::Folder && other.kind == EntryKind::File {
            Ordering::Less
        } else if self.kind == EntryKind::File && other.kind == EntryKind::Folder {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleEntry {
    pub depth: usize,
    pub path: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Enter,
    Refresh,
    RefreshSelected,
    GotoStart,
    GotoEnd,
    CloseAll,

    NavigateUp,
    SetRoot,
}

#[derive(Debug)]
pub struct FilesState {
    // internal
    pub entries: Vec<Entry>,
    pub selected: Vec<usize>,

    // UI
    // traverse entries to create flattened view
    pub visible: Vec<VisibleEntry>,
    // control how many entries to render
    pub view_offset: usize,
}

impl FilesState {
    fn cmd(&mut self, c: Command) {
        use Command::*;
        match c {
            Enter => {
                if let Some(entry) = selected_entry_mut(&mut self.entries, &self.selected) {
                    if entry.kind == EntryKind::Folder {
                        entry.expanded = !entry.expanded;
                        if !entry.visited {
                            entry.children = Some(entries(&entry.path));
                            entry.visited = true;
                        }
                        update_visible(self);
                    } else {
                        // do something with file
                    }
                }
            }
            Refresh => {
                *self = Self::new(".");
                init_visible(&self.entries, &mut self.visible, 0, vec![]);
            }
            RefreshSelected => {
                if let Some(entry) = selected_entry_mut(&mut self.entries, &self.selected) {
                    if entry.kind == EntryKind::Folder && entry.visited {
                        if let Some(children) = entry.children.as_mut() {
                            children.clear();
                        }
                        entry.children = Some(entries(&entry.path));
                    }
                    if entry.expanded {
                        update_visible(self);
                    }
                }
            }
            GotoStart => self.selected = vec![0],
            GotoEnd => self.selected = vec![self.entries.len() - 1],
            CloseAll => {
                let queue = &mut self.entries;
                while let Some(mut entry) = queue.pop() {
                    if entry.expanded {
                        entry.expanded = false
                    }
                    if entry.kind == EntryKind::Folder && entry.children.is_some() {
                        queue.extend_from_slice(&entry.children.unwrap());
                    }
                }
                init_visible(&self.entries, &mut self.visible, 0, vec![]);
            }
            NavigateUp => {
                *self = Self::new("../");
                init_visible(&self.entries, &mut self.visible, 0, vec![]);
            }
            SetRoot => {
                if let Some(entry) = selected_entry(&self.entries, &self.selected) {
                    if entry.kind == EntryKind::Folder {
                        *self = Self::new(&entry.path);
                        init_visible(&self.entries, &mut self.visible, 0, vec![]);
                    }
                }
            }
        }
    }
}

impl FilesState {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        let mut entries = entries(dir);
        entries.sort();

        let mut state = Self {
            entries,
            selected: vec![0],
            visible: vec![],
            view_offset: 0,
        };
        init_visible(&state.entries, &mut state.visible, 0, vec![]);
        state
    }
}

pub fn selected_entry<'a>(entries: &'a [Entry], path: &'a [usize]) -> Option<&'a Entry> {
    let (first, rest) = path.split_first()?;

    let entry = entries.get(*first)?;
    if rest.is_empty() {
        return Some(entry);
    }
    if entry.kind == EntryKind::Folder {
        let children = entry.children.as_ref()?;
        selected_entry(children, rest)
    } else {
        None
    }
}

pub fn selected_entry_mut<'a>(
    entries: &'a mut [Entry],
    path: &'a [usize],
) -> Option<&'a mut Entry> {
    let (first, rest) = path.split_first()?;

    let entry = entries.get_mut(*first)?;
    if rest.is_empty() {
        return Some(entry);
    }
    if entry.kind == EntryKind::Folder {
        let children = entry.children.as_mut()?;
        selected_entry_mut(children, rest)
    } else {
        None
    }
}

pub fn update_visible(state: &mut FilesState) {
    if let Some(i) = state
        .visible
        .iter()
        .position(|ve| ve.path == state.selected)
    {
        let d = state.visible[i].depth;
        let path = &state.selected;

        if let Some(entry) = selected_entry(&state.entries, path) {
            if !entry.expanded {
                let mut j = i + 1;
                while j < state.visible.len() && state.visible[j].depth > d {
                    j += 1;
                }
                state.visible.drain(i + 1..j);
            } else {
                let mut to_insert = Vec::new();
                let mut queue = vec![(path.clone(), d + 1)];

                while let Some((current_path, depth)) = queue.pop() {
                    if let Some(entry) = selected_entry(&state.entries, &current_path) {
                        if entry.expanded {
                            if let Some(children) = &entry.children {
                                for (i, _e) in children.iter().enumerate() {
                                    let mut child_path = current_path.clone();
                                    child_path.push(i);
                                    to_insert.push(VisibleEntry {
                                        path: child_path.clone(),
                                        depth,
                                    });
                                    if let Some(_entry) = entry
                                        .children
                                        .as_ref()
                                        .and_then(|v| v.get(i))
                                        .filter(|e| e.expanded)
                                    {
                                        queue.push((child_path.clone(), depth + 1));
                                    }
                                }
                            }
                        }
                    }
                }
                state.visible.splice(i + 1..i + 1, to_insert);
            }
        }
    }
}

/// depth-first visit for entries (+expanded) in current directory
pub fn init_visible<'a>(
    entries: &'a [Entry],
    out: &mut Vec<VisibleEntry>,
    depth: usize,
    index: Vec<usize>,
) {
    for (i, entry) in entries.iter().enumerate() {
        let mut path = index.clone();
        path.push(i);

        out.push(VisibleEntry {
            depth,
            path: path.clone(),
        });

        if entry.kind == EntryKind::Folder && entry.expanded && entry.children.is_some() {
            init_visible(entry.children.as_ref().unwrap(), out, depth + 1, path);
        }
    }
}

/// Get unsorted entries of directory
pub fn entries<P: AsRef<Path>>(dir: P) -> Vec<Entry> {
    let mut read_dir = fs::read_dir(dir).expect("read_dir .");
    let mut entries = Vec::new();
    while let Some(Ok(entry)) = read_dir.next() {
        if let Ok(meta) = entry.metadata() {
            let path = entry.path();
            if meta.is_file() {
                entries.push(Entry {
                    path,
                    kind: EntryKind::File,
                    visited: false,
                    expanded: false,
                    children: None,
                });
            } else if meta.is_dir() {
                entries.push(Entry {
                    path,
                    kind: EntryKind::Folder,
                    visited: false,
                    expanded: false,
                    children: Some(Vec::new()),
                });
            }
        }
    }
    entries
}

#[cfg(test)]
mod test {
    use super::*;

    use std::fs::OpenOptions;

    #[test]
    fn selected_entry_exists() {
        let mut state = FilesState::new(".");
        state.selected = vec![1];

        let entry = selected_entry_mut(&mut state.entries, &state.selected);
        assert_eq!(
            entry,
            Some(Entry {
                path: PathBuf::from("./src"),
                kind: EntryKind::Folder,
                visited: false,
                expanded: false,
                children: Some(Vec::new()),
            })
            .as_mut()
        )
    }

    #[test]
    fn selected_entry_none() {
        let mut state = FilesState::new(".");
        // .git/, src/, target/
        // src/main.rs
        state.selected = vec![1, 0];

        let entry = selected_entry(&state.entries, &state.selected);
        assert_eq!(entry, None);
    }

    #[test]
    fn selected_after_enter() {
        let mut state = FilesState::new(".");
        // ./.git, ./src, ./target
        // visit ./src
        state.selected = vec![1];
        state.cmd(Command::Enter);

        // visit ./src/main.rs
        state.selected = vec![1, 0];

        let entry = selected_entry(&state.entries, &state.selected);
        assert_eq!(
            entry,
            Some(Entry {
                path: PathBuf::from("./src/main.rs"),
                kind: EntryKind::File,
                visited: false,
                expanded: false,
                children: None,
            })
            .as_ref()
        );
    }

    #[test]
    fn visible_after_enter() {
        let mut state = FilesState::new(".");
        // .target/
        state.selected = vec![2];
        state.cmd(Command::Enter);

        // NOTE: state.visible contains the flattened view, state.entries contains the nodes
        //
        // ./target -> [./target/debug, ./target/rust-analyzer, ./target/.rustc_info.json]
        let entry = selected_entry(&state.entries, &state.selected);
        assert_eq!(10, state.visible.len());
        assert_eq!(
            entry,
            Some(Entry {
                path: PathBuf::from("./target"),
                kind: EntryKind::Folder,
                visited: true,
                expanded: true,
                children: Some(vec![
                    Entry {
                        path: PathBuf::from("./target/debug"),
                        kind: EntryKind::Folder,
                        visited: false,
                        expanded: false,
                        children: Some(vec![]),
                    },
                    Entry {
                        path: PathBuf::from("./target/rust-analyzer"),
                        kind: EntryKind::Folder,
                        visited: false,
                        expanded: false,
                        children: Some(vec![]),
                    },
                    Entry {
                        path: PathBuf::from("./target/.rustc_info.json"),
                        kind: EntryKind::Folder,
                        visited: false,
                        expanded: false,
                        children: None,
                    }
                ])
            })
            .as_ref()
        );
    }

    #[test]
    fn refresh_full() {
        let mut state = FilesState::new(".");

        assert_eq!(7, state.entries.len());

        if fs::exists("foo").expect("exists foo") {
            fs::remove_file("foo").expect("remove foo");
        };
        OpenOptions::new()
            .write(true)
            .create(true)
            .open("foo")
            .expect("create foo");

        state.cmd(Command::Refresh);

        assert_eq!(8, state.entries.len());
        assert!(state.entries.contains(&Entry {
            path: PathBuf::from("./foo"),
            kind: EntryKind::Folder,
            visited: false,
            expanded: false,
            children: None,
        }));

        fs::remove_file("foo").expect("remove foo");
    }

    #[test]
    fn refresh_selected() {
        let mut state = FilesState::new(".");

        assert!(state.entries.contains(&Entry {
            path: PathBuf::from("./temp"),
            kind: EntryKind::Folder,
            visited: false,
            expanded: false,
            children: Some(vec![])
        }));
        let visible_snapshot = state.visible.clone();

        if fs::exists("temp/foo").expect("exists temp/foo") {
            fs::remove_file("temp/foo").expect("remove temp/foo");
        };
        OpenOptions::new()
            .write(true)
            .create(true)
            .open("temp/foo")
            .expect("create foo");

        // ./temp -> [./temp/foo]
        state.selected = vec![3];
        state.cmd(Command::RefreshSelected);

        assert!(state.entries.contains(&Entry {
            path: PathBuf::from("./temp"),
            kind: EntryKind::Folder,
            visited: false,
            expanded: false,
            children: Some(vec![Entry {
                path: PathBuf::from("./temp/foo"),
                kind: EntryKind::File,
                visited: false,
                expanded: false,
                children: None,
            }])
        }));
        // we never expanded the directory so the UI should remain the same
        assert_eq!(state.visible, visible_snapshot);

        fs::remove_file("temp/foo").expect("remove temp/foo");
    }
}
