//! THROWAWAY spike — docs/mle.md Milestone 0.
//!
//! The smallest possible MLE-ish interpreter embedded straight into the runner,
//! to answer two questions before any real language investment:
//!
//!   1. Can a tree-walking interpreter run per-frame game logic (tick + draw
//!      for ~50 entities) inside a 60fps frame budget?
//!   2. What does hot-reload latency look like when a logic edit is a re-parse
//!      + global rebind instead of a cargo rebuild + dylib reload?
//!
//! Run with `functor-runner --mle --game-path <file.mle>`. Per-frame eval cost
//! is printed every 300 frames; a reload prints its re-parse latency. Set
//! `MLE_SPIKE_BENCH=<iters>` to run tick+draw in a tight loop at startup and
//! exit (no window needed).
//!
//! Deliberately not real: no types, no effects, no diagnostics beyond a line
//! number, unsophisticated parsing. Numbers are all f64. The model persists
//! across reloads; functions rebind by name (late-bound globals), which is the
//! rebind-on-reload semantics chosen in the MLE design notes.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

use cgmath::{Matrix4, SquareMatrix};
use functor_runtime_common::math::Angle;
use functor_runtime_common::ui::View;
use functor_runtime_common::{Camera, Frame, FrameTime, MaterialDescription, Scene3D, SceneObject};

use crate::game::Game;

// --- Lexer ---

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Let,
    If,
    Then,
    Else,
    True,
    False,
    // punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    Arrow,  // =>
    Eq,     // =
    EqEq,   // ==
    Lt,
    Gt,
    Plus,
    Minus,
    Star,
    Slash,
}

fn lex(src: &str) -> Result<Vec<(Tok, u32)>, String> {
    let mut toks = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut line: u32 = 1;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            '(' => {
                toks.push((Tok::LParen, line));
                i += 1;
            }
            ')' => {
                toks.push((Tok::RParen, line));
                i += 1;
            }
            '{' => {
                toks.push((Tok::LBrace, line));
                i += 1;
            }
            '}' => {
                toks.push((Tok::RBrace, line));
                i += 1;
            }
            '[' => {
                toks.push((Tok::LBracket, line));
                i += 1;
            }
            ']' => {
                toks.push((Tok::RBracket, line));
                i += 1;
            }
            ',' => {
                toks.push((Tok::Comma, line));
                i += 1;
            }
            ':' => {
                toks.push((Tok::Colon, line));
                i += 1;
            }
            '.' => {
                toks.push((Tok::Dot, line));
                i += 1;
            }
            '+' => {
                toks.push((Tok::Plus, line));
                i += 1;
            }
            '-' => {
                toks.push((Tok::Minus, line));
                i += 1;
            }
            '*' => {
                toks.push((Tok::Star, line));
                i += 1;
            }
            '/' => {
                toks.push((Tok::Slash, line));
                i += 1;
            }
            '<' => {
                toks.push((Tok::Lt, line));
                i += 1;
            }
            '>' => {
                toks.push((Tok::Gt, line));
                i += 1;
            }
            '=' => {
                if bytes.get(i + 1) == Some(&b'>') {
                    toks.push((Tok::Arrow, line));
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'=') {
                    toks.push((Tok::EqEq, line));
                    i += 2;
                } else {
                    toks.push((Tok::Eq, line));
                    i += 1;
                }
            }
            _ if c.is_ascii_digit() => {
                let start = i;
                while i < bytes.len()
                    && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.')
                {
                    i += 1;
                }
                let text = &src[start..i];
                let n = text
                    .parse::<f64>()
                    .map_err(|_| format!("line {line}: bad number `{text}`"))?;
                toks.push((Tok::Num(n), line));
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len()
                    && ((bytes[i] as char).is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let word = &src[start..i];
                let tok = match word {
                    "let" => Tok::Let,
                    "if" => Tok::If,
                    "then" => Tok::Then,
                    "else" => Tok::Else,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    _ => Tok::Ident(word.to_string()),
                };
                toks.push((tok, line));
            }
            _ => return Err(format!("line {line}: unexpected character `{c}`")),
        }
    }
    Ok(toks)
}

