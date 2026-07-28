//! Restricted, declarative automation source for `functor mcp`.
//!
//! This is intentionally not a JavaScript runtime. The parser accepts one
//! narrow JavaScript-shaped fluent expression and lowers it to plain data.
//! Nothing submitted by an MCP client is evaluated, imported, or called.

use std::fmt;

use serde::Serialize;
use serde_json::{Map, Value};

pub const AUTOMATION_DIALECT: &str = "functor-automation-poc-v1";
pub const MAX_SOURCE_BYTES: usize = 16 * 1024;
pub const MAX_STEPS: usize = 64;
pub const MAX_LITERAL_DEPTH: usize = 8;
pub const MAX_TOTAL_FRAMES: u32 = 10_000;
pub const MAX_CAPTURES: usize = 4;
const MAX_NAME_BYTES: usize = 80;
const MAX_LABEL_BYTES: usize = 80;
const MAX_KEY_BYTES: usize = 32;
const MAX_MODEL_PATH_BYTES: usize = 256;
const DEFAULT_DTS: f64 = 0.016;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AutomationPlan {
    pub version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub steps: Vec<AutomationStep>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationStep {
    Pause {
        #[serde(skip_serializing_if = "Option::is_none")]
        tts: Option<f64>,
    },
    Key {
        key: String,
        down: bool,
    },
    PressKey {
        key: String,
    },
    MouseMove {
        x: f64,
        y: f64,
    },
    MouseButton {
        button: String,
        down: bool,
    },
    MouseWheel {
        delta: f64,
    },
    UiClick {
        slot: u32,
    },
    Step {
        frames: u32,
        dts: f64,
    },
    Inspect {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    ExpectModel {
        path: String,
        equals: Value,
    },
    Capture {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl Diagnostic {
    fn at(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationLimits {
    pub source_bytes: usize,
    pub steps: usize,
    pub literal_depth: usize,
    pub total_frames: u32,
    pub captures: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutomationUsage {
    pub source_bytes: usize,
    pub canonical_source_bytes: usize,
    pub steps: usize,
    pub literal_depth: usize,
    pub total_frames: u32,
    pub captures: usize,
}

pub fn limits() -> AutomationLimits {
    AutomationLimits {
        source_bytes: MAX_SOURCE_BYTES,
        steps: MAX_STEPS,
        literal_depth: MAX_LITERAL_DEPTH,
        total_frames: MAX_TOTAL_FRAMES,
        captures: MAX_CAPTURES,
    }
}

pub fn parse_automation(source: &str) -> Result<AutomationPlan, Diagnostic> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Diagnostic::at(
            format!(
                "source is {} bytes; the {} dialect allows at most {MAX_SOURCE_BYTES}",
                source.len(),
                AUTOMATION_DIALECT
            ),
            1,
            1,
        ));
    }
    let plan = Parser::new(source)?.parse()?;
    let canonical_bytes = canonical_code(&plan).len();
    if canonical_bytes > MAX_SOURCE_BYTES {
        return Err(Diagnostic::at(
            format!(
                "canonical source is {canonical_bytes} bytes; the {AUTOMATION_DIALECT} limit is {MAX_SOURCE_BYTES}"
            ),
            1,
            1,
        ));
    }
    Ok(plan)
}

pub fn usage(source: &str, plan: &AutomationPlan) -> AutomationUsage {
    AutomationUsage {
        source_bytes: source.len(),
        canonical_source_bytes: canonical_code(plan).len(),
        steps: plan.steps.len(),
        literal_depth: plan
            .steps
            .iter()
            .map(|step| match step {
                AutomationStep::Step { .. } => 1,
                AutomationStep::ExpectModel { equals, .. } => literal_depth(equals),
                _ => 0,
            })
            .max()
            .unwrap_or(0),
        total_frames: plan
            .steps
            .iter()
            .map(|step| match step {
                AutomationStep::Step { frames, .. } => *frames,
                AutomationStep::PressKey { .. } => 1,
                _ => 0,
            })
            .sum(),
        captures: plan
            .steps
            .iter()
            .filter(|step| matches!(step, AutomationStep::Capture { .. }))
            .count(),
    }
}

fn literal_depth(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(literal_depth).max().unwrap_or(0),
        Value::Object(fields) => 1 + fields.values().map(literal_depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// Deterministically regenerate source in the same restricted fluent dialect.
/// The validation tool returns this so an agent can inspect the normalized
/// lowering, edit it, and submit it again.
pub fn canonical_code(plan: &AutomationPlan) -> String {
    let mut code = match &plan.name {
        Some(name) => format!("automation({})", quote(name)),
        None => "automation()".to_owned(),
    };
    for step in &plan.steps {
        code.push_str("\n  .");
        code.push_str(&match step {
            AutomationStep::Pause { tts: None } => "pause()".to_owned(),
            AutomationStep::Pause { tts: Some(tts) } => {
                format!("pause({})", number(*tts))
            }
            AutomationStep::Key { key, down: true } => format!("keyDown({})", quote(key)),
            AutomationStep::Key { key, down: false } => format!("keyUp({})", quote(key)),
            AutomationStep::PressKey { key } => format!("pressKey({})", quote(key)),
            AutomationStep::MouseMove { x, y } => {
                format!("mouseMove({}, {})", number(*x), number(*y))
            }
            AutomationStep::MouseButton { button, down: true } => {
                format!("mouseDown({})", quote(button))
            }
            AutomationStep::MouseButton {
                button,
                down: false,
            } => format!("mouseUp({})", quote(button)),
            AutomationStep::MouseWheel { delta } => {
                format!("mouseWheel({})", number(*delta))
            }
            AutomationStep::UiClick { slot } => format!("uiClick({slot})"),
            AutomationStep::Step { frames, dts } => {
                format!("step({{ frames: {frames}, dts: {} }})", number(*dts))
            }
            AutomationStep::Inspect { label: None } => "inspect()".to_owned(),
            AutomationStep::Inspect { label: Some(label) } => {
                format!("inspect({})", quote(label))
            }
            AutomationStep::ExpectModel { path, equals } => format!(
                "expectModel({}, {})",
                quote(path),
                serde_json::to_string(equals).expect("a JSON value serializes")
            ),
            AutomationStep::Capture { label: None } => "capture()".to_owned(),
            AutomationStep::Capture { label: Some(label) } => {
                format!("capture({})", quote(label))
            }
        });
    }
    code.push_str(";\n");
    code
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("a string serializes")
}

fn number(value: f64) -> String {
    serde_json::to_string(&value).expect("the parser rejects non-finite numbers")
}

/// Read a plain model path. A dotted path (`camera.yaw`) is the convenient
/// form; RFC 6901 JSON Pointer (`/camera/yaw`) is available for keys containing
/// dots. An empty path names the complete model.
pub fn model_value_at<'a>(model: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(model);
    }
    if path.starts_with('/') {
        return model.pointer(path);
    }
    path.split('.')
        .try_fold(model, |value, segment| match value {
            Value::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            Value::Object(fields) => fields.get(segment),
            _ => None,
        })
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Identifier(String),
    String(String),
    Number(f64),
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Dot,
    Comma,
    Colon,
    Semicolon,
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    line: usize,
    column: usize,
}

struct Lexer<'a> {
    source: &'a str,
    index: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            index: 0,
            line: 1,
            column: 1,
        }
    }

    fn next(&mut self) -> Result<Token, Diagnostic> {
        self.skip_trivia()?;
        let line = self.line;
        let column = self.column;
        let Some(ch) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                line,
                column,
            });
        };
        let kind = match ch {
            '(' => {
                self.bump();
                TokenKind::LeftParen
            }
            ')' => {
                self.bump();
                TokenKind::RightParen
            }
            '{' => {
                self.bump();
                TokenKind::LeftBrace
            }
            '}' => {
                self.bump();
                TokenKind::RightBrace
            }
            '[' => {
                self.bump();
                TokenKind::LeftBracket
            }
            ']' => {
                self.bump();
                TokenKind::RightBracket
            }
            '.' => {
                self.bump();
                TokenKind::Dot
            }
            ',' => {
                self.bump();
                TokenKind::Comma
            }
            ':' => {
                self.bump();
                TokenKind::Colon
            }
            ';' => {
                self.bump();
                TokenKind::Semicolon
            }
            '"' | '\'' => TokenKind::String(self.string(ch)?),
            '-' | '0'..='9' => TokenKind::Number(self.number()?),
            ch if is_identifier_start(ch) => TokenKind::Identifier(self.identifier()),
            other => {
                return Err(Diagnostic::at(
                    format!(
                        "unexpected {other:?}; this is a restricted declarative dialect, not arbitrary JavaScript"
                    ),
                    line,
                    column,
                ))
            }
        };
        Ok(Token { kind, line, column })
    }

    fn skip_trivia(&mut self) -> Result<(), Diagnostic> {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if self.source[self.index..].starts_with("//") {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.bump();
                }
                continue;
            }
            if self.source[self.index..].starts_with("/*") {
                let line = self.line;
                let column = self.column;
                self.bump();
                self.bump();
                while !self.source[self.index..].starts_with("*/") {
                    if self.bump().is_none() {
                        return Err(Diagnostic::at("unterminated block comment", line, column));
                    }
                }
                self.bump();
                self.bump();
                continue;
            }
            return Ok(());
        }
    }

    fn identifier(&mut self) -> String {
        let start = self.index;
        self.bump();
        while self.peek().is_some_and(is_identifier_continue) {
            self.bump();
        }
        self.source[start..self.index].to_owned()
    }

    fn number(&mut self) -> Result<f64, Diagnostic> {
        let start = self.index;
        let line = self.line;
        let column = self.column;
        if self.peek() == Some('-') {
            self.bump();
        }
        let mut digits = 0;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            digits += 1;
            self.bump();
        }
        if digits == 0 {
            return Err(Diagnostic::at("expected digits after '-'", line, column));
        }
        if self.peek() == Some('.') {
            self.bump();
            let mut fraction_digits = 0;
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                fraction_digits += 1;
                self.bump();
            }
            if fraction_digits == 0 {
                return Err(Diagnostic::at(
                    "expected digits after the decimal point",
                    line,
                    column,
                ));
            }
        }
        if self.peek().is_some_and(|ch| matches!(ch, 'e' | 'E')) {
            self.bump();
            if self.peek().is_some_and(|ch| matches!(ch, '+' | '-')) {
                self.bump();
            }
            let mut exponent_digits = 0;
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                exponent_digits += 1;
                self.bump();
            }
            if exponent_digits == 0 {
                return Err(Diagnostic::at(
                    "expected an exponent after 'e'",
                    line,
                    column,
                ));
            }
        }
        let text = &self.source[start..self.index];
        let value = text
            .parse::<f64>()
            .map_err(|_| Diagnostic::at(format!("invalid number {text:?}"), line, column))?;
        if !value.is_finite() {
            return Err(Diagnostic::at("numbers must be finite", line, column));
        }
        Ok(value)
    }

    fn string(&mut self, quote: char) -> Result<String, Diagnostic> {
        let line = self.line;
        let column = self.column;
        self.bump();
        let mut result = String::new();
        loop {
            let Some(ch) = self.bump() else {
                return Err(Diagnostic::at("unterminated string", line, column));
            };
            match ch {
                ch if ch == quote => return Ok(result),
                '\n' | '\r' => {
                    return Err(Diagnostic::at(
                        "a string literal cannot cross a line",
                        line,
                        column,
                    ))
                }
                '\\' => {
                    let Some(escape) = self.bump() else {
                        return Err(Diagnostic::at("unterminated string escape", line, column));
                    };
                    match escape {
                        '\\' => result.push('\\'),
                        '\'' => result.push('\''),
                        '"' => result.push('"'),
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        'b' => result.push('\u{0008}'),
                        'f' => result.push('\u{000c}'),
                        'u' => result.push(self.unicode_escape(line, column)?),
                        other => {
                            return Err(Diagnostic::at(
                                format!("unsupported string escape \\\\{other}"),
                                self.line,
                                self.column.saturating_sub(1),
                            ))
                        }
                    }
                }
                other => result.push(other),
            }
        }
    }

    fn unicode_escape(&mut self, line: usize, column: usize) -> Result<char, Diagnostic> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return Err(Diagnostic::at("unterminated unicode escape", line, column));
            };
            let Some(digit) = ch.to_digit(16) else {
                return Err(Diagnostic::at(
                    "unicode escapes require exactly four hexadecimal digits",
                    self.line,
                    self.column.saturating_sub(1),
                ));
            };
            value = value * 16 + digit;
        }
        char::from_u32(value).ok_or_else(|| {
            Diagnostic::at(
                "unicode surrogate escapes are not supported; write the character directly",
                line,
                column,
            )
        })
    }

    fn peek(&self) -> Option<char> {
        self.source[self.index..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    total_frames: u32,
    captures: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Result<Self, Diagnostic> {
        let mut lexer = Lexer::new(source);
        let current = lexer.next()?;
        Ok(Self {
            lexer,
            current,
            total_frames: 0,
            captures: 0,
        })
    }

    fn parse(mut self) -> Result<AutomationPlan, Diagnostic> {
        let root = self.take_identifier()?;
        if root != "automation" {
            let message = if is_forbidden_name(&root) {
                format!(
                    "{root} is not allowed; submit one automation(...).method(...) chain with no imports, variables, callbacks, loops, async code, or globals"
                )
            } else {
                format!(
                    "expected automation(...), found {root}; arbitrary calls and variables are not allowed"
                )
            };
            return Err(self.error_here(message));
        }
        self.expect(TokenKind::LeftParen, "'(' after automation")?;
        let name = if self.at(&TokenKind::RightParen) {
            None
        } else {
            let token = self.current.clone();
            let name = self.take_string("automation's optional name must be a string literal")?;
            check_text_len("automation name", &name, MAX_NAME_BYTES, &token)?;
            if name.is_empty() {
                return Err(Diagnostic::at(
                    "automation name cannot be empty",
                    token.line,
                    token.column,
                ));
            }
            Some(name)
        };
        self.expect(
            TokenKind::RightParen,
            "')' after automation's optional name",
        )?;

        let mut steps = Vec::new();
        while self.at(&TokenKind::Dot) {
            self.advance()?;
            let method_token = self.current.clone();
            let method = self.take_identifier()?;
            self.expect(TokenKind::LeftParen, "'(' after the method name")?;
            let step = self.parse_method(&method, &method_token)?;
            self.expect(TokenKind::RightParen, "')' after the method arguments")?;
            steps.push(step);
            if steps.len() > MAX_STEPS {
                return Err(Diagnostic::at(
                    format!("a plan may contain at most {MAX_STEPS} steps"),
                    method_token.line,
                    method_token.column,
                ));
            }
        }
        if self.at(&TokenKind::Semicolon) {
            self.advance()?;
        }
        if !self.at(&TokenKind::Eof) {
            return Err(self.error_here(
                "expected the end of the automation chain; assignments, dynamic properties, callbacks, and control flow are not allowed",
            ));
        }
        if steps.is_empty() {
            return Err(Diagnostic::at(
                "an automation plan must contain at least one method step",
                1,
                1,
            ));
        }
        Ok(AutomationPlan {
            version: 1,
            name,
            steps,
        })
    }

    fn parse_method(
        &mut self,
        method: &str,
        method_token: &Token,
    ) -> Result<AutomationStep, Diagnostic> {
        match method {
            "pause" => {
                let tts = if self.at(&TokenKind::RightParen) {
                    None
                } else {
                    Some(self.take_number("pause(tts) requires a numeric literal")?)
                };
                if tts.is_some_and(|value| value < 0.0) {
                    return Err(Diagnostic::at(
                        "pause time must be non-negative",
                        method_token.line,
                        method_token.column,
                    ));
                }
                Ok(AutomationStep::Pause { tts })
            }
            "keyDown" | "keyUp" | "pressKey" => {
                let token = self.current.clone();
                let key =
                    self.take_string("keyDown/keyUp/pressKey require a literal key name")?;
                check_text_len("key name", &key, MAX_KEY_BYTES, &token)?;
                if key.is_empty() {
                    return Err(Diagnostic::at(
                        "key name cannot be empty",
                        token.line,
                        token.column,
                    ));
                }
                if method == "pressKey" {
                    self.total_frames += 1;
                    if self.total_frames > MAX_TOTAL_FRAMES {
                        return Err(Diagnostic::at(
                            format!(
                                "the plan requests {} total frames; the PoC budget is {MAX_TOTAL_FRAMES}",
                                self.total_frames
                            ),
                            method_token.line,
                            method_token.column,
                        ));
                    }
                    Ok(AutomationStep::PressKey { key })
                } else {
                    Ok(AutomationStep::Key {
                        key,
                        down: method == "keyDown",
                    })
                }
            }
            "mouseMove" => {
                let x = self.take_number("mouseMove(x, y) requires numeric literals")?;
                self.expect(TokenKind::Comma, "',' between mouseMove coordinates")?;
                let y = self.take_number("mouseMove(x, y) requires numeric literals")?;
                for (name, value) in [("x", x), ("y", y)] {
                    if value.abs() > 1_000_000.0 {
                        return Err(Diagnostic::at(
                            format!("mouse {name} is outside the ±1,000,000 PoC bound"),
                            method_token.line,
                            method_token.column,
                        ));
                    }
                }
                Ok(AutomationStep::MouseMove { x, y })
            }
            "mouseDown" | "mouseUp" => {
                let button = if self.at(&TokenKind::RightParen) {
                    "left".to_owned()
                } else {
                    self.take_string("mouseDown/mouseUp require a literal button name")?
                };
                if !matches!(button.as_str(), "left" | "right" | "middle") {
                    return Err(Diagnostic::at(
                        "mouse button must be \"left\", \"right\", or \"middle\"",
                        method_token.line,
                        method_token.column,
                    ));
                }
                Ok(AutomationStep::MouseButton {
                    button,
                    down: method == "mouseDown",
                })
            }
            "mouseWheel" => {
                let delta = self.take_number("mouseWheel(delta) requires a numeric literal")?;
                if delta.abs() > 1_000_000.0 {
                    return Err(Diagnostic::at(
                        "mouse wheel delta is outside the ±1,000,000 PoC bound",
                        method_token.line,
                        method_token.column,
                    ));
                }
                Ok(AutomationStep::MouseWheel { delta })
            }
            "uiClick" => {
                let value = self.take_number("uiClick(slot) requires an integer literal")?;
                let slot = bounded_u32(value, "UI slot", u32::MAX, method_token)?;
                Ok(AutomationStep::UiClick { slot })
            }
            "step" => {
                let (frames, dts) = if self.at(&TokenKind::RightParen) {
                    (1, DEFAULT_DTS)
                } else {
                    let options = self.parse_literal(0)?;
                    parse_step_options(options, method_token)?
                };
                self.total_frames = self.total_frames.checked_add(frames).ok_or_else(|| {
                    Diagnostic::at(
                        "total step frame count overflowed",
                        method_token.line,
                        method_token.column,
                    )
                })?;
                if self.total_frames > MAX_TOTAL_FRAMES {
                    return Err(Diagnostic::at(
                        format!(
                            "the plan requests {} total frames; the PoC budget is {MAX_TOTAL_FRAMES}",
                            self.total_frames
                        ),
                        method_token.line,
                        method_token.column,
                    ));
                }
                Ok(AutomationStep::Step { frames, dts })
            }
            "inspect" => {
                let label = self.optional_label(method_token)?;
                Ok(AutomationStep::Inspect { label })
            }
            "expectModel" => {
                let path_token = self.current.clone();
                let path =
                    self.take_string("expectModel(path, equals) requires a literal model path")?;
                check_text_len("model path", &path, MAX_MODEL_PATH_BYTES, &path_token)?;
                validate_model_path(&path, &path_token)?;
                self.expect(TokenKind::Comma, "',' between model path and expected value")?;
                let equals = self.parse_literal(0)?;
                Ok(AutomationStep::ExpectModel { path, equals })
            }
            "capture" => {
                self.captures += 1;
                if self.captures > MAX_CAPTURES {
                    return Err(Diagnostic::at(
                        format!("a plan may contain at most {MAX_CAPTURES} captures"),
                        method_token.line,
                        method_token.column,
                    ));
                }
                let label = self.optional_label(method_token)?;
                Ok(AutomationStep::Capture { label })
            }
            _ => Err(Diagnostic::at(
                format!(
                    "unknown automation method {method:?}; allowed methods are pause, keyDown, keyUp, pressKey, mouseMove, mouseDown, mouseUp, mouseWheel, uiClick, step, inspect, expectModel, capture"
                ),
                method_token.line,
                method_token.column,
            )),
        }
    }

    fn optional_label(&mut self, method_token: &Token) -> Result<Option<String>, Diagnostic> {
        if self.at(&TokenKind::RightParen) {
            return Ok(None);
        }
        let token = self.current.clone();
        let label = self.take_string("the optional label must be a string literal")?;
        check_text_len("label", &label, MAX_LABEL_BYTES, &token)?;
        if label.is_empty() {
            return Err(Diagnostic::at(
                "label cannot be empty",
                method_token.line,
                method_token.column,
            ));
        }
        Ok(Some(label))
    }

    fn parse_literal(&mut self, depth: usize) -> Result<Value, Diagnostic> {
        if depth > MAX_LITERAL_DEPTH {
            return Err(self.error_here(format!(
                "JSON literal nesting exceeds the maximum depth of {MAX_LITERAL_DEPTH}"
            )));
        }
        let token = self.current.clone();
        match token.kind {
            TokenKind::String(value) => {
                self.advance()?;
                Ok(Value::String(value))
            }
            TokenKind::Number(value) => {
                self.advance()?;
                Ok(Value::Number(
                    serde_json::Number::from_f64(value)
                        .expect("the lexer rejects non-finite numbers"),
                ))
            }
            TokenKind::Identifier(ref value) if value == "true" => {
                self.advance()?;
                Ok(Value::Bool(true))
            }
            TokenKind::Identifier(ref value) if value == "false" => {
                self.advance()?;
                Ok(Value::Bool(false))
            }
            TokenKind::Identifier(ref value) if value == "null" => {
                self.advance()?;
                Ok(Value::Null)
            }
            TokenKind::LeftBracket => {
                self.advance()?;
                let mut items = Vec::new();
                if !self.at(&TokenKind::RightBracket) {
                    loop {
                        items.push(self.parse_literal(depth + 1)?);
                        if !self.at(&TokenKind::Comma) {
                            break;
                        }
                        self.advance()?;
                    }
                }
                self.expect(TokenKind::RightBracket, "']' after the array literal")?;
                Ok(Value::Array(items))
            }
            TokenKind::LeftBrace => {
                self.advance()?;
                let mut fields = Map::new();
                if !self.at(&TokenKind::RightBrace) {
                    loop {
                        let key_token = self.current.clone();
                        let key = match key_token.kind {
                            TokenKind::Identifier(key) | TokenKind::String(key) => {
                                self.advance()?;
                                key
                            }
                            _ => {
                                return Err(self.error_here(
                                    "object keys must be literal identifiers or strings",
                                ))
                            }
                        };
                        self.expect(TokenKind::Colon, "':' after the object key")?;
                        let value = self.parse_literal(depth + 1)?;
                        if fields.insert(key.clone(), value).is_some() {
                            return Err(Diagnostic::at(
                                format!("duplicate object key {key:?}"),
                                key_token.line,
                                key_token.column,
                            ));
                        }
                        if !self.at(&TokenKind::Comma) {
                            break;
                        }
                        self.advance()?;
                    }
                }
                self.expect(TokenKind::RightBrace, "'}' after the object literal")?;
                Ok(Value::Object(fields))
            }
            TokenKind::Identifier(value) if is_forbidden_name(&value) => Err(Diagnostic::at(
                format!(
                    "{value} is not available; submitted automation code can contain only literal data and allowlisted builder calls"
                ),
                token.line,
                token.column,
            )),
            _ => Err(Diagnostic::at(
                "expected a JSON literal; variables, functions, callbacks, expressions, and dynamic properties are not allowed",
                token.line,
                token.column,
            )),
        }
    }

    fn take_identifier(&mut self) -> Result<String, Diagnostic> {
        let token = self.current.clone();
        match token.kind {
            TokenKind::Identifier(value) => {
                self.advance()?;
                Ok(value)
            }
            _ => Err(Diagnostic::at(
                "expected an identifier",
                token.line,
                token.column,
            )),
        }
    }

    fn take_string(&mut self, message: &str) -> Result<String, Diagnostic> {
        let token = self.current.clone();
        match token.kind {
            TokenKind::String(value) => {
                self.advance()?;
                Ok(value)
            }
            _ => Err(Diagnostic::at(message, token.line, token.column)),
        }
    }

    fn take_number(&mut self, message: &str) -> Result<f64, Diagnostic> {
        let token = self.current.clone();
        match token.kind {
            TokenKind::Number(value) => {
                self.advance()?;
                Ok(value)
            }
            _ => Err(Diagnostic::at(message, token.line, token.column)),
        }
    }

    fn expect(&mut self, expected: TokenKind, message: &str) -> Result<(), Diagnostic> {
        if self.at(&expected) {
            self.advance()
        } else {
            Err(self.error_here(format!("{message}, found {}", self.describe_current())))
        }
    }

    fn at(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.current.kind) == std::mem::discriminant(kind)
    }

    fn advance(&mut self) -> Result<(), Diagnostic> {
        self.current = self.lexer.next()?;
        Ok(())
    }

    fn error_here(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::at(message, self.current.line, self.current.column)
    }

    fn describe_current(&self) -> String {
        match &self.current.kind {
            TokenKind::Identifier(value) => format!("identifier {value:?}"),
            TokenKind::String(_) => "a string".into(),
            TokenKind::Number(value) => format!("number {value}"),
            TokenKind::Eof => "end of source".into(),
            other => format!("{other:?}"),
        }
    }
}

