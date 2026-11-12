//! A task's change: what differs between the repo's working tree when the
//! task's first run started (its baseline snapshot, a git tree) and the
//! working tree now. The shell runs git; reading its output and planning a
//! revert live here, purely.

use serde::{Deserialize, Serialize};

/// A file with more changed lines than this is summarised, not rendered.
pub const FILE_LINES: u32 = 2000;
/// A file whose patch is longer than this (bytes) is summarised.
pub const FILE_BYTES: usize = 256 * 1024;
/// Lines one diff renders; files past the budget are summarised.
pub const PATCH_LINES: u32 = 10_000;
/// Files a change summary lists (it rides with every state); the rest are counted.
pub const MAX_FILES: usize = 200;

/// The summary that travels with the task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Change {
    /// In git's order (by path), at most [`MAX_FILES`].
    pub files: Vec<FileChange>,
    /// Changed files past [`MAX_FILES`], not listed.
    pub more: u32,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.more == 0
    }

    /// `files` as a summary, listing at most [`MAX_FILES`].
    pub fn from_files(mut files: Vec<FileChange>) -> Self {
        let more = files.len().saturating_sub(MAX_FILES) as u32;
        files.truncate(MAX_FILES);
        Self { files, more }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileChange {
    /// Relative to the repo root; a rename's new path.
    pub path: String,
    /// A rename's old path.
    pub from: Option<String>,
    pub added: u32,
    pub removed: u32,
    /// Git counts no lines in it.
    pub binary: bool,
    /// More than [`FILE_LINES`] lines changed: summarised, not rendered.
    pub large: bool,
}

/// One file of a full diff (`ServerMsg::Diff`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileDiff {
    pub file: FileChange,
    /// From the first hunk header on; empty when `omitted`.
    pub lines: Vec<DiffLine>,
    pub omitted: Option<Omitted>,
}

