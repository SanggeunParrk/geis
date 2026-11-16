//! The documents an agent can read about this app.
//!
//! Written for a model rather than for a person: what the node graph is, what a slot means,
//! how a selection is expressed. Embedded in the binary rather than read from disk so that
//! they are there in a packaged build and cannot drift from the version of the app that
//! ships them.
//!
//! The index - a name and a line each - is small enough to put in a system prompt; the
//! bodies are fetched by the `read_doc` tool when the agent decides it needs one. Putting
//! all of them in every prompt would spend the context on things most turns do not need.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Doc {
    pub name: String,
    pub title: String,
    pub summary: String,
    pub body: String,
}

const OVERVIEW: &str = include_str!("../../../docs/agents/overview.md");
const GRAPH: &str = include_str!("../../../docs/agents/graph.md");
const SELECTION: &str = include_str!("../../../docs/agents/selection.md");
const TOOLS: &str = include_str!("../../../docs/agents/tools.md");

pub fn all() -> Vec<Doc> {
    vec![
        Doc {
            name: "overview".into(),
            title: "Geis, for an agent".into(),
            summary: "What the app is, what a structure and a chain are, and how to work.".into(),
            body: OVERVIEW.into(),
        },
        Doc {
            name: "graph".into(),
            title: "The node graph".into(),
            summary: "Node types, links, style slots, representations and colour schemes.".into(),
            body: GRAPH.into(),
        },
        Doc {
            name: "selection".into(),
            title: "Selection".into(),
            summary: "Views, levels, and the conditions a selection is expressed with.".into(),
            body: SELECTION.into(),
        },
        Doc {
            name: "tools".into(),
            title: "Working with the tools".into(),
            summary: "Order of work, argument conventions, failures, permission, reporting.".into(),
            body: TOOLS.into(),
        },
    ]
}

pub fn by_name(name: &str) -> Option<Doc> {
    all().into_iter().find(|doc| doc.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_doc_is_embedded_and_findable() {
        let docs = all();
        assert_eq!(docs.len(), 4);
        for doc in &docs {
            assert!(doc.body.len() > 200, "{} is empty", doc.name);
            assert!(doc.body.starts_with('#'), "{} has no heading", doc.name);
            assert!(!doc.summary.is_empty());
        }
        assert_eq!(by_name("graph").unwrap().title, "The node graph");
        assert!(by_name("nope").is_none());
    }

    #[test]
    fn the_overview_says_what_cannot_be_done() {
        // The one thing an agent must not be vague about.
        let overview = by_name("overview").unwrap().body;
        assert!(overview.contains("no tool that touches Geis's own files"));
    }
}
