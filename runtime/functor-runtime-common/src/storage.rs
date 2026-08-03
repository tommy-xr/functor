//! Durable local save slots — the store behind `Effect.save` / `Effect.load`.
//!
//! A slot is a named blob of PLAIN DATA (the same structural codec
//! `Effect.sendMsg` uses: [`crate::functor_lang_prelude::EffectValue`] as
//! canonical JSON), so what a game persists is exactly what can cross a wire
//! or land in the effect log — no closures, no host handles.
//!
//! Two backends, one API:
//!
//! - **native** — `<project>/.functor/saves/<slot>.json`, written atomically
//!   (a per-process temp file in the same directory, then rename) so a reader
//!   never sees a half-written slot. The project root is declared by the
//!   producer at load ([`set_project_root`]); until then it falls back to the
//!   process's working directory — which is the project directory for
//!   `functor run native` (the CLI chdirs), but NOT for the embedded producer
//!   on Quest, where saves are not yet wired to a writable app directory.
//! - **wasm** — one `localStorage` key per slot, scoped by the page path AND
//!   the entry the page booted (`functor:save:<pathname>|<entry>:<slot>`):
//!   the site serves every game from one `player.html`, so the page alone
//!   would make all of them share an `"autosave"`.
//!
//! The store is process-global (one root, one page) because a process runs
//! one game. Two producers in one process would share it — the same
//! constraint the physics world and the audio queue already carry.
//!
//! Slot keys are restricted to a safe charset ([`validate_slot`]) — a slot is
//! a NAME, never a path, so `../` can never escape the saves directory.

/// The longest accepted slot key. Long enough for any human-authored slot
/// name (`"autosave"`, `"profile-2"`), short enough to stay a filename on
/// every filesystem.
pub const MAX_SLOT_LEN: usize = 64;

