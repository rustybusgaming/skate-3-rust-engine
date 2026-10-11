//! The browser build's file tree: the player's installation, listed by the loader page, plus
//! whatever the engine writes during the session. Paths are absolute and normalised lexically
//! (there is no current directory, and no links to resolve). Plain data, so it is tested natively.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A file's contents: bytes written this session, or a file in the player's installation, which
/// the page holds and reads on request.
#[derive(Clone, Debug)]
pub(crate) enum Content {
    Bytes(Arc<[u8]>),
    Installation { len: u64 },
}

impl Content {
    pub(crate) fn len(&self) -> u64 {
        match self {
            Content::Bytes(bytes) => bytes.len() as u64,
            Content::Installation { len } => *len,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Dir,
}

#[derive(Default)]
pub(crate) struct Tree {
    files: BTreeMap<PathBuf, Content>,
    dirs: BTreeSet<PathBuf>,
}

/// `path` made absolute and stripped of `.` and `..`. `..` above the root stays at the root, as
/// on Unix.
pub(crate) fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    out
}

pub(crate) fn not_found(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{}: no such file in the installation", path.display()))
}

impl Tree {
    pub(crate) fn kind(&self, path: &Path) -> Option<Kind> {
        let path = normalise(path);
        if self.files.contains_key(&path) {
            Some(Kind::File)
        } else if path.parent().is_none() || self.dirs.contains(&path) {
            Some(Kind::Dir)
        } else {
            None
        }
    }

    pub(crate) fn content(&self, path: &Path) -> io::Result<Content> {
        let path = normalise(path);
        match self.files.get(&path) {
            Some(content) => Ok(content.clone()),
            None if self.kind(&path) == Some(Kind::Dir) => {
                Err(io::Error::new(io::ErrorKind::IsADirectory, format!("{}: is a directory", path.display())))
            }
            None => Err(not_found(&path)),
        }
    }

    /// Adds `path` and its parent directories. Fails if a parent is a file or `path` is a directory.
    pub(crate) fn insert(&mut self, path: &Path, content: Content) -> io::Result<()> {
        let path = normalise(path);
        if self.kind(&path) == Some(Kind::Dir) {
            return Err(io::Error::new(io::ErrorKind::IsADirectory, format!("{}: is a directory", path.display())));
        }
        self.create_dirs(path.parent().ok_or_else(|| not_found(&path))?)?;
        self.files.insert(path, content);
        Ok(())
    }

    /// Like `std::fs::write`: the parent directory must exist.
    pub(crate) fn write(&mut self, path: &Path, bytes: Arc<[u8]>) -> io::Result<()> {
        let path = normalise(path);
        let parent = path.parent().ok_or_else(|| not_found(&path))?;
        if self.kind(parent) != Some(Kind::Dir) {
            return Err(not_found(parent));
        }
        self.insert(&path, Content::Bytes(bytes))
    }

