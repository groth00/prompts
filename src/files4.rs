use std::{
    cmp::Ordering,
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use iced::widget::{image::Handle, shader::wgpu::naga::FastHashMap};
use slotmap::{SlotMap, new_key_type};

pub const MAX_VISIBLE: usize = 40;
const FILE_EXTENSIONS: [&'static str; 3] = ["jpeg", "jpg", "png"];

new_key_type! { pub struct EntryId; }

#[derive(Debug)]
pub enum CreateEntryKind {
    Folder(PathBuf),
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntryKind {
    Folder,
    File,
}

#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub parent: Option<EntryId>,
    pub children: Vec<EntryId>,
    pub expanded: bool,
    pub visited: bool,
    pub marked: bool,
}

#[derive(Debug)]
pub struct VisibleEntry {
    pub id: EntryId,
    pub depth: usize,
}

pub struct FileTree {
    pub entries: SlotMap<EntryId, Entry>,
    pub root: EntryId,

    pub selected: EntryId,
    pub visible: Vec<VisibleEntry>,
    pub view_offset: usize,

    pub temp: Vec<EntryId>,

    pub cache: FastHashMap<PathBuf, Handle>,

    pub create_flag: bool,
}

impl FileTree {
    pub fn new<P: AsRef<Path>>(dir: P) -> Self {
        let mut entries = SlotMap::with_key();

        let root = entries.insert(Entry {
            path: dir.as_ref().to_path_buf(),
            kind: EntryKind::Folder,
            parent: None,
            children: Vec::new(),
            expanded: true,
            visited: false,
            marked: false,
        });

        let mut ret = Self {
            entries,
            root,

            selected: root,
            visible: Vec::new(),
            view_offset: 0,

            temp: Vec::new(),

            cache: FastHashMap::default(),

            create_flag: false,
        };

        if let Ok(mut read_dir) = fs::read_dir(dir.as_ref()) {
            while let Some(Ok(entry)) = read_dir.next() {
                if let Ok(meta) = entry.metadata() {
                    let kind = if meta.is_dir() {
                        EntryKind::Folder
                    } else {
                        EntryKind::File
                    };

                    ret.add(root, entry.path(), kind);
                }
            }
        }

        ret.visible = ret.visible_entries();
        println!("{:?}", &ret.visible);

        ret
    }

    pub fn enter(&mut self) {
        let id = self.selected;
        let entry = &mut self.entries[id];
        println!("entered {:?}", entry);
        match entry.kind {
            EntryKind::Folder => {
                entry.expanded = !entry.expanded;

                if !entry.visited {
                    entry.visited = true;

                    if let Ok(mut read_dir) = fs::read_dir(&entry.path) {
                        while let Some(Ok(entry)) = read_dir.next() {
                            if let Ok(meta) = entry.metadata() {
                                let kind = if meta.is_dir() {
                                    EntryKind::Folder
                                } else {
                                    EntryKind::File
                                };

                                self.add(id, entry.path(), kind);
                            }
                        }
                    }
                }

                self.visible = self.visible_entries();
                println!("{:?}", &self.visible);
            }
            EntryKind::File => {
                if let Some(_) = entry
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .filter(|ext| FILE_EXTENSIONS.contains(&ext))
                {
                    if !self.cache.contains_key(&entry.path) {
                        let handle = Handle::from_path(&entry.path);
                        self.cache.insert(entry.path.clone(), handle);
                    } else {
                        self.cache.remove(&entry.path);
                    }
                }
            }
        }
    }

    pub fn add(&mut self, parent: EntryId, path: PathBuf, kind: EntryKind) -> Option<EntryId> {
        if !self.entries.contains_key(parent) {
            return None;
        }

        println!("add {:?} to {:?}", &path, &parent);
        let id = self.entries.insert(Entry {
            path,
            kind,
            parent: Some(parent),
            children: Vec::new(),
            expanded: false,
            visited: false,
            marked: false,
        });

        self.insert_sorted_child(parent, id);
        Some(id)
    }

    pub fn visible_entries(&self) -> Vec<VisibleEntry> {
        let mut result = Vec::new();
        self.collect_visible(self.root, &mut result);
        result
    }

    pub fn create(&mut self, kind: CreateEntryKind) -> Result<(), io::Error> {
        let id = self.selected;
        let entry = &self.entries[id];

        let (entry_id, parent_path) = match entry.kind {
            EntryKind::Folder => (id, &entry.path),
            EntryKind::File => {
                if let Some(pid) = entry.parent {
                    (pid, &self.entries[pid].path)
                } else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "selected entry is a file without a parent",
                    ));
                }
            }
        };

        let (newpath, entry_kind) = match kind {
            CreateEntryKind::Folder(path) => {
                println!("create dir: {:?}", &parent_path.join(&path));
                let _ = fs::create_dir_all(parent_path.join(&path))?;
                (path, EntryKind::Folder)
            }
            CreateEntryKind::File(path) => {
                println!("create file: {:?}", &parent_path.join(&path));
                let _ = fs::File::create_new(parent_path.join(&path))?;
                (path, EntryKind::File)
            }
        };

        self.add(id, newpath, entry_kind);

        // reset visible entries for parent
        self.selected = entry_id;
        self.enter();
        self.entries[entry_id].visited = false;
        self.entries[entry_id].children.clear();
        self.enter();

        // self.selected = new_id;

        Ok(())
    }

    pub fn delete(&mut self, id: EntryId) -> Result<EntryId, io::Error> {
        if id == self.root {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot delete root",
            ));
        }

        let entry = &self.entries[id];
        println!("deleting {:?}", &entry.path);
        match entry.kind {
            EntryKind::Folder => {
                let _ = fs::remove_dir_all(&entry.path)?;
            }
            EntryKind::File => {
                let _ = fs::remove_file(&entry.path)?;
            }
        }

        assert!(self.entries[id].parent.is_some());
        let parent_id = self.entries[id].parent.unwrap();
        self.entries[parent_id].children.retain(|cid| *cid != id);

        self._delete_recursive(id);

        // since the EntryId will be removed from the SlotMap, the access in the view function is
        // invalid; we need to find a valid EntryId
        // because selected is an EntryId and not an index, we need to lookup its position in visible and then take
        // the EntryId from the previous idx
        let idx = self
            .visible
            .iter()
            .position(|i| i.id == self.selected)
            .unwrap_or(1);
        self.selected = self.visible[idx - 1].id;

        self.visible = self.visible_entries();

        Ok(id)
    }

    fn _delete_recursive(&mut self, id: EntryId) {
        let children = self.entries[id].children.clone();
        for c in children {
            self._delete_recursive(c);
        }
        self.entries.remove(id);
    }

    pub fn batch_delete(&mut self) -> Result<(), io::Error> {
        let mut parents = HashSet::new();
        for id in self.temp.clone() {
            if id == self.root {
                return Ok(());
            }

            let entry = &self.entries[id];
            let r = match entry.kind {
                EntryKind::Folder => fs::remove_dir_all(&entry.path),
                EntryKind::File => fs::remove_file(&entry.path),
            };
            if let Err(e) = r {
                if e.kind() == io::ErrorKind::NotFound {
                    continue;
                } else {
                    return Err(e);
                }
            }

            assert!(self.entries[id].parent.is_some());
            let parent_id = self.entries[id].parent.unwrap();

            self.entries[parent_id].children.retain(|cid| *cid != id);
            parents.insert(parent_id);

            self._delete_recursive(id);
        }

        // when finalizing batch delete, the selected entry may be an entry that will be deleted,
        // so we need use the EntryId of the position of the earliest marked entry - 1
        // however, looking this up is stupid
        let m_id = self
            .temp
            .iter()
            .min_by(|a, b| {
                let a_pos = self.visible.iter().position(|i| i.id == **a);
                let b_pos = self.visible.iter().position(|i| i.id == **b);
                a_pos.cmp(&b_pos)
            })
            .expect("empty visible");
        let idx = self.visible.iter().position(|i| i.id == *m_id).unwrap_or(1);
        self.selected = self.visible[idx - 1].id;

        self.temp.clear();
        self.visible = self.visible_entries();

        Ok(())
    }

    pub fn batch_move(&mut self) -> Result<(), io::Error> {
        let new_parent = self.selected;

        let ids = &self.temp.clone();

        if !self.entries.contains_key(new_parent) {
            return Ok(());
        }

        for &id in ids {
            if id == self.root {
                continue;
            }

            let filename = self.entries[id].path.file_name().expect("invalid filename");
            let newpath = self.entries[new_parent].path.join(filename);

            if let Err(e) = fs::rename(&self.entries[id].path, &newpath) {
                return Err(e);
            }

            if let Some(old_parent) = self.entries[id].parent {
                self.entries[old_parent].children.retain(|cid| *cid != id);

                // need to revisit the old parent
                self.entries[old_parent].visited = false;

                // we need to explicitly set expanded to false because the children have changed,
                // and if we were to only clear the children, the entry is still in an expanded
                // state
                // so visually it would have no children but still be in the expanded state
                // thus pressing enter would collapse it but have no visual effect
                // therefore we do this to prevent needing to "double enter"
                self.entries[old_parent].expanded = false;
                self.entries[old_parent].children.clear();
            }

            self.entries[id].path = newpath;
            self.entries[id].marked = false;
            self.entries[id].parent = Some(new_parent);
            self.entries[new_parent].visited = false;
            self.entries[new_parent].expanded = false;
            self.entries[new_parent].children.clear();
        }

        self.sort_children(new_parent);

        self.temp.clear();
        self.visible = self.visible_entries();

        Ok(())
    }

    fn insert_sorted_child(&mut self, parent: EntryId, child: EntryId) {
        let pos = self.entries[parent]
            .children
            .binary_search_by(|&cid| self.cmp_entries(cid, child))
            .unwrap_or_else(|e| e);
        self.entries[parent].children.insert(pos, child);
    }

    fn sort_children(&mut self, parent_id: EntryId) {
        let mut ids = self.entries[parent_id]
            .children
            .iter()
            .copied()
            .collect::<Vec<EntryId>>();
        ids.sort_by(|&a, &b| self.cmp_entries(a, b));
        self.entries[parent_id].children = ids;
    }

    fn collect_visible(&self, id: EntryId, out: &mut Vec<VisibleEntry>) {
        out.push(VisibleEntry {
            id,
            depth: self.depth(id),
        });
        let entry = &self.entries[id];
        if entry.kind == EntryKind::Folder && entry.expanded {
            for &child_id in &entry.children {
                self.collect_visible(child_id, out);
            }
        }
    }

    pub fn mark(&mut self) {
        let id = self.selected;
        self.entries[id].marked = !self.entries[id].marked;
        self.temp.push(id);
    }

    pub fn clear_marked(&mut self) {
        if !self.temp.is_empty() {
            for &i in &self.temp {
                self.entries[i].marked = false;
            }

            self.temp.clear();
        }
    }

    pub fn cd_parent(&mut self) {
        if let Ok(dir) = std::env::current_dir() {
            if let Some(parent) = dir.parent() {
                std::env::set_current_dir(parent).expect("failed to cd into parent");
                *self = Self::new(parent);
            }
        }
    }

    pub fn cd_selected(&mut self) {
        let id = self.selected;
        if self.entries[id].kind == EntryKind::Folder {
            std::env::set_current_dir(&self.entries[id].path).expect("failed to cd");
            *self = Self::new(&self.entries[id].path);
        }
    }

    pub fn select_start(&mut self) {
        assert!(self.visible.len() > 0);
        self.selected = self.visible[0].id;
    }

    pub fn select_end(&mut self) {
        assert!(self.visible.len() > 0);
        self.selected = self.visible[self.visible.len() - 1].id;
    }

    pub fn move_up(&mut self) {
        let current_index = self.visible.iter().position(|ve| ve.id == self.selected);

        if let Some(i) = current_index {
            if i > 0 {
                self.selected = self.visible[i - 1].id;
                self.view_offset = self.view_offset.saturating_sub(1);
            }
        }
    }

    pub fn move_down(&mut self) {
        let current_index = self.visible.iter().position(|ve| ve.id == self.selected);

        if let Some(i) = current_index {
            if i + 1 < self.visible.len() {
                self.selected = self.visible[i + 1].id;
                if i + 1 >= self.view_offset + MAX_VISIBLE {
                    self.view_offset += 1;
                }
            }
        } else if !self.visible.is_empty() {
            self.selected = self.visible[0].id;
        }
    }

    pub fn depth(&self, id: EntryId) -> usize {
        let mut depth = 0;
        let mut curr = id;

        while let Some(parent) = self.entries[curr].parent {
            depth += 1;
            curr = parent;
        }
        depth
    }

    fn cmp_entries(&self, a: EntryId, b: EntryId) -> Ordering {
        let a = &self.entries[a];
        let b = &self.entries[b];

        match (&a.kind, &b.kind) {
            (EntryKind::Folder, EntryKind::File) => Ordering::Less,
            (EntryKind::File, EntryKind::Folder) => Ordering::Greater,
            _ => a.path.cmp(&b.path),
        }
    }
}

