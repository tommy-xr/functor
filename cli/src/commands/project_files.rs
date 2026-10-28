//! Project files as `File.bytes` sees them on this machine: the entry's
//! folder, except `external` prefixes. `functor.json` declares those
//! (`"external": {"ss2/": "player SS2 data"}`); they're never pushed or
//! bundled. The uncommitted `functor.local.json` beside it maps each to a
//! folder or a zip entry root (`{"external": {"ss2/":
//! "~/ss2-25th/sshock2.kpf!data/res/"}}`). Game code only ever says
//! `File.bytes("ss2/mesh/GRUNT_P.BIN")`.
//!
//! Folder mappings are trusted like the project folder: symlinks may not
//! resolve outside them, but a folder changed mid-read isn't guarded against.

use std::collections::HashMap;
use std::io::{Error, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use functor_runtime_common::debug_protocol::validate_project_asset_path;
use functor_runtime_common::storage::{self, ProjectReadError, ProjectReader};

/// The largest zip entry read, unpacked (a lying header can't exceed it either).
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

pub const LOCAL_CONFIG: &str = "functor.local.json";

/// `File.bytes` paths resolved on the host (see the module doc).
pub struct Resolver {
    /// The entry's folder: `File.bytes` paths are relative to it.
    root: PathBuf,
    externals: Vec<External>,
}

struct External {
    /// `"ss2/"`: matched against `File.bytes` paths, case-sensitively.
    prefix: String,
    description: String,
    mount: Option<Mount>,
}

enum Mount {
    Dir(PathBuf),
    Zip(ZipMount),
}

/// `functor.json`'s `external` declarations, checked: `[("ss2/", "what it
/// holds")]`. Prefixes are folder paths ending in `/`, none inside another.
pub fn declared(json: &serde_json::Value) -> Result<Vec<(String, String)>, String> {
    let declared = match json.get("external") {
        None => return Ok(Vec::new()),
        Some(serde_json::Value::Object(map)) => map,
        Some(_) => return Err(EXTERNAL_SHAPE.replace("{file}", "functor.json")),
    };
    let mut out = Vec::new();
    for (prefix, description) in declared {
        let description = description.as_str().ok_or_else(|| {
            format!(
                "functor.json external {prefix:?} must describe what it holds, e.g. \
\"player SS2 data\""
            )
        })?;
        let valid = prefix
            .strip_suffix('/')
            .is_some_and(|dir| validate_project_asset_path(dir).is_ok());
        if !valid {
            return Err(format!(
                "functor.json external {prefix:?} must be a folder path ending in `/`, like \
\"ss2/\""
            ));
        }
        if let Some(other) = declared
            .keys()
            .find(|other| *other != prefix && other.starts_with(prefix.as_str()))
        {
            return Err(format!(
                "functor.json external {prefix:?} contains {other:?}; declare one or the other"
            ));
        }
        out.push((prefix.clone(), description.to_string()));
    }
    Ok(out)
}

/// [`declared`] in `project_dir`'s `functor.json` (none without one).
pub fn declared_in(project_dir: &Path) -> Result<Vec<(String, String)>, Error> {
    match read_json(&project_dir.join("functor.json"))? {
        Some(json) => declared(&json).map_err(Error::other),
        None => Ok(Vec::new()),
    }
}

impl Resolver {
    /// The `declared` prefixes (see [`declared`]) with this machine's
    /// mappings (`functor.local.json` in `project_dir`). `entry_dir` is the
    /// entry file's folder.
    pub fn load(
        project_dir: &Path,
        entry_dir: &Path,
        declared: &[(String, String)],
    ) -> Result<Resolver, Error> {
        let bad = |message: String| Error::other(message);
        let project_dir = std::path::absolute(project_dir)?;
        let mapped = match read_json(&project_dir.join(LOCAL_CONFIG))? {
            Some(json) => match json.get("external") {
                None => serde_json::Map::new(),
                Some(serde_json::Value::Object(map)) => map.clone(),
                Some(_) => return Err(bad(EXTERNAL_SHAPE.replace("{file}", LOCAL_CONFIG))),
            },
            None => serde_json::Map::new(),
        };
        if let Some(prefix) = mapped
            .keys()
            .find(|prefix| !declared.iter().any(|(declared, _)| declared == *prefix))
        {
            return Err(bad(format!(
                "{LOCAL_CONFIG} maps {prefix:?}, but functor.json doesn't declare it: add \
\"external\": {{{prefix:?}: \"what it holds\"}} to functor.json"
            )));
        }
        let mut externals = Vec::new();
        for (prefix, description) in declared {
            let mount = match mapped.get(prefix) {
                None => None,
                Some(serde_json::Value::String(target)) => Some(
                    Mount::parse(&project_dir, target)
                        .map_err(|e| bad(format!("{LOCAL_CONFIG} external {prefix:?}: {e}")))?,
                ),
                Some(_) => {
                    return Err(bad(format!(
                        "{LOCAL_CONFIG} external {prefix:?} must be a path string, like \
\"~/data/\" or \"~/game.zip!data/\""
                    )))
                }
            };
            externals.push(External {
                prefix: prefix.clone(),
                description: description.clone(),
                mount,
            });
        }
        Ok(Resolver {
            root: std::path::absolute(entry_dir)?,
            externals,
        })
    }

    fn external<'a>(&self, path: &'a str) -> Option<(&External, &'a str)> {
        self.externals.iter().find_map(|external| {
            path.strip_prefix(external.prefix.as_str())
                .map(|rest| (external, rest))
        })
    }

    /// A prefix `path` matches only ignoring case (`SS2/` for `ss2/`): on a
    /// case-insensitive disk it would quietly read the project folder.
    fn miscased(&self, path: &str) -> Option<&External> {
        self.externals.iter().find(|external| {
            path.get(..external.prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(&external.prefix))
        })
    }
}

