//! [`Shareable`]: the form a value takes to cross a thread (a job's input or
//! result). An immutable DAG of plain data and immutable `Arc` host values,
//! so a shared `Value` stays shared instead of expanding into a tree.
//! Separate from `EffectValue`, the plain-data wire codec.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use functor_lang::value::MapKey;
use functor_lang::Value;

use crate::functor_lang_prelude::bytes::{ByteView, FunctorLangBytes};
use crate::functor_lang_prelude::model::FunctorLangModel;
use crate::functor_lang_prelude::path::FunctorLangPath;
use crate::functor_lang_prelude::{FunctorLangShape, FunctorLangTexture};
use crate::model::ModelData;
use crate::physics;
use crate::scene3d::TextureDescription;

/// Nodes a conversion may visit before it refuses (a shared node counts at
/// each reference, its subtree once).
pub const NODE_CAP: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub enum Shareable {
    Number(f64),
    Bool(bool),
    Text(Arc<str>),
    List(Arc<[Shareable]>),
    Tuple(Arc<[Shareable]>),
    Record(Arc<[(Arc<str>, Shareable)]>),
    Variant(Arc<str>, Arc<[Shareable]>),
    /// Entries in the map's canonical key order.
    Map(Arc<[(Key, Shareable)]>),
    Host(HostValue),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Bool(bool),
    Number(f64),
    Text(Arc<str>),
}

/// The host values that may cross: immutable, `Arc` inside. `Path` is
/// re-wrapped (its text is `Rc`).
#[derive(Clone)]
pub enum HostValue {
    Bytes(ByteView),
    Path(Arc<str>),
    Model(Arc<ModelData>),
    Texture(TextureDescription),
    Shape(physics::Shape),
}

impl std::fmt::Debug for HostValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostValue::Bytes(view) => write!(f, "Bytes({})", view.as_slice().len()),
            HostValue::Path(path) => write!(f, "Path({path:?})"),
            HostValue::Model(model) => write!(f, "Model({:p})", Arc::as_ptr(model)),
            HostValue::Texture(texture) => write!(f, "Texture({texture:?})"),
            HostValue::Shape(shape) => write!(f, "{shape:?}"),
        }
    }
}

impl PartialEq for HostValue {
    fn eq(&self, other: &HostValue) -> bool {
        match (self, other) {
            (HostValue::Bytes(a), HostValue::Bytes(b)) => a.as_slice() == b.as_slice(),
            (HostValue::Path(a), HostValue::Path(b)) => a == b,
            (HostValue::Model(a), HostValue::Model(b)) => Arc::ptr_eq(a, b),
            (HostValue::Texture(a), HostValue::Texture(b)) => a == b,
            (HostValue::Shape(a), HostValue::Shape(b)) => a == b,
            _ => false,
        }
    }
}

const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    send_sync::<Shareable>()
};

impl HostValue {
    fn of(data: &dyn std::any::Any) -> Option<HostValue> {
        if let Some(bytes) = data.downcast_ref::<FunctorLangBytes>() {
            return Some(HostValue::Bytes(bytes.0.clone()));
        }
        if let Some(path) = data.downcast_ref::<FunctorLangPath>() {
            return Some(HostValue::Path(Arc::from(&*path.0)));
        }
        if let Some(model) = data.downcast_ref::<FunctorLangModel>() {
            return Some(HostValue::Model(model.0.clone()));
        }
        if let Some(texture) = data.downcast_ref::<FunctorLangTexture>() {
            return Some(HostValue::Texture(texture.0.clone()));
        }
        data.downcast_ref::<FunctorLangShape>()
            .map(|shape| HostValue::Shape(shape.0.clone()))
    }

    fn to_value(&self) -> Value {
        let data: Rc<dyn functor_lang::value::HostData> = match self {
            HostValue::Bytes(view) => Rc::new(FunctorLangBytes(view.clone())),
            HostValue::Path(path) => Rc::new(FunctorLangPath(Rc::from(&**path))),
            HostValue::Model(model) => Rc::new(FunctorLangModel(model.clone())),
            HostValue::Texture(texture) => Rc::new(FunctorLangTexture(texture.clone())),
            HostValue::Shape(shape) => Rc::new(FunctorLangShape(shape.clone())),
        };
        Value::HostData(data)
    }
}