#[allow(dead_code)]
#[cfg(test)]
mod test {
    use crate::PROJECT_DIRS;

    use super::*;

    fn setup() {
        std::env::set_current_dir(PROJECT_DIRS.data_dir()).unwrap();
    }

    fn find_expect<'a>(tree: &'a FileTree, s: &'static str) -> (EntryId, &'a Entry) {
        tree.entries
            .iter()
            .find(|(_i, e)| e.path.ends_with(s))
            .expect("not found")
    }

    #[test]
    fn v4_t_delete() {
        setup();
        let _ = fs::File::create_new("temp/apple");

        let mut tree = FileTree::new(PROJECT_DIRS.data_dir());

        let (i, _) = tree
            .entries
            .iter()
            .find(|(_i, e)| e.path.ends_with("temp"))
            .expect("not found");

        tree.selected = i;
        tree.enter();

        // println!("before delete: {:#?}\n\n", &tree.entries);
        // println!("before visible: {:#?}\n\n", &tree.visible);

        let entry = &tree.entries[i];
        let j = entry
            .children
            .iter()
            .find(|&&i| tree.entries[i].path.ends_with("apple"))
            .expect("apple not found");

        let deleted_id = tree.delete(*j).expect("delete");

        assert!(!tree.entries.contains_key(deleted_id));

        // println!("after delete: {:#?}\n\n", &tree.entries);
        // println!("after visible: {:#?}\n\n", &tree.visible);
    }
}
