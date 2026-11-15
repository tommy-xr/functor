//! The spoken reply to a spoken request (D47): one or two sentences for
//! the ear, said when the request reaches its outcome. The agent writes
//! them as a `Spoken:` line ending its last message; without one, its
//! reply's first sentences are used. The shell turns the text into sound.

use crate::tasks::Item;
use crate::voice::Stage;

/// What marks the agent's line for the ear, before its colon.
const MARK: &str = "spoken";
/// The most said.
const MAX_CHARS: usize = 280;
const MAX_SENTENCES: usize = 2;

/// Added to a spoken request's prompt: asks for the line.
pub const ASK: &str = "(Your answer is also read aloud. End your last message \
with one line that starts with `Spoken:`, then one or two short sentences saying what you did \
or the answer, as you would say it to someone not looking at a screen: plain words, no file \
names, paths, code or markdown.)";

/// What is said for run `run` of `items`, which ended as `stage`; `review`
/// when its change waits for the user to keep or revert it.
pub fn spoken(items: &[Item], run: u64, stage: &Stage, review: bool) -> Option<String> {
    let replies = || {
        items
            .iter()
            .skip(run as usize + 1)
            .take_while(|i| !matches!(i, Item::User { .. }))
            .filter_map(|i| match i {
                Item::Agent { text, .. } => Some(text.as_str()),
                _ => None,
            })
    };
    // The last reply's line for the ear, else that reply.
    let last = replies().last();
    // Not a `Spoken:` inside fenced code.
    let marked = last.and_then(|reply| {
        let mut fenced = false;
        let prose = reply.lines().filter(move |l| {
            if l.trim_start().starts_with("```") {
                fenced = !fenced;
                return false;
            }
            !fenced
        });
        let lines = prose.filter_map(|l| strip_mark(l.trim()));
        lines.filter(|l| !l.is_empty()).last()
    });
    let line = marked.or(last).map(for_the_ear);
    let line = line.filter(|l| !l.is_empty());
    match stage {
        Stage::Working => None,
        Stage::Failed(reason) => Some(format!("That did not work: {}", for_the_ear(reason))),
        Stage::Answered(_) => {
            Some(line.unwrap_or_else(|| "I have answered in the conversation.".to_string()))
        }
        Stage::Reloaded => {
            let line = line.unwrap_or_else(|| "Done.".to_string());
            let asks = line.to_lowercase().contains("ready for you");
            Some(match review && !asks {
                true => format!("{line} It's ready for you to look at."),
                false => line,
            })
        }
    }
}

/// `line` after its `Spoken:` mark, whatever its case or emphasis
/// (`**Spoken:**`, `**Spoken**:`, `*Spoken:*`).
pub(crate) fn strip_mark(line: &str) -> Option<&str> {
    let line = line.trim_start_matches(['*', '_', '>', '-', ' ']);
    let head = line.get(..MARK.len())?;
    let rest = line[MARK.len()..].trim_start_matches(['*', '_']);
    let rest = rest.strip_prefix(':').filter(|_| head.eq_ignore_ascii_case(MARK))?;
    Some(rest.trim_start_matches(['*', '_', ' ']))
}

/// `text` as it is said: no code blocks, markdown marks, links, paths or
/// file names; its first sentences, cut at a word.
pub fn for_the_ear(text: &str) -> String {
    // Fenced code is never said.
    let mut prose = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            // A heading is a title, not a sentence.
            if line.trim_start().starts_with('#') {
                continue;
            }
            let line = line.trim().trim_start_matches(['>', ' ']);
            let line = line.strip_prefix("- ").or(line.strip_prefix("* ")).unwrap_or(line);
            prose.push_str(line);
            prose.push(' ');
        }
    }
    let words: Vec<String> = prose.split_whitespace().filter_map(word).collect();
    let mut out = String::new();
    let mut sentences = 0;
    for word in words {
        if out.chars().count() + word.chars().count() + 1 > MAX_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
        if ends_sentence(&word) {
            sentences += 1;
            if sentences == MAX_SENTENCES {
                break;
            }
        }
    }
    out
}

/// One word as it is said: marks dropped, a link as its text, a path or
/// file name as "a file"; `None` when nothing is left.
fn word(raw: &str) -> Option<String> {
    // `text](url)`: the text, and what follows the link.
    if let Some((text, rest)) = raw.split_once("](") {
        let after = rest.split_once(')').map_or("", |(_, after)| after);
        return word(&format!("{text}{after}"));
    }
    let raw = raw.trim_matches(|c: char| "*_`[]<>".contains(c));
    let body = raw.trim_start_matches(['(', '"']);
    let lead = &raw[..raw.len() - body.len()];
    let core = body.trim_end_matches(|c: char| ".,!?:;)\"".contains(c));
    let tail = &body[core.len()..];
    let core: String = core.chars().filter(|c| !"*`[]".contains(*c)).collect();
    if core.is_empty() {
        return None;
    }
    let link = core.starts_with("http://") || core.starts_with("https://");
    let said = match (link, path(&core) || file_name(&core)) {
        (true, _) => "a link",
        (_, true) => "a file",
        _ => &core,
    };
    Some(format!("{lead}{said}{tail}"))
}

/// Whether `word` ends a sentence: not "e.g.", "etc." or a list's "1.".
fn ends_sentence(word: &str) -> bool {
    const SHORT: [&str; 7] = ["e.g", "i.e", "etc", "vs", "mr", "mrs", "dr"];
    let Some(core) = word.strip_suffix(['.', '!', '?']) else {
        return false;
    };
    let core = core.trim_start_matches(['(', '"']).to_lowercase();
    !SHORT.contains(&core.as_str()) && !core.chars().all(|c| c.is_ascii_digit())
}