// --- AST ---

#[derive(Debug)]
enum Expr {
    Num(f64),
    Bool(bool),
    Var(String),
    Record(Vec<(String, Expr)>),
    List(Vec<Expr>),
    Field(Box<Expr>, String),
    Call(Box<Expr>, Vec<Expr>),
    Lambda(Rc<Vec<String>>, Rc<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    BinOp(Tok, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
}

// --- Parser (recursive descent) ---

struct Parser {
    toks: Vec<(Tok, u32)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn line(&self) -> u32 {
        self.toks
            .get(self.pos.min(self.toks.len().saturating_sub(1)))
            .map(|(_, l)| *l)
            .unwrap_or(0)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|(t, _)| t.clone());
        self.pos += 1;
        t
    }

    fn expect(&mut self, tok: Tok) -> Result<(), String> {
        let line = self.line();
        match self.next() {
            Some(t) if t == tok => Ok(()),
            other => Err(format!("line {line}: expected {tok:?}, found {other:?}")),
        }
    }

    fn ident(&mut self) -> Result<String, String> {
        let line = self.line();
        match self.next() {
            Some(Tok::Ident(name)) => Ok(name),
            other => Err(format!("line {line}: expected identifier, found {other:?}")),
        }
    }

    /// program := ("let" IDENT "=" expr)*
    fn program(&mut self) -> Result<Vec<(String, Expr)>, String> {
        let mut decls = Vec::new();
        while self.peek().is_some() {
            self.expect(Tok::Let)?;
            let name = self.ident()?;
            self.expect(Tok::Eq)?;
            let body = self.expr()?;
            decls.push((name, body));
        }
        Ok(decls)
    }

    /// Is the paren group starting at `pos` a lambda parameter list — i.e. does
    /// its matching close paren have `=>` after it?
    fn lambda_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while let Some((t, _)) = self.toks.get(i) {
            match t {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(i + 1), Some((Tok::Arrow, _)));
                    }
                }
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn expr(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Tok::LParen) && self.lambda_ahead() {
            return self.lambda();
        }
        if self.peek() == Some(&Tok::If) {
            self.next();
            let cond = self.expr()?;
            self.expect(Tok::Then)?;
            let then_branch = self.expr()?;
            self.expect(Tok::Else)?;
            let else_branch = self.expr()?;
            return Ok(Expr::If(
                Box::new(cond),
                Box::new(then_branch),
                Box::new(else_branch),
            ));
        }
        self.comparison()
    }

    fn lambda(&mut self) -> Result<Expr, String> {
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        if self.peek() != Some(&Tok::RParen) {
            loop {
                params.push(self.ident()?);
                if self.peek() == Some(&Tok::Comma) {
                    self.next();
                } else {
                    break;
                }
            }
        }
        self.expect(Tok::RParen)?;
        self.expect(Tok::Arrow)?;
        let body = self.expr()?;
        Ok(Expr::Lambda(Rc::new(params), Rc::new(body)))
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let left = self.additive()?;
        match self.peek() {
            Some(Tok::Lt) | Some(Tok::Gt) | Some(Tok::EqEq) => {
                let op = self.next().unwrap();
                let right = self.additive()?;
                Ok(Expr::BinOp(op, Box::new(left), Box::new(right)))
            }
            _ => Ok(left),
        }
    }

    fn additive(&mut self) -> Result<Expr, String> {
        let mut left = self.multiplicative()?;
        while matches!(self.peek(), Some(Tok::Plus) | Some(Tok::Minus)) {
            let op = self.next().unwrap();
            let right = self.multiplicative()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while matches!(self.peek(), Some(Tok::Star) | Some(Tok::Slash)) {
            let op = self.next().unwrap();
            let right = self.unary()?;
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.peek() == Some(&Tok::Minus) {
            self.next();
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    /// postfix := atom ( "." IDENT | "(" args ")" )*
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.atom()?;
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.next();
                    let field = self.ident()?;
                    expr = Expr::Field(Box::new(expr), field);
                }
                Some(Tok::LParen) => {
                    self.next();
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.expr()?);
                            if self.peek() == Some(&Tok::Comma) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen)?;
                    expr = Expr::Call(Box::new(expr), args);
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        let line = self.line();
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::True) => Ok(Expr::Bool(true)),
            Some(Tok::False) => Ok(Expr::Bool(false)),
            Some(Tok::Ident(name)) => Ok(Expr::Var(name)),
            Some(Tok::LParen) => {
                let inner = self.expr()?;
                self.expect(Tok::RParen)?;
                Ok(inner)
            }
            Some(Tok::LBrace) => {
                let mut fields = Vec::new();
                if self.peek() != Some(&Tok::RBrace) {
                    loop {
                        let name = self.ident()?;
                        self.expect(Tok::Colon)?;
                        fields.push((name, self.expr()?));
                        if self.peek() == Some(&Tok::Comma) {
                            self.next();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RBrace)?;
                Ok(Expr::Record(fields))
            }
            Some(Tok::LBracket) => {
                let mut items = Vec::new();
                if self.peek() != Some(&Tok::RBracket) {
                    loop {
                        items.push(self.expr()?);
                        if self.peek() == Some(&Tok::Comma) {
                            self.next();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Tok::RBracket)?;
                Ok(Expr::List(items))
            }
            other => Err(format!("line {line}: unexpected {other:?}")),
        }
    }
}

// --- Values + environment ---

#[derive(Clone, Copy, Debug, PartialEq)]
enum Builtin {
    Cube,
    Sphere,
    Group,
    Colored,
    Translate,
    RotateX,
    RotateY,
    Scale,
    CameraAt,
    MakeFrame,
    Range,
    Map,
    Sin,
    Cos,
}

#[derive(Clone)]
enum Value {
    Num(f64),
    Bool(bool),
    List(Rc<Vec<Value>>),
    Record(Rc<Vec<(String, Value)>>),
    Closure(Rc<Vec<String>>, Rc<Expr>, Env),
    Builtin(Builtin),
    Scene(Rc<Scene3D>),
    Cam(Rc<Camera>),
    FrameV(Rc<Frame>),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{n}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::List(items) => f.debug_list().entries(items.iter()).finish(),
            Value::Record(fields) => {
                let mut m = f.debug_map();
                for (k, v) in fields.iter() {
                    m.entry(&format_args!("{k}"), v);
                }
                m.finish()
            }
            Value::Closure(params, _, _) => write!(f, "<fn({})>", params.join(", ")),
            Value::Builtin(b) => write!(f, "<builtin {b:?}>"),
            Value::Scene(_) => write!(f, "<scene>"),
            Value::Cam(_) => write!(f, "<camera>"),
            Value::FrameV(_) => write!(f, "<frame>"),
        }
    }
}

