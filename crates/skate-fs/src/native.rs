//! `std::fs`, unchanged. The `Path` helpers are the methods the engine called before.
use std::path::Path;

pub use std::env::temp_dir;

pub use std::fs::{
    DirEntry, File, FileType, Metadata, OpenOptions, ReadDir, canonicalize, copy, create_dir, create_dir_all,
    metadata, read, read_dir, read_to_string, remove_dir, remove_dir_all, remove_file, rename, write,
};

/// [`Path::exists`].
pub fn exists(path: impl AsRef<Path>) -> bool {
    path.as_ref().exists()
}

/// [`Path::is_file`].
pub fn is_file(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_file()
}

/// [`Path::is_dir`].
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    path.as_ref().is_dir()
}