const EXTERNAL_SHAPE: &str = "{file} `external` must be an object of folder prefixes, like \
{\"ss2/\": …}";

fn read_json(path: &Path) -> Result<Option<serde_json::Value>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::other(format!("{}: {e}", path.display()))),
    }
}

impl External {
    fn unmapped(&self) -> String {
        format!(
            "{prefix} is external ({description}) and not mapped on this machine — map it in \
{LOCAL_CONFIG} beside functor.json: {{\"external\": {{\"{prefix}\": \"path/to/folder/\"}}}}, or \
\"path/to/archive.zip!folder/\" for a zip",
            prefix = self.prefix,
            description = self.description,
        )
    }
}

impl ProjectReader for Resolver {
    fn read(&self, path: &str) -> Result<Arc<[u8]>, ProjectReadError> {
        match self.external(path) {
            None => match self.miscased(path) {
                Some(external) => Err(ProjectReadError::Unreadable(format!(
                    "external prefixes are case-sensitive: {}?",
                    external.prefix
                ))),
                None => storage::read_confined(&self.root, path),
            },
            Some((external, rest)) => match &external.mount {
                None => Err(ProjectReadError::Unreadable(external.unmapped())),
                Some(Mount::Dir(dir)) => storage::read_confined(dir, rest),
                Some(Mount::Zip(zip)) => zip
                    .read(rest, &external.prefix)
                    .map_err(ProjectReadError::Unreadable),
            },
        }
    }

    fn exists(&self, path: &str) -> bool {
        match self.external(path) {
            None => self.miscased(path).is_none() && storage::is_file_confined(&self.root, path),
            Some((external, rest)) => match &external.mount {
                None => false,
                Some(Mount::Dir(dir)) => storage::is_file_confined(dir, rest),
                Some(Mount::Zip(zip)) => zip.exists(rest),
            },
        }
    }
}