/// `src/game.fun`, `./assets`, `~/notes`, `a/b/c`, `src\\x`: not "and/or",
/// "1/60" or "client/server".
fn path(word: &str) -> bool {
    let parts: Vec<&str> = word.split('/').filter(|p| !p.is_empty()).collect();
    let rooted = word.starts_with(['/', '~']) || word.starts_with("./") || word.starts_with("../");
    let names = rooted || parts.len() > 2 || parts.last().is_some_and(|p| file_name(p));
    word.contains('\\') || (word.contains('/') && names)
}

/// `game.fun`, `Cargo.toml`, `Ball.PNG`: a name, a dot and a file
/// extension. Not `Color.red` or `functor.games`.
fn file_name(word: &str) -> bool {
    use crate::decision::{ASSETS, CONFIG};
    const CODE: [&str; 12] =
        ["fun", "funi", "rs", "md", "txt", "ts", "tsx", "js", "mjs", "css", "html", "lock"];
    crate::decision::extension(word).is_some_and(|ext| {
        let ext = ext.as_str();
        CODE.contains(&ext) || ASSETS.contains(&ext) || CONFIG.contains(&ext)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(text: &str) -> Item {
        Item::Agent {
            text: text.to_string(),
            partial: false,
        }
    }

    fn run(replies: &[&str]) -> Vec<Item> {
        let mut items = vec![Item::User {
            text: "make it red".to_string(),
            spoken: None,
        }];
        items.extend(replies.iter().map(|r| agent(r)));
        items
    }

    #[test]
    fn text_for_the_ear_has_no_code_marks_links_or_paths() {
        let reply = "## Done\n\nI changed `ballColor` in **examples/a/game.fun** to red \
(see [the docs](https://functor.games/docs)).\n\n```fun\nlet ballColor = Color.red\n```\n\
It bounces slower now. A third sentence is not said.";
        assert_eq!(
            for_the_ear(reply),
            "I changed ballColor in a file to red (see the docs). It bounces slower now."
        );
        assert_eq!(
            for_the_ear("Edit game.fun and/or src\\main.rs, e.g. v1.5 of it. Then ./run it."),
            "Edit a file and/or a file, e.g. v1.5 of it. Then a file it."
        );
        // The engine's own words, fractions and pairs are not files.
        let kept = "Set Color.red on the client/server at 1/60 s, true/false. See functor.games now.";
        assert_eq!(for_the_ear(kept), kept);
        assert_eq!(for_the_ear("1. First step. 2. Second step. Third."), "1. First step. 2. Second step.");
        assert_eq!(for_the_ear("- one\n- two"), "one two");
        assert_eq!(for_the_ear("```\ncode\n```"), "");
        // Cut at a word, under the limit.
        let long = for_the_ear(&"word ".repeat(200));
        assert!(long.chars().count() <= MAX_CHARS && long.ends_with("word"));
    }

    #[test]
    fn the_agents_spoken_line_is_said_else_its_replys_first_sentences() {
        let marked = run(&[
            "Spoken: Looking at the ball.",
            "I set `ballColor` in game.fun.\n\n**Spoken:** The ball is red now, and it bounces slower.",
        ]);
        assert_eq!(
            spoken(&marked, 0, &Stage::Reloaded, false).as_deref(),
            Some("The ball is red now, and it bounces slower.")
        );
        // Its change waits for the user: said, once.
        assert_eq!(
            spoken(&marked, 0, &Stage::Reloaded, true).as_deref(),
            Some("The ball is red now, and it bounces slower. It's ready for you to look at.")
        );
        let asks = run(&["spoken: It is red and ready for you to look at."]);
        assert_eq!(
            spoken(&asks, 0, &Stage::Reloaded, true).as_deref(),
            Some("It is red and ready for you to look at.")
        );
        // No line (or an empty one) in the last reply: that reply, cleaned.
        let plain = run(&[
            "Spoken: First.",
            "The ball is **red** already. Nothing to change. More.\nSpoken:",
        ]);
        let answered = Stage::Answered("The ball is red already.".to_string());
        assert_eq!(
            spoken(&plain, 0, &answered, false).as_deref(),
            Some("The ball is red already. Nothing to change.")
        );
        // Only this run's replies count.
        let mut later = marked.clone();
        later.extend(run(&[]));
        assert_eq!(spoken(&later, 3, &Stage::Reloaded, false).as_deref(), Some("Done."));
        assert_eq!(
            spoken(&later, 3, &answered, false).as_deref(),
            Some("I have answered in the conversation.")
        );
    }

    #[test]
    fn a_failure_is_said_and_work_in_progress_is_not() {
        let items = run(&["Spoken: All done."]);
        let failed = Stage::Failed("the agent failed".to_string());
        assert_eq!(
            spoken(&items, 0, &failed, false).as_deref(),
            Some("That did not work: the agent failed")
        );
        assert_eq!(spoken(&items, 0, &Stage::Working, false), None);
    }

    #[test]
    fn the_mark_takes_emphasis_and_is_not_read_from_code() {
        for line in ["**Spoken**: It is red.", "*Spoken:* It is red.", "__SPOKEN:__ It is red."] {
            assert_eq!(strip_mark(line), Some("It is red."), "{line}");
        }
        assert_eq!(strip_mark("Spokenly: no"), None);
        let fenced = run(&["Spoken: It is red.\n\n```\nSpoken: a code line\n```"]);
        assert_eq!(
            spoken(&fenced, 0, &Stage::Reloaded, false).as_deref(),
            Some("It is red.")
        );
        // A file name whatever its case or kind.
        assert_eq!(for_the_ear("See Ball.PNG and notes.yaml."), "See a file and a file.");
    }
}
