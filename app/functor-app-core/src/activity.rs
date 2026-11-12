//! What each task's agent is doing, for its character (D34): the kind of
//! each call still running, a stance, and an error mark that stays until the
//! user's next message. Derived purely from the task.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tasks::{Item, Task, TaskStatus};

/// Longest call label or error line, in characters.
const LABEL_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Activity {
    pub stance: Stance,
    /// The run's calls still waiting on their results, oldest first.
    pub calls: Vec<Call>,
    /// The last run's error or refusal, one short line; cleared by the
    /// user's next message.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    /// Running, with no call out.
    Thinking,
    /// Running a call.
    Working,
    /// The next move is the user's: an answer, a review, or a question.
    Waiting,
    Done,
    Failed,
}

/// What a call touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    /// Planning: a checklist or a plan.
    Thinking,
    Files,
    Shell,
    Network,
    /// The app's own game and view tools.
    Game,
    /// Subagents.
    Agents,
    Skills,
    /// A question for the user.
    Ask,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Call {
    pub kind: CallKind,
    /// Short: "reading weapons.fun", "running cargo test".
    pub label: String,
}

/// Every task's activity, by id.
pub fn activities(tasks: &[Task]) -> BTreeMap<u64, Activity> {
    tasks.iter().map(|t| (t.id, activity(t))).collect()
}

/// `task`'s activity, from its run, status and last run's items.
pub fn activity(task: &Task) -> Activity {
    let items = &task.transcript;
    let start = items
        .iter()
        .rposition(|i| matches!(i, Item::User { .. }))
        .map_or(0, |i| i + 1);
    let last = &items[start..];
    let calls: Vec<Call> = match task.run {
        Some(_) => last
            .iter()
            .filter_map(|i| match i {
                Item::Tool {
                    name,
                    input,
                    result: None,
                } => Some(classify(name, input)),
                _ => None,
            })
            .collect(),
        None => vec![],
    };
    let stance = match &task.status {
        _ if task.run.is_some() => {
            if calls.iter().any(|c| c.kind == CallKind::Ask) {
                Stance::Waiting
            } else if calls.is_empty() {
                Stance::Thinking
            } else {
                Stance::Working
            }
        }
        TaskStatus::WaitingOnYou | TaskStatus::ReadyForReview => Stance::Waiting,
        TaskStatus::Done | TaskStatus::Kept | TaskStatus::Reverted => Stance::Done,
        TaskStatus::Failed { .. } => Stance::Failed,
        TaskStatus::InProgress => Stance::Thinking,
    };
    let error = last
        .iter()
        .rev()
        .find_map(error_of)
        .or_else(|| match &task.status {
            TaskStatus::Failed { summary } => Some(line(summary.as_deref())),
            _ => None,
        });
    Activity {
        stance,
        calls,
        error,
    }
}

/// A failed run's summary, or a refusal, as one line. Errors the agent
/// works past (a missed edit, a failing command) are not marked: they are
/// routine. Tool results carry no refusal flag in the event stream, so a
/// refusal is read from Claude's wording.
fn error_of(item: &Item) -> Option<String> {
    match item {
        Item::Finished { ok: false, summary } => Some(line(summary.as_deref())),
        Item::Tool {
            result: Some(text), ..
        } if text.starts_with("Claude requested permissions to") => Some(line(Some(text))),
        _ => None,
    }
}

/// `text`'s first non-blank line, cut to [`LABEL_CHARS`].
fn line(text: Option<&str>) -> String {
    let first = text
        .and_then(|t| t.lines().find(|l| !l.trim().is_empty()))
        .map(str::trim);
    cut(first.unwrap_or("the run failed"))
}

