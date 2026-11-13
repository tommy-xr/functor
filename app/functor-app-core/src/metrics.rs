//! A finished task run's numbers, for `functor-app task --metrics` (plan
//! 1.19): what the agent said and called, and the usage it reported.
//! Timings and why a run was cut off come from the shell.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::activity::{classify, CallKind};
use crate::tasks::{Harness, Item, Task};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunMetrics {
    pub harness: Harness,
    /// As the agent reported it.
    pub model: Option<String>,
    /// `ok`, `failed: <summary>` or `stopped`.
    pub ended: String,
    /// Agent text blocks.
    pub messages: u32,
    pub tool_calls: u32,
    /// Tool calls by the D34 kind.
    pub calls: BTreeMap<CallKind, u32>,
    /// Tool calls by name.
    pub tools: BTreeMap<String, u32>,
    /// Cumulative, as the agent reported it; input includes cached tokens.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// The last agent text block.
    pub answer: Option<String>,
}

/// `task`'s last run: the items after its last user message.
pub fn run_metrics(task: &Task) -> RunMetrics {
    let start = task
        .transcript
        .iter()
        .rposition(|i| matches!(i, Item::User { .. }))
        .map_or(0, |i| i + 1);
    let mut m = RunMetrics {
        harness: task.harness,
        model: task.model.clone(),
        ended: "unfinished".to_string(),
        messages: 0,
        tool_calls: 0,
        calls: BTreeMap::new(),
        tools: BTreeMap::new(),
        input_tokens: None,
        output_tokens: None,
        answer: None,
    };
    for item in &task.transcript[start..] {
        match item {
            Item::Agent { text, .. } => {
                m.messages += 1;
                m.answer = Some(text.clone());
            }
            Item::Tool { name, input, .. } => {
                m.tool_calls += 1;
                *m.calls.entry(classify(name, input).kind).or_default() += 1;
                *m.tools.entry(name.clone()).or_default() += 1;
            }
            Item::Usage { input, output } => {
                m.input_tokens = Some(*input);
                m.output_tokens = Some(*output);
            }
            Item::Finished { ok: true, .. } => m.ended = "ok".to_string(),
            Item::Finished { ok: false, summary } => {
                let why = summary.as_deref().unwrap_or("no summary");
                m.ended = format!("failed: {why}");
            }
            Item::Stopped => m.ended = "stopped".to_string(),
            _ => {}
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_run_counts_its_calls_by_kind_and_name() {
        let mut task = Task {
            id: 1,
            project: "a".into(),
            title: "old".into(),
            status: crate::tasks::TaskStatus::Done,
            harness: Harness::Claude,
            model: Some("m".into()),
            session_id: None,
            run: None,
            baseline: None,
            change: None,
            reverting: false,
            created_at: None,
            updated_at: None,
            transcript: vec![],
        };
        let tool = |name: &str, input| Item::Tool {
            name: name.into(),
            input,
            result: Some("r".into()),
        };
        task.transcript = vec![
            Item::User { text: "old".into() },
            tool("Read", json!({})),
            Item::Finished {
                ok: false,
                summary: None,
            },
            Item::User { text: "new".into() },
            tool("ToolSearch", json!({"query": "x"})),
            tool("mcp__mis__get_state", json!({})),
            tool("mcp__mis__get_state", json!({})),
            Item::Agent {
                text: "first".into(),
                partial: false,
            },
            Item::Agent {
                text: "last".into(),
                partial: false,
            },
            Item::Usage {
                input: 900,
                output: 30,
            },
            Item::Finished {
                ok: true,
                summary: None,
            },
        ];
        let m = run_metrics(&task);
        assert_eq!(m.ended, "ok");
        assert_eq!((m.messages, m.tool_calls), (2, 3));
        let calls = BTreeMap::from([(CallKind::Skills, 1), (CallKind::Game, 2)]);
        assert_eq!(m.calls, calls);
        assert_eq!(m.tools["mcp__mis__get_state"], 2);
        assert_eq!((m.input_tokens, m.output_tokens), (Some(900), Some(30)));
        assert_eq!(m.answer.as_deref(), Some("last"));
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["calls"], json!({"game": 2, "skills": 1}));
    }
}