fn parse_step_options(value: Value, token: &Token) -> Result<(u32, f64), Diagnostic> {
    let Value::Object(mut options) = value else {
        return Err(Diagnostic::at(
            "step takes no arguments or one literal object: step({ frames: 2, dts: 0.016 })",
            token.line,
            token.column,
        ));
    };
    let frames = match options.remove("frames") {
        Some(Value::Number(value)) => bounded_u32(
            value.as_f64().expect("JSON numbers are finite"),
            "step frames",
            MAX_TOTAL_FRAMES,
            token,
        )?,
        Some(_) => {
            return Err(Diagnostic::at(
                "step frames must be an integer literal",
                token.line,
                token.column,
            ))
        }
        None => 1,
    };
    if frames == 0 {
        return Err(Diagnostic::at(
            "step frames must be at least 1",
            token.line,
            token.column,
        ));
    }
    let dts = match options.remove("dts") {
        Some(Value::Number(value)) => value.as_f64().expect("JSON numbers are finite"),
        Some(_) => {
            return Err(Diagnostic::at(
                "step dts must be a numeric literal",
                token.line,
                token.column,
            ))
        }
        None => DEFAULT_DTS,
    };
    if !(0.0 < dts && dts <= 1.0) {
        return Err(Diagnostic::at(
            "step dts must be greater than 0 and at most 1 second",
            token.line,
            token.column,
        ));
    }
    if let Some(unknown) = options.keys().next() {
        return Err(Diagnostic::at(
            format!("unknown step option {unknown:?}; expected frames and/or dts"),
            token.line,
            token.column,
        ));
    }
    Ok((frames, dts))
}