/// Lexical scope for lambda locals: a persistent linked list. Globals are not
/// in here — they resolve by name at lookup time (late binding), which is what
/// makes hot-reload rebind work: re-parsing swaps the globals map and every
/// call site picks up the new definition on its next call.
#[derive(Clone)]
struct Env(Option<Rc<Scope>>);

struct Scope {
    vars: Vec<(String, Value)>,
    parent: Env,
}

impl Env {
    fn empty() -> Env {
        Env(None)
    }

    fn child(&self, vars: Vec<(String, Value)>) -> Env {
        Env(Some(Rc::new(Scope {
            vars,
            parent: self.clone(),
        })))
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        let mut cur = self;
        while let Some(scope) = &cur.0 {
            for (k, v) in scope.vars.iter().rev() {
                if k == name {
                    return Some(v.clone());
                }
            }
            cur = &scope.parent;
        }
        None
    }
}

// --- Evaluator ---

struct Program {
    globals: HashMap<String, Value>,
}

fn builtin_by_name(name: &str) -> Option<Builtin> {
    Some(match name {
        "cube" => Builtin::Cube,
        "sphere" => Builtin::Sphere,
        "group" => Builtin::Group,
        "colored" => Builtin::Colored,
        "translate" => Builtin::Translate,
        "rotateX" => Builtin::RotateX,
        "rotateY" => Builtin::RotateY,
        "scale" => Builtin::Scale,
        "camera" => Builtin::CameraAt,
        "frame" => Builtin::MakeFrame,
        "range" => Builtin::Range,
        "map" => Builtin::Map,
        "sin" => Builtin::Sin,
        "cos" => Builtin::Cos,
        _ => return None,
    })
}