fn cut(text: &str) -> String {
    if text.chars().count() <= LABEL_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(LABEL_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

/// The kind and label of a call to tool `name` with `input`, as either
/// harness names it (Codex runs commands as `shell`; both name MCP tools
/// `mcp__<server>__<tool>`).
fn classify(name: &str, input: &Value) -> Call {
    let s = |key: &str| input[key].as_str().unwrap_or("");
    let file = |key: &str| short(s(key));
    let (kind, label) = match name {
        "Read" => (CallKind::Files, format!("reading {}", file("file_path"))),
        "Write" => (CallKind::Files, format!("writing {}", file("file_path"))),
        "Edit" | "MultiEdit" => (CallKind::Files, format!("editing {}", file("file_path"))),
        "NotebookEdit" => (
            CallKind::Files,
            format!("editing {}", file("notebook_path")),
        ),
        "Grep" => (CallKind::Files, format!("searching for {}", s("pattern"))),
        "Glob" => (CallKind::Files, format!("finding {}", s("pattern"))),
        "Bash" | "shell" => return shell(&input["command"]),
        "WebFetch" => (CallKind::Network, format!("fetching {}", short(s("url")))),
        "WebSearch" => (
            CallKind::Network,
            format!("searching the web for {}", s("query")),
        ),
        "Task" | "Agent" => (CallKind::Agents, format!("subagent: {}", s("description"))),
        "SendMessage" | "TaskStop" => (CallKind::Agents, "talking to a subagent".into()),
        "Skill" => (CallKind::Skills, format!("using {}", s("skill"))),
        "ToolSearch" => (CallKind::Skills, "loading tools".into()),
        "AskUserQuestion" => {
            let question = input["questions"][0]["question"].as_str().unwrap_or("");
            (CallKind::Ask, format!("asking: {question}"))
        }
        "TodoWrite" | "ExitPlanMode" => (CallKind::Thinking, "planning".into()),
        _ => match name.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
            Some(("mis", tool)) => (CallKind::Game, tool.replace('_', " ")),
            Some((server, tool)) => (
                CallKind::Network,
                format!("{server}: {}", tool.replace('_', " ")),
            ),
            None => (CallKind::Shell, name.to_string()),
        },
    };
    Call {
        kind,
        label: cut(label.trim_end_matches([' ', ':'])),
    }
}

/// A path as its file name, a URL as its host.
fn short(arg: &str) -> &str {
    match arg.split_once("://") {
        Some((_, rest)) => rest.split('/').next().unwrap_or(rest),
        None => arg.trim_end_matches('/').rsplit('/').next().unwrap_or(arg),
    }
}

/// A shell call (`command` a script, or argv as Codex may send it), sorted
/// by what its commands touch: any network command makes it network; only
/// file commands (beside neutral ones like `cd`) make it files.
fn shell(command: &Value) -> Call {
    let commands = match command {
        Value::Array(argv) => vec![argv
            .iter()
            .filter_map(|w| w.as_str().map(String::from))
            .collect()],
        other => commands(other.as_str().unwrap_or("")),
    };
    let commands: Vec<Vec<String>> = commands.into_iter().flat_map(unwrap).collect();
    let real: Vec<&Vec<String>> = commands
        .iter()
        .filter(|c| !NEUTRAL.contains(&c[0].as_str()))
        .collect();
    let kind = if commands.iter().any(|c| is_network(c)) {
        CallKind::Network
    } else if !real.is_empty() && real.iter().all(|c| FILES.contains(&c[0].as_str())) {
        CallKind::Files
    } else {
        CallKind::Shell
    };
    // A file command by its last operand (usually the file), another by its
    // first (usually a subcommand): `sed -n 1p game.fun`, `git -C x push`.
    let shown = real.first().copied().or(commands.first()).map(|c| {
        let arg = match FILES.contains(&c[0].as_str()) {
            true => c[1..]
                .iter()
                .rfind(|a| !a.starts_with('-'))
                .map(String::as_str),
            false => first_arg(c, &["-C", "-c"]),
        };
        match arg {
            Some(arg) => format!("{} {}", c[0], short(arg)),
            None => c[0].clone(),
        }
    });
    Call {
        kind,
        label: cut(format!("running {}", shown.unwrap_or_default()).trim_end()),
    }
}

#[rustfmt::skip]
const FILES: &[&str] = &[
    "cat", "head", "tail", "less", "more", "nl", "sed", "grep", "rg", "find", "fd", "ls", "wc",
    "cp", "mv", "mkdir", "touch", "diff", "cmp", "sort", "uniq", "cut", "tr", "awk", "tee", "stat",
    "file", "du", "tree", "ln", "rm", "rmdir", "jq", "basename", "dirname", "realpath",
    "apply_patch",
];

const NEUTRAL: &[&str] = &[
    "echo", "printf", "cd", "export", "true", "false", "set", "pwd", "sleep", "which", "test", "[",
    ":",
];

const NETWORK: &[&str] = &[
    "curl", "wget", "gh", "ssh", "scp", "sftp", "ftp", "rsync", "nc", "ncat", "telnet", "ping",
    "dig", "nslookup",
];

/// Package managers, network with one of [`FETCH_VERBS`].
const PACKAGERS: &[&str] = &[
    "npm", "pnpm", "yarn", "bun", "pip", "pip3", "pipx", "uv", "poetry", "cargo", "go", "brew",
    "apt", "apt-get", "docker",
];

const FETCH_VERBS: &[&str] = &[
    "install", "i", "ci", "add", "update", "upgrade", "publish", "pull", "push", "get", "fetch",
];

const GIT_NETWORK: &[&str] = &["push", "pull", "fetch", "clone", "ls-remote"];

/// Commands that run the command after their own options, with the
/// options that take a value: `sudo -u root curl …`.
#[rustfmt::skip]
const WRAPPERS: &[(&str, &[&str])] = &[
    ("sudo", &["-u", "-g", "-h", "-p", "-C", "-U"]),
    ("doas", &["-u", "-C"]),
    ("env", &["-u", "-C"]),
    ("timeout", &["-s", "-k"]),
    ("nice", &["-n"]),
    ("ionice", &["-c", "-n", "-p"]),
    ("stdbuf", &["-i", "-o", "-e"]),
    ("caffeinate", &["-t", "-w"]),
    ("exec", &["-a"]),
    ("xargs", &["-I", "-n", "-P", "-L", "-d", "-s", "-E", "-a"]),
    ("time", &[]), ("nohup", &[]), ("command", &[]),
];

const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash"];

fn is_network(c: &[String]) -> bool {
    let name = c[0].as_str();
    let sub = first_arg(c, &["-C", "-c"]);
    NETWORK.contains(&name)
        || (name == "git" && sub.is_some_and(|s| GIT_NETWORK.contains(&s)))
        || (PACKAGERS.contains(&name) && sub.is_some_and(|s| FETCH_VERBS.contains(&s)))
}

/// Command `c`'s first operand, skipping flags and the values of `valued` ones.
fn first_arg<'a>(c: &'a [String], valued: &[&str]) -> Option<&'a str> {
    let mut args = c[1..].iter();
    while let Some(a) = args.next() {
        if valued.contains(&a.as_str()) {
            args.next();
        } else if !a.starts_with('-') {
            return Some(a);
        }
    }
    None
}

