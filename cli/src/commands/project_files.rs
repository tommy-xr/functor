//! Project files as `File.bytes` sees them on this machine: the entry's
//! folder, except the prefixes `config` declares external
//! (`Config.external("ss2/", Source.mount("ss2"))`), which are never pushed or
//! bundled. Each machine maps a mount name to a folder or a folder inside a
//! zip, in `~/.config/functor/mounts.json` (`{"ss2":
//! "~/ss2-25th/sshock2.kpf!data/res/"}`) or with `--mount ss2=PATH`. Game code
//! only ever says `File.bytes("ss2/mesh/GRUNT_P.BIN")`.
//!
//! Folder mappings are trusted like the project folder: symlinks may not
//! resolve outside them, but a folder changed mid-read isn't guarded against.

use std::collections::HashMap;
use std::io::{Error, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use functor_runtime_common::debug_protocol::{
    validate_project_asset_path, ProjectFileReply, ProjectFileRequest, ProjectFileStatus,
    MAX_ASSET_BYTES,
};
use functor_runtime_common::runtime_settings::ExternalPrefix;
use functor_runtime_common::storage::{self, ProjectReadError, ProjectReader};

/// The largest zip entry read, unpacked (a lying header can't exceed it either).
const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;

/// `File.bytes` paths resolved on the host (see the module doc).
pub struct Resolver {
    /// The entry's folder: `File.bytes` paths are relative to it.
    root: PathBuf,
    externals: Vec<External>,
}

struct External {
    /// `"ss2/"`: matched against `File.bytes` paths, case-sensitively.
    prefix: String,
    /// The `Source.mount` name.
    name: String,
    mount: Option<Mount>,
}

enum Mount {
    Dir(PathBuf),
    Zip(ZipMount),
}

/// This machine's mounts, name → target (a folder, or `archive!folder/`):
/// `--mount name=PATH` over `~/.config/functor/mounts.json`.
#[derive(Default)]
pub struct Mounts {
    /// Target, and the folder a relative target starts at.
    targets: HashMap<String, (String, PathBuf)>,
}

/// `$XDG_CONFIG_HOME/functor/mounts.json` (when absolute), else
/// `~/.config/functor/mounts.json` (`%USERPROFILE%` on Windows).
pub fn mounts_file() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".config"))
        })?;
    Some(config.join("functor").join("mounts.json"))
}

impl Mounts {
    /// [`mounts_file`], then each `--mount name=PATH` (relative to the
    /// current folder) over it.
    pub fn load(overrides: &[String]) -> Result<Mounts, Error> {
        let mounts = match mounts_file() {
            Some(file) => Mounts::read(&file)?,
            None => Mounts::default(),
        };
        mounts.with_overrides(overrides, &std::env::current_dir()?)
    }

    fn with_overrides(mut self, overrides: &[String], here: &Path) -> Result<Mounts, Error> {
        for flag in overrides {
            let (name, target) = flag.split_once('=').ok_or_else(|| {
                Error::other(format!(
                    "--mount expects NAME=PATH, like ss2=~/ss2/data (got {flag:?})"
                ))
            })?;
            self.targets
                .insert(name.to_string(), (target.to_string(), here.to_path_buf()));
        }
        Ok(self)
    }

    /// A mounts file: `{"name": "target"}`; relative targets start at its
    /// folder. A missing file is no mounts.
    pub fn read(file: &Path) -> Result<Mounts, Error> {
        let bad = |why: &str| Error::other(format!("{}: {why}", file.display()));
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Mounts::default()),
            Err(e) => return Err(bad(&e.to_string())),
        };
        let json: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| bad(&e.to_string()))?;
        let base = file.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut targets = HashMap::new();
        for (name, target) in json
            .as_object()
            .ok_or_else(|| bad("must be an object of mount names, like {\"ss2\": \"~/ss2/\"}"))?
        {
            let target = target.as_str().ok_or_else(|| {
                bad(&format!(
                    "mount {name:?} must be a path, like \"~/data/\" or \"~/game.zip!data/\""
                ))
            })?;
            targets.insert(name.clone(), (target.to_string(), base.clone()));
        }
        Ok(Mounts { targets })
    }
}

impl Resolver {
    /// `config`'s `externals`, mapped through this machine's `mounts`.
    /// `entry_dir` is the entry file's folder.
    pub fn load(
        entry_dir: &Path,
        externals: &[ExternalPrefix],
        mounts: &Mounts,
    ) -> Result<Resolver, Error> {
        let mut resolved = Vec::new();
        for external in externals {
            let mount = match mounts.targets.get(&external.mount) {
                None => None,
                Some((target, base)) => Some(
                    Mount::parse(&std::path::absolute(base)?, target).map_err(|e| {
                        Error::other(format!("mount {:?} ({target}): {e}", external.mount))
                    })?,
                ),
            };
            resolved.push(External {
                prefix: external.prefix.clone(),
                name: external.mount.clone(),
                mount,
            });
        }
        Ok(Resolver {
            root: std::path::absolute(entry_dir)?,
            externals: resolved,
        })
    }