impl Mount {
    /// `"~/data/"` (a folder) or `"~/game.zip!data/res/"` (entries under
    /// `data/res/` in a zip; `"~/game.zip"` alone is its root). Relative
    /// paths start at the project folder.
    fn parse(project_dir: &Path, target: &str) -> Result<Mount, String> {
        let resolve = |path: &str| match path.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(rest))
                .ok_or("`~` needs $HOME"),
            None => Ok(project_dir.join(path)),
        };
        // `!` separates an archive from its folder only after a file, so a
        // folder named `Games!` still maps as a folder.
        let split = target
            .rsplit_once('!')
            .map(|(archive, inner)| resolve(archive).map(|archive| (archive, inner)))
            .transpose()?
            .filter(|(archive, _)| archive.is_file());
        let (path, inner) = match split {
            Some((archive, inner)) => (archive, inner),
            None => {
                let path = resolve(target)?;
                if path.is_dir() {
                    return Ok(Mount::Dir(path));
                }
                (path, "")
            }
        };
        let inner = inner.trim_end_matches('/');
        if !inner.is_empty() && validate_project_asset_path(inner).is_err() {
            return Err(format!(
                "the folder after `!` must be a plain relative path like data/res/ (got {inner:?})"
            ));
        }
        Ok(Mount::Zip(ZipMount {
            archive: path,
            inner: if inner.is_empty() {
                String::new()
            } else {
                format!("{inner}/")
            },
            max_entry: MAX_ENTRY_BYTES,
            index: Mutex::new(None),
        }))
    }
}

/// Entries under `inner` in a zip, read in place (never extracted). Names
/// are case-sensitive; traversal, absolute and backslash names and symlinks
/// are refused, and an archive with duplicate names isn't read at all.
struct ZipMount {
    archive: PathBuf,
    /// `""` or `"data/res/"`.
    inner: String,
    max_entry: u64,
    /// Rebuilt when the archive's size or mtime changes.
    index: Mutex<Option<ZipIndex>>,
}

struct ZipIndex {
    stamp: (u64, Option<std::time::SystemTime>),
    zip: zip::ZipArchive<std::fs::File>,
    entries: HashMap<String, Entry>,
    /// Lowercased name → a real one, for the case-mismatch hint.
    folded: HashMap<String, String>,
}

enum Entry {
    File { index: usize, size: u64 },
    Symlink,
}

impl ZipMount {
    fn with_index<T>(
        &self,
        f: impl FnOnce(&mut ZipIndex) -> Result<T, String>,
    ) -> Result<T, String> {
        let unreadable = |e: &dyn std::fmt::Display| format!("{}: {e}", self.archive.display());
        let meta = std::fs::metadata(&self.archive).map_err(|e| unreadable(&e))?;
        let stamp = (meta.len(), meta.modified().ok());
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.as_ref().is_none_or(|index| index.stamp != stamp) {
            *index = None;
            let file = std::fs::File::open(&self.archive).map_err(|e| unreadable(&e))?;
            *index = Some(ZipIndex::build(stamp, file).map_err(|e| unreadable(&e))?);
        }
        f(index.as_mut().expect("just built"))
    }

    fn exists(&self, rest: &str) -> bool {
        let name = format!("{}{rest}", self.inner);
        self.with_index(|index| Ok(matches!(index.entries.get(&name), Some(Entry::File { .. }))))
            .unwrap_or(false)
    }

    /// The entry for `File.bytes(prefix + rest)`.
    fn read(&self, rest: &str, prefix: &str) -> Result<Arc<[u8]>, String> {
        let name = format!("{}{rest}", self.inner);
        let archive = self.archive.display();
        self.with_index(|index| match index.entries.get(&name) {
            Some(Entry::File { index: at, size }) => {
                let (at, size) = (*at, *size);
                if size > self.max_entry {
                    return Err(too_big(&name, size, self.max_entry));
                }
                let mut entry = index
                    .zip
                    .by_index(at)
                    .map_err(|e| format!("{archive}: {e}"))?;
                let mut bytes = Vec::with_capacity(size as usize);
                // `take` caps a header that understates the size.
                Read::by_ref(&mut entry)
                    .take(self.max_entry + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("{archive} {name}: {e}"))?;
                if bytes.len() as u64 > self.max_entry {
                    return Err(too_big(&name, bytes.len() as u64, self.max_entry));
                }
                Ok(bytes.into())
            }
            Some(Entry::Symlink) => Err(format!("{archive} entry {name} is a symlink; not read")),
            None => {
                let hint = index
                    .folded
                    .get(&name.to_lowercase())
                    .map(|real| match real.strip_prefix(self.inner.as_str()) {
                        Some(rest) => format!(" — names are case-sensitive: \"{prefix}{rest}\"?"),
                        None => format!(" — names are case-sensitive: entry {real}?"),
                    })
                    .unwrap_or_default();
                Err(format!("no entry {name} in {archive}{hint}"))
            }
        })
    }
}

