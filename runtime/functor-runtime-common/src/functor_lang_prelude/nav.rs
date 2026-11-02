//! `Nav.*` — graph routing on the job pool (see `nav.funi`).

use std::sync::Arc;

use crate::jobs::{HostValue, Shareable};
use crate::nav::{Graph, Link, Outcome, Penalty, Query, Route};

use super::cells::{field, list, number, point, whole};
use super::*;

/// A validated [`Graph`] as an opaque value.
pub struct FunctorLangNavGraph(pub Arc<Graph>);

impl HostData for FunctorLangNavGraph {
    fn type_name(&self) -> &'static str {
        "Nav.graph"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    // Immutable data: safe to keep in a model across reloads.
    fn is_reload_safe_snapshot(&self) -> bool {
        true
    }
}

crate::host_returnable!(FunctorLangNavGraph);

/// An id or bit set: a whole number ≥ 0.
fn whole_number(value: &Value, what: &str) -> Result<u32, String> {
    let n = number(value, what)?;
    whole(n).ok_or_else(|| format!("{what} must be a whole number ≥ 0, got {n}"))
}

fn whole_field(value: &Value, name: &str, what: &str) -> Result<u32, String> {
    whole_number(field(value, name, what)?, &format!("{what} `{name}`"))
}

fn number_field(value: &Value, name: &str, what: &str) -> Result<f64, String> {
    number(field(value, name, what)?, &format!("{what} `{name}`"))
}

/// `{ nodes, links }`, as [`Graph::new`]'s arguments.
struct GraphData(Vec<[f32; 3]>, Vec<Link>);

fn graph_data(value: &Value, span: Span) -> Result<GraphData, String> {
    let nodes = list(field(value, "nodes", "the graph")?, "nodes")?
        .iter()
        .enumerate()
        .map(|(i, at)| point(at, &format!("node {i}"), span))
        .collect::<Result<_, _>>()?;
    let link = |(i, link): (usize, &Value)| {
        let what = format!("link {i}");
        let door = match field(link, "door", &what)? {
            Value::Variant { ctor, args } if &**ctor == "Option.Some" && args.len() == 1 => {
                Some(whole_number(&args[0], &format!("{what} `door`"))?)
            }
            Value::Variant { ctor, .. } if &**ctor == "Option.None" => None,
            other => return Err(format!("{what} `door` must be an Option, got {}", other.kind_name())),
        };
        Ok(Link {
            from: whole_field(link, "from", &what)?,
            to: whole_field(link, "to", &what)?,
            cost: number_field(link, "cost", &what)?,
            bits: whole_field(link, "bits", &what)?,
            door,
        })
    };
    let links = list(field(value, "links", "the graph")?, "links")?
        .iter()
        .enumerate()
        .map(link)
        .collect::<Result<_, String>>()?;
    Ok(GraphData(nodes, links))
}

impl crate::host_registry::FromArg for GraphData {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        graph_data(value, span).map_err(|e| RunError { message: format!("Nav.graph: {e}"), span })
    }
}

/// A `{ penalty, until }` record's [`Penalty`]: `Nav.Cut` is an infinite weight.
fn penalty(value: &Value, what: &str) -> Result<Penalty, String> {
    let weight = match field(value, "penalty", what)? {
        Value::Variant { ctor, args } if &**ctor == "Nav.Cut" && args.is_empty() => f64::INFINITY,
        Value::Variant { ctor, args } if &**ctor == "Nav.Costly" && args.len() == 1 => {
            let weight = number(&args[0], &format!("{what} weight"))?;
            if !(weight.is_finite() && weight >= 0.0) {
                return Err(format!("{what} weight must be finite and ≥ 0, got {weight}"));
            }
            weight
        }
        other => return Err(format!("{what} `penalty` must be Nav.Costly(w) or Nav.Cut, got {}", other.kind_name())),
    };
    Ok(Penalty { weight, until: number_field(value, "until", what)? })
}

/// A query, and itself as the value its result echoes.
struct PathQuery(Query, Shareable);

fn path_query(value: &Value, span: Span) -> Result<PathQuery, String> {
    let what = "the query";
    let entries = |name: &str| list(field(value, name, what)?, name);
    let links = entries("links")?
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let what = format!("links[{i}]");
            Ok(((whole_field(p, "from", &what)?, whole_field(p, "to", &what)?), penalty(p, &what)?))
        })
        .collect::<Result<_, String>>()?;
    let nodes = entries("nodes")?
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let what = format!("nodes[{i}]");
            Ok((whole_field(p, "node", &what)?, penalty(p, &what)?))
        })
        .collect::<Result<_, String>>()?;
    let query = Query {
        from: point(field(value, "from", what)?, "the query `from`", span)?,
        to: point(field(value, "to", what)?, "the query `to`", span)?,
        bits: whole_field(value, "bits", what)?,
        closed_doors: entries("closedDoors")?
            .iter()
            .map(|d| whole_number(d, "a closed door"))
            .collect::<Result<_, _>>()?,
        links,
        nodes,
        now: number_field(value, "now", what)?,
    };
    // `seq` is only echoed, but every result carries one.
    number_field(value, "seq", what)?;
    Ok(PathQuery(query, Shareable::from_value(value)?))
}

