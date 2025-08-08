use std::{
    cmp::Ordering,
    collections::VecDeque,
    fs::{self, FileType},
    io,
    path::{Path, PathBuf},
};

use iced::widget::{image::Handle, shader::wgpu::naga::FastHashMap};

const FILE_EXTENSIONS: [&'static str; 3] = ["jpeg", "jpg", "png"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
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
pub struct VisibleEntry {
    pub depth: usize,
    pub id: usize,
}

#[derive(Debug)]
pub struct Entry {
    pub id: usize,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub parent: Option<usize>,
    pub children: Option<Vec<usize>>,
    pub expanded: bool,
    pub visited: bool,
    pub marked: bool,
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
pub struct FileTree {
    pub entries: FastHashMap<usize, Entry>,
    pub lookup: FastHashMap<PathBuf, usize>,
    pub next_id: usize,

    pub selected: usize,
    pub visible: Vec<VisibleEntry>,
    pub view_offset: usize,

    // hold indices of marked entries for batch move/delete
    pub temp: Vec<usize>,

    // if UI is taking user input for new folder name
    pub create_flag: bool,
}

impl FileTree {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        let mut tree = Self {
            entries: FastHashMap::default(),
            lookup: FastHashMap::default(),
            next_id: 0,

            selected: 0,
            visible: Vec::new(),
            view_offset: 0,

            temp: Vec::new(),

            create_flag: false,
        };

        tree.add_many(dir);

        Self::visit_root(&tree.entries, &mut tree.visible);

        tree.select_start();

        tree
    }

    pub fn add_many<P: AsRef<Path>>(&mut self, dir: P) -> Vec<usize> {
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

    // replace children of parent entry after fs operation (create, rename, delete)
    pub fn update_children(&mut self, parent: usize) {
        if let Some(p) = self.get_entry_mut(parent) {
            let dir = p.path.clone();

            let mut to_remove = Vec::new();
            if let Some(children) = &mut p.children {
                for c in children.drain(..) {
                    to_remove.push(c);
                }
            } else {
                p.children = Some(vec![]);
            }

            for j in to_remove {
                self.entries.remove(&j);
            }

            let mut to_insert = Vec::new();
            if let Ok(mut read_dir) = fs::read_dir(dir) {
                while let Some(Ok(entry)) = read_dir.next() {
                    if let Ok(metadata) = entry.metadata() {
                        to_insert.push((entry.path(), EntryKind::from(metadata.file_type())));
                    }
                }
            }

            for (path, kind) in to_insert {
                self.add(path, Some(parent), kind);
            }

            self.visit(parent);
        }
    }

    pub fn add(&mut self, path: PathBuf, parent: Option<usize>, kind: EntryKind) -> usize {
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
                path: path.clone(),
                kind,
                parent,
                children,
                expanded: false,
                visited: false,
                marked: false,
            },
        );
        self.lookup.insert(path, id);

        if let Some(pid) = parent {
            if let Some(entry) = self.entries.get_mut(&pid) {
                entry.children.as_mut().unwrap().push(id);
            }
        }
        id
    }

    pub fn visit_root(entries: &FastHashMap<usize, Entry>, visible: &mut Vec<VisibleEntry>) {
        visible.clear();

        // initialize queue with top-level entries only
        let mut init_entries: Vec<_> = entries
            .values()
            .filter_map(|entry| entry.parent.map_or(Some((0, entry)), |_i| None))
            .collect();

        // the queue needs to be initialized such that the entries are sorted
        // because this loop only pushes to visible
        // and visible does not have a notion of sorted order, it's only read to render the UI
        init_entries.sort_by(|a, b| a.1.cmp(b.1));
        let mut queue = VecDeque::from(init_entries);

        while let Some((depth, entry)) = queue.pop_front() {
            visible.push(VisibleEntry {
                depth,
                id: entry.id,
            });

            if entry.expanded {
                if let Some(children) = &entry.children {
                    children.iter().for_each(|cid| {
                        if let Some(child_entry) = entries.get(cid) {
                            queue.push_front((depth + 1, child_entry));
                        }
                    });
                }
            }
        }
        println!("visit_root: {:?}\n\n", &visible);
    }

    // BUG: using a hashmap is not the answer
    // modifications to FileTree.entries doesn't preserve any kind of order
    // so visiting the entries will always result in the incorrect order
    // wrong data structure.
    pub fn visit(&mut self, id: usize) {
        if let Some(entry) = self.entries.get(&id) {
            match entry.kind {
                EntryKind::Folder => {
                    if let Some(entry) = self.get_entry(id) {
                        // dbg!("preparing to update entry", entry);
                        let expanded = entry.expanded;

                        // find the insertion position into self.visible
                        // by checking which visible entry's id matches selected entry id
                        let i = self
                            .visible
                            .iter()
                            .position(|v| v.id == entry.id)
                            .expect("invalid id");
                        let d = self.visible[i].depth;

                        // dbg!("before drain", &self.visible);
                        // remove child visible entries
                        let mut j = i + 1;
                        while j < self.visible.len() && self.visible[j].depth > d {
                            j += 1;
                        }
                        self.visible.drain(i + 1..j);
                        // dbg!("after drain", &self.visible);

                        if expanded {
                            let mut to_insert = Vec::new();
                            let mut queue = VecDeque::new();
                            queue.push_front((self.selected, d + 1));

                            while let Some((i, depth)) = queue.pop_front() {
                                // dbg!("processing", i, depth);
                                if let Some(entry) = self.get_entry(i) {
                                    if entry.expanded {
                                        if let Some(children) = &entry.children {
                                            // dbg!(children);
                                            for &j in children {
                                                if let Some(child_entry) = self.get_entry(j) {
                                                    // dbg!(child_entry);
                                                    to_insert.push(VisibleEntry { id: j, depth });
                                                    if child_entry.expanded {
                                                        queue.push_front((j, depth + 1));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            self.visible.splice(i + 1..i + 1, to_insert);
                            // dbg!("after insertion: {:?}", &self.visible);
                        }
                    }
                }
                _ => (),
            }
        }
        println!("visit: {:?}\n\n", &self.visible);
    }

    pub fn enter(&mut self, cache: &mut FastHashMap<PathBuf, Handle>) {
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
                EntryKind::File => {
                    if let Some(_) = entry
                        .path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .filter(|ext| FILE_EXTENSIONS.contains(&ext))
                    {
                        if !cache.contains_key(&entry.path) {
                            let handle = Handle::from_path(&entry.path);
                            cache.insert(entry.path.clone(), handle);
                        } else {
                            cache.remove(&entry.path);
                        }
                    }
                }
                _ => (),
            }
        }
    }

    // TODO: separate keybind for create file and create folder
    pub fn create(&mut self, path: PathBuf) -> Result<(), io::Error> {
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

    // BUG: if the deleted entry has children, have to recursively delete
    pub fn delete(&mut self, id: usize) -> Result<(), io::Error> {
        self._delete(id)?;

        Self::visit_root(&self.entries, &mut self.visible);

        Ok(())
    }

    fn _delete(&mut self, id: usize) -> Result<(), io::Error> {
        if let Some(entry) = self.get_entry(id) {
            match entry.kind {
                EntryKind::Folder => {
                    let _ = fs::remove_dir_all(&entry.path)?;
                }
                EntryKind::File | EntryKind::Symlink => {
                    let _ = fs::remove_file(&entry.path)?;
                }
                _ => (),
            }
        } else {
            return Ok(());
        }

        // entry to remove lives in some entry in self.entries
        if let Some(parent) = self.get_parent_mut(id) {
            if let Some(children) = &mut parent.children {
                if let Some(i) = children.iter().position(|i| *i == id) {
                    println!("removing child from parent at position {}", i);
                    let j = children.remove(i);
                    println!("removing entry from entries {}", j);
                    self.entries.remove(&j);
                }
            }
        } else {
            self.entries.remove(&id);
        }

        Ok(())
    }

    pub fn rename(&mut self, id: usize, new_parent: Option<usize>) -> Result<(), io::Error> {
        let old_parent_id = self._rename(id, new_parent)?;

        // _rename returns initial id if no action was taken
        if id != old_parent_id {
            if let Some(new_parent) = new_parent.and_then(|pid| self.get_parent(pid)) {
                println!("visit new parent {}", new_parent.id);
                self.visit(new_parent.id);
            } else {
                println!("visit root");
                Self::visit_root(&self.entries, &mut self.visible);
            }
        } else {
            panic!("nothing is visited after rename");
        }

        Ok(())
    }

    // NOTE: things are not sorted after being moved
    fn _rename(&mut self, id: usize, new_parent: Option<usize>) -> Result<usize, io::Error> {
        if self.get_entry(id).is_none() {
            return Ok(id);
        }

        let newpath = if let Some(entry) = self.get_entry(id) {
            let oldpath = &entry.path;
            let filename = entry
                .path
                .file_name()
                .expect("bad filename")
                .to_str()
                .expect("non utf-8 filename");

            // if new_parent = None then move the file to the current_dir
            let new_parent_path =
                if let Some(new_parent) = new_parent.and_then(|pid| self.entries.get(&pid)) {
                    if new_parent.kind == EntryKind::Folder {
                        &new_parent.path
                    } else {
                        // selected entry is not a folder, so find its parent (may be None)
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

            // no-op
            if *oldpath == newpath {
                return Ok(id);
            }

            if let Err(e) = fs::rename(oldpath, &newpath) {
                return Err(e);
            }

            Some(newpath)
        } else {
            None
        };

        // remove entry from old parent and add to new parent
        let mut old_parent_id = id;
        if let Some(old_parent) = self.get_parent_mut(id) {
            old_parent_id = old_parent.id;
            if let Some(children) = &mut old_parent.children {
                if let Some(i) = children.iter().position(|i| *i == id) {
                    let node = Some(children.remove(i));
                    // need to change the parent field of the removed node

                    if let Some(node) = node {
                        if let Some(entry) = new_parent.and_then(|pid| self.entries.get_mut(&pid)) {
                            if entry.kind == EntryKind::Folder {
                                match &mut entry.children {
                                    Some(children) => children.push(node),
                                    None => entry.children = Some(vec![node]),
                                }
                            }
                        }
                    }
                }
            }
        }

        let new_parent_kind = new_parent.map_or(EntryKind::File, |pid| {
            self.get_entry(pid)
                .as_ref()
                .map_or(EntryKind::File, |e| e.kind)
        });

        // update path of entry to reflect new parent
        if let Some(entry) = self.get_entry_mut(id) {
            if let Some(newpath) = newpath {
                entry.path = newpath;

                // even though the parameter is Option<usize>,
                // if it's Some it could be a file in which case
                // it wouldn't have children
                if new_parent_kind == EntryKind::Folder {
                    entry.parent = new_parent;
                } else {
                    entry.parent = None;
                }
            }
        }

        Ok(old_parent_id)
    }

    pub fn batch_delete(&mut self) -> Result<(), io::Error> {
        for i in self.temp.clone() {
            self._delete(i)?;
        }

        self.temp.clear();

        Self::visit_root(&self.entries, &mut self.visible);

        Ok(())
    }

    pub fn batch_move(&mut self) -> Result<(), io::Error> {
        let to_rename = self.temp.clone();
        println!("entries to move: {:?}\n\n", to_rename);

        let new_parent = if let Some(entry) = self.get_entry(self.selected) {
            Some(entry.id)
        } else {
            None
        };

        println!("before entries: {:?}\n\n", &self.entries.values());
        println!("before visible: {:?}\n\n", &self.visible);

        for i in to_rename {
            println!("moving {}\n", i);
            self._rename(i, new_parent)?;
        }

        match new_parent {
            Some(p) => {
                println!("visiting new parent");
                self.visit(p);
            }
            None => {
                println!("resetting ui");
                Self::visit_root(&self.entries, &mut self.visible);
            }
        }

        println!("after entries: {:?}\n\n", &self.entries.values());

        // this always is wrong despite the calls to visit / visit_root
        println!("after visible: {:?}\n\n", &self.visible);

        // NOTE: this clears self.temp
        self.clear_marked();

        // this fixes things but isn't ideal to reset ui state
        // *self = FileTree::new(std::env::current_dir().unwrap());

        Ok(())
    }

    pub fn mark(&mut self) {
        if let Some(entry) = self.get_entry_mut(self.selected) {
            entry.marked = !entry.marked;
            self.temp.push(self.selected);
        }
    }

    pub fn clear_marked(&mut self) {
        for i in self.temp.clone() {
            if let Some(entry) = self.get_entry_mut(i) {
                entry.marked = false;
            }
        }

        self.temp.clear();
    }

    pub fn cd_parent(&mut self) {
        if let Ok(dir) = std::env::current_dir() {
            if let Some(parent) = dir.parent() {
                std::env::set_current_dir(parent).expect("failed to cd into parent");
                *self = FileTree::new(parent);
            }
        }
    }

    pub fn cd_selected(&mut self) {
        if let Some(entry) = self.get_entry(self.selected) {
            if entry.kind == EntryKind::Folder {
                std::env::set_current_dir(&entry.path).expect("failed to cd");
                *self = FileTree::new(&entry.path);
            }
        }
    }

    #[inline(always)]
    pub fn select_start(&mut self) {
        self.selected = self.visible[0].id;
    }

    #[inline(always)]
    pub fn select_end(&mut self) {
        assert!(self.visible.len() > 0);
        self.selected = self.visible[self.visible.len() - 1].id;
    }

    #[inline(always)]
    pub fn get_entry(&self, id: usize) -> Option<&Entry> {
        self.entries.get(&id)
    }

    #[inline(always)]
    pub fn get_entry_mut(&mut self, id: usize) -> Option<&mut Entry> {
        self.entries.get_mut(&id)
    }

    #[inline(always)]
    pub fn get_parent(&self, id: usize) -> Option<&Entry> {
        self.entries
            .get(&id)?
            .parent
            .and_then(|pid| self.entries.get(&pid))
    }

    #[inline(always)]
    pub fn get_parent_mut(&mut self, id: usize) -> Option<&mut Entry> {
        self.entries
            .get(&id)?
            .parent
            .and_then(|pid| self.entries.get_mut(&pid))
    }
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;

    use crate::PROJECT_DIRS;

    use super::*;

    fn find<P: AsRef<Path>>(tree: &FileTree, path: P) -> Option<(&usize, &Entry)> {
        tree.entries.iter().find(|(_i, e)| e.path.ends_with(&path))
    }

    fn create_temp_files(names: &[PathBuf]) {
        for n in names {
            fs::File::create_new(n).expect("create_name");
        }
    }

    fn remove_temp_files(names: &[PathBuf]) {
        for n in names {
            fs::remove_file(n).expect("remove_file");
        }
    }

    #[test]
    fn t_rename_deep() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");

        let c = std::env::current_dir().unwrap();
        let mut filenames = Vec::with_capacity(3);
        for p in ["temp/foobar", "temp/grok", "temp/baz"] {
            filenames.push(c.join(p));
        }
        println!("creating: {:?}\n\n\n", filenames);
        create_temp_files(&filenames);

        let mut tree = FileTree::new(&c);
        let mut _cache = FastHashMap::default();

        let i = *find(&tree, "temp").expect("temp not found").0;
        tree.selected = i;
        tree.enter(&mut _cache);

        println!("before entries: {:?}\n\n\n", &tree.entries);
        println!("before visible: {:?}\n\n\n", &tree.visible);

        let temp_files = filenames
            .iter()
            .map(|f| *tree.lookup.get(f).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(temp_files.len(), 3);
        println!("{:?}", temp_files);
        for j in temp_files.clone() {
            println!("{:?}", tree.get_entry(j).unwrap());
        }

        // move each file from temp/ into ./
        for j in temp_files.clone() {
            tree.selected = j;
            tree.rename(j, None).expect("rename");
        }

        for j in temp_files.clone() {
            // moved up to current_dir
            let e = tree.get_entry(j).unwrap();
            assert_eq!(e.parent, None);

            // path was updated
            assert_eq!(
                e.path,
                c.join(e.path.file_name().unwrap().to_str().unwrap())
            );

            // old parent has no children
            let old_parent = tree.get_entry(i).unwrap();
            assert_eq!(old_parent.children, Some(vec![]));
        }

        // rename should visit the root because all of the entries were moved into the current_dir
        println!("after entries: {:?}\n\n\n", &tree.entries);
        println!("after visible: {:?}\n\n\n", &tree.visible);

        // visible entries assertions
        // there should be nothing at depth 1
        assert!(tree.visible.iter().all(|e| e.depth == 0));

        // nothing has been sorted, so we just expect the entries to be present
        for j in temp_files.clone() {
            let found = tree.visible.iter().find(|v| v.id == j);
            assert!(found.is_some());
        }

        remove_temp_files(&filenames);
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
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());
        let mut _cache = FastHashMap::default();

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
            tree.enter(&mut _cache);
        }

        assert!(tree.entries.len() > num_init_entries);
        assert!(tree.visible.len() > num_init_visible);
    }

    #[test]
    fn t_enter2() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());
        let mut _cache = FastHashMap::default();

        let num_init_entries = tree.entries.len();
        let num_init_visible = tree.visible.len();

        let i = find(&tree, "test").expect("test not found");

        tree.selected = *i.0;
        tree.enter(&mut _cache);

        let num_after_entries = tree.entries.len();

        // file1, file2, file3, file99, subtest
        assert_eq!(tree.entries.len(), num_init_entries + 5);
        assert_eq!(tree.visible.len(), num_init_visible + 5);

        tree.enter(&mut _cache);
        // expect removal of visible entries when collapsing dir
        assert_eq!(tree.visible.len(), num_init_visible);

        // expect tree state does not change
        assert_eq!(tree.entries.len(), num_after_entries);
    }

    #[test]
    fn t_create_fs() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
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
    fn t_delete_fs() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let _ = fs::File::create("temp/foo");
        let mut _cache = FastHashMap::default();

        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let i = find(&tree, "temp").expect("temp dir not found");

        tree.selected = *i.0;
        tree.enter(&mut _cache);

        let i = find(&tree, "foo").expect("foo not found");
        tree.delete(*i.0).expect("delete");

        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));
    }

    #[test]
    fn t_rename_fs() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let _ = fs::File::create("foo");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());
        let mut _cache = FastHashMap::default();

        let i = find(&tree, "temp").expect("temp dir not found");
        tree.selected = *i.0;
        tree.enter(&mut _cache);

        let i = find(&tree, "foo").expect("foo not found");
        tree.rename(*i.0, None).expect("rename");

        assert!(fs::exists("foo").is_ok_and(|b| b));
        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));

        let _ = fs::remove_file("foo");
    }

    #[test]
    fn t_batch_delete() {
        let _ = fs::File::create("temp/foo");
        let _ = fs::File::create("baz");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());
        let mut _cache = FastHashMap::default();

        let i = find(&tree, "temp").expect("temp dir not found").0;
        tree.selected = *i;
        tree.enter(&mut _cache);

        let i = find(&tree, "foo").expect("foo not found").0;
        tree.selected = *i;
        tree.mark();

        let i = find(&tree, "baz").expect("baz not found").0;
        tree.selected = *i;
        tree.mark();

        tree.batch_delete().expect("batch_delete");

        assert!(fs::exists("temp/foo").is_ok_and(|b| !b));
        assert!(fs::exists("baz").is_ok_and(|b| !b));
    }

    #[test]
    fn t_batch_move() {
        let _ = fs::File::create("temp/foo");
        let _ = fs::File::create("baz");

        let mut tree = FileTree::new(std::env::current_dir().unwrap());
        let mut _cache = FastHashMap::default();

        let i = find(&tree, "temp").expect("temp dir not found").0;
        tree.selected = *i;
        tree.enter(&mut _cache);

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
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        tree.select_end();

        let entry = tree.get_entry(tree.selected).expect("get_entry");
        assert_eq!(entry.path.file_name(), Some(OsStr::new("prompts.db")));
    }

    #[test]
    fn t_select_start() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        tree.select_start();

        let entry = tree.get_entry(tree.selected).expect("get_entry");
        assert_eq!(entry.path.file_name(), Some(OsStr::new("output")));
    }

    #[test]
    fn t_cd_selected() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let c = std::env::current_dir().unwrap();

        let i = tree
            .entries
            .iter()
            .find(|&(_, e)| e.path.ends_with("output"));
        assert!(i.is_some());

        tree.selected = *i.unwrap().0;
        tree.cd_selected();

        assert_eq!(std::env::current_dir().unwrap(), c.join("output"));
    }

    #[test]
    fn t_cd_parent() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).expect("set_current_dir");
        let mut tree = FileTree::new(std::env::current_dir().unwrap());

        let c = std::env::current_dir().unwrap();

        // doesn't matter what we set this to
        tree.selected = 0;

        // this shouldn't fail unless we're at the root dir
        tree.cd_parent();

        assert_eq!(std::env::current_dir().unwrap(), c.parent().unwrap());
    }
}