impl Program {
    fn parse(src: &str) -> Result<Program, String> {
        let toks = lex(src)?;
        let mut parser = Parser { toks, pos: 0 };
        let decls = parser.program()?;
        let mut globals = HashMap::new();
        for (name, expr) in decls {
            // Top-level bindings evaluate in order against the globals defined
            // so far (constants can use earlier constants; lambdas late-bind).
            let value = eval(&expr, &Env::empty(), &globals)?;
            globals.insert(name, value);
        }
        Ok(Program { globals })
    }

    fn call_global(&self, name: &str, args: &[Value]) -> Result<Value, String> {
        let func = self
            .globals
            .get(name)
            .cloned()
            .ok_or_else(|| format!("no top-level `let {name}` in the game file"))?;
        call(&func, args, &self.globals)
    }
}

fn num(value: &Value, what: &str) -> Result<f64, String> {
    match value {
        Value::Num(n) => Ok(*n),
        other => Err(format!("{what}: expected a number, got {other:?}")),
    }
}

fn scene(value: &Value, what: &str) -> Result<Scene3D, String> {
    match value {
        Value::Scene(s) => Ok((**s).clone()),
        other => Err(format!("{what}: expected a scene, got {other:?}")),
    }
}

fn scene_value(s: Scene3D) -> Value {
    Value::Scene(Rc::new(s))
}

fn call(func: &Value, args: &[Value], globals: &HashMap<String, Value>) -> Result<Value, String> {
    match func {
        Value::Closure(params, body, env) => {
            if params.len() != args.len() {
                return Err(format!(
                    "<fn({})> called with {} argument(s)",
                    params.join(", "),
                    args.len()
                ));
            }
            let vars = params.iter().cloned().zip(args.iter().cloned()).collect();
            eval(body, &env.child(vars), globals)
        }
        Value::Builtin(b) => call_builtin(*b, args, globals),
        other => Err(format!("cannot call {other:?}")),
    }
}