impl crate::host_registry::FromArg for PathQuery {
    fn from_arg(value: &Value, _path: &str, span: Span) -> Result<Self, RunError> {
        path_query(value, span).map_err(|e| RunError { message: format!("Nav.path: {e}"), span })
    }
}

/// A penalty naming a node the graph doesn't have, as an error.
fn check_nodes(query: &Query, graph: &Graph) -> Result<(), String> {
    let n = graph.len();
    let named = |id: u32, what: String| {
        if id as usize >= n {
            Err(format!("Nav.path: {what} names node {id}, but the graph has {n} nodes"))
        } else {
            Ok(())
        }
    };
    for (i, ((from, to), _)) in query.links.iter().enumerate() {
        named(*from, format!("links[{i}]"))?;
        named(*to, format!("links[{i}]"))?;
    }
    for (i, (node, _)) in query.nodes.iter().enumerate() {
        named(*node, format!("nodes[{i}]"))?;
    }
    Ok(())
}

/// Run `query` on `graph`: the `Nav.result` its tagger receives. A
/// cancelled search's result is never delivered.
pub(crate) fn route(graph: &Graph, query: &Query, echo: &Shareable, cancelled: &dyn Fn() -> bool) -> Shareable {
    let route = graph.find(query, cancelled).unwrap_or_else(Route::failed);
    let outcome = match route.outcome {
        Outcome::Full => "Nav.Full",
        Outcome::Partial => "Nav.Partial",
        Outcome::Failed => "Nav.Failed",
    };
    result(graph, echo, Shareable::Variant(outcome.into(), Vec::new().into()), &route)
}

/// A `Nav.path` that didn't run (a full queue, a crash): `NotRun(why)`.
pub(crate) fn not_run(graph: &Graph, echo: &Shareable, why: String) -> Shareable {
    let outcome = Shareable::Variant("Nav.NotRun".into(), vec![Shareable::Text(why.into())].into());
    result(graph, echo, outcome, &Route::failed())
}

fn result(graph: &Graph, echo: &Shareable, outcome: Shareable, route: &Route) -> Shareable {
    let retry_at = match route.retry_at {
        Some(at) => Shareable::Variant("Option.Some".into(), vec![Shareable::Number(at)].into()),
        None => Shareable::Variant("Option.None".into(), Vec::new().into()),
    };
    let nodes = route.nodes.iter().map(|n| Shareable::Number(f64::from(*n)));
    let waypoints = route.nodes.iter().map(|n| Shareable::Host(HostValue::Vec3(graph.at(*n))));
    let fields: Vec<(Arc<str>, Shareable)> = vec![
        ("graph".into(), Shareable::Number(graph.revision())),
        ("query".into(), echo.clone()),
        ("outcome".into(), outcome),
        ("nodes".into(), Shareable::List(nodes.collect())),
        ("waypoints".into(), Shareable::List(waypoints.collect())),
        ("retryAt".into(), retry_at),
    ];
    Shareable::Record(fields.into())
}

pub(super) fn register(reg: &mut crate::host_registry::Registry) {
    reg.fn1("Nav.graph", "Nav.graph({ nodes, links })", |GraphData(nodes, links)| {
        Graph::new(nodes, links)
            .map(|graph| FunctorLangNavGraph(Arc::new(graph)))
            .map_err(|e| format!("Nav.graph: {e}"))
    });
    reg.fn1("Nav.revision", "Nav.revision(graph)", |graph: FunctorLangNavGraph| {
        Value::Number(graph.0.revision())
    });
    reg.fn4(
        "Nav.path",
        "Nav.path(graph, key, query, tagger)",
        |graph: FunctorLangNavGraph, key: String, PathQuery(query, echo), tagger: Tagger| {
            check_nodes(&query, &graph.0)?;
            Ok::<_, String>(FunctorLangEffect(EffectTree::NavPath {
                key,
                graph: graph.0,
                query: Arc::new(query),
                echo,
                tagger: tagger.0,
            }))
        },
    );
    reg.fn1("Nav.cancel", "Nav.cancel(key)", |key: String| {
        FunctorLangEffect(EffectTree::NavCancel { key })
    });
}