    fn external<'a>(&self, path: &'a str) -> Option<(&External, &'a str)> {
        self.externals.iter().find_map(|external| {
            path.strip_prefix(external.prefix.as_str())
                .map(|rest| (external, rest))
        })
    }

    /// What the headset gets for `request` (`functor run vr` serving the
    /// files it lacks): `Current` when its copy (`have`) matches this
    /// machine's version, else the bytes, else why not.
    pub fn reply(&self, session: u64, request: &ProjectFileRequest) -> ProjectFileReply {
        let path = &request.path;
        let version = self.version(path);
        // What the push never sends, the proxy doesn't either.
        let hidden = path.split('/').any(|part| part.starts_with('.'));
        let status = match (&request.have, version) {
            _ if hidden => ProjectFileStatus::Missing {
                message: "hidden files aren't sent to the headset".to_string(),
            },
            (Some(have), Some(version)) if *have == version => {
                ProjectFileStatus::Current { version }
            }
            (_, version) => match self.read(path) {
                // One upload's limit: a bigger file would be refused (413)
                // and asked for forever.
                Ok(bytes) if bytes.len() > MAX_ASSET_BYTES => ProjectFileStatus::Missing {
                    message: format!(
                        "it's {:.0} MB, over the {} MB a headset fetch carries",
                        bytes.len() as f64 / 1048576.0,
                        MAX_ASSET_BYTES >> 20
                    ),
                },
                Ok(bytes) => ProjectFileStatus::Bytes {
                    version: version.unwrap_or_default(),
                    bytes: bytes.to_vec(),
                },
                Err(ProjectReadError::Outside) => ProjectFileStatus::Missing {
                    message: "it resolves outside the project".to_string(),
                },
                Err(ProjectReadError::Unreadable(message)) => {
                    ProjectFileStatus::Missing { message }
                }
            },
        };
        ProjectFileReply {
            session,
            path: path.clone(),
            status,
        }
    }

    /// Changes when the file does: size + mtime on disk, crc + size in a zip.
    fn version(&self, path: &str) -> Option<String> {
        let on_disk = |root: &Path, path: &str| {
            let meta = std::fs::metadata(storage::resolve_confined(root, path).ok()?).ok()?;
            let mtime = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?;
            Some(format!("{}:{}", meta.len(), mtime.as_nanos()))
        };
        match self.external(path) {
            None => on_disk(&self.root, path),
            Some((external, rest)) => match &external.mount {
                None => None,
                Some(Mount::Dir(dir)) => on_disk(dir, rest),
                Some(Mount::Zip(zip)) => zip.version(rest),
            },
        }
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

impl External {
    fn unmapped(&self) -> String {
        let file = mounts_file().map_or("~/.config/functor/mounts.json".to_string(), |file| {
            file.display().to_string()
        });
        format!(
            "{prefix} is external (`Source.mount({name:?})`) and this machine has no `{name}` \
mount — add {{\"{name}\": \"path/to/folder/\"}} (or \"path/to/archive.zip!folder/\") to {file}, \
or pass --mount {name}=PATH",
            prefix = self.prefix,
            name = self.name,
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
    /// paths start at `base`.
    fn parse(base: &Path, target: &str) -> Result<Mount, String> {
        let resolve = |path: &str| match path.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(rest))
                .ok_or("`~` needs $HOME"),
            None => Ok(base.join(path)),
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
    File { index: usize, size: u64, crc: u32 },
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

    /// The entry's crc and size: changes when its contents do.
    fn version(&self, rest: &str) -> Option<String> {
        let name = format!("{}{rest}", self.inner);
        self.with_index(|index| match index.entries.get(&name) {
            Some(Entry::File { size, crc, .. }) => Ok(Some(format!("zip:{crc:08x}:{size}"))),
            _ => Ok(None),
        })
        .ok()
        .flatten()
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
            Some(Entry::File {
                index: at, size, ..
            }) => {
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
                    crc: entry.crc32(),
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
        /// `declared`: `{"ss2/": …}`, each prefix `Source.mount`ing the name
        /// without its `/`; `mapped`: this machine's targets by prefix
        /// (written as a mounts file in the project).
        fn config(&self, declared: &str, mapped: Option<&str>) {
            self.write("declared.json", declared.as_bytes());
            match mapped {
                Some(mapped) => {
                    let json: serde_json::Value = serde_json::from_str(mapped).unwrap();
                    let mounts: serde_json::Map<String, serde_json::Value> = json
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(prefix, target)| {
                            (prefix.trim_end_matches('/').to_string(), target.clone())
                        })
                        .collect();
                    self.write(
                        "mounts.json",
                        serde_json::Value::from(mounts).to_string().as_bytes(),
                    )
                }
                None => {
                    let _ = std::fs::remove_file(self.0.join("mounts.json"));
                }
            }
        }
        fn externals(&self) -> Vec<ExternalPrefix> {
            let Ok(declared) = std::fs::read(self.0.join("declared.json")) else {
                return Vec::new();
            };
            let declared: serde_json::Value = serde_json::from_slice(&declared).unwrap();
            declared
                .as_object()
                .unwrap()
                .keys()
                .map(|prefix| ExternalPrefix {
                    prefix: prefix.clone(),
                    mount: prefix.trim_end_matches('/').to_string(),
                    device: None,
                })
                .collect()
        }
        /// The resolver `build()` installs, or the mapping error.
        fn load(&self) -> Result<Resolver, String> {
            let mounts = Mounts::read(&self.0.join("mounts.json")).map_err(|e| e.to_string())?;
            Resolver::load(&self.0, &self.externals(), &mounts).map_err(|e| e.to_string())
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
        // A mounts file found by a relative path: its targets still resolve.
        let mounts = Mounts::read(&relative.join("mounts.json")).unwrap();
        let resolver = Resolver::load(&relative, &project.externals(), &mounts).unwrap();
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
            err.contains("ss2/ is external (`Source.mount(\"ss2\")`)")
                && err.contains("no `ss2` mount")
                && err.contains("mounts.json")
                && err.contains("--mount ss2=PATH"),
            "{err}"
        );
        assert!(!resolver.exists("ss2/mesh/a.bin"));
        // A mapping to a missing archive fails at read time, naming it.
        project.config(r#"{"ss2/": "data"}"#, Some(r#"{"ss2/": "nope.kpf!data/"}"#));
        let err = read(&project.resolver(), "ss2/a.bin").unwrap_err();
        assert!(err.contains("nope.kpf"), "{err}");
    }

    /// What the headset gets per request: bytes with a version, `Current`
    /// when its copy matches, why not when the file isn't here either.
    #[test]
    fn replies_carry_versions_and_confirm_current_copies() {
        let project = TempProject::new("reply");
        project.write("data/a.bin", &[1, 2]);
        write_zip(&project.0.join("game.kpf"), &[("res/m.bin", Some(b"zip"))]);
        project.config(
            r#"{"ss2/": "game data", "gone/": "unmapped"}"#,
            Some(r#"{"ss2/": "game.kpf!res/"}"#),
        );
        let resolver = project.resolver();
        let ask = |path: &str, have: Option<&str>| {
            let request = ProjectFileRequest {
                path: path.to_string(),
                have: have.map(str::to_string),
            };
            resolver.reply(3, &request).status
        };
        let ProjectFileStatus::Bytes { version, bytes } = ask("ss2/m.bin", None) else {
            panic!("bytes");
        };
        assert_eq!(
            (version.as_str(), bytes),
            ("zip:421d9546:3", b"zip".to_vec())
        );
        assert_eq!(
            ask("ss2/m.bin", Some(&version)),
            ProjectFileStatus::Current { version }
        );
        let ProjectFileStatus::Bytes { version, .. } = ask("data/a.bin", Some("stale")) else {
            panic!("bytes for a stale copy");
        };
        assert!(version.starts_with("2:"), "size:mtime, got {version}");
        let ProjectFileStatus::Missing { message } = ask("gone/x.bin", None) else {
            panic!("missing");
        };
        assert!(message.contains("no `gone` mount"), "{message}");
        assert!(matches!(
            ask("data/none.bin", None),
            ProjectFileStatus::Missing { .. }
        ));
        // Never sent, like the push: hidden files.
        project.write(".env", b"secret");
        let ProjectFileStatus::Missing { message } = ask(".env", None) else {
            panic!(".env sent");
        };
        assert!(message.contains("aren't sent"), "{message}");
    }

    #[test]
    fn malformed_mounts_are_refused() {
        let project = TempProject::new("config");
        project.write("a.zip", b"");
        let refused = |mounts: &str| {
            project.write("mounts.json", mounts.as_bytes());
            project.write("declared.json", br#"{"ss2/": "x"}"#);
            project.load().err().unwrap_or_default()
        };
        assert!(refused(r#"["ss2"]"#).contains("must be an object"));
        assert!(refused(r#"{"ss2": 1}"#).contains("must be a path"));
        assert!(refused(r#"{"ss2": "a.zip!../up/"}"#).contains("plain relative path"));
        assert!(refused("{").contains("mounts.json"));
        // `--mount` takes NAME=PATH, relative to the current folder.
        let here = std::env::current_dir().unwrap();
        let mounts = Mounts::default()
            .with_overrides(&["ss2=data/".to_string()], &here)
            .unwrap();
        assert_eq!(mounts.targets["ss2"], ("data/".to_string(), here.clone()));
        let err = Mounts::default()
            .with_overrides(&["ss2".to_string()], &here)
            .err()
            .unwrap();
        assert!(err.to_string().contains("NAME=PATH"), "{err}");
    }
}