/// A container being built: its kind, child count, and memo key.
enum Open<'v> {
    List(usize, usize),
    Tuple(usize, usize),
    Record(&'v [(String, Value)], usize),
    Variant(&'v str, usize, usize),
    Map(Vec<Key>, usize),
}

/// A memo key: a container's children allocation plus what wraps it, since
/// one allocation can sit under a list and a tuple, or two constructors.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind<'a> {
    List,
    Tuple,
    Record,
    Map,
    Variant(&'a str),
}

impl Shareable {
    /// Convert `value`, keeping shared nodes shared. Refuses functions and
    /// host values outside [`HostValue`], naming where: `.ai.onSeen is a
    /// function`.
    pub fn from_value(value: &Value) -> Result<Shareable, String> {
        Shareable::from_value_capped(value, NODE_CAP)
    }

    /// [`Self::from_value`], refusing past `cap` visited nodes.
    pub fn from_value_capped(root: &Value, cap: usize) -> Result<Shareable, String> {
        enum Step<'v> {
            Enter(&'v Value, String),
            Exit(Open<'v>),
        }
        // Iterative post-order: `path` is the open containers' segments,
        // `out` the finished children awaiting their parent.
        let mut steps = vec![Step::Enter(root, String::new())];
        let mut path: Vec<String> = Vec::new();
        let mut out: Vec<Shareable> = Vec::new();
        let mut memo: HashMap<(usize, Kind), Shareable> = HashMap::new();
        let mut nodes = 0usize;
        while let Some(step) = steps.pop() {
            let open = match step {
                Step::Enter(value, segment) => {
                    nodes += 1;
                    if nodes > cap {
                        return Err(format!("the value expands past {cap} nodes"));
                    }
                    let leaf = match value {
                        Value::Number(n) => Shareable::Number(*n),
                        Value::Bool(b) => Shareable::Bool(*b),
                        Value::String(s) => Shareable::Text(Arc::from(&**s)),
                        Value::HostData(data) => match HostValue::of(data.as_any()) {
                            Some(host) => Shareable::Host(host),
                            None => {
                                let what = format!("a {} (host values can't cross)", data.type_name());
                                return Err(refusal(&path, &segment, &what));
                            }
                        },
                        _ if value_key(value).is_some_and(|key| memo.contains_key(&key)) => {
                            memo[&value_key(value).expect("checked")].clone()
                        }
                        Value::List(items) | Value::Tuple(items) | Value::Variant { args: items, .. } => {
                            let key = ptr(items);
                            let open = match value {
                                Value::List(_) => Open::List(items.len(), key),
                                Value::Tuple(_) => Open::Tuple(items.len(), key),
                                Value::Variant { ctor, .. } => Open::Variant(ctor, items.len(), key),
                                _ => unreachable!(),
                            };
                            steps.push(Step::Exit(open));
                            path.push(segment);
                            for (i, item) in items.iter().enumerate().rev() {
                                steps.push(Step::Enter(item, format!("[{i}]")));
                            }
                            continue;
                        }
                        Value::Record(fields) => {
                            steps.push(Step::Exit(Open::Record(fields, ptr(fields))));
                            path.push(segment);
                            for (name, field) in fields.iter().rev() {
                                steps.push(Step::Enter(field, format!(".{name}")));
                            }
                            continue;
                        }
                        Value::Map(map) => {
                            let keys = map.iter().map(|(k, _)| Key::of(k)).collect();
                            steps.push(Step::Exit(Open::Map(keys, map.root_ptr() as usize)));
                            path.push(segment);
                            let entries: Vec<_> = map.iter().collect();
                            for (key, entry) in entries.into_iter().rev() {
                                steps.push(Step::Enter(entry, format!("[{:?}]", Key::of(key))));
                            }
                            continue;
                        }
                        Value::Closure(_)
                        | Value::Partial(_)
                        | Value::Builtin(_)
                        | Value::HostFn(_)
                        | Value::Ctor { .. } => {
                            return Err(refusal(&path, &segment, "a function"));
                        }
                    };
                    out.push(leaf);
                    continue;
                }
                Step::Exit(open) => open,
            };
            path.pop();
            let (built, key) = match open {
                Open::List(n, key) => (Shareable::List(take(&mut out, n).into()), (key, Kind::List)),
                Open::Tuple(n, key) => (Shareable::Tuple(take(&mut out, n).into()), (key, Kind::Tuple)),
                Open::Variant(ctor, n, key) => (
                    Shareable::Variant(Arc::from(ctor), take(&mut out, n).into()),
                    (key, Kind::Variant(ctor)),
                ),
                Open::Record(fields, key) => {
                    let values = take(&mut out, fields.len());
                    let named = fields.iter().zip(values).map(|((name, _), v)| (Arc::from(name.as_str()), v));
                    (Shareable::Record(named.collect::<Vec<_>>().into()), (key, Kind::Record))
                }
                Open::Map(keys, key) => {
                    let values = take(&mut out, keys.len());
                    let entries = keys.into_iter().zip(values).collect::<Vec<_>>();
                    (Shareable::Map(entries.into()), (key, Kind::Map))
                }
            };
            memo.insert(key, built.clone());
            out.push(built);
        }
        Ok(out.pop().expect("the root converts to one value"))
    }

    /// The `Value` a job's receiver sees; shared nodes stay shared, and host
    /// values wrap the same inner `Arc` (pointer fast paths still hit).
    pub fn to_value(&self) -> Value {
        enum Step<'s> {
            Enter(&'s Shareable),
            Exit(&'s Shareable, usize),
        }
        let mut steps = vec![Step::Enter(self)];
        let mut out: Vec<Value> = Vec::new();
        let mut memo: HashMap<(usize, Kind), Value> = HashMap::new();
        while let Some(step) = steps.pop() {
            match step {
                Step::Enter(node) => {
                    if let Some(done) = node.memo_kind().and_then(|k| memo.get(&k)) {
                        out.push(done.clone());
                        continue;
                    }
                    match node {
                        Shareable::Number(n) => out.push(Value::Number(*n)),
                        Shareable::Bool(b) => out.push(Value::Bool(*b)),
                        Shareable::Text(s) => out.push(Value::String(Rc::from(&**s))),
                        Shareable::Host(host) => out.push(host.to_value()),
                        _ => {
                            let children = node.children();
                            steps.push(Step::Exit(node, children.len()));
                            steps.extend(children.into_iter().rev().map(Step::Enter));
                        }
                    }
                }
                Step::Exit(node, n) => {
                    let values = take(&mut out, n);
                    let value = match node {
                        Shareable::List(_) => Value::List(Rc::new(values)),
                        Shareable::Tuple(_) => Value::Tuple(Rc::new(values)),
                        Shareable::Variant(ctor, _) => Value::Variant {
                            ctor: Rc::from(&**ctor),
                            args: Rc::new(values),
                        },
                        Shareable::Record(fields) => Value::Record(Rc::new(
                            fields.iter().zip(values).map(|((name, _), v)| (name.to_string(), v)).collect(),
                        )),
                        Shareable::Map(entries) => {
                            let pairs = entries.iter().zip(values).map(|((k, _), v)| (k.to_map_key(), v));
                            Value::Map(functor_lang::pmap::PMap::from_sorted(pairs.collect()).0)
                        }
                        _ => unreachable!("leaves finish on entry"),
                    };
                    if let Some(key) = node.memo_kind() {
                        memo.insert(key, value.clone());
                    }
                    out.push(value);
                }
            }
        }
        out.pop().expect("one value")
    }

    /// The buffers this value keeps alive, each once, as `(address, bytes)`:
    /// a small slice of a big buffer charges the buffer. Only `Bytes` are
    /// charged; models and shapes are decoded from them.
    pub fn buffers(&self) -> Vec<(usize, usize)> {
        let mut seen = HashSet::new();
        let mut buffers = Vec::new();
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            if node.memo_key().is_some_and(|key| !seen.insert(key)) {
                continue;
            }
            if let Shareable::Host(HostValue::Bytes(view)) = node {
                let data = view.backing();
                let address = Arc::as_ptr(data) as *const u8 as usize;
                if seen.insert(address) {
                    buffers.push((address, data.len()));
                }
            }
            stack.extend(node.children());
        }
        buffers
    }

    fn children(&self) -> Vec<&Shareable> {
        match self {
            Shareable::List(items) | Shareable::Tuple(items) | Shareable::Variant(_, items) => {
                items.iter().collect()
            }
            Shareable::Record(fields) => fields.iter().map(|(_, v)| v).collect(),
            Shareable::Map(entries) => entries.iter().map(|(_, v)| v).collect(),
            _ => Vec::new(),
        }
    }

    /// A container's identity and wrapper, for sharing.
    fn memo_kind(&self) -> Option<(usize, Kind<'_>)> {
        let kind = match self {
            Shareable::List(_) => Kind::List,
            Shareable::Tuple(_) => Kind::Tuple,
            Shareable::Record(_) => Kind::Record,
            Shareable::Map(_) => Kind::Map,
            Shareable::Variant(ctor, _) => Kind::Variant(ctor),
            _ => return None,
        };
        Some((self.memo_key()?, kind))
    }

    /// A container's children allocation (its `Arc`), for dedupe.
    fn memo_key(&self) -> Option<usize> {
        let ptr = |p: *const ()| Some(p as usize);
        match self {
            Shareable::List(items) | Shareable::Tuple(items) | Shareable::Variant(_, items) => {
                ptr(Arc::as_ptr(items) as *const ())
            }
            Shareable::Record(fields) => ptr(Arc::as_ptr(fields) as *const ()),
            Shareable::Map(entries) => ptr(Arc::as_ptr(entries) as *const ()),
            _ => None,
        }
    }
}

impl Key {
    fn of(key: &MapKey) -> Key {
        match key {
            MapKey::Bool(b) => Key::Bool(*b),
            MapKey::Number(n) => Key::Number(*n),
            MapKey::String(s) => Key::Text(Arc::from(&**s)),
        }
    }

    fn to_map_key(&self) -> MapKey {
        match self {
            Key::Bool(b) => MapKey::Bool(*b),
            Key::Number(n) => MapKey::Number(*n),
            Key::Text(s) => MapKey::String(Rc::from(&**s)),
        }
    }
}

fn ptr<T>(rc: &Rc<T>) -> usize {
    Rc::as_ptr(rc) as *const () as usize
}

/// A container value's memo key; `None` for a leaf.
fn value_key(value: &Value) -> Option<(usize, Kind<'_>)> {
    Some(match value {
        Value::List(items) => (ptr(items), Kind::List),
        Value::Tuple(items) => (ptr(items), Kind::Tuple),
        Value::Variant { ctor, args } => (ptr(args), Kind::Variant(ctor)),
        Value::Record(fields) => (ptr(fields), Kind::Record),
        Value::Map(map) => (map.root_ptr() as usize, Kind::Map),
        _ => return None,
    })
}

/// The last `n` finished values, in order.
fn take<T>(out: &mut Vec<T>, n: usize) -> Vec<T> {
    out.split_off(out.len() - n)
}

fn refusal(path: &[String], segment: &str, what: &str) -> String {
    let at: String = path.iter().map(String::as_str).chain([segment]).collect();
    if at.is_empty() {
        format!("the value is {what}")
    } else {
        format!("`{at}` is {what}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str) -> Value {
        let module = functor_lang::lower(functor_lang::parse(src).expect("parse")).expect("lower");
        match functor_lang::run(&module, functor_lang::Tracing::Off).map(|r| r.outcome) {
            Ok(functor_lang::RunOutcome::Main(value)) => value,
            _ => panic!("expected a main value"),
        }
    }

    #[test]
    fn plain_data_round_trips() {
        let value = eval(
            "let main = () => { a: [1.0, 2.0], b: (true, \"x\"), c: Map.fromList([(\"k\", 3.0)]) }",
        );
        let back = Shareable::from_value(&value).unwrap().to_value();
        assert_eq!(back.to_string(), value.to_string());
    }

    #[test]
    fn shared_nodes_stay_shared_and_do_not_expand() {
        // 40 levels of `[x, x]`: 2^40 nodes as a tree, 41 as a DAG.
        let mut value = Value::Number(1.0);
        for _ in 0..40 {
            value = Value::List(Rc::new(vec![value.clone(), value]));
        }
        let shared = Shareable::from_value_capped(&value, 100).expect("the DAG fits");
        let Shareable::List(items) = &shared else { panic!() };
        assert!(Arc::ptr_eq(
            match &items[0] { Shareable::List(a) => a, _ => panic!() },
            match &items[1] { Shareable::List(b) => b, _ => panic!() },
        ));
        let Value::List(back) = shared.to_value() else { panic!() };
        let (Value::List(a), Value::List(b)) = (&back[0], &back[1]) else { panic!() };
        assert!(Rc::ptr_eq(a, b), "sharing survives the way back");
    }

    #[test]
    fn one_allocation_under_two_wrappers_keeps_each_wrapper() {
        let args = Rc::new(vec![Value::Number(1.0), Value::Number(2.0)]);
        let value = Value::Tuple(Rc::new(vec![
            Value::Variant { ctor: "A".into(), args: args.clone() },
            Value::Variant { ctor: "B".into(), args: args.clone() },
            Value::List(args.clone()),
            Value::Tuple(args),
        ]));
        let shared = Shareable::from_value(&value).unwrap();
        assert_eq!(shared.to_value().to_string(), value.to_string());
        let Shareable::Tuple(items) = &shared else { panic!() };
        assert!(matches!(&items[1], Shareable::Variant(ctor, _) if &**ctor == "B"));
        assert!(matches!(&items[2], Shareable::List(_)));
    }

    #[test]
    fn shared_maps_do_not_expand() {
        let mut value = eval("let main = () => Map.fromList([(\"k\", 0.0)])");
        for _ in 0..40 {
            let entries = vec![
                (MapKey::String("a".into()), value.clone()),
                (MapKey::String("b".into()), value.clone()),
            ];
            value = Value::Map(functor_lang::pmap::PMap::from_sorted(entries).0);
        }
        assert!(Shareable::from_value_capped(&value, 200).is_ok());
    }

    #[test]
    fn past_the_cap_is_refused() {
        let value = Value::List(Rc::new((0..10).map(|i| Value::Number(i as f64)).collect()));
        let refused = Shareable::from_value_capped(&value, 5).unwrap_err();
        assert!(refused.contains("expands past 5 nodes"), "{refused}");
    }

    #[test]
    fn functions_are_refused_with_their_path() {
        let value = eval("let main = () => { ai: { onSeen: (x) => x } }");
        assert_eq!(
            Shareable::from_value(&value).unwrap_err(),
            "`.ai.onSeen` is a function"
        );
    }

    #[test]
    fn host_values_keep_their_inner_arc() {
        let data: Arc<[u8]> = Arc::from(vec![7u8; 64]);
        let value = Value::HostData(Rc::new(FunctorLangBytes(ByteView::from_shared(data.clone()))));
        let back = Shareable::from_value(&value).unwrap().to_value();
        let Value::HostData(host) = back else { panic!() };
        let bytes = host.as_any().downcast_ref::<FunctorLangBytes>().unwrap();
        assert!(Arc::ptr_eq(bytes.0.backing(), &data));
    }

    #[test]
    fn backing_bytes_charge_each_buffer_once() {
        let data: Arc<[u8]> = Arc::from(vec![0u8; 1000]);
        let view = || Shareable::Host(HostValue::Bytes(ByteView::from_shared(data.clone())));
        // Two views of one buffer, plus a second buffer.
        let value = Shareable::List(
            vec![view(), view(), Shareable::Host(HostValue::Bytes(ByteView::new(vec![0; 10])))].into(),
        );
        let total: usize = value.buffers().iter().map(|(_, bytes)| bytes).sum();
        assert_eq!(total, 1010);
    }
}