/// Windows reserved device names: `con.json` still resolves to the console
/// device there, so a slot named `con` would write nowhere on that platform
/// while working everywhere else. Refuse them on every platform, so a game
/// cannot be portable-by-accident.
const RESERVED_SLOTS: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Accept a slot key, or teach why it was refused. The charset is
/// deliberately narrow — letters, digits, `_` and `-` — so a slot cannot
/// carry a path separator, a `..`, a drive letter, or a leading dot: the key
/// is a name the store places, never a path the game chooses.
///
/// Note that slots are case-SENSITIVE in `localStorage` but not on a
/// case-insensitive filesystem (macOS, Windows), so `"Save"` and `"save"` are
/// one slot natively and two on the web. Pick one casing per game.
pub fn validate_slot(slot: &str) -> Result<(), String> {
    if slot.is_empty() {
        return Err("slot key must not be empty — name the slot, e.g. \"autosave\"".to_string());
    }
    if slot.len() > MAX_SLOT_LEN {
        return Err(format!(
            "slot key is too long ({} chars, max {MAX_SLOT_LEN}) — name the slot, \
e.g. \"autosave\"",
            slot.len()
        ));
    }
    if let Some(bad) = slot
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
    {
        return Err(format!(
            "slot key {slot:?} contains {bad:?} — a slot is a NAME, not a path: use \
letters, digits, '_' or '-' (e.g. \"autosave\", \"profile-2\")"
        ));
    }
    if RESERVED_SLOTS.contains(&slot.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "slot key {slot:?} is a reserved device name on Windows — pick another \
name (e.g. \"autosave\")"
        ));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use once_cell::sync::Lazy;

    /// The project directory saves live under. Declared by the producer at
    /// load; `None` means "the working directory" (the embedded native
    /// producer runs from pushed sources and has no directory of its own).
    static ROOT: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

    /// Point the store at a project — its saves land in
    /// `<project>/.functor/saves`. The producers name a project by either its
    /// DIRECTORY or its entry FILE (both are valid `functor run` inputs), so
    /// a file path is normalized to its containing directory here rather than
    /// at each call site.
    pub fn set_project_root(root: PathBuf) {
        let root = if root.extension().is_some() && !root.is_dir() {
            root.parent()
                .map(|dir| dir.to_path_buf())
                .filter(|dir| !dir.as_os_str().is_empty())
                .unwrap_or_else(|| PathBuf::from("."))
        } else {
            root
        };
        *ROOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(root);
    }

    /// `<project>/.functor/saves` — the directory holding every slot file.
    pub fn saves_dir() -> PathBuf {
        let root = ROOT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        root.join(".functor").join("saves")
    }

    /// The on-disk file for a slot. Callers validate the key first (see
    /// [`super::validate_slot`]), so this join cannot escape the directory —
    /// which is why it stays private to the backend.
    fn slot_path(slot: &str) -> PathBuf {
        saves_dir().join(format!("{slot}.json"))
    }

    /// Write a slot ATOMICALLY: a temp file beside the target, flushed, then
    /// renamed over it. A crash (or a full disk) leaves the previous save
    /// readable instead of a half-written one.
    ///
    /// The temp name is unique per process AND per call (the `io` disk
    /// cache's rule, one step further): two `functor` processes on one project
    /// — a game plus an MCP session, a `run` plus a debug client — must not
    /// interleave into one shared temp, and `create_new` refuses to follow a
    /// pre-existing file or symlink at that path.
    pub fn write_slot(slot: &str, text: &str) -> Result<(), String> {
        use std::io::Write;
        super::validate_slot(slot)?;
        let path = slot_path(slot);
        let dir = saves_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        static NEXT_TMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tmp = dir.join(format!(
            "{slot}.{}-{}.tmp",
            std::process::id(),
            NEXT_TMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let write = || -> Result<(), String> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
            file.write_all(text.as_bytes())
                .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
            // Deliberately NO `fsync`: this runs inline on the frame thread
            // (a game may autosave on a timer), and the property being bought
            // is that a READER never sees a half-written slot — which the
            // rename alone gives. Surviving a power cut mid-write is not
            // promised, and the crate's other atomic write (`io::cache_write`)
            // makes the same trade.
            drop(file);
            std::fs::rename(&tmp, &path)
                .map_err(|e| format!("could not replace {}: {e}", path.display()))
        };
        let result = write();
        if result.is_err() {
            // Never leave a stray temp behind for a later `create_new` to trip on.
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// Read a slot's text, or `None` when the slot has never been written.
    /// An unreadable file is an error (the caller reports it and answers
    /// `Option.None`, so a broken save never kills a running game).
    pub fn read_slot(slot: &str) -> Result<Option<String>, String> {
        super::validate_slot(slot)?;
        let path = slot_path(slot);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    /// Every slot lives under one prefix, scoped by the PROJECT: the page's
    /// path plus the entry the page booted (`window.__functorLangGamePath` —
    /// the wasm counterpart of `--game-path`, which every host page sets).
    /// The page alone is not enough: the site's player serves many games from
    /// one `player.html`, and they must not share an `"autosave"`. The entry
    /// path is stable across edits (a source hash would not be), so a save
    /// survives hot-pushed source.
    fn storage_key(slot: &str) -> String {
        let page = web_sys::window()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_default();
        let entry = web_sys::window()
            .and_then(|w| {
                js_sys::Reflect::get(&w, &wasm_bindgen::JsValue::from_str("__functorLangGamePath"))
                    .ok()
            })
            .and_then(|value| value.as_string())
            .unwrap_or_default();
        // A `data:` entry is a whole inline program (the docs' "try it"
        // buttons) — it is neither stable nor short, so those share one
        // scratch scope instead of minting a key per edit.
        let entry = if entry.starts_with("data:") {
            "inline"
        } else {
            entry
                .char_indices()
                .nth(128)
                .map_or(entry.as_str(), |(byte, _)| &entry[..byte])
        };
        format!("functor:save:{page}|{entry}:{slot}")
    }

    fn local_storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .ok_or_else(|| "no window".to_string())?
            .local_storage()
            .map_err(|_| "localStorage is unavailable (blocked by the browser?)".to_string())?
            .ok_or_else(|| "localStorage is unavailable".to_string())
    }

    /// Write a slot. `localStorage` is already atomic per key — a set either
    /// lands whole or raises (a quota error, which is reported like a failed
    /// disk write on native).
    pub fn write_slot(slot: &str, text: &str) -> Result<(), String> {
        super::validate_slot(slot)?;
        local_storage()?
            .set_item(&storage_key(slot), text)
            .map_err(|_| "localStorage write failed (quota exceeded?)".to_string())
    }

    /// Read a slot's text, or `None` when the key has never been written.
    pub fn read_slot(slot: &str) -> Result<Option<String>, String> {
        super::validate_slot(slot)?;
        local_storage()?
            .get_item(&storage_key(slot))
            .map_err(|_| "localStorage read failed".to_string())
    }
}

pub use backend::{read_slot, write_slot};

#[cfg(not(target_arch = "wasm32"))]
pub use backend::set_project_root;

#[cfg(all(test, not(target_arch = "wasm32")))]
use backend::saves_dir;

/// Serializes tests that point the (process-global) native store at a
/// throwaway directory — `Effect.save`/`load` tests live in the prelude too.
#[cfg(test)]
pub(crate) static SAVE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot key is a NAME: traversal, separators, and empties are refused
    /// at the door, with a message that teaches the shape.
    #[test]
    fn slot_keys_are_names_not_paths() {
        for ok in ["autosave", "profile-2", "SLOT_9", "a"] {
            assert!(validate_slot(ok).is_ok(), "{ok} should be a valid slot");
        }
        for bad in ["", "../escape", "a/b", "a\\b", ".hidden", "с", "with space", "x.json"] {
            assert!(validate_slot(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(validate_slot(&"a".repeat(MAX_SLOT_LEN)).is_ok());
        assert!(validate_slot(&"a".repeat(MAX_SLOT_LEN + 1)).is_err());
        let message = validate_slot("../escape").unwrap_err();
        assert!(
            message.contains("a slot is a NAME, not a path"),
            "expected the teaching error, got: {message}"
        );
    }

    /// Native round-trip through a real directory: absent reads as `None`,
    /// a write lands under `<root>/.functor/saves/<slot>.json`, and no temp
    /// file survives a successful write.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn writes_land_under_the_project_root_and_read_back() {
        let _guard = SAVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_project_root(dir.path().to_path_buf());
        assert_eq!(read_slot("fresh").unwrap(), None);
        write_slot("fresh", "{\"n\":1}").unwrap();
        let path = dir.path().join(".functor/saves/fresh.json");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"n\":1}");
        assert_eq!(read_slot("fresh").unwrap().as_deref(), Some("{\"n\":1}"));
        // Overwrite in place, leaving no temp behind.
        write_slot("fresh", "{\"n\":2}").unwrap();
        assert_eq!(read_slot("fresh").unwrap().as_deref(), Some("{\"n\":2}"));
        assert!(!dir.path().join(".functor/saves/fresh.json.tmp").exists());
    }

    /// A project is named by its directory OR its entry file (both are valid
    /// `functor run` inputs) — an entry file resolves to the directory
    /// holding it, so saves never land beside a path like `game.fun/…`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn an_entry_file_names_its_directory() {
        let _guard = SAVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("game.fun"), "let init = 0.0\n").unwrap();
        set_project_root(dir.path().join("game.fun"));
        assert_eq!(saves_dir(), dir.path().join(".functor").join("saves"));
        set_project_root(dir.path().to_path_buf());
        assert_eq!(saves_dir(), dir.path().join(".functor").join("saves"));
    }
}