/// One command as the commands it runs: leading `VAR=value`s dropped, the
/// program named by its file name, wrappers (`sudo`, `env`, `timeout`, …)
/// read as what they run, and `bash -c <script>` as the script's commands.
fn unwrap(mut words: Vec<String>) -> Vec<Vec<String>> {
    loop {
        let assignments = words.iter().take_while(|w| is_assignment(w)).count();
        words.drain(..assignments);
        let Some(first) = words.first_mut() else {
            return vec![];
        };
        *first = short(first).to_string();
        let name = first.clone();
        if let Some((_, valued)) = WRAPPERS.iter().find(|(w, _)| *w == name) {
            // Its options (with their values) and `env`'s assignments; then
            // `timeout`'s duration.
            let mut at = 1;
            while let Some(w) = words.get(at) {
                if valued.contains(&w.as_str()) {
                    at += 2;
                } else if w.starts_with('-') || is_assignment(w) {
                    at += 1;
                } else {
                    break;
                }
            }
            if name == "timeout" {
                at += 1;
            }
            words.drain(..at.min(words.len()));
            continue;
        }
        if SHELLS.contains(&name.as_str()) {
            let flags = words[1..].iter().take_while(|w| w.starts_with('-'));
            if flags
                .clone()
                .any(|f| !f.starts_with("--") && f.contains('c'))
            {
                let script = words.get(1 + flags.count()).cloned().unwrap_or_default();
                return commands(&script).into_iter().flat_map(unwrap).collect();
            }
        }
        return vec![words];
    }
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn is_number(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())
}