fn call_builtin(
    b: Builtin,
    args: &[Value],
    globals: &HashMap<String, Value>,
) -> Result<Value, String> {
    match b {
        Builtin::Cube => Ok(scene_value(Scene3D::cube())),
        Builtin::Sphere => Ok(scene_value(Scene3D::sphere())),
        Builtin::Group => {
            let items = match args {
                [Value::List(items)] => items,
                _ => return Err("group(list) expects one list".to_string()),
            };
            let scenes = items
                .iter()
                .map(|v| scene(v, "group item"))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(scene_value(Scene3D {
                obj: SceneObject::Group(scenes),
                xform: Matrix4::identity(),
            }))
        }
        Builtin::Colored => match args {
            [r, g, bl, s] => {
                let material = MaterialDescription::color(
                    num(r, "colored r")? as f32,
                    num(g, "colored g")? as f32,
                    num(bl, "colored b")? as f32,
                    1.0,
                );
                Ok(scene_value(Scene3D {
                    obj: SceneObject::Material(material, vec![scene(s, "colored scene")?]),
                    xform: Matrix4::identity(),
                }))
            }
            _ => Err("colored(r, g, b, scene) expects 4 arguments".to_string()),
        },
        Builtin::Translate => match args {
            [s, x, y, z] => Ok(scene_value(
                scene(s, "translate scene")?
                    .translate_x(num(x, "translate x")? as f32)
                    .translate_y(num(y, "translate y")? as f32)
                    .translate_z(num(z, "translate z")? as f32),
            )),
            _ => Err("translate(scene, x, y, z) expects 4 arguments".to_string()),
        },
        Builtin::RotateX => match args {
            [s, radians] => Ok(scene_value(scene(s, "rotateX scene")?.rotate_x(
                Angle::from_radians(num(radians, "rotateX radians")? as f32),
            ))),
            _ => Err("rotateX(scene, radians) expects 2 arguments".to_string()),
        },
        Builtin::RotateY => match args {
            [s, radians] => Ok(scene_value(scene(s, "rotateY scene")?.rotate_y(
                Angle::from_radians(num(radians, "rotateY radians")? as f32),
            ))),
            _ => Err("rotateY(scene, radians) expects 2 arguments".to_string()),
        },
        Builtin::Scale => match args {
            [s, k] => {
                let k = num(k, "scale k")? as f32;
                Ok(scene_value(
                    scene(s, "scale scene")?.scale_x(k).scale_y(k).scale_z(k),
                ))
            }
            _ => Err("scale(scene, k) expects 2 arguments".to_string()),
        },
        Builtin::CameraAt => match args {
            [ex, ey, ez, tx, ty, tz] => Ok(Value::Cam(Rc::new(Camera::look_at(
                [
                    num(ex, "camera eye x")? as f32,
                    num(ey, "camera eye y")? as f32,
                    num(ez, "camera eye z")? as f32,
                ],
                [
                    num(tx, "camera target x")? as f32,
                    num(ty, "camera target y")? as f32,
                    num(tz, "camera target z")? as f32,
                ],
                [0.0, 1.0, 0.0],
                Angle::from_degrees(45.0),
            )))),
            _ => Err("camera(ex, ey, ez, tx, ty, tz) expects 6 arguments".to_string()),
        },
        Builtin::MakeFrame => match args {
            [Value::Cam(cam), s] => Ok(Value::FrameV(Rc::new(Frame::new(
                (**cam).clone(),
                scene(s, "frame scene")?,
            )))),
            _ => Err("frame(camera, scene) expects a camera and a scene".to_string()),
        },
        Builtin::Range => match args {
            [n] => {
                let n = num(n, "range n")?.max(0.0) as usize;
                Ok(Value::List(Rc::new(
                    (0..n).map(|i| Value::Num(i as f64)).collect(),
                )))
            }
            _ => Err("range(n) expects 1 argument".to_string()),
        },
        Builtin::Map => match args {
            [f, Value::List(items)] => {
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(call(f, std::slice::from_ref(item), globals)?);
                }
                Ok(Value::List(Rc::new(out)))
            }
            _ => Err("map(f, list) expects a function and a list".to_string()),
        },
        Builtin::Sin => match args {
            [n] => Ok(Value::Num(num(n, "sin")?.sin())),
            _ => Err("sin(x) expects 1 argument".to_string()),
        },
        Builtin::Cos => match args {
            [n] => Ok(Value::Num(num(n, "cos")?.cos())),
            _ => Err("cos(x) expects 1 argument".to_string()),
        },
    }
}

