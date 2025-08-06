use std::{
    cmp::Ordering,
    collections::VecDeque,
    fs::{self, FileType},
    io,
    path::{Path, PathBuf},
};

use iced::widget::shader::wgpu::naga::FastHashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Folder,
    Symlink,
    Unknown,
}

impl From<FileType> for EntryKind {
    fn from(value: FileType) -> Self {
        if value.is_dir() {
            Self::Folder
        } else if value.is_file() {
            Self::File
        } else if value.is_symlink() {
            Self::Symlink
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleEntry {
    depth: usize,
    id: usize,
}

#[derive(Debug)]
struct Entry {
    id: usize,
    path: PathBuf,
    kind: EntryKind,
    parent: Option<usize>,
    children: Option<Vec<usize>>,
    expanded: bool,
    visited: bool,
    marked: bool,
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
        if self.kind == other.kind {
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

#[derive(Debug)]
struct FileTree {
    entries: FastHashMap<usize, Entry>,
    next_id: usize,

    selected: usize,
    visible: Vec<VisibleEntry>,

    // hold indices of marked entries for batch move/delete
    temp: Vec<usize>,
}

impl FileTree {
    fn new<P: AsRef<Path>>(dir: P) -> Self {
        let mut tree = Self {
            entries: FastHashMap::default(),
            next_id: 0,

            selected: 0,
            visible: Vec::new(),

            temp: Vec::new(),
        };

        tree.add_many(dir);

        Self::visit_root(&tree.entries, &mut tree.visible);

        tree
    }

    fn add_many<P: AsRef<Path>>(&mut self, dir: P) -> Vec<usize> {
        let mut ret = Vec::new();
        if let Ok(mut read_dir) = fs::read_dir(dir.as_ref()) {
            while let Some(Ok(entry)) = read_dir.next() {
                if let Ok(metadata) = entry.metadata() {
                    let j = self.add(entry.path(), None, EntryKind::from(metadata.file_type()));
                    ret.push(j);
                }
            }
        }
        ret
    }

    fn add(&mut self, path: PathBuf, parent: Option<usize>, kind: EntryKind) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let children = if kind == EntryKind::Folder {
            Some(vec![])
        } else {
            None
        };

        self.entries.insert(
            id,
            Entry {
                id,
                path,
                kind,
                parent,
                children,
                expanded: false,
                visited: false,
                marked: false,
            },
        );

        if let Some(pid) = parent {
            if let Some(entry) = self.entries.get_mut(&pid) {
                entry.children.as_mut().unwrap().push(id);
            }
        }
        id
    }

    fn visit_root(entries: &FastHashMap<usize, Entry>, visible: &mut Vec<VisibleEntry>) {
        let mut queue: VecDeque<_> = entries.values().map(|e| (0, e)).collect();
        while let Some((depth, entry)) = queue.pop_front() {
            visible.push(VisibleEntry {
                depth,
                id: entry.id,
            });

            if let Some(children) = &entry.children {
                children.iter().for_each(|cid| {
                    if let Some(child_entry) = entries.get(cid) {
                        queue.push_back((depth + 1, child_entry));
                    }
                });
            }
        }

        visible.sort_by(|a, b| entries.get(&a.id).cmp(&entries.get(&b.id)));
    }

    fn visit(&mut self, id: usize) {
        if let Some(entry) = self.entries.get(&id) {
            match entry.kind {
                EntryKind::Folder => {
                    println!("visit {:?}", entry);

                    if let Some(entry) = self.get_entry(self.selected) {
                        let expanded = entry.expanded;

                        // find the insertion position into self.visible
                        // by checking which visible entry's id matches selected entry id
                        let i = self
                            .visible
                            .iter()
                            .position(|j| j.id == entry.id)
                            .expect("invalid id");
                        let d = self.visible[i].depth;

                        // remove child visible entries
                        let mut j = i + 1;
                        while j < self.visible.len() && self.visible[j].depth > d {
                            j += 1;
                        }
                        self.visible.drain(i + 1..j);

                        if expanded {
                            let mut to_insert = Vec::new();
                            let mut queue = vec![(self.selected, d + 1)];

                            while let Some((i, depth)) = queue.pop() {
                                if let Some(entry) = self.get_entry(i) {
                                    if entry.expanded {
                                        if let Some(children) = &entry.children {
                                            for &j in children {
                                                to_insert.push(VisibleEntry { id: j, depth });
                                                if let Some(child_entry) = self.get_entry(j) {
                                                    if child_entry.expanded {
                                                        queue.push((j, depth + 1));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            self.visible.splice(i + 1..i + 1, to_insert);
                        }
                    }
                }
                _ => {
                    if let Some(parent) = entry.parent {
                        self.visit(parent);
                    }
                }
            }
        }
    }

    fn enter(&mut self) {
        if let Some(entry) = self.get_entry_mut(self.selected) {
            match entry.kind {
                EntryKind::Folder => {
                    entry.expanded = !entry.expanded;
                    if !entry.visited {
                        entry.visited = true;
                        if let Ok(mut read_dir) = fs::read_dir(&entry.path) {
                            while let Some(Ok(entry)) = read_dir.next() {
                                if let Ok(metadata) = entry.metadata() {
                                    self.add(
                                        entry.path(),
                                        Some(self.selected),
                                        EntryKind::from(metadata.file_type()),
                                    );
                                }
                            }
                        }
                    }

                    self.visit(self.selected);
                }
                _ => (),
            }
        }
    }

    fn create(&mut self, path: PathBuf) -> Result<(), io::Error> {
        let kind = if path.ends_with("/") {
            EntryKind::Folder
        } else {
            EntryKind::File
        };

        if let Some(entry) = self.get_entry(self.selected) {
            match kind {
                EntryKind::Folder => {
                    let _ = fs::create_dir(entry.path.join(&path))?;
                }
                _ => {
                    let _ = fs::File::create_new(entry.path.join(&path))?;
                }
            }
            self.add(path, Some(entry.id), kind);
        }

        self.visit(self.selected);

        Ok(())
    }

    fn delete(&mut self, id: usize) -> Result<(), io::Error> {
        match self._delete(id) {
            Err(e) => return Err(e),
            Ok(old_parent_id) => {
                // entry has no parent, it lives in self.entries
                // either way we need to remove the entry from the map
                self.entries.remove(&id);

                if let Some(parent) = self.get_parent(old_parent_id) {
                    self.visit(parent.id);
                } else {
                    Self::visit_root(&self.entries, &mut self.visible);
                }
            }
        }

        Ok(())
    }

    fn _delete(&mut self, id: usize) -> Result<usize, io::Error> {
        if let Some(entry) = self.get_entry(id) {
            let r = match entry.kind {
                EntryKind::Folder => fs::remove_dir_all(&entry.path),
                _ => fs::remove_file(&entry.path),
            };

            if let Err(e) = r {
                return Err(e);
            }
        }

        // entry to remove lives in some entry in self.entries
        let mut old_parent_id = id;
        if let Some(parent) = self.get_parent_mut(id) {
            old_parent_id = parent.id;
            if let Some(children) = &mut parent.children {
                if let Some(i) = children.iter().position(|i| *i == id) {
                    children.remove(i);
                }
            }
        }

        Ok(old_parent_id)
    }

    fn rename(&mut self, id: usize, new_parent: Option<usize>) -> Result<(), io::Error> {
        match self._rename(id, new_parent) {
            Err(e) => return Err(e),
            Ok(old_parent_id) => {
                // _rename returns initial id if no action was taken
                if id != old_parent_id {
                    self.visit(id);
                }

                if let Some(new_parent) = new_parent.and_then(|pid| self.get_parent(pid)) {
                    self.visit(new_parent.id);
                } else {
                    Self::visit_root(&self.entries, &mut self.visible);
                }
            }
        }

        Ok(())
    }

    fn _rename(&mut self, id: usize, new_parent: Option<usize>) -> Result<usize, io::Error> {
        if let Some(entry) = self.get_entry(id) {
            let oldpath = &entry.path;
            let filename = entry
                .path
                .file_name()
                .expect("bad filename")
                .to_str()
                .expect("non utf-8 filename");

            // if are are not provided a new_parent index then we assume that the user
            // wants to move the file to the top (current_dir)
            let new_parent_path =
                if let Some(new_parent) = new_parent.and_then(|pid| self.get_entry(pid)) {
                    if new_parent.kind == EntryKind::Folder {
                        &new_parent.path
                    } else {
                        if let Some(p) = new_parent.parent {
                            if let Some(e) = self.entries.get(&p) {
                                &e.path
                            } else {
                                panic!("file is not in current directory but doesn't have parent")
                            }
                        } else {
                            &std::env::current_dir().expect("current_dir")
                        }
                    }
                } else {
                    &std::env::current_dir().expect("current_dir")
                };

            let newpath = new_parent_path.join(filename);

            // not renaming
            if *oldpath == newpath {
                return Ok(id);
            }

            if let Err(e) = fs::rename(oldpath, newpath) {
                return Err(e);
            }
        }

        let mut old_parent_id = id;
        if let Some(old_parent) = self.get_parent_mut(id) {
            old_parent_id = old_parent.id;
            if let Some(children) = &mut old_parent.children {
                if let Some(i) = children.iter().position(|i| *i == id) {
                    let node = Some(children.remove(i));

                    if let Some(node) = node {
                        if let Some(entry) = new_parent.and_then(|pid| self.get_entry_mut(pid)) {
                            match &mut entry.children {
                                Some(children) => children.push(node),
                                None => entry.children = Some(vec![node]),
                            }
                        }
                    }
                }
            }
        }

        Ok(old_parent_id)
    }

    fn batch_delete(&mut self) -> Result<(), io::Error> {
        for i in self.temp.clone() {
            self.delete(i)?;
        }

        self.temp.clear();

        Self::visit_root(&self.entries, &mut self.visible);

        Ok(())
    }

    fn batch_move(&mut self) -> Result<(), io::Error> {
        let to_rename = self.temp.clone();

        let new_parent = if let Some(entry) = self.get_entry(self.selected) {
            Some(entry.id)
        } else {
            None
        };

        for i in to_rename {
            self.rename(i, new_parent)?;
        }

        self.temp.clear();

        Self::visit_root(&self.entries, &mut self.visible);

        Ok(())
    }

    fn mark(&mut self) {
        if let Some(entry) = self.get_entry_mut(self.selected) {
            entry.marked = !entry.marked;
            self.temp.push(self.selected);
        }
    }

    fn cd_parent(&mut self) {
        if let Ok(dir) = std::env::current_dir() {
            if let Some(parent) = dir.parent() {
                *self = FileTree::new(parent);
            }
        }
    }

    fn cd_selected(&mut self) {
        if let Some(entry) = self.get_entry(self.selected) {
            if entry.kind == EntryKind::Folder {
                *self = FileTree::new(&entry.path);
            }
        }
    }

    fn select_start(&mut self) {
        self.selected = self.visible[0].id;
    }

    fn select_end(&mut self) {
        assert!(self.visible.len() > 0);
        self.selected = self.visible[self.visible.len() - 1].id;
    }

    fn get_entry(&self, id: usize) -> Option<&Entry> {
        self.entries.get(&id)
    }

    fn get_entry_mut(&mut self, id: usize) -> Option<&mut Entry> {
        self.entries.get_mut(&id)
    }

    fn get_parent(&self, id: usize) -> Option<&Entry> {
        self.entries
            .get(&id)?
            .parent
            .and_then(|pid| self.entries.get(&pid))
    }

    fn get_parent_mut(&mut self, id: usize) -> Option<&mut Entry> {
        self.entries
            .get(&id)?
            .parent
            .and_then(|pid| self.entries.get_mut(&pid))
    }
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use super::*;

    fn find<P: AsRef<Path>>(tree: &FileTree, path: P) -> Option<(&usize, &Entry)> {
        tree.entries.iter().find(|(_i, e)| e.path.ends_with(&path))
    }

    #[test]
    fn t_mark() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        tree.selected = 0;
        tree.mark();

        let entry = tree.get_entry(0).unwrap();
        assert!(entry.marked);
    }

    #[test]
    fn t_enter() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let num_init_entries = tree.entries.len();
        let num_init_visible = tree.visible.len();

        let mut folder_entries: Vec<usize> = vec![];
        for (k, v) in &tree.entries {
            if v.kind == EntryKind::Folder {
                folder_entries.push(*k);
            }
        }

        for i in folder_entries {
            tree.selected = i;
            tree.enter();
        }

        assert!(tree.entries.len() > num_init_entries);
        assert!(tree.visible.len() > num_init_visible);
    }

    #[test]
    fn t_enter2() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let num_init_entries = tree.entries.len();
        let num_init_visible = tree.visible.len();

        let i = find(&tree, "src").expect("src not found");

        tree.selected = *i.0;
        tree.enter();

        let num_after_entries = tree.entries.len();

        // expect increase in both entries and visible entries
        assert!(tree.entries.len() > num_init_entries);
        assert!(tree.visible.len() > num_init_visible);

        tree.enter();
        // expect removal of visible entries when collapsing dir
        assert_eq!(tree.visible.len(), num_init_visible);

        // expect tree state does not change
        assert_eq!(tree.entries.len(), num_after_entries);
    }

    #[test]
    fn t_create() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let entry = find(&tree, "temp").expect("temp not found");

        tree.selected = entry.1.id;
        tree.create(PathBuf::from("foo")).expect("create");

        let foo_path = std::env::current_dir().unwrap().join("temp/foo");
        let exists = fs::exists(foo_path);
        assert!(exists.is_ok_and(|b| b));

        let _ = fs::remove_file("temp/foo");
    }

    #[test]
    fn t_delete() {
        let _ = fs::File::create("temp/foo");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let i = find(&tree, "temp").expect("temp dir not found");

        tree.selected = *i.0;
        tree.enter();

        let i = find(&tree, "foo").expect("foo not found");
        tree.delete(*i.0).expect("delete");

        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));
    }

    #[test]
    fn t_rename() {
        let _ = fs::File::create("foo");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let i = find(&tree, "temp").expect("temp dir not found");
        tree.selected = *i.0;
        tree.enter();

        let i = find(&tree, "foo").expect("foo not found");
        tree.rename(*i.0, None).expect("rename");

        assert!(fs::exists("foo").is_ok_and(|b| b));
        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));

        println!("{:?}", tree.entries);

        let _ = fs::remove_file("foo");
    }

    #[test]
    fn t_batch_delete() {
        let _ = fs::File::create("temp/foo");
        let _ = fs::File::create("baz");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        // println!("{:?}", tree.entries);

        let i = find(&tree, "temp").expect("temp dir not found").0;
        tree.selected = *i;
        tree.enter();

        let i = find(&tree, "foo").expect("foo not found").0;
        tree.selected = *i;
        tree.mark();

        let i = find(&tree, "baz").expect("baz not found").0;
        tree.selected = *i;
        tree.mark();

        tree.batch_delete().expect("batch_delete");

        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));
        assert!(fs::exists("baz").is_ok_and(|b| !b));

        // println!("{:?}", tree.entries);
    }

    #[test]
    fn t_batch_move() {
        let _ = fs::File::create("temp/foo");
        let _ = fs::File::create("baz");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let i = find(&tree, "temp").expect("temp dir not found").0;
        tree.selected = *i;
        tree.enter();

        let i = find(&tree, "foo").expect("foo not found").0;
        tree.selected = *i;
        tree.mark();

        let i = find(&tree, "baz").expect("baz not found").0;
        tree.selected = *i;
        tree.mark();

        // move temp/foo and baz to the current directory, which should be a no-op for baz
        let i = find(&tree, "Cargo.toml").expect("Cargo.toml not found").0;
        tree.selected = *i;
        tree.batch_move().expect("batch_move");

        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));
        assert!(fs::exists("foo").is_ok_and(|b| b));
        assert!(fs::exists("baz").is_ok_and(|b| b));

        let _ = fs::remove_file("foo");
        let _ = fs::remove_file("baz");
    }

    #[test]
    fn t_select_end() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        tree.select_end();

        let entry = tree.get_entry(tree.selected).expect("get_entry");
        assert_eq!(entry.path.file_name(), Some(OsStr::new("todo.txt")));
    }

    #[test]
    fn t_select_start() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        tree.select_start();

        let entry = tree.get_entry(tree.selected).expect("get_entry");
        assert_eq!(entry.path.file_name(), Some(OsStr::new(".git")));
    }

    #[test]
    fn t_cd_selected() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let i = tree.entries.iter().find(|&(_, e)| e.path.ends_with("src"));
        assert!(i.is_some());

        tree.selected = *i.unwrap().0;
        tree.cd_selected();

        let i = find(&tree, "main");
        assert!(i.is_some());
    }

    #[test]
    fn t_cd_parent() {
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        // doesn't matter what we set this to
        tree.selected = 0;

        // this shouldn't fail unless we're at the root dir
        tree.cd_parent();

        let i = find(&tree, "async_fs");
        assert!(i.is_some());
    }
}