fn too_big(name: &str, size: u64, max: u64) -> String {
    format!(
        "zip entry {name} unpacks to {:.1} MB, over the {:.0} MB limit",
        size as f64 / 1048576.0,
        max as f64 / 1048576.0
    )
}

impl ZipIndex {
    fn build(
        stamp: (u64, Option<std::time::SystemTime>),
        mut file: std::fs::File,
    ) -> Result<ZipIndex, String> {
        let records = central_directory_records(&mut file).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        // The zip reader keeps one entry per (decoded) name: fewer entries
        // than records means two entries share a name, so which one a path
        // reads would be arbitrary.
        if zip.len() != records {
            return Err(format!(
                "it lists {records} entries under {} distinct names; duplicate names are \
ambiguous, so it isn't read",
                zip.len()
            ));
        }
        let mut entries = HashMap::new();
        let mut folded = HashMap::new();
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
            let name = entry.name().to_string();
            // Unaskable names (absolute, `..`, `\`) stay unreachable.
            if entry.is_dir() || validate_project_asset_path(&name).is_err() {
                continue;
            }
            let state = if entry.is_symlink() {
                Entry::Symlink
            } else {
                Entry::File {
                    index,
                    size: entry.size(),
                }
            };
            folded.entry(name.to_lowercase()).or_insert(name.clone());
            entries.insert(name, state);
        }
        Ok(ZipIndex {
            stamp,
            zip,
            entries,
            folded,
        })
    }
}