fn bounded_u32(value: f64, name: &str, max: u32, token: &Token) -> Result<u32, Diagnostic> {
    if value.fract() != 0.0 || value < 0.0 || value > max as f64 {
        return Err(Diagnostic::at(
            format!("{name} must be an integer from 0 through {max}"),
            token.line,
            token.column,
        ));
    }
    Ok(value as u32)
}

fn check_text_len(name: &str, text: &str, max: usize, token: &Token) -> Result<(), Diagnostic> {
    if text.len() > max {
        return Err(Diagnostic::at(
            format!("{name} is {} bytes; the maximum is {max}", text.len()),
            token.line,
            token.column,
        ));
    }
    Ok(())
}

fn validate_model_path(path: &str, token: &Token) -> Result<(), Diagnostic> {
    if path.is_empty() {
        return Ok(());
    }
    if path.starts_with('/') {
        return Ok(());
    }
    if path.split('.').any(str::is_empty) {
        return Err(Diagnostic::at(
            "a dotted model path cannot contain an empty segment; use JSON Pointer for unusual keys",
            token.line,
            token.column,
        ));
    }
    if path
        .split('.')
        .any(|segment| matches!(segment, "__proto__" | "prototype" | "constructor"))
    {
        return Err(Diagnostic::at(
            "prototype-related path segments are not accepted",
            token.line,
            token.column,
        ));
    }
    Ok(())
}

