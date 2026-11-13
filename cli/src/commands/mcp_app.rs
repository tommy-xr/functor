//! `functor mcp --http --app-url <url>`: the view tools, which forward typed
//! requests to the embedding app (docs/mcp.md); with `--review <brief>`, a
//! review's tools instead, which read the brief and forward its report. This
//! server knows only their shapes; the app decides what they do and answers
//! in words for the agent.

use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::mcp::{ok_text, tool_error, FunctorMcp};

/// The environment variable holding the app endpoint's secret (never argv).
pub const APP_TOKEN_ENV: &str = "FUNCTOR_MCP_APP_TOKEN";

/// How long the app gets to answer a view request.
const APP_TIMEOUT: Duration = Duration::from_secs(5);
/// Most steps one walkthrough may have.
pub const MAX_STEPS: usize = 8;

/// Where view requests go: the app's loopback URL and its bearer secret.
#[derive(Clone)]
pub struct AppLink {
    pub url: String,
    pub token: String,
}

/// `--app-url` with the secret from the environment, or why not: the URL
/// must be plain HTTP to loopback, as the app's own server is.
pub fn app_link(url: &str) -> Result<AppLink, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("--app-url {url}: {e}"))?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if parsed.scheme() != "http" || !loopback {
        return Err(format!(
            "--app-url {url}: must be an http:// URL on 127.0.0.1, localhost or [::1]"
        ));
    }
    let token = super::mcp_http::secret_from_env(APP_TOKEN_ENV).ok_or_else(|| {
        format!("--app-url needs its secret: set {APP_TOKEN_ENV} to visible ASCII characters")
    })?;
    Ok(AppLink {
        url: url.to_string(),
        token,
    })
}

// A view in the app's canvas: a project file, this task's changes, or the
// game. Plain comments, not docs, so the schema the agent reads every turn
// carries only what `kind` does not already say.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppView {
    File {
        /// Relative to the project directory, e.g. `game.fun`.
        path: String,
        /// `[first, last]`, 1-based, inclusive: scrolled to and highlighted.
        #[serde(default)]
        #[schemars(schema_with = "line_range")]
        lines: Option<[u32; 2]>,
    },
    Changes {
        /// A changed file to scroll to, relative to the project directory.
        path: Option<String>,
    },
    Game,
}