/// Why a file's lines are not in the diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Omitted {
    Binary,
    /// Over [`FILE_LINES`] lines or [`FILE_BYTES`] bytes.
    Large,
    /// Past the diff's [`PATCH_LINES`] budget.
    Budget,
    /// Git's output for it could not be read.
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct DiffLine {
    pub kind: LineKind,
    /// Without its `+`/`-`/` ` marker; a hunk header whole; a note without `\ `.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Hunk,
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`.
    Note,
}

/// `git diff --numstat -z -M` output: `added\tremoved\tpath\0`, or for a
/// rename `added\tremoved\t\0old\0new\0`; binary files count `-\t-`.
pub fn parse_numstat(out: &str) -> Result<Vec<FileChange>, String> {
    let mut fields = out.split('\0');
    let mut files = Vec::new();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let bad = || format!("unexpected numstat record {record:?}");
        let mut parts = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(bad());
        };
        let (from, path) = match path {
            "" => {
                let old = fields.next().filter(|p| !p.is_empty()).ok_or_else(bad)?;
                let new = fields.next().filter(|p| !p.is_empty()).ok_or_else(bad)?;
                (Some(old.to_string()), new.to_string())
            }
            path => (None, path.to_string()),
        };
        let binary = added == "-" && removed == "-";
        let count = |n: &str| match binary {
            true => Ok(0),
            false => n.parse::<u32>().map_err(|_| bad()),
        };
        let (added, removed) = (count(added)?, count(removed)?);
        files.push(FileChange {
            path,
            from,
            added,
            removed,
            binary,
            large: added.saturating_add(removed) > FILE_LINES,
        });
    }
    Ok(files)
}

/// Which of `files` a diff renders: in order, those neither binary nor
/// large, while the running line total stays within [`PATCH_LINES`].
pub fn patch_plan(files: &[FileChange]) -> Vec<bool> {
    let mut budget = PATCH_LINES;
    files
        .iter()
        .map(|f| {
            let lines = f.added + f.removed;
            let fits = !f.binary && !f.large && lines <= budget;
            if fits {
                budget -= lines;
            }
            fits
        })
        .collect()
}

/// The full diff of `files` from `patch`: `git diff -M` of exactly the
/// planned files ([`patch_plan`]), which git prints in the same order.
/// `truncated` when the patch was cut short; its last file is then unreadable.
pub fn file_diffs(
    files: &[FileChange],
    planned: &[bool],
    patch: &str,
    truncated: bool,
) -> Vec<FileDiff> {
    // A hunk line starts with ` `, `+`, `-` or `\`, so only a file header starts so.
    let mut chunks: Vec<&str> = Vec::new();
    let mut start = None;
    for (at, _) in patch.match_indices("diff --git ") {
        if at == 0 || patch.as_bytes()[at - 1] == b'\n' {
            if let Some(s) = start {
                chunks.push(&patch[s..at]);
            }
            start = Some(at);
        }
    }
    if let Some(s) = start {
        chunks.push(&patch[s..]);
    }
    let wanted = planned.iter().filter(|p| **p).count();
    // A cut patch's last chunk is partial; any other mismatch makes them all unreadable.
    let complete = if truncated {
        chunks.len().saturating_sub(1)
    } else {
        chunks.len()
    };
    let readable = chunks.len() == wanted || (truncated && chunks.len() <= wanted);
    let mut chunks = chunks.into_iter().take(complete);
    files
        .iter()
        .zip(planned)
        .map(|(file, &planned)| {
            let omitted = |why| FileDiff {
                file: file.clone(),
                lines: vec![],
                omitted: Some(why),
            };
            if file.binary {
                return omitted(Omitted::Binary);
            }
            if file.large {
                return omitted(Omitted::Large);
            }
            if !planned {
                return omitted(Omitted::Budget);
            }
            match chunks.next().filter(|_| readable) {
                Some(chunk) if chunk.len() > FILE_BYTES => omitted(Omitted::Large),
                Some(chunk) => match parse_file_patch(chunk) {
                    Some(lines) => FileDiff {
                        file: file.clone(),
                        lines,
                        omitted: None,
                    },
                    // Binary to git after all (an attribute): no hunks.
                    None => omitted(Omitted::Binary),
                },
                None => omitted(Omitted::Unreadable),
            }
        })
        .collect()
}

/// One file's patch as lines from its first hunk; `None` when git calls it
/// binary. Header lines (`new file mode`, `rename from`, `---`, …) are skipped.
pub fn parse_file_patch(chunk: &str) -> Option<Vec<DiffLine>> {
    let mut lines = Vec::new();
    let mut in_hunk = false;
    for line in chunk.lines() {
        if !in_hunk {
            if line.starts_with("Binary files ") || line == "GIT binary patch" {
                return None;
            }
            if !line.starts_with("@@") {
                continue;
            }
            in_hunk = true;
        }
        let (kind, text) = match line.as_bytes().first() {
            Some(b'@') => (LineKind::Hunk, line),
            Some(b'+') => (LineKind::Added, &line[1..]),
            Some(b'-') => (LineKind::Removed, &line[1..]),
            Some(b'\\') => (LineKind::Note, line.trim_start_matches('\\').trim_start()),
            Some(b' ') => (LineKind::Context, &line[1..]),
            // `diff.suppressBlankEmpty` prints a blank context line as empty.
            None => (LineKind::Context, ""),
            Some(_) => continue,
        };
        lines.push(DiffLine {
            kind,
            text: text.to_string(),
        });
    }
    Some(lines)
}

/// Splits NUL-terminated `-z` output into its entries.
pub fn nul_list(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// One step of a revert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevertOp {
    /// Remove a file the task created (or that took a removed path's place).
    Delete { path: String },
    /// Write the baseline's blob `oid` at `path`.
    Write {
        path: String,
        oid: String,
        mode: FileMode,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
}

/// The steps that put each of `changed` (repo-relative paths that differ
/// from the baseline) back as `listing` (`git ls-tree -r -z` of the
/// baseline) has it: deletes first, so a path a file took from a directory
/// (or the reverse) is free again before its writes. Refused when a path is
/// not a plain relative path or is a submodule.
pub fn revert_plan(listing: &str, changed: &[String]) -> Result<Vec<RevertOp>, String> {
    let mut entries = std::collections::HashMap::new();
    for record in listing.split('\0').filter(|r| !r.is_empty()) {
        let bad = || format!("unexpected ls-tree record {record:?}");
        let (meta, path) = record.split_once('\t').ok_or_else(bad)?;
        let mut meta = meta.split(' ');
        let (Some(mode), Some(_kind), Some(oid)) = (meta.next(), meta.next(), meta.next()) else {
            return Err(bad());
        };
        entries.insert(path, (mode, oid));
    }
    let mut deletes = Vec::new();
    let mut writes = Vec::new();
    for path in changed {
        if !safe_path(path) {
            return Err(format!("refusing to revert an unexpected path: {path:?}"));
        }
        match entries.get(path.as_str()) {
            None => deletes.push(RevertOp::Delete { path: path.clone() }),
            Some(&(mode, oid)) => {
                let mode = match mode {
                    "100644" => FileMode::Regular,
                    "100755" => FileMode::Executable,
                    "120000" => FileMode::Symlink,
                    _ => return Err(format!("cannot revert {path}: it is a submodule")),
                };
                writes.push(RevertOp::Write {
                    path: path.clone(),
                    oid: oid.to_string(),
                    mode,
                });
            }
        }
    }
    deletes.extend(writes);
    Ok(deletes)
}

/// A relative path of plain components, outside `.git`.
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|c| !matches!(c, "" | "." | "..") && !c.eq_ignore_ascii_case(".git"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, added: u32, removed: u32) -> FileChange {
        FileChange {
            path: path.to_string(),
            from: None,
            added,
            removed,
            binary: false,
            large: false,
        }
    }

    #[test]
    fn numstat_reads_counts_renames_binaries_and_odd_paths() {
        // As `git diff --numstat -z -M` prints them.
        let out = "1\t1\ta.txt\0-\t-\tbin.dat\0\
                   0\t50\tlong.txt\0\
                   1\t0\t\0r1.txt\0r 2.txt\0\
                   3\t0\tdir/with space.fun\0\
                   2500\t1\tbig.json\0";
        let files = parse_numstat(out).unwrap();
        assert_eq!(
            files,
            [
                file("a.txt", 1, 1),
                FileChange {
                    binary: true,
                    ..file("bin.dat", 0, 0)
                },
                file("long.txt", 0, 50),
                FileChange {
                    from: Some("r1.txt".to_string()),
                    ..file("r 2.txt", 1, 0)
                },
                file("dir/with space.fun", 3, 0),
                FileChange {
                    large: true,
                    ..file("big.json", 2500, 1)
                },
            ]
        );
        assert_eq!(parse_numstat(""), Ok(vec![]));
        assert!(parse_numstat("x\t1\ta\0").is_err());
        assert!(parse_numstat("1\t1\t\0only-old\0").is_err());
    }

    #[test]
    fn a_summary_lists_at_most_max_files() {
        let files: Vec<_> = (0..MAX_FILES + 3)
            .map(|i| file(&format!("f{i}"), 1, 0))
            .collect();
        let change = Change::from_files(files);
        assert_eq!((change.files.len(), change.more), (MAX_FILES, 3));
        assert!(!change.is_empty());
        assert!(Change::from_files(vec![]).is_empty());
    }

    #[test]
    fn the_plan_skips_binary_and_large_files_and_stops_at_the_budget() {
        let files = [
            file("a", 10, 0),
            FileChange {
                binary: true,
                ..file("b", 0, 0)
            },
            FileChange {
                large: true,
                ..file("c", FILE_LINES + 1, 0)
            },
            file("d", PATCH_LINES - 10, 0),
            file("e", 1, 0),
            file("f", 0, 0),
        ];
        assert_eq!(patch_plan(&files), [true, false, false, true, false, true]);
    }

    const PATCH: &str = "\
diff --git a/a.txt b/a.txt
index 92cc6e1..5083a16 100644
--- a/a.txt
+++ b/a.txt
@@ -1,3 +1,3 @@ let init
 a
-nonl
\\ No newline at end of file
+nonlnonl
\\ No newline at end of file
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index 96cc558..0000000
--- a/gone.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-1
-2
diff --git a/new file.txt b/new file.txt
new file mode 100644
index 0000000..8ba3a16
--- /dev/null
+++ b/new file.txt\t
@@ -0,0 +1 @@
+diff --git looks like a header
diff --git a/r1.txt b/r 2.txt
similarity index 96%
rename from r1.txt
rename to r 2.txt
index e8823e1..10adcaf 100644
--- a/r1.txt
+++ b/r 2.txt\t
@@ -28,3 +28,4 @@
 28

+31
";

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            text: text.to_string(),
        }
    }

    fn patch_files() -> Vec<FileChange> {
        vec![
            file("a.txt", 1, 1),
            file("gone.txt", 0, 2),
            file("new file.txt", 1, 0),
            FileChange {
                from: Some("r1.txt".to_string()),
                ..file("r 2.txt", 1, 0)
            },
        ]
    }

    #[test]
    fn a_patch_splits_per_file_and_parses_its_lines() {
        use LineKind::*;
        let files = patch_files();
        let diffs = file_diffs(&files, &[true; 4], PATCH, false);
        let lines: Vec<&[DiffLine]> = diffs.iter().map(|d| d.lines.as_slice()).collect();
        assert_eq!(
            lines,
            [
                &[
                    line(Hunk, "@@ -1,3 +1,3 @@ let init"),
                    line(Context, "a"),
                    line(Removed, "nonl"),
                    line(Note, "No newline at end of file"),
                    line(Added, "nonlnonl"),
                    line(Note, "No newline at end of file"),
                ][..],
                &[
                    line(Hunk, "@@ -1,2 +0,0 @@"),
                    line(Removed, "1"),
                    line(Removed, "2")
                ],
                // A `+` line reading like a header stays in its file.
                &[
                    line(Hunk, "@@ -0,0 +1 @@"),
                    line(Added, "diff --git looks like a header")
                ],
                &[
                    line(Hunk, "@@ -28,3 +28,4 @@"),
                    line(Context, "28"),
                    line(Context, ""),
                    line(Added, "31")
                ],
            ]
        );
        assert!(diffs.iter().all(|d| d.omitted.is_none()));
        assert_eq!(diffs[3].file, files[3]);
    }

    #[test]
    fn omitted_files_say_why() {
        let mut files = patch_files();
        files.insert(
            1,
            FileChange {
                binary: true,
                ..file("bin.dat", 0, 0)
            },
        );
        let planned = [true, false, true, true, true];
        let diffs = file_diffs(&files, &planned, PATCH, false);
        assert_eq!(diffs[1].omitted, Some(Omitted::Binary));
        assert_eq!(diffs[2].lines.len(), 3);

        // Cut short: the last chunk is partial.
        let cut = &PATCH[..PATCH.find("rename from").unwrap()];
        let diffs = file_diffs(&patch_files(), &[true; 4], cut, true);
        let omitted: Vec<_> = diffs.iter().map(|d| d.omitted).collect();
        assert_eq!(omitted, [None, None, None, Some(Omitted::Unreadable)]);

        // A count that does not match the plan reads nothing.
        let diffs = file_diffs(&patch_files()[..3], &[true; 3], PATCH, false);
        assert!(diffs.iter().all(|d| d.omitted == Some(Omitted::Unreadable)));

        // Not planned: past the budget.
        let diffs = file_diffs(&patch_files()[..1], &[false], "", false);
        assert_eq!(diffs[0].omitted, Some(Omitted::Budget));
    }

    #[test]
    fn a_binary_marker_means_no_lines() {
        let chunk = "diff --git a/b.png b/b.png\nindex 1..2 100644\nBinary files a/b.png and b/b.png differ\n";
        assert_eq!(parse_file_patch(chunk), None);
        let diffs = file_diffs(&[file("b.png", 1, 0)], &[true], chunk, false);
        assert_eq!(diffs[0].omitted, Some(Omitted::Binary));
        let oversized = format!(
            "diff --git a/x b/x\n@@ -0,0 +1 @@\n+{}\n",
            "y".repeat(FILE_BYTES)
        );
        let diffs = file_diffs(&[file("x", 1, 0)], &[true], &oversized, false);
        assert_eq!(diffs[0].omitted, Some(Omitted::Large));
    }

    #[test]
    fn a_revert_deletes_what_is_new_and_writes_back_the_rest() {
        let listing = "100644 blob aaaa\tkept.txt\0\
                       100755 blob bbbb\trun me.sh\0\
                       120000 blob cccc\tlink\0\
                       160000 commit dddd\tvendor/sub\0\
                       100644 blob eeee\tdir/old.fun\0";
        let changed: Vec<String> = ["created.txt", "run me.sh", "link", "dir/old.fun"]
            .map(String::from)
            .to_vec();
        let write = |path: &str, oid: &str, mode| RevertOp::Write {
            path: path.to_string(),
            oid: oid.to_string(),
            mode,
        };
        assert_eq!(
            revert_plan(listing, &changed).unwrap(),
            [
                RevertOp::Delete {
                    path: "created.txt".to_string()
                },
                write("run me.sh", "bbbb", FileMode::Executable),
                write("link", "cccc", FileMode::Symlink),
                write("dir/old.fun", "eeee", FileMode::Regular),
            ]
        );
        assert_eq!(
            revert_plan(listing, &["vendor/sub".to_string()]),
            Err("cannot revert vendor/sub: it is a submodule".to_string())
        );
        for path in ["../x", "/etc/passwd", "a/./b", ".git/config", "a//b", ""] {
            assert!(revert_plan(listing, &[path.to_string()]).is_err(), "{path}");
        }
        assert_eq!(nul_list("a\0b c\0"), ["a".to_string(), "b c".to_string()]);
    }
}