    pub(crate) fn create_dirs(&mut self, path: &Path) -> io::Result<()> {
        let path = normalise(path);
        for dir in path.ancestors() {
            if self.files.contains_key(dir) {
                return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("{}: not a directory", dir.display())));
            }
        }
        for dir in path.ancestors().filter(|dir| dir.parent().is_some()) {
            self.dirs.insert(dir.to_path_buf());
        }
        Ok(())
    }

    pub(crate) fn create_dir(&mut self, path: &Path) -> io::Result<()> {
        let path = normalise(path);
        if self.kind(&path).is_some() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{}: already exists", path.display())));
        }
        let parent = path.parent().ok_or_else(|| not_found(&path))?;
        if self.kind(parent) != Some(Kind::Dir) {
            return Err(not_found(parent));
        }
        self.dirs.insert(path);
        Ok(())
    }

    /// Entries directly inside `path`, in name order.
    pub(crate) fn list(&self, path: &Path) -> io::Result<Vec<(PathBuf, Kind)>> {
        let path = normalise(path);
        match self.kind(&path) {
            Some(Kind::Dir) => {}
            Some(Kind::File) => {
                return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("{}: not a directory", path.display())));
            }
            None => return Err(not_found(&path)),
        }
        // Paths order by component, so everything under `path` is one contiguous range.
        let under = |p: &&PathBuf| p.starts_with(&path);
        let direct = |p: &&PathBuf| p.parent() == Some(path.as_path());
        let dirs = self.dirs.range(path.clone()..).take_while(under).filter(direct).map(|d| (d.clone(), Kind::Dir));
        let files = self.files.range(path.clone()..).map(|(f, _)| f).take_while(under).filter(direct).map(|f| (f.clone(), Kind::File));
        let mut entries: Vec<_> = dirs.chain(files).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    pub(crate) fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        let path = normalise(path);
        self.files.remove(&path).map(drop).ok_or_else(|| not_found(&path))
    }

    pub(crate) fn remove_dir(&mut self, path: &Path, recursive: bool) -> io::Result<()> {
        let path = normalise(path);
        if self.kind(&path) != Some(Kind::Dir) || path.parent().is_none() {
            return Err(not_found(&path));
        }
        if !recursive && !self.list(&path)?.is_empty() {
            return Err(io::Error::new(io::ErrorKind::DirectoryNotEmpty, format!("{}: directory not empty", path.display())));
        }
        self.files.retain(|file, _| !file.starts_with(&path));
        self.dirs.retain(|dir| !dir.starts_with(&path));
        Ok(())
    }

    pub(crate) fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (normalise(from), normalise(to));
        match self.kind(&from) {
            None => Err(not_found(&from)),
            Some(Kind::File) => {
                let content = self.content(&from)?;
                self.insert(&to, content)?;
                if from != to {
                    self.files.remove(&from);
                }
                Ok(())
            }
            Some(Kind::Dir) => {
                if to.starts_with(&from) {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "cannot move a directory into itself"));
                }
                self.create_dirs(&to)?;
                let moved = |p: &PathBuf| to.join(p.strip_prefix(&from).unwrap());
                let files: Vec<_> = self.files.iter().filter(|(f, _)| f.starts_with(&from)).map(|(f, c)| (f.clone(), c.clone())).collect();
                let dirs: Vec<_> = self.dirs.iter().filter(|d| d.starts_with(&from)).cloned().collect();
                for (file, content) in files {
                    self.files.remove(&file);
                    self.files.insert(moved(&file), content);
                }
                for dir in dirs {
                    self.dirs.remove(&dir);
                    self.dirs.insert(moved(&dir));
                }
                Ok(())
            }
        }
    }

    /// Files under `dir` at any depth, with their sizes, in path order.
    pub(crate) fn files_under(&self, dir: &Path) -> Vec<(PathBuf, u64)> {
        let dir = normalise(dir);
        self.files.range(dir.clone()..).take_while(|(f, _)| f.starts_with(&dir)).map(|(f, c)| (f.clone(), c.len())).collect()
    }

    /// `path` normalised, if it exists (the browser build's `canonicalize`).
    pub(crate) fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalise(path);
        self.kind(&path).map(|_| path.clone()).ok_or_else(|| not_found(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Content {
        Content::Bytes(Arc::from(text.as_bytes()))
    }

    #[test]
    fn paths_normalise_lexically() {
        assert_eq!(normalise(Path::new("a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normalise(Path::new("/../x")), Path::new("/x"));
        assert_eq!(normalise(Path::new("/")), Path::new("/"));
    }

    #[test]
    fn insert_creates_parents_and_lists_children_in_order() {
        let mut tree = Tree::default();
        tree.insert(Path::new("/i/assets/private/game.json"), Content::Installation { len: 7 }).unwrap();
        tree.insert(Path::new("/i/assets/a.bin"), bytes("a")).unwrap();
        tree.insert(Path::new("/i/maps/x.skate"), bytes("x")).unwrap();
        assert_eq!(tree.kind(Path::new("/i/assets/private")), Some(Kind::Dir));
        assert_eq!(tree.kind(Path::new("/i/assets/../assets/a.bin")), Some(Kind::File));
        assert_eq!(tree.kind(Path::new("/i/nothing")), None);
        assert_eq!(tree.kind(Path::new("/")), Some(Kind::Dir));
        assert_eq!(
            tree.list(Path::new("/i/assets")).unwrap(),
            vec![(PathBuf::from("/i/assets/a.bin"), Kind::File), (PathBuf::from("/i/assets/private"), Kind::Dir)]
        );
        assert_eq!(tree.list(Path::new("/i")).unwrap().len(), 2);
        assert_eq!(
            tree.files_under(Path::new("/i/assets")),
            vec![(PathBuf::from("/i/assets/a.bin"), 1), (PathBuf::from("/i/assets/private/game.json"), 7)]
        );
        assert_eq!(tree.content(Path::new("/i/assets/private/game.json")).unwrap().len(), 7);
        assert_eq!(tree.list(Path::new("/i/assets/a.bin")).unwrap_err().kind(), io::ErrorKind::NotADirectory);
        assert_eq!(tree.content(Path::new("/i/assets")).unwrap_err().kind(), io::ErrorKind::IsADirectory);
        assert_eq!(tree.content(Path::new("/i/none")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn listing_does_not_mix_in_sibling_prefixes() {
        let mut tree = Tree::default();
        tree.insert(Path::new("/a/b/one"), bytes("1")).unwrap();
        tree.insert(Path::new("/a/bc/two"), bytes("2")).unwrap();
        assert_eq!(tree.list(Path::new("/a/b")).unwrap(), vec![(PathBuf::from("/a/b/one"), Kind::File)]);
    }

    #[test]
    fn writes_need_a_parent_like_std() {
        let mut tree = Tree::default();
        assert_eq!(tree.write(Path::new("/s/settings.json"), Arc::from(&b"{}"[..])).unwrap_err().kind(), io::ErrorKind::NotFound);
        tree.create_dirs(Path::new("/s")).unwrap();
        tree.write(Path::new("/s/settings.json"), Arc::from(&b"{}"[..])).unwrap();
        assert_eq!(tree.create_dirs(Path::new("/s/settings.json/x")).unwrap_err().kind(), io::ErrorKind::NotADirectory);
        assert_eq!(tree.create_dir(Path::new("/s")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn rename_and_remove() {
        let mut tree = Tree::default();
        tree.insert(Path::new("/s/a.tmp"), bytes("new")).unwrap();
        tree.insert(Path::new("/s/a.json"), bytes("old")).unwrap();
        tree.rename(Path::new("/s/a.tmp"), Path::new("/s/a.json")).unwrap();
        assert!(matches!(tree.content(Path::new("/s/a.json")).unwrap(), Content::Bytes(b) if &*b == b"new"));
        assert_eq!(tree.kind(Path::new("/s/a.tmp")), None);
        tree.insert(Path::new("/d/x/y"), bytes("y")).unwrap();
        tree.rename(Path::new("/d"), Path::new("/e/f")).unwrap();
        assert_eq!(tree.kind(Path::new("/e/f/x/y")), Some(Kind::File));
        assert_eq!(tree.kind(Path::new("/d")), None);
        assert_eq!(tree.remove_dir(Path::new("/e"), false).unwrap_err().kind(), io::ErrorKind::DirectoryNotEmpty);
        tree.remove_dir(Path::new("/e"), true).unwrap();
        assert_eq!(tree.kind(Path::new("/e/f/x/y")), None);
        assert_eq!(tree.remove_file(Path::new("/s/a.tmp")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn canonical_paths_exist() {
        let mut tree = Tree::default();
        tree.insert(Path::new("/i/a"), Content::Installation { len: 1 }).unwrap();
        assert_eq!(tree.canonical(Path::new("/i/./a")).unwrap(), Path::new("/i/a"));
        assert!(tree.canonical(Path::new("/i/b")).is_err());
    }
}