/// `script`'s simple commands as words, quotes removed: split at `;`, `&`,
/// `|`, `(`, `)`, `` ` ``, `$(` and newlines; redirection targets,
/// comments and here-document bodies left out.
fn commands(script: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = vec![vec![]];
    let mut word: Option<String> = None;
    // Here-document delimiters whose bodies start at the next newline.
    let mut heredocs: Vec<String> = vec![];
    let mut redirect = false;
    // `$(…)` and `` `…` `` inside double quotes, split out at the end.
    let mut substitutions: Vec<String> = vec![];
    let mut chars = script.chars().peekable();
    let end_word = |word: &mut Option<String>, out: &mut Vec<Vec<String>>, redirect: &mut bool| {
        if let Some(w) = word.take() {
            if !std::mem::take(redirect) {
                out.last_mut().expect("never empty").push(w);
            }
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                w.extend(chars.by_ref().take_while(|&c| c != '\''));
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => w.extend(chars.next()),
                        // A substitution still runs: `"$(curl …)"`.
                        '$' if chars.peek() == Some(&'(') => {
                            chars.next();
                            let mut depth = 1;
                            let inner: String = chars
                                .by_ref()
                                .take_while(|&c| {
                                    depth += i32::from(c == '(') - i32::from(c == ')');
                                    depth > 0
                                })
                                .collect();
                            substitutions.push(inner);
                        }
                        '`' => {
                            substitutions.push(chars.by_ref().take_while(|&c| c != '`').collect())
                        }
                        c => w.push(c),
                    }
                }
            }
            '#' if word.is_none() => while chars.next_if(|&c| c != '\n').is_some() {},
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some(n) => word.get_or_insert_with(String::new).push(n),
            },
            ' ' | '\t' => end_word(&mut word, &mut out, &mut redirect),
            '<' if chars.peek() == Some(&'<') => {
                end_word(&mut word, &mut out, &mut redirect);
                chars.next();
                if chars.peek() == Some(&'<') {
                    // A here-string: its word is data.
                    chars.next();
                    redirect = true;
                    continue;
                }
                if chars.peek() == Some(&'-') {
                    chars.next();
                }
                while chars.peek() == Some(&' ') {
                    chars.next();
                }
                let mut delimiter = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || ";&|<>()".contains(c) {
                        break;
                    }
                    if c != '\'' && c != '"' {
                        delimiter.push(c);
                    }
                    chars.next();
                }
                heredocs.push(delimiter);
            }
            '>' | '<' => {
                // `2>`: the number is the descriptor, not an argument.
                if word.as_deref().is_some_and(is_number) {
                    word = None;
                }
                end_word(&mut word, &mut out, &mut redirect);
                while matches!(chars.peek(), Some('>' | '&' | '|')) {
                    chars.next();
                }
                redirect = true;
            }
            '&' if chars.peek() == Some(&'>') => {
                end_word(&mut word, &mut out, &mut redirect);
            }
            '$' if chars.peek() == Some(&'(') => end_word(&mut word, &mut out, &mut redirect),
            '\n' | ';' | '&' | '|' | '(' | ')' | '`' => {
                end_word(&mut word, &mut out, &mut redirect);
                redirect = false;
                if !out.last().expect("never empty").is_empty() {
                    out.push(vec![]);
                }
                if c == '\n' {
                    for delimiter in std::mem::take(&mut heredocs) {
                        loop {
                            let body: String = chars.by_ref().take_while(|&c| c != '\n').collect();
                            if body.trim() == delimiter || chars.peek().is_none() {
                                break;
                            }
                        }
                    }
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    end_word(&mut word, &mut out, &mut redirect);
    out.extend(substitutions.iter().flat_map(|s| commands(s)));
    out.retain(|c| !c.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{apply, derive_status, end_run, from_stored, AgentEvent, INTERRUPTED};
    use serde_json::json;

    #[test]
    #[rustfmt::skip]
    fn tools_classify_by_name() {
        use CallKind::*;
        let cases: &[(&str, Value, CallKind, &str)] = &[
            ("Read", json!({"file_path": "/w/examples/fps/weapons.fun"}), Files, "reading weapons.fun"),
            ("Edit", json!({"file_path": "game.fun"}), Files, "editing game.fun"),
            ("Write", json!({"file_path": "a/b.fun"}), Files, "writing b.fun"),
            ("Grep", json!({"pattern": "fn tick"}), Files, "searching for fn tick"),
            ("Glob", json!({"pattern": "**/*.fun"}), Files, "finding **/*.fun"),
            ("WebFetch", json!({"url": "https://functor.games/docs/x"}), Network, "fetching functor.games"),
            ("WebSearch", json!({"query": "rapier joints"}), Network, "searching the web for rapier joints"),
            ("Task", json!({"description": "Find the spawn code"}), Agents, "subagent: Find the spawn code"),
            ("Agent", json!({}), Agents, "subagent"),
            ("Skill", json!({"skill": "functor-lang"}), Skills, "using functor-lang"),
            ("ToolSearch", json!({"query": "select:X"}), Skills, "loading tools"),
            ("AskUserQuestion", json!({"questions": [{"question": "Which weapon?"}]}), Ask, "asking: Which weapon?"),
            ("TodoWrite", json!({"todos": []}), Thinking, "planning"),
            ("mcp__mis__get_state", json!({"session": "s1"}), Game, "get state"),
            ("mcp__mis__open_view", json!({}), Game, "open view"),
            ("mcp__github__create_issue", json!({}), Network, "github: create issue"),
            ("SomethingNew", json!({}), Shell, "SomethingNew"),
        ];
        for (name, input, kind, label) in cases {
            let want = Call { kind: *kind, label: label.to_string() };
            assert_eq!(classify(name, input), want, "{name}");
        }
    }

    #[test]
    #[rustfmt::skip]
    fn shell_commands_sort_by_what_they_touch() {
        use CallKind::*;
        let cases: &[(&str, CallKind, &str)] = &[
            ("cat weapons.fun", Files, "running cat weapons.fun"),
            ("grep -n tick src/game.fun | head -5", Files, "running grep game.fun"),
            ("cd examples/fps && sed -n 1,20p game.fun", Files, "running sed game.fun"),
            ("mv a.fun b.fun; rm -rf old", Files, "running mv b.fun"),
            ("ls 2>/dev/null", Files, "running ls"),
            ("curl -s https://example.com/x.json", Network, "running curl example.com"),
            ("gh pr view 12", Network, "running gh pr"),
            ("ssh box uptime", Network, "running ssh box"),
            ("git push origin HEAD", Network, "running git push"),
            ("git -C repo fetch", Network, "running git fetch"),
            ("git status", Shell, "running git status"),
            ("npm install", Network, "running npm install"),
            ("npm test", Shell, "running npm test"),
            ("cargo test -p functor-lang", Shell, "running cargo test"),
            ("cargo test 2>&1 | tail -20", Shell, "running cargo test"),
            ("RUST_LOG=debug cargo run", Shell, "running cargo run"),
            ("echo hi", Shell, "running echo hi"),
            // Wrappers read as the command they run.
            ("sudo rm -rf build", Files, "running rm build"),
            ("env FOO=1 curl x.dev", Network, "running curl x.dev"),
            ("timeout 30 cargo test", Shell, "running cargo test"),
            ("nice -n 10 cat f", Files, "running cat f"),
            ("find . -name '*.fun' | xargs grep -l tick", Files, "running find *.fun"),
            ("ls $(git rev-parse --show-toplevel)", Shell, "running ls"),
            ("/usr/bin/curl x.dev", Network, "running curl x.dev"),
            ("sudo -u root curl x.dev", Network, "running curl x.dev"),
            ("timeout -s TERM 30 curl x.dev", Network, "running curl x.dev"),
            ("xargs -I {} cat {}", Files, "running cat {}"),
            // Substitutions run, comments do not.
            ("cat \"$(curl -s x.dev)\"", Network, "running cat"),
            ("echo \"`gh api user`\"", Network, "running gh api"),
            ("cat game.fun # then: curl x.dev", Files, "running cat game.fun"),
            // Codex wraps each command in a login shell.
            ("/bin/zsh -lc 'cat game.fun'", Files, "running cat game.fun"),
            ("/bin/zsh -lc \"git push && echo ok\"", Network, "running git push"),
            ("bash -c 'cargo build'", Shell, "running cargo build"),
            // A here-document's body is data, not commands.
            ("apply_patch <<'PATCH'\n*** Begin Patch\n*** Update File: game.fun\nPATCH", Files, "running apply_patch"),
            ("cat > notes.md <<EOF\ncurl is not run\nEOF\nwc -l notes.md", Files, "running cat"),
        ];
        for (command, kind, label) in cases {
            let want = Call { kind: *kind, label: label.to_string() };
            assert_eq!(classify("Bash", &json!({ "command": command })), want, "{command}");
            assert_eq!(classify("shell", &json!({ "command": command })).kind, *kind);
        }
        let argv = json!({"command": ["bash", "-lc", "curl x.dev"]});
        assert_eq!(classify("shell", &argv).kind, Network);
    }

    #[test]
    fn labels_are_cut_short() {
        let long = "a".repeat(80);
        let call = classify("Grep", &json!({ "pattern": long }));
        assert_eq!(call.label.chars().count(), LABEL_CHARS);
        assert!(call.label.ends_with('…'));
    }

    /// A fixture's events (`{"ms", "event"}` lines; `limits` skipped).
    fn events(text: &str) -> Vec<AgentEvent> {
        text.lines()
            .filter_map(|l| {
                let line: Value = serde_json::from_str(l).unwrap();
                serde_json::from_value(line["event"].clone()).ok()
            })
            .collect()
    }

    fn running() -> Task {
        from_stored(
            r#"{"version": 1, "transcript": [{"kind": "user", "text": "hi"}], "task": {
                "id": 1, "project": "p", "title": "hi", "status": {"kind": "in_progress"},
                "harness": "claude", "model": null, "session_id": null,
                "run": {"id": 0, "usage": null}}}"#,
        )
        .unwrap()
    }

    /// The activity after each event that changed it, as `stance kind:label… !error`.
    fn replay(task: &mut Task, log: &str) -> Vec<String> {
        let show = |task: &Task| {
            let a = activity(task);
            let mut s = format!("{:?}", a.stance);
            for call in &a.calls {
                s += &format!(" {:?}:{}", call.kind, call.label);
            }
            if let Some(e) = a.error {
                s += &format!(" !{e}");
            }
            s
        };
        let mut seen: Vec<String> = vec![];
        for event in events(log) {
            apply(task, event);
            let now = show(task);
            if seen.last() != Some(&now) {
                seen.push(now);
            }
        }
        end_run(task, None);
        seen.push(show(task));
        seen
    }

    const CLAUDE: &str = include_str!("../fixtures/agent/claude-activity.jsonl");
    const CODEX: &str = include_str!("../fixtures/agent/codex-shell.jsonl");
    const MISSING: &str = include_str!("../fixtures/agent/claude-missing.jsonl");

    #[test]
    fn a_claude_run_reads_as_calls_a_refusal_and_a_question() {
        let mut task = running();
        let refused = "!Claude requested permissions to use Bas…";
        assert_eq!(
            replay(&mut task, CLAUDE),
            [
                "Thinking".to_string(),
                "Working Files:reading weapons.fun".to_string(),
                "Working Files:reading weapons.fun Files:searching for fireRate".to_string(),
                "Working Files:searching for fireRate".to_string(),
                "Thinking".to_string(),
                "Working Shell:running cargo test".to_string(),
                format!("Thinking {refused}"),
                format!("Waiting Ask:asking: Raise the fire rate or the dama… {refused}"),
                format!("Thinking {refused}"),
                // Ended: an answer with no change is done; the mark stays.
                format!("Done {refused}"),
            ]
        );
        // The user's next message clears it.
        task.transcript.push(Item::User { text: "ok".into() });
        assert_eq!(activity(&task).error, None);
    }

    #[test]
    fn a_codex_run_sorts_its_shell_calls() {
        let mut task = running();
        assert_eq!(
            replay(&mut task, CODEX),
            [
                "Thinking",
                "Working Files:running cat game.fun",
                "Thinking",
                "Working Network:running curl functor.games",
                // A note it worked past is not an error.
                "Thinking",
                "Done",
            ]
        );
    }

    #[test]
    fn a_failed_run_is_failed_with_its_summary() {
        let mut task = running();
        assert_eq!(
            replay(&mut task, MISSING),
            ["Thinking !spawning claude", "Failed !spawning claude"]
        );
        // Interrupted: no outcome item, the status's summary.
        let mut task = running();
        task.run = None;
        task.status = derive_status(&task);
        assert_eq!(activity(&task).stance, Stance::Failed);
        assert_eq!(activity(&task).error, Some(cut(INTERRUPTED)));
    }

    #[test]
    fn stances_follow_the_status_once_the_run_ends() {
        let mut task = running();
        task.run = None;
        for (status, stance) in [
            (TaskStatus::WaitingOnYou, Stance::Waiting),
            (TaskStatus::ReadyForReview, Stance::Waiting),
            (TaskStatus::Done, Stance::Done),
            (TaskStatus::Kept, Stance::Done),
            (TaskStatus::Reverted, Stance::Done),
            (TaskStatus::Failed { summary: None }, Stance::Failed),
        ] {
            task.status = status;
            assert_eq!(activity(&task).stance, stance);
        }
        // An unanswered call of an ended run is not running.
        task.transcript.push(Item::Tool {
            name: "Read".into(),
            input: json!({}),
            result: None,
        });
        assert_eq!(activity(&task).calls, []);
    }

    #[test]
    fn only_a_result_that_is_a_refusal_marks_an_error() {
        let mut task = running();
        let read = |content: &str| Item::Tool {
            name: "Read".into(),
            input: json!({}),
            result: Some(content.into()),
        };
        // A file that quotes a refusal is not one, nor an error worked past.
        task.transcript.push(read(CLAUDE));
        task.transcript.push(read(
            "<tool_use_error>File does not exist.</tool_use_error>",
        ));
        assert_eq!(activity(&task).error, None);
        task.transcript
            .push(read("Claude requested permissions to write to x.fun"));
        let refused = "Claude requested permissions to write t…";
        assert_eq!(activity(&task).error.as_deref(), Some(refused));
    }
}