fn eval(expr: &Expr, env: &Env, globals: &HashMap<String, Value>) -> Result<Value, String> {
    match expr {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Var(name) => env
            .lookup(name)
            .or_else(|| globals.get(name).cloned())
            .or_else(|| builtin_by_name(name).map(Value::Builtin))
            .ok_or_else(|| format!("unknown name `{name}`")),
        Expr::Record(fields) => {
            let mut out = Vec::with_capacity(fields.len());
            for (name, e) in fields {
                out.push((name.clone(), eval(e, env, globals)?));
            }
            Ok(Value::Record(Rc::new(out)))
        }
        Expr::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for e in items {
                out.push(eval(e, env, globals)?);
            }
            Ok(Value::List(Rc::new(out)))
        }
        Expr::Field(e, name) => match eval(e, env, globals)? {
            Value::Record(fields) => fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| format!("record has no field `{name}`")),
            other => Err(format!("`.{name}` on non-record {other:?}")),
        },
        Expr::Call(f, args) => {
            let func = eval(f, env, globals)?;
            let mut argv = Vec::with_capacity(args.len());
            for a in args {
                argv.push(eval(a, env, globals)?);
            }
            call(&func, &argv, globals)
        }
        Expr::Lambda(params, body) => {
            Ok(Value::Closure(params.clone(), body.clone(), env.clone()))
        }
        Expr::If(cond, then_branch, else_branch) => match eval(cond, env, globals)? {
            Value::Bool(true) => eval(then_branch, env, globals),
            Value::Bool(false) => eval(else_branch, env, globals),
            other => Err(format!("if condition must be a bool, got {other:?}")),
        },
        Expr::Neg(e) => Ok(Value::Num(-num(&eval(e, env, globals)?, "negation")?)),
        Expr::BinOp(op, l, r) => {
            let lv = eval(l, env, globals)?;
            let rv = eval(r, env, globals)?;
            let (a, b) = (num(&lv, "left operand")?, num(&rv, "right operand")?);
            Ok(match op {
                Tok::Plus => Value::Num(a + b),
                Tok::Minus => Value::Num(a - b),
                Tok::Star => Value::Num(a * b),
                Tok::Slash => Value::Num(a / b),
                Tok::Lt => Value::Bool(a < b),
                Tok::Gt => Value::Bool(a > b),
                Tok::EqEq => Value::Bool(a == b),
                other => return Err(format!("bad operator {other:?}")),
            })
        }
    }
}

// --- The Game impl ---

pub struct MleGame {
    path: String,
    mtime: SystemTime,
    program: Program,
    model: Value,
    // rolling per-frame eval cost, printed every STATS_EVERY frames
    frames: u64,
    tick_ns: u64,
    draw_ns: u64,
}

const STATS_EVERY: u64 = 300;

fn file_mtime(path: &str) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

impl MleGame {
    pub fn create(path: &str) -> MleGame {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        let program = Program::parse(&src).unwrap_or_else(|e| panic!("{path}: {e}"));
        let model = program
            .globals
            .get("init")
            .cloned()
            .unwrap_or_else(|| panic!("{path}: no top-level `let init = ...`"));
        println!("[mle] loaded {path}");

        let mut game = MleGame {
            path: path.to_string(),
            mtime: file_mtime(path),
            program,
            model,
            frames: 0,
            tick_ns: 0,
            draw_ns: 0,
        };

        // MLE_SPIKE_BENCH=<iters>: run tick+draw in a tight loop and exit —
        // the raw interpreter throughput number, no window or frame pacing.
        if let Ok(iters) = std::env::var("MLE_SPIKE_BENCH") {
            let iters: u64 = iters.parse().unwrap_or(10_000);
            let started = Instant::now();
            for i in 0..iters {
                let t = FrameTime {
                    dts: 1.0 / 60.0,
                    tts: i as f32 / 60.0,
                };
                game.tick(t.clone());
                let _ = game.render(t);
            }
            let total = started.elapsed();
            let per_frame_us = total.as_micros() as f64 / iters as f64;
            println!(
                "[mle-bench] {iters} frames (tick+draw) in {:.1}ms — {:.1}µs/frame, {:.1}% of a 60fps budget",
                total.as_secs_f64() * 1000.0,
                per_frame_us,
                per_frame_us / 16_666.0 * 100.0
            );
            std::process::exit(0);
        }

        game
    }

