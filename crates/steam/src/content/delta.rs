//! What an update has to do, worked out from the manifest of the installed build and the new one.
//!
//! The installer (`install.rs`) applies an update in place: unchanged files are left alone, a changed file
//! keeps every chunk whose bytes are already at the right place (checked by SHA-1 on disk), and only the
//! other chunks are fetched. This module computes the same plan without touching the disk, for the
//! "download size" shown before an update and for tests.

use super::manifest::{FileEntry, Manifest};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeltaPlan {
    /// Files with the same size and content hash in both builds.
    pub unchanged_files: usize,
    /// Files in both builds whose content differs.
    pub changed_files: usize,
    /// Files only the new build has.
    pub new_files: usize,
    /// Paths only the old build has: deleted after the update. User files not named in any manifest stay.
    pub removed: Vec<String>,
    pub chunks_to_fetch: usize,
    /// Uncompressed bytes of those chunks.
    pub bytes_to_fetch: u64,
    pub chunks_reused: usize,
    pub bytes_reused: u64,
}

fn is_data(f: &FileEntry) -> bool {
    !f.is_dir() && f.link.is_none()
}

/// Compare the installed manifest (`None` = unknown: everything is checked on disk) with the new one.
/// Without an old manifest every chunk counts as "to fetch", the worst case.
pub fn plan(old: Option<&Manifest>, new: &Manifest) -> DeltaPlan {
    let old_files: HashMap<&str, &FileEntry> =
        old.into_iter().flat_map(|m| &m.files).filter(|f| is_data(f)).map(|f| (f.path.as_str(), f)).collect();
    let mut p = DeltaPlan::default();
    let mut seen: HashSet<&str> = HashSet::new();
    for file in new.files.iter().filter(|f| is_data(f)) {
        seen.insert(file.path.as_str());
        match old_files.get(file.path.as_str()) {
            None => {
                p.new_files += 1;
                fetch_all(&mut p, file);
            }
            Some(o) if o.size == file.size && o.sha_content == file.sha_content => {
                p.unchanged_files += 1;
                for c in &file.chunks {
                    p.chunks_reused += 1;
                    p.bytes_reused += u64::from(c.size);
                }
            }
            Some(o) => {
                p.changed_files += 1;
                for c in &file.chunks {
                    let same_place =
                        o.chunks.iter().any(|x| x.sha == c.sha && x.offset == c.offset && x.size == c.size);
                    if same_place {
                        p.chunks_reused += 1;
                        p.bytes_reused += u64::from(c.size);
                    } else {
                        p.chunks_to_fetch += 1;
                        p.bytes_to_fetch += u64::from(c.size);
                    }
                }
            }
        }
    }
    let mut removed: Vec<String> =
        old_files.keys().filter(|path| !seen.contains(**path)).map(|path| path.to_string()).collect();
    removed.sort();
    p.removed = removed;
    p
}

fn fetch_all(p: &mut DeltaPlan, file: &FileEntry) {
    for c in &file.chunks {
        p.chunks_to_fetch += 1;
        p.bytes_to_fetch += u64::from(c.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::manifest::Chunk;

    fn chunk(tag: u8, offset: u64, size: u32) -> Chunk {
        Chunk { sha: [tag; 20], crc: 0, offset, size, compressed_size: size }
    }

    fn file(path: &str, content: u8, chunks: Vec<Chunk>) -> FileEntry {
        FileEntry {
            path: path.into(),
            size: chunks.iter().map(|c| u64::from(c.size)).sum(),
            flags: 0,
            sha_content: vec![content; 20],
            chunks,
            link: None,
        }
    }

    fn manifest(gid: u64, files: Vec<FileEntry>) -> Manifest {
        Manifest { depot: 1, gid, created: 0, files, total_size: 0, raw: Vec::new() }
    }

    #[test]
    fn unchanged_changed_new_and_removed() {
        let old = manifest(
            1,
            vec![
                file("same.bin", 1, vec![chunk(1, 0, 100)]),
                file("patched.bin", 2, vec![chunk(2, 0, 100), chunk(3, 100, 100), chunk(4, 200, 50)]),
                file("gone.txt", 3, vec![chunk(5, 0, 10)]),
            ],
        );
        let new = manifest(
            2,
            vec![
                file("same.bin", 1, vec![chunk(1, 0, 100)]),
                // First chunk kept, second replaced, third moved to another offset (so it must be fetched).
                file("patched.bin", 9, vec![chunk(2, 0, 100), chunk(7, 100, 100), chunk(4, 220, 50)]),
                file("fresh.bin", 4, vec![chunk(8, 0, 30), chunk(9, 30, 30)]),
            ],
        );
        let p = plan(Some(&old), &new);
        assert_eq!((p.unchanged_files, p.changed_files, p.new_files), (1, 1, 1));
        assert_eq!(p.removed, ["gone.txt"]);
        assert_eq!(p.chunks_reused, 2); // same.bin's chunk and patched.bin's first
        assert_eq!(p.bytes_reused, 200);
        assert_eq!(p.chunks_to_fetch, 4); // two in patched.bin, two in fresh.bin
        assert_eq!(p.bytes_to_fetch, 100 + 50 + 60);
    }

    #[test]
    fn without_an_old_manifest_everything_is_fetched() {
        let new = manifest(2, vec![file("a", 1, vec![chunk(1, 0, 10), chunk(2, 10, 10)])]);
        let p = plan(None, &new);
        assert_eq!((p.new_files, p.chunks_to_fetch, p.bytes_to_fetch), (1, 2, 20));
        assert!(p.removed.is_empty());
    }

    #[test]
    fn same_manifest_is_a_no_op() {
        let m = manifest(5, vec![file("a", 1, vec![chunk(1, 0, 10)])]);
        let p = plan(Some(&m), &m);
        assert_eq!((p.unchanged_files, p.chunks_to_fetch, p.bytes_to_fetch), (1, 0, 0));
    }
}