fn is_forbidden_name(name: &str) -> bool {
    matches!(
        name,
        "import"
            | "export"
            | "const"
            | "let"
            | "var"
            | "function"
            | "async"
            | "await"
            | "for"
            | "while"
            | "do"
            | "class"
            | "new"
            | "eval"
            | "Function"
            | "require"
            | "process"
            | "global"
            | "globalThis"
            | "window"
            | "document"
            | "fetch"
            | "setTimeout"
            | "setInterval"
            | "queueMicrotask"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_code, model_value_at, parse_automation, usage, AutomationStep, MAX_CAPTURES,
        MAX_LITERAL_DEPTH, MAX_SOURCE_BYTES, MAX_STEPS,
    };
    use serde_json::json;

    #[test]
    fn parses_the_jam_driving_loop_into_plain_data() {
        let plan = parse_automation(
            r#"
// This is source-shaped data, not evaluated JavaScript.
automation("photo mouse-look proof")
  .pause(2)
  .mouseMove(400, 300)
  .mouseMove(600, 200)
  .step({ frames: 2, dts: 0.016 })
  .expectModel("yawOffset", -0.6)
  .inspect("after mouse look")
  .capture("proof");
"#,
        )
        .unwrap();

        assert_eq!(plan.version, 1);
        assert_eq!(plan.name.as_deref(), Some("photo mouse-look proof"));
        assert_eq!(plan.steps.len(), 7);
        assert_eq!(
            plan.steps[3],
            AutomationStep::Step {
                frames: 2,
                dts: 0.016
            }
        );
        assert_eq!(
            serde_json::to_value(&plan).unwrap()["steps"][4],
            json!({"type":"expect_model","path":"yawOffset","equals":-0.6})
        );
    }

    #[test]
    fn accepts_literal_assertions_and_every_input_shortcut() {
        let plan = parse_automation(
            r#"automation()
              .keyDown("3").keyUp("3").pressKey("r")
              .mouseDown().mouseUp("right").mouseWheel(-1)
              .uiClick(0)
              .expectModel("", { ready: true, values: [1, null, "ok"] });"#,
        )
        .unwrap();
        assert_eq!(plan.steps.len(), 8);
        assert_eq!(usage("ignored", &plan).total_frames, 1);
    }

    #[test]
    fn canonical_source_round_trips_to_the_identical_plan() {
        let source = r#"automation('round trip')
          .pause()
          .pressKey("2")
          .step({dts: 0.02, frames: 3})
          .expectModel("/stats/kills", { exact: 8, tags: ["jam", true] })
          .capture('result')"#;
        let plan = parse_automation(source).unwrap();
        let canonical = canonical_code(&plan);
        let reparsed = parse_automation(&canonical).unwrap();

        assert_eq!(reparsed, plan);
        assert!(canonical.starts_with("automation(\"round trip\")\n"));
        assert!(canonical.contains(".step({ frames: 3, dts: 0.02 })"));
    }

    #[test]
    fn rejects_code_execution_constructs_and_unknown_calls() {
        let rejected = [
            "import { automation } from '@functor/sdk'; automation().pause()",
            "const plan = automation().pause()",
            "async () => automation().pause()",
            "for (;;) automation().step()",
            "while (true) automation().step()",
            "new Function('return process')()",
            "eval('automation().pause()')",
            "require('fs')",
            "process.exit()",
            "globalThis.fetch('https://example.com')",
            "fetch('https://example.com')",
            "setTimeout(() => {}, 1)",
            "automation().step().then(() => process.exit())",
            "automation()['pause']()",
            "automation().unknown()",
            "automation().expectModel('n', process.env.N)",
        ];
        for source in rejected {
            assert!(
                parse_automation(source).is_err(),
                "must reject submitted code: {source}"
            );
        }
    }

    #[test]
    fn reports_line_and_column_for_a_bad_method() {
        let error = parse_automation("automation()\n  .pause()\n  .launchMissiles()").unwrap_err();
        assert_eq!((error.line, error.column), (3, 4));
        assert!(
            error.message.contains("unknown automation method"),
            "{error}"
        );
    }

    #[test]
    fn enforces_source_step_depth_frame_and_capture_budgets() {
        let too_large = " ".repeat(MAX_SOURCE_BYTES + 1);
        assert!(parse_automation(&too_large)
            .unwrap_err()
            .message
            .contains("source"));

        // A single-quoted input can contain double quotes without escapes, but
        // canonical JSON-style strings must escape them. A successful
        // validation promises its canonical_code is itself resubmittable.
        let expands_when_canonical =
            format!("automation().expectModel('x', '{}')", "\"".repeat(9_000));
        assert!(parse_automation(&expands_when_canonical)
            .unwrap_err()
            .message
            .contains("canonical source"));

        let too_many_steps = format!("automation(){}", ".inspect()".repeat(MAX_STEPS + 1));
        assert!(parse_automation(&too_many_steps)
            .unwrap_err()
            .message
            .contains("steps"));

        let mut nested = "true".to_owned();
        for _ in 0..=MAX_LITERAL_DEPTH {
            nested = format!("[{nested}]");
        }
        assert!(
            parse_automation(&format!("automation().expectModel('x', {nested})"))
                .unwrap_err()
                .message
                .contains("depth")
        );

        assert!(parse_automation("automation().step({frames:10000}).step()")
            .unwrap_err()
            .message
            .contains("total frames"));

        let too_many_captures = format!("automation(){}", ".capture()".repeat(MAX_CAPTURES + 1));
        assert!(parse_automation(&too_many_captures)
            .unwrap_err()
            .message
            .contains("captures"));
    }

    #[test]
    fn model_paths_support_convenient_dots_and_precise_json_pointer() {
        let model = json!({
            "camera": {"yaw": -0.6},
            "players": [{"score": 3}],
            "odd.key": {"value": true}
        });
        assert_eq!(model_value_at(&model, "camera.yaw"), Some(&json!(-0.6)));
        assert_eq!(model_value_at(&model, "players.0.score"), Some(&json!(3)));
        assert_eq!(model_value_at(&model, "/odd.key/value"), Some(&json!(true)));
        assert_eq!(model_value_at(&model, "missing"), None);
    }
}