/// `[first, last]` without the `uint32` noise `[u32; 2]` would add.
fn line_range(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["array", "null"],
        "items": { "type": "integer" },
        "minItems": 2,
        "maxItems": 2,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowViewArgs {
    pub view: AppView,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentStep {
    /// A few words, e.g. "The climb in ladder.fun".
    pub title: String,
    /// One or two sentences.
    pub note: Option<String>,
    pub view: AppView,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentArgs {
    /// 1 to 8 steps, in order.
    pub steps: Vec<PresentStep>,
    /// The whole change so far, in one paragraph of Simplified Technical
    /// English: sentences of at most 20 words, active voice, one word per thing.
    pub summary: Option<String>,
    /// What you did not verify, assumed, or changed as a side effect; `[]`
    /// when there is nothing.
    pub notes: Option<Vec<String>>,
    /// What you ran to check the change, and the result.
    #[serde(default)]
    pub evidence: Vec<PresentCheck>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresentCheck {
    /// e.g. `functor test`.
    pub ran: String,
    /// e.g. "14 expects pass".
    pub result: String,
    pub ok: bool,
}

/// Why `view` is malformed, before it is forwarded.
fn check_view(view: &AppView) -> Result<(), String> {
    match view {
        AppView::File {
            lines: Some([first, last]),
            ..
        } if *first == 0 || first > last => Err(format!(
            "lines [{first}, {last}]: give [first, last] with 1 <= first <= last"
        )),
        _ => Ok(()),
    }
}

#[tool_router(router = app_tool_router, vis = "pub(super)")]
impl FunctorMcp {
    /// Show the user one view in the Functor app: a project file (scrolled to
    /// and highlighting `lines`), this task's changes, or the game. It opens
    /// beside where the user is working, never moving their focus or cursor.
    /// Use it only to show something the user asked to see; at the end of a
    /// change, use `present` instead. The reply says what happened.
    #[tool]
    async fn show_view(
        &self,
        Parameters(args): Parameters<ShowViewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Err(e) = check_view(&args.view) {
            return tool_error(e);
        }
        self.forward_view(json!({ "op": "show", "view": args.view }))
            .await
    }

    /// Walk the user through your change: 1 to 8 ordered steps, each a short
    /// title, an optional note and a view (as in `show_view`) — the changed
    /// file ranges, then the game. It becomes one card in the chat that the
    /// user steps through; when your run ends, its first step opens beside
    /// their work. Call it once, at the end of a change, with `summary`,
    /// `notes` and `evidence`: the user decides from them. Returns at once.
    #[tool]
    async fn present(
        &self,
        Parameters(args): Parameters<PresentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.steps.is_empty() || args.steps.len() > MAX_STEPS {
            return tool_error(format!(
                "a walkthrough has 1 to {MAX_STEPS} steps, not {}",
                args.steps.len()
            ));
        }
        for (i, step) in args.steps.iter().enumerate() {
            if let Err(e) = check_view(&step.view) {
                return tool_error(format!("step {}: {e}", i + 1));
            }
        }
        self.forward_view(json!({
            "op": "present",
            "steps": args.steps,
            "summary": args.summary,
            "notes": args.notes,
            "evidence": args.evidence,
        }))
        .await
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportArgs {
    /// One paragraph in Simplified Technical English (ASD-STE100): what
    /// changed, its effect, and what to check.
    pub summary: String,
    /// What nobody verified, what was assumed, side effects; `[]` only when
    /// you checked and there is nothing.
    pub notes: Vec<String>,
    /// Checks that ran, with their real results.
    #[serde(default)]
    pub evidence: Vec<PresentCheck>,
    /// Highlights: 1 to 8 changed file ranges, only when `task_change` says
    /// the agent gave none.
    #[serde(default)]
    pub steps: Vec<PresentStep>,
}

#[tool_router(router = review_tool_router, vis = "pub(super)")]
impl FunctorMcp {
    /// The change to present: the user's request, what the agent did, each
    /// changed file with its line counts, the diff, and the evidence so far.
    #[tool]
    pub(super) async fn task_change(&self) -> Result<CallToolResult, ErrorData> {
        let Some(brief) = &self.review else {
            return tool_error("this server has no change to present");
        };
        match tokio::fs::read_to_string(brief).await {
            Ok(text) => ok_text(text),
            Err(e) => tool_error(format!("cannot read the change: {e}")),
        }
    }

    /// Write the decision page's report on the change: the user keeps or
    /// reverts it from this. Call it once, when you have checked what you
    /// can. The reply says if it was saved.
    #[tool]
    async fn report(
        &self,
        Parameters(args): Parameters<ReportArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if args.steps.len() > MAX_STEPS {
            return tool_error(format!("at most {MAX_STEPS} steps"));
        }
        for (i, step) in args.steps.iter().enumerate() {
            if let Err(e) = check_view(&step.view) {
                return tool_error(format!("step {}: {e}", i + 1));
            }
        }
        self.forward_view(json!({
            "op": "report",
            "summary": args.summary,
            "notes": args.notes,
            "evidence": args.evidence,
            "steps": args.steps,
        }))
        .await
    }
}

impl FunctorMcp {
    /// POSTs `body` to the app; its `message` is the tool's text, an error
    /// when `ok` is false.
    async fn forward_view(&self, body: Value) -> Result<CallToolResult, ErrorData> {
        let Some(app) = &self.app else {
            return tool_error("no app is attached to this server");
        };
        let sent = self
            .http
            .post(&app.url)
            .bearer_auth(&app.token)
            .timeout(APP_TIMEOUT)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(e) if e.is_timeout() => {
                let secs = APP_TIMEOUT.as_secs();
                return tool_error(format!("the app did not answer within {secs}s"));
            }
            Err(e) => return tool_error(format!("cannot reach the app: {e}")),
        };
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let reply: Option<Value> = serde_json::from_str(&text).ok();
        let message = reply.as_ref().and_then(|r| r["message"].as_str());
        match (
            status.is_success(),
            reply.as_ref().map(|r| &r["ok"]),
            message,
        ) {
            (true, Some(Value::Bool(true)), Some(message)) => ok_text(message),
            (true, Some(Value::Bool(false)), Some(message)) => tool_error(message),
            _ => {
                let text: String = text.chars().take(200).collect();
                tool_error(format!("the app answered {status}: {text}"))
            }
        }
    }
}