/// How many entry records the central directory holds (duplicates included).
fn central_directory_records(file: &mut std::fs::File) -> std::io::Result<usize> {
    let bad = || Error::other("not a zip archive (no end of central directory)");
    let len = file.seek(SeekFrom::End(0))?;
    // The end record (22 bytes) plus at most a 64 KiB comment.
    let tail_len = len.min(22 + 65535);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0; tail_len as usize];
    file.read_exact(&mut tail)?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&at| tail[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(bad)?;
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]) as usize;
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let (size, offset) = (u32_at(&tail, end + 12), u32_at(&tail, end + 16));
    if size == u32::MAX || offset == u32::MAX || u16_at(&tail, end + 10) == 0xffff {
        return Err(Error::other("zip64 archives aren't supported"));
    }
    // A prefixed archive (e.g. a self-extractor) shifts every offset.
    let start = (len - tail_len + end as u64)
        .checked_sub(size as u64)
        .ok_or_else(bad)?;
    file.seek(SeekFrom::Start(start))?;
    let mut directory = vec![0; size as usize];
    file.read_exact(&mut directory)?;
    let mut records = 0;
    let mut at = 0;
    while at + 46 <= directory.len() && directory[at..at + 4] == [0x50, 0x4b, 0x01, 0x02] {
        records += 1;
        at += 46
            + u16_at(&directory, at + 28)
            + u16_at(&directory, at + 30)
            + u16_at(&directory, at + 32);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A throwaway project folder, removed on drop.
    struct TempProject(PathBuf);

    impl TempProject {
        fn new(name: &str) -> TempProject {
            let dir =
                std::env::temp_dir().join(format!("functor-files-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempProject(dir)
        }
        fn write(&self, rel: &str, bytes: &[u8]) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        fn config(&self, declared: &str, mapped: Option<&str>) {
            self.write(
                "functor.json",
                format!("{{\"language\": \"functor-lang\", \"external\": {declared}}}").as_bytes(),
            );
            match mapped {
                Some(mapped) => self.write(
                    LOCAL_CONFIG,
                    format!("{{\"external\": {mapped}}}").as_bytes(),
                ),
                None => {
                    let _ = std::fs::remove_file(self.0.join(LOCAL_CONFIG));
                }
            }
        }
        /// The resolver `build()` installs, or the config error.
        fn load(&self) -> Result<Resolver, String> {
            declared_in(&self.0)
                .and_then(|declared| Resolver::load(&self.0, &self.0, &declared))
                .map_err(|e| e.to_string())
        }
        fn resolver(&self) -> Resolver {
            self.load().unwrap()
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A zip at `path` with stored entries; `None` contents = a symlink entry.
    fn write_zip(path: &Path, entries: &[(&str, Option<&[u8]>)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in entries {
            match contents {
                Some(bytes) => {
                    zip.start_file(*name, stored).unwrap();
                    zip.write_all(bytes).unwrap();
                }
                None => zip.add_symlink(*name, "../../outside.bin", stored).unwrap(),
            }
        }
        zip.finish().unwrap();
    }

    fn read(resolver: &Resolver, path: &str) -> Result<Vec<u8>, String> {
        match resolver.read(path) {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(ProjectReadError::Outside) => Err("outside".to_string()),
            Err(ProjectReadError::Unreadable(e)) => Err(e),
        }
    }

    #[test]
    fn project_paths_read_the_entry_folder() {
        let project = TempProject::new("plain");
        project.write("data/a.bin", &[1, 2]);
        let resolver = project.resolver();
        assert_eq!(read(&resolver, "data/a.bin"), Ok(vec![1, 2]));
        assert!(resolver.exists("data/a.bin") && !resolver.exists("data/b.bin"));
    }

    /// An external prefix mapped to a folder reads from it, confined:
    /// symlinks out of the mapped folder don't resolve.
    #[cfg(unix)]
    #[test]
    fn external_folder_mapping_reads_from_the_folder_only() {
        let project = TempProject::new("dir");
        let data = TempProject::new("dir-data");
        data.write("mesh/a.bin", &[7]);
        data.write("../functor-files-secret.bin", &[9]);
        std::os::unix::fs::symlink(
            data.0.parent().unwrap().join("functor-files-secret.bin"),
            data.0.join("mesh/out.bin"),
        )
        .unwrap();
        // The project's own ss2/ is shadowed by the mapping.
        project.write("ss2/mesh/a.bin", &[1]);
        project.config(
            r#"{"ss2/": "game data"}"#,
            Some(&format!("{{\"ss2/\": {:?}}}", data.0.display().to_string())),
        );
        let resolver = project.resolver();
        assert_eq!(read(&resolver, "ss2/mesh/a.bin"), Ok(vec![7]));
        assert_eq!(
            read(&resolver, "ss2/mesh/out.bin"),
            Err("outside".to_string())
        );
        assert!(resolver.exists("ss2/mesh/a.bin") && !resolver.exists("ss2/mesh/out.bin"));
        // A miscased prefix doesn't fall through to the project's own ss2/.
        let err = read(&resolver, "SS2/mesh/a.bin").unwrap_err();
        assert!(err.contains("case-sensitive: ss2/?"), "{err}");
        assert!(!resolver.exists("SS2/mesh/a.bin"));
        let _ = std::fs::remove_file(data.0.parent().unwrap().join("functor-files-secret.bin"));
    }

    /// A `.kpf`-like archive: `ss2/` maps to entries under `data/res/`;
    /// names are case-sensitive, with a hint on a case mismatch.
    #[test]
    fn external_zip_mapping_reads_entries_in_place() {
        let project = TempProject::new("zip");
        write_zip(
            &project.0.join("game.kpf"),
            &[
                ("data/res/mesh/GRUNT_P.BIN", Some(b"LGMD")),
                ("data/res/motions/ogpstd_.mc", Some(&[1, 2, 3])),
                ("data/other.bin", Some(&[0])),
            ],
        );
        project.config(
            r#"{"ss2/": "SS2 data"}"#,
            Some(r#"{"ss2/": "game.kpf!data/res/"}"#),
        );
        let resolver = project.resolver();
        assert_eq!(
            read(&resolver, "ss2/mesh/GRUNT_P.BIN"),
            Ok(b"LGMD".to_vec())
        );
        assert_eq!(read(&resolver, "ss2/motions/ogpstd_.mc"), Ok(vec![1, 2, 3]));
        assert!(resolver.exists("ss2/mesh/GRUNT_P.BIN"));
        let err = read(&resolver, "ss2/mesh/grunt_p.bin").unwrap_err();
        assert!(
            err.contains("no entry data/res/mesh/grunt_p.bin")
                && err.contains("case-sensitive: \"ss2/mesh/GRUNT_P.BIN\"?"),
            "{err}"
        );
        assert!(!resolver.exists("ss2/mesh/grunt_p.bin"));
        // Only entries under the mapped folder.
        assert!(!resolver.exists("ss2/../other.bin") && !resolver.exists("ss2/other.bin"));
        // The whole archive, mapped without `!`.
        project.config(r#"{"ss2/": "SS2 data"}"#, Some(r#"{"ss2/": "game.kpf"}"#));
        let whole = project.resolver();
        assert_eq!(read(&whole, "ss2/data/other.bin"), Ok(vec![0]));
    }

    /// Entries that could escape are never served: traversal and absolute
    /// names (unaskable) and symlinks.
    #[test]
    fn unsafe_zip_entries_are_refused() {
        let project = TempProject::new("slip");
        write_zip(
            &project.0.join("game.zip"),
            &[
                ("../evil.bin", Some(b"x")),
                ("/abs.bin", Some(b"x")),
                ("res/../../up.bin", Some(b"x")),
                ("res/link.bin", None),
                ("res/ok.bin", Some(b"ok")),
            ],
        );
        project.config(r#"{"x/": "data"}"#, Some(r#"{"x/": "game.zip!res/"}"#));
        let resolver = project.resolver();
        assert_eq!(read(&resolver, "x/ok.bin"), Ok(b"ok".to_vec()));
        let err = read(&resolver, "x/link.bin").unwrap_err();
        assert!(err.contains("is a symlink"), "{err}");
        assert!(!resolver.exists("x/link.bin"));
        // No validated path names the escaping entries.
        for path in ["x/../evil.bin", "x//abs.bin", "x/../../up.bin"] {
            assert!(!resolver.exists(path), "{path}");
        }
    }

    /// Two entries under one name: which one a path reads would be arbitrary,
    /// so the archive isn't read at all.
    #[test]
    fn zips_with_duplicate_names_are_refused() {
        let project = TempProject::new("dup");
        let archive = project.0.join("game.zip");
        write_zip(
            &archive,
            &[
                ("res/dup-a.bin", Some(b"first")),
                ("res/dup-b.bin", Some(b"second")),
                ("res/ok.bin", Some(b"ok")),
            ],
        );
        // Rename the second entry onto the first (same length), in both the
        // local header and the central directory.
        let bytes = std::fs::read(&archive).unwrap();
        let (from, to) = (b"res/dup-b.bin", b"res/dup-a.bin");
        let mut patched = Vec::with_capacity(bytes.len());
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at..].starts_with(from) {
                patched.extend_from_slice(to);
                at += from.len();
            } else {
                patched.push(bytes[at]);
                at += 1;
            }
        }
        std::fs::write(&archive, patched).unwrap();
        project.config(r#"{"x/": "data"}"#, Some(r#"{"x/": "game.zip!res/"}"#));
        let resolver = project.resolver();
        for path in ["x/dup-a.bin", "x/ok.bin"] {
            let err = read(&resolver, path).unwrap_err();
            assert!(err.contains("3 entries under 2 distinct names"), "{err}");
            assert!(!resolver.exists(path));
        }
    }

    /// Mappings resolve against the project folder once, at load: a relative
    /// `-d` stays right after `run` changes directory. A `!` after a folder
    /// is part of its name.
    #[test]
    fn relative_mappings_survive_a_directory_change() {
        let project = TempProject::new("relative");
        project.write("data!/a.bin", &[3]);
        project.config(r#"{"x/": "data"}"#, Some(r#"{"x/": "data!/"}"#));
        // The same folder, relative to the working directory.
        let up = std::env::current_dir().unwrap().components().count() - 1;
        let relative = PathBuf::from("../".repeat(up)).join(project.0.strip_prefix("/").unwrap());
        assert!(relative.is_relative() && relative.is_dir());
        let declared = declared_in(&project.0).unwrap();
        let resolver = Resolver::load(&relative, &relative, &declared).unwrap();
        let Some(Mount::Dir(dir)) = &resolver.externals[0].mount else {
            panic!("a folder mount");
        };
        assert!(dir.is_absolute(), "{}", dir.display());
        assert_eq!(read(&resolver, "x/a.bin"), Ok(vec![3]));
    }

    /// An entry unpacking past the cap is refused, whatever its header says.
    #[test]
    fn zip_entries_are_capped_when_unpacked() {
        let project = TempProject::new("bomb");
        let archive = project.0.join("bomb.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file("big.bin", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&vec![0; 64 * 1024]).unwrap();
        zip.finish().unwrap();
        let Ok(Mount::Zip(mut mount)) = Mount::parse(&project.0, "bomb.zip") else {
            panic!("a zip mount");
        };
        mount.max_entry = 1024;
        let err = mount.read("big.bin", "x/").unwrap_err();
        assert!(err.contains("over the"), "{err}");
        mount.max_entry = 64 * 1024;
        assert_eq!(mount.read("big.bin", "x/").map(|b| b.len()), Ok(64 * 1024));
    }

    /// A declared but unmapped prefix teaches how to map it; `exists` is false.
    #[test]
    fn unmapped_external_prefixes_teach_the_mapping() {
        let project = TempProject::new("unmapped");
        project.write("ss2/mesh/a.bin", &[1]);
        project.config(r#"{"ss2/": "player SS2 data"}"#, None);
        let resolver = project.resolver();
        let err = read(&resolver, "ss2/mesh/a.bin").unwrap_err();
        assert!(
            err.contains("ss2/ is external (player SS2 data) and not mapped")
                && err.contains(LOCAL_CONFIG),
            "{err}"
        );
        assert!(!resolver.exists("ss2/mesh/a.bin"));
        // A mapping to a missing archive fails at read time, naming it.
        project.config(r#"{"ss2/": "data"}"#, Some(r#"{"ss2/": "nope.kpf!data/"}"#));
        let err = read(&project.resolver(), "ss2/a.bin").unwrap_err();
        assert!(err.contains("nope.kpf"), "{err}");
    }

    #[test]
    fn malformed_external_config_is_refused() {
        let project = TempProject::new("config");
        project.write("a.zip", b"");
        let refused = |declared: &str, mapped: Option<&str>| {
            project.config(declared, mapped);
            project.load().err().unwrap_or_default()
        };
        assert!(refused(r#"{"ss2": "x"}"#, None).contains("ending in `/`"));
        assert!(refused(r#"{"../x/": "x"}"#, None).contains("ending in `/`"));
        assert!(refused(r#"{"ss2/": 1}"#, None).contains("describe what it holds"));
        assert!(refused(r#"{"a/": "x", "a/b/": "y"}"#, None).contains("contains"));
        assert!(refused(r#"{}"#, Some(r#"{"ss2/": "x/"}"#)).contains("doesn't declare it"));
        assert!(
            refused(r#"{"ss2/": "x"}"#, Some(r#"{"ss2/": "a.zip!../up/"}"#))
                .contains("plain relative path")
        );
        assert!(refused(r#"["ss2/"]"#, None).contains("must be an object"));
    }
}