    fn report_stats(&mut self) {
        if self.frames > 0 && self.frames % STATS_EVERY == 0 {
            let tick_us = self.tick_ns as f64 / STATS_EVERY as f64 / 1000.0;
            let draw_us = self.draw_ns as f64 / STATS_EVERY as f64 / 1000.0;
            println!(
                "[mle] avg over {STATS_EVERY} frames: tick {tick_us:.1}µs, draw {draw_us:.1}µs ({:.1}% of a 60fps budget)",
                (tick_us + draw_us) / 16_666.0 * 100.0
            );
            self.tick_ns = 0;
            self.draw_ns = 0;
        }
    }
}

impl Game for MleGame {
    fn check_hot_reload(&mut self, _frame_time: FrameTime) {
        let mtime = file_mtime(&self.path);
        if mtime == self.mtime {
            return;
        }
        self.mtime = mtime;
        let started = Instant::now();
        let src = match std::fs::read_to_string(&self.path) {
            Ok(src) => src,
            Err(e) => {
                eprintln!("[mle] reload: cannot read {}: {e}", self.path);
                return;
            }
        };
        match Program::parse(&src) {
            Ok(program) => {
                // Rebind: swap the globals; the model value is untouched, so
                // state survives and all functions pick up their new bodies.
                self.program = program;
                println!(
                    "[mle] hot-reloaded {} in {:.2}ms (model preserved)",
                    self.path,
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(e) => eprintln!("[mle] reload failed, keeping old program: {e}"),
        }
    }

    fn tick(&mut self, frame_time: FrameTime) {
        let started = Instant::now();
        let args = [
            self.model.clone(),
            Value::Num(frame_time.dts as f64),
            Value::Num(frame_time.tts as f64),
        ];
        match self.program.call_global("tick", &args) {
            Ok(model) => self.model = model,
            Err(e) => eprintln!("[mle] tick error: {e}"),
        }
        self.tick_ns += started.elapsed().as_nanos() as u64;
        self.frames += 1;
        self.report_stats();
    }

    fn key_event(&mut self, _code: i32, _is_down: bool) {}
    fn mouse_move(&mut self, _x: i32, _y: i32) {}
    fn mouse_wheel(&mut self, _delta: i32) {}

    fn render(&mut self, frame_time: FrameTime) -> Frame {
        let started = Instant::now();
        let args = [self.model.clone(), Value::Num(frame_time.tts as f64)];
        let frame = match self.program.call_global("draw", &args) {
            Ok(Value::FrameV(frame)) => (*frame).clone(),
            Ok(other) => {
                eprintln!("[mle] draw must return frame(camera, scene), got {other:?}");
                Frame::new(
                    Camera::default(),
                    Scene3D {
                        obj: SceneObject::Group(vec![]),
                        xform: Matrix4::identity(),
                    },
                )
            }
            Err(e) => {
                eprintln!("[mle] draw error: {e}");
                Frame::new(
                    Camera::default(),
                    Scene3D {
                        obj: SceneObject::Group(vec![]),
                        xform: Matrix4::identity(),
                    },
                )
            }
        };
        self.draw_ns += started.elapsed().as_nanos() as u64;
        frame
    }

    fn ui(&self) -> View {
        View::empty()
    }

    fn state_debug(&self) -> String {
        format!("{:#?}", self.model)
    }

    fn net_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn net_push_http_response(&mut self, _token: i32, _status: i32, _body: String) {}
    fn net_push_http_error(&mut self, _token: i32, _message: String) {}
    fn audio_drain_commands(&self) -> String {
        "[]".to_string()
    }
    fn audio_scene_json(&self) -> String {
        "{\"sources\":[]}".to_string()
    }
    fn net_drain_conn_commands(&self) -> String {
        "[]".to_string()
    }
    fn net_push_connected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_message(&mut self, _key: String, _conn: i32, _text: String) {}
    fn net_push_disconnected(&mut self, _key: String, _conn: i32) {}
    fn net_push_conn_error(&mut self, _key: String, _conn: i32, _message: String) {}
    fn audio_push_finished(&mut self, _token: i32) {}

    fn quit(&mut self) {}
}
