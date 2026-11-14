//! The spoken reply to a spoken request (D47): one or two sentences for
//! the ear, said when the request reaches its outcome. The agent writes
//! them as a `Spoken:` line ending its last message; without one, its
//! reply's first sentences are used. The shell turns the text into sound.

use crate::tasks::Item;
use crate::voice::Stage;

/// What marks the agent's line for the ear.
const MARK: &str = "spoken:";
/// The most said.
const MAX_CHARS: usize = 280;
const MAX_SENTENCES: usize = 2;

/// Added to a spoken request's prompt: asks for the line.
pub const ASK: &str = "(Your answer is also read aloud in the headset. End your last message \
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
    let marked = replies()
        .flat_map(str::lines)
        .filter_map(|l| strip_mark(l.trim()))
        .last();
    let line = marked.or_else(|| replies().last()).map(for_the_ear);
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

/// `line` after its `Spoken:` mark, whatever its case or emphasis.
fn strip_mark(line: &str) -> Option<&str> {
    let line = line.trim_start_matches(['*', '_', '>', '-', ' ']);
    let head = line.get(..MARK.len())?;
    head.eq_ignore_ascii_case(MARK)
        .then(|| line[MARK.len()..].trim_start_matches(['*', '_', ' ']))
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
        if word.ends_with(['.', '!', '?']) {
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
    let parts = core.split('/').filter(|p| !p.is_empty()).count();
    let path = core.contains('\\') || (parts > 1 && !slash_word(&core));
    let said = match (link, path || file_name(&core)) {
        (true, _) => "a link",
        (_, true) => "a file",
        _ => &core,
    };
    Some(format!("{lead}{said}{tail}"))
}

/// "and/or", "w/o": words, not paths.
fn slash_word(word: &str) -> bool {
    word.split('/').all(|p| p.len() <= 3 && p.chars().all(char::is_alphabetic))
}

/// `game.fun`, `Cargo.toml`: a name, a dot, a short extension of letters.
fn file_name(word: &str) -> bool {
    match word.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (2..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_lowercase())
                && stem.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c))
                && !stem.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
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
            for_the_ear("Edit game.fun and/or src\\main.rs, e.g. v1.5 of it."),
            "Edit a file and/or a file, e.g. v1.5 of it."
        );
        assert_eq!(for_the_ear("- one\n- two"), "one two");
        assert_eq!(for_the_ear("```\ncode\n```"), "");
        // Cut at a word, under the limit.
        let long = for_the_ear(&"word ".repeat(200));
        assert!(long.chars().count() <= MAX_CHARS && long.ends_with("word"));
    }

    #[test]
    fn the_agents_spoken_line_is_said_else_its_replys_first_sentences() {
        let marked = run(&[
            "Looking at the ball.",
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
        // No line: the last reply, cleaned.
        let plain = run(&["First.", "The ball is **red** already. Nothing to change. More."]);
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
        let failed = Stage::Failed("the game stopped on the headset".to_string());
        assert_eq!(
            spoken(&items, 0, &failed, false).as_deref(),
            Some("That did not work: the game stopped on the headset")
        );
        assert_eq!(spoken(&items, 0, &Stage::Working, false), None);
    }
}
