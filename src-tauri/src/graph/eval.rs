//! Evaluating a graph.
//!
//! Deliberately pure: this decides *what* to do - which structure, drawn how, lit how -
//! and never does it. Fetching, prediction and geometry building all happen in the command
//! layer with this as the plan. That split is what makes the graph testable at all, since
//! the interesting cases (a cycle, a missing input, a prediction that has not finished)
//! need no network, no GPU and no Blender.

use super::model::{Graph, Node, NodeKind};
use crate::compute::job::JobState;
use crate::geometry::{Assignment, SceneOptions, SceneStyle, Style};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where a structure comes from, once the graph has been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "source", rename_all = "kebab-case")]
pub enum StructureRef {
    File {
        path: PathBuf,
        /// Built as its biological assembly rather than as written.
        assembly: bool,
        /// Which assembly, model and conformers.
        version: super::model::Version,
    },
    Pdb {
        id: String,
        assembly: bool,
        version: super::model::Version,
    },
    /// A prediction's output. The job may not have finished.
    Job { job_id: String },
}

/// One molecule in the scene: where it comes from, and how its chains are drawn.
#[derive(Debug, Clone, Serialize)]
pub struct PlanPart {
    pub structure: StructureRef,
    /// Which atoms this molecule includes.
    pub options: SceneOptions,
    /// One style per chain, plus the unassigned one.
    pub assignments: Vec<Assignment>,
    /// The molecule's residue-level features.
    pub features: Vec<crate::geometry::Feature>,
}

/// What the output node resolves to.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// One per Molecule group feeding the output, in graph order. A viewport draws a
    /// scene, and a scene is not limited to one molecule.
    pub parts: Vec<PlanPart>,
    pub style: SceneStyle,
}

/// Per-node status, drawn on the node in the editor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum NodeState {
    /// Configured and reachable from the output.
    Ready,
    /// Usable, but something in it is being ignored and the user should know which.
    ///
    /// A state rather than a message on the side, because it is the node that is odd -
    /// a Molecule node holding both a file and a PDB id works, and uses one of them.
    Warned {
        message: String,
    },
    /// Configured but not connected to anything that uses it.
    Unused,
    /// Waiting on a job.
    Running {
        job_id: String,
    },
    /// Needs something before it can be used.
    Incomplete {
        reason: String,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Evaluation {
    pub plan: Option<Plan>,
    /// One per Render node. Each is its own output with its own pipeline behind it, so a
    /// graph can hold a draft render and a final one and they need not agree.
    pub renders: Vec<RenderTarget>,
    /// Keyed by node id, so the editor can mark each node without searching.
    pub node_states: BTreeMap<String, NodeState>,
    /// Problems in graph terms, for a status line under the editor.
    pub problems: Vec<String>,
}

/// What a Render node resolves to. `plan` is `None` when it is not ready to render.
#[derive(Debug, Clone, Serialize)]
pub struct RenderTarget {
    pub node_id: String,
    pub plan: Option<Plan>,
    /// Why it cannot render, when it cannot.
    pub problem: Option<String>,
}

/// One output's resolved pipeline: the plan, the states it touched, what went wrong.
///
/// Returned rather than written in place so that resolving a Render node cannot disturb
/// the Viewport's own result - each output is read independently and then merged.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub plan: Option<Plan>,
    pub states: BTreeMap<String, NodeState>,
    pub problems: Vec<String>,
}

/// What a prediction node needs to know from outside to report its state.
#[derive(Debug, Clone)]
pub struct JobStatus {
    pub job_id: String,
    pub state: JobState,
    /// The primary structure the job produced, once it succeeded.
    pub structure: Option<PathBuf>,
    pub error: Option<String>,
}

/// Reads the graph and produces the plan.
///
/// `jobs` supplies the state of any prediction node's job. Passed in rather than looked up
/// so this stays pure.
pub fn evaluate(graph: &Graph, jobs: &BTreeMap<String, JobStatus>) -> Evaluation {
    let errors = graph.validate();
    // Asked of the errors themselves, not of their wording: a node whose id happens to
    // contain "cycle", named in any other message, would otherwise switch the whole
    // evaluation off.
    let cyclic = errors
        .iter()
        .any(|error| matches!(error, super::model::GraphError::Cycle(_)));
    let mut problems: Vec<String> = errors.into_iter().map(|error| error.to_string()).collect();
    // Said, so a node that is missing from the editor is not mistaken for one deleted.
    if !graph.unread.is_empty() {
        problems.push(format!(
            "{} node(s) from a newer version could not be read; they are kept but not shown",
            graph.unread.len()
        ));
    }

    let mut node_states: BTreeMap<String, NodeState> = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), NodeState::Unused))
        .collect();

    // Anything a cycle touches cannot be evaluated without recursing forever, and the
    // problem list already says so.

    let output = graph.output_node();
    if let Err(error) = &output {
        if problems.is_empty() {
            problems.push(error.to_string());
        }
    }

    let mut plan = None;
    if let (Ok(output), false) = (&output, cyclic) {
        let resolved = resolve(graph, output, jobs);
        node_states.extend(resolved.states);
        plan = resolved.plan;
        for problem in resolved.problems {
            problems = push_unique(problems, &problem);
        }
    }

    // Each Render node is resolved on its own, and what it cannot do is reported on it
    // rather than merged into the viewport's problems - a draft render that is still being
    // wired up must not make a working viewport read as broken. `validate` still counts an
    // unconnected required input as a graph problem, because it is one.
    let mut renders = Vec::new();
    if !cyclic {
        for node in graph.render_nodes() {
            let resolved = resolve(graph, node, jobs);
            node_states.extend(resolved.states);
            // Whatever the render hands its image to is part of the live path. Without
            // this an Image node is never marked, so it draws as unused - dimmed, and
            // showing the plane's grid through the picture.
            for consumer in graph.consumers(&node.id) {
                mark(&mut node_states, &consumer.id, NodeState::Ready);
            }
            renders.push(RenderTarget {
                node_id: node.id.clone(),
                plan: resolved.plan,
                problem: resolved.problems.first().cloned(),
            });
        }
    }

    Evaluation {
        plan,
        renders,
        node_states,
        problems,
    }
}

/// Reads one output node's pipeline back to its structure source.
///
/// Public because a render is started from the command layer with the same plan the
/// viewport was drawn from - resolving it twice, by two different routes, is how the
/// preview and the render drift apart.
pub fn resolve(graph: &Graph, output: &Node, jobs: &BTreeMap<String, JobStatus>) -> Resolution {
    // A render resolves through the viewport it is watching: a view carries the geometry,
    // the lighting and the world, so there is nothing else for it to read.
    if matches!(output.kind, NodeKind::Render { .. }) {
        let Some(viewed) = graph
            .sources(&output.id, "View")
            .into_iter()
            .find(|node| matches!(node.kind, NodeKind::Viewport))
        else {
            let mut states = BTreeMap::new();
            mark(
                &mut states,
                &output.id,
                NodeState::Incomplete {
                    reason: "no view connected".into(),
                },
            );
            return Resolution {
                plan: None,
                states,
                problems: vec!["a Render has no view connected".into()],
            };
        };
        let mut resolution = resolve(graph, viewed, jobs);
        mark(&mut resolution.states, &output.id, NodeState::Ready);
        return resolution;
    }

    let mut states = BTreeMap::new();
    let mut problems = Vec::new();
    mark(&mut states, &output.id, NodeState::Ready);

    // Style comes from the World, Material and Light nodes; mark them as used.
    for socket in ["World", "Material", "Lights"] {
        for node in graph.sources(&output.id, socket) {
            mark(&mut states, &node.id, NodeState::Ready);
        }
    }
    let style = graph.style_for(&output.id);

    let incomplete = |states: BTreeMap<String, NodeState>, problems: Vec<String>, reason: &str| {
        let mut states = states;
        mark(
            &mut states,
            &output.id,
            NodeState::Incomplete {
                reason: reason.into(),
            },
        );
        Resolution {
            plan: None,
            states,
            problems: push_unique(problems, reason),
        }
    };

    // The output draws chains. Each is a node with its own style, and each names the
    // molecule it reads from - so several chains of one structure become one part of the
    // plan with per-chain assignments, and two structures become two parts.
    let chains = graph.sources(&output.id, "Geometry");
    if chains.is_empty() {
        return incomplete(states, problems, "no chain connected");
    }

    // A chain asking for a surface the renderers cannot give it is said out loud rather
    // than half-applied.
    let unrendered = graph.unrendered_materials(&output.id);

    let mut parts: Vec<PlanPart> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut blocked: Option<String> = None;

    for chain in chains {
        let NodeKind::Chain {
            chain: chain_id, ..
        } = &chain.kind
        else {
            return incomplete(states, problems, "the geometry input is not a Chain");
        };
        mark(&mut states, &chain.id, NodeState::Ready);
        if unrendered.contains(&chain.id) {
            mark(
                &mut states,
                &chain.id,
                NodeState::Warned {
                    message: "this chain's surface is not rendered - one material covers \
                              the scene"
                        .into(),
                },
            );
        }
        for node in graph.sources(&chain.id, "Style") {
            mark(&mut states, &node.id, NodeState::Ready);
        }

        let Some(molecule) = graph.molecule_of(&chain.id) else {
            blocked = Some(
                blocked
                    .unwrap_or_else(|| format!("chain {chain_id} is not connected to a molecule")),
            );
            continue;
        };

        // A molecule reads its structure from upstream when something is connected, and
        // from its own id or file otherwise.
        let source = graph
            .sources(&molecule.id, "Structure")
            .first()
            .copied()
            .unwrap_or(molecule);
        let (structure, source_state) = resolve_structure(graph, source, jobs);
        mark(&mut states, &source.id, source_state.clone());
        if source.id != molecule.id {
            mark(&mut states, &molecule.id, NodeState::Ready);
        }
        for upstream in graph.sources(&source.id, "Sequence") {
            mark(&mut states, &upstream.id, NodeState::Ready);
        }

        let Some(structure) = structure else {
            let reason = match &source_state {
                NodeState::Running { .. } => "waiting for a prediction".to_string(),
                NodeState::Failed { message } => message.clone(),
                NodeState::Incomplete { reason } => reason.clone(),
                _ => "the structure source produced nothing".to_string(),
            };
            blocked = Some(blocked.unwrap_or(reason));
            continue;
        };

        let assignment = Assignment {
            chain: Some(chain_id.clone()),
            style: graph.chain_style(&chain.id),
        };
        // One part per molecule: the chains of one structure share its atoms, so they
        // share a part and differ only in their assignments.
        match sources.iter().position(|id| id == &molecule.id) {
            Some(index) => parts[index].assignments.push(assignment),
            None => {
                sources.push(molecule.id.clone());
                parts.push(PlanPart {
                    structure,
                    options: graph.options(&molecule.id),
                    features: graph.features(&molecule.id),
                    // The unassigned style draws nothing: a chain with no node of its own
                    // is not in the scene at all, which is how leaving one out works.
                    assignments: vec![
                        Assignment {
                            chain: Some(String::new()),
                            style: Style {
                                representations: Vec::new(),
                                ..Default::default()
                            },
                        },
                        assignment,
                    ],
                });
            }
        }
    }

    let plan = match &blocked {
        Some(reason) => {
            problems = push_unique(problems, reason);
            None
        }
        None => Some(Plan { parts, style }),
    };

    Resolution {
        plan,
        states,
        problems,
    }
}

fn resolve_structure(
    graph: &Graph,
    node: &Node,
    jobs: &BTreeMap<String, JobStatus>,
) -> (Option<StructureRef>, NodeState) {
    match &node.kind {
        NodeKind::Molecule {
            pdb_id,
            path,
            assembly,
            version,
            ..
        } => {
            // An entry is fetched to be looked at as the molecule it is, so its assembly
            // unless told otherwise; a file is somebody's own and is shown as written
            // unless told otherwise.
            let of_entry = assembly.unwrap_or(true);
            let of_file = assembly.unwrap_or(false);
            let id = pdb_id.trim();
            let file = path
                .as_ref()
                .filter(|path| !path.as_os_str().is_empty())
                .cloned();
            match (file, id.len()) {
                // Both filled in is not an error - it works - but only one of them is
                // being used, and which one has to be said rather than guessed at.
                (Some(path), 4) => (
                    Some(StructureRef::File {
                        path,
                        assembly: of_file,
                        version: version.clone(),
                    }),
                    NodeState::Warned {
                        message: format!("using the file; {} is ignored", id.to_uppercase()),
                    },
                ),
                (Some(path), _) => (
                    Some(StructureRef::File {
                        path,
                        assembly: of_file,
                        version: version.clone(),
                    }),
                    NodeState::Ready,
                ),
                (None, 4) => (
                    Some(StructureRef::Pdb {
                        id: id.to_string(),
                        assembly: of_entry,
                        version: version.clone(),
                    }),
                    NodeState::Ready,
                ),
                (None, 0) => (
                    None,
                    NodeState::Incomplete {
                        reason: "no molecule yet".into(),
                    },
                ),
                (None, _) => (
                    None,
                    NodeState::Incomplete {
                        reason: "a PDB id is four characters, like 1ubq".into(),
                    },
                ),
            }
        }
        NodeKind::Predict { job_id, .. } => {
            let Some(job_id) = job_id else {
                // The command layer submits it; until then the node is what needs doing.
                let ready = graph
                    .job_request(node)
                    .map(|request| !request.inputs.is_empty())
                    .unwrap_or(false);
                return (
                    None,
                    if ready {
                        NodeState::Incomplete {
                            reason: "not run yet".into(),
                        }
                    } else {
                        NodeState::Incomplete {
                            reason: "no sequence to predict from".into(),
                        }
                    },
                );
            };
            match jobs.get(job_id) {
                None => (
                    None,
                    NodeState::Running {
                        job_id: job_id.clone(),
                    },
                ),
                Some(status) if status.state == JobState::Succeeded => match &status.structure {
                    Some(_) => (
                        Some(StructureRef::Job {
                            job_id: job_id.clone(),
                        }),
                        NodeState::Ready,
                    ),
                    None => (
                        None,
                        NodeState::Failed {
                            message: "the prediction produced no structure".into(),
                        },
                    ),
                },
                Some(status) if status.state.is_terminal() => (
                    None,
                    NodeState::Failed {
                        message: status
                            .error
                            .clone()
                            .unwrap_or_else(|| format!("the prediction {}", status.state.as_str())),
                    },
                ),
                Some(_) => (
                    None,
                    NodeState::Running {
                        job_id: job_id.clone(),
                    },
                ),
            }
        }
        other => (
            None,
            NodeState::Failed {
                message: format!("{} is not a structure source", other.type_name()),
            },
        ),
    }
}

fn mark(states: &mut BTreeMap<String, NodeState>, id: &str, state: NodeState) {
    states.insert(id.to_string(), state);
}

fn push_unique(mut problems: Vec<String>, message: &str) -> Vec<String> {
    if !problems.iter().any(|existing| existing == message) {
        problems.push(message.to_string());
    }
    problems
}

/// The graph a new session starts with.
///
/// The scene without a molecule in it: a light, a world and the two outputs, wired to each
/// other. Fetching is what adds a molecule, a node for each of its chains and a style for
/// them, so shipping an empty Molecule and an unused Style would only be two nodes to
/// delete before starting.
pub fn default_graph() -> Graph {
    use crate::geometry::SceneStyle;
    use crate::graph::RenderEngine;
    let style = SceneStyle::default();

    let mut nodes = vec![
        Node::new(
            "world",
            NodeKind::World {
                background: style.background,
                ambient: style.ambient,
                ambient_occlusion: style.ambient_occlusion,
                outline: style.outline,
            },
            [300.0, 330.0],
        ),
        Node::new("viewport", NodeKind::Viewport, [610.0, 150.0]),
        // Folded, because its settings are not what a new user needs to see first - but
        // present, so that rendering is something you can find rather than something you
        // have to know to add.
        Node {
            collapsed: true,
            ..Node::new(
                "render",
                NodeKind::Render {
                    engine: RenderEngine::Cycles,
                    samples: 64,
                    width: 1600,
                    height: 1200,
                    transparent: false,
                    output: None,
                },
                [610.0, 250.0],
            )
        },
    ];

    let mut links = vec![
        // The render follows the viewport out of the box: most renders are of what is on
        // screen, and without a camera object there is nothing else to follow.
        Link {
            from: "viewport".into(),
            to: "render".into(),
            socket: "View".into(),
        },
        // No chain links yet: chains are nodes, and they do not exist until a molecule
        // has been fetched. Fetching wires them to the viewport.
        Link {
            from: "world".into(),
            to: "viewport".into(),
            socket: "World".into(),
        },
    ];

    // A key and a fill, not the whole three-point rig and not one lamp either. One light
    // leaves everything facing away from it on the ambient term alone, which reads as a
    // structure with a dark half; a fill at a third of the key's strength from the other
    // side fixes that with one more node. A rim is one node away when a figure needs it.
    for (index, light) in style.lights.iter().take(2).enumerate() {
        let id = format!("light-{}", light.id);
        nodes.push(Node::new(
            id.clone(),
            NodeKind::Light {
                light: light.clone(),
            },
            [40.0, 340.0 + index as f32 * 150.0],
        ));
        // The viewport only: the render watches the view, and a view carries the lighting
        // with it.
        links.push(Link {
            from: id,
            to: "viewport".into(),
            socket: "Lights".into(),
        });
    }

    Graph {
        nodes,
        links,
        ..Default::default()
    }
}

use super::model::Link;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Representation;
    use crate::graph::GraphError;

    fn jobs() -> BTreeMap<String, JobStatus> {
        BTreeMap::new()
    }

    /// The graph after fetching 1ubq: what `buildChains` in the UI puts together - a
    /// molecule, a style, and a node for each chain wired to both outputs.
    fn graph_with_id() -> Graph {
        use crate::graph::Link;
        let mut graph = default_graph();
        graph.nodes.push(Node::new(
            "source",
            NodeKind::Molecule {
                version: Default::default(),
                pdb_id: "1ubq".into(),
                assembly: None,
                path: None,
                options: SceneOptions::default(),
                chains: vec!["A".into()],
                expanded: true,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            },
            [40.0, 60.0],
        ));
        graph.nodes.push(Node::new(
            "style",
            NodeKind::Style {
                style: Style {
                    representations: vec![Representation::Cartoon],
                    scheme: crate::geometry::ColorScheme::Secondary,
                    ..Default::default()
                },
            },
            [40.0, 250.0],
        ));
        graph.nodes.push(Node::new(
            "chain-A",
            NodeKind::Chain {
                chain: "A".into(),
                slot: None,
                style: None,
            },
            [340.0, 60.0],
        ));
        for (from, to, socket) in [
            ("source", "chain-A", "Molecule"),
            ("style", "chain-A", "Style"),
            ("chain-A", "viewport", "Geometry"),
        ] {
            graph.links.push(Link {
                from: from.into(),
                to: to.into(),
                socket: socket.into(),
            });
        }
        graph
    }

    #[test]
    fn the_default_render_follows_the_viewport() {
        let graph = default_graph();
        let render = graph.node("render").expect("a Render node");
        assert!(render.collapsed, "it should not be in the way on first run");
        assert!(graph.validate().is_empty());
        // Wired out of the box: without a camera object there is nothing else to follow.
        assert_eq!(graph.sources("render", "View")[0].id, "viewport");
    }

    #[test]
    fn a_render_wired_to_the_view_draws_exactly_what_the_viewport_draws() {
        let graph = graph_with_id();
        let evaluation = evaluate(&graph, &jobs());
        let viewport = evaluation.plan.expect("the viewport has a plan");
        let render = evaluation.renders[0]
            .plan
            .as_ref()
            .expect("the render has one too");

        assert_eq!(render.parts.len(), viewport.parts.len());
        assert_eq!(render.parts[0].structure, viewport.parts[0].structure);
        assert_eq!(render.parts[0].assignments, viewport.parts[0].assignments);
        // Same rig, so the render matches the preview rather than resembling it.
        assert_eq!(render.style.background, viewport.style.background);
        assert_eq!(render.style.lights.len(), viewport.style.lights.len());
        assert_eq!(evaluation.node_states["render"], NodeState::Ready);
    }

    #[test]
    fn a_chain_can_pick_a_style_without_a_node_for_it() {
        let mut graph = graph_with_id();
        // Unwire the style node and give the chain one of its own, as the picker does.
        graph.links.retain(|link| link.socket != "Style");
        if let Some(node) = graph.node_mut("chain-A") {
            node.kind = NodeKind::Chain {
                chain: "A".into(),
                slot: None,
                style: Some(Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                }),
            };
        }
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::Spacefill
        );

        // A wired node wins over the picked one: the socket is the override.
        graph.links.push(crate::graph::Link {
            from: "style".into(),
            to: "chain-A".into(),
            socket: "Style".into(),
        });
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::Cartoon
        );
    }

    #[test]
    fn a_chain_draws_with_the_molecules_slot_it_names() {
        let mut graph = graph_with_id();
        graph.links.retain(|link| link.socket != "Style");
        // Two slots on the molecule, the way the Style tab's slot list writes them.
        if let Some(node) = graph.node_mut("source") {
            if let NodeKind::Molecule {
                slots, active_slot, ..
            } = &mut node.kind
            {
                *slots = vec![
                    crate::graph::StyleSlot::new(
                        "Cartoon",
                        Style {
                            representations: vec![Representation::Cartoon],
                            ..Default::default()
                        },
                    ),
                    crate::graph::StyleSlot::new(
                        "Sticks",
                        Style {
                            representations: vec![Representation::BallAndStick],
                            ..Default::default()
                        },
                    ),
                ];
                *active_slot = 1;
            }
        }

        // No slot named: the first one, so a chain added later is not invisible.
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::Cartoon
        );

        let name_slot = |graph: &mut crate::graph::Graph, index: Option<usize>| {
            if let Some(node) = graph.node_mut("chain-A") {
                if let NodeKind::Chain { slot, .. } = &mut node.kind {
                    *slot = index;
                }
            }
        };

        name_slot(&mut graph, Some(1));
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::BallAndStick
        );
        assert_eq!(graph.slots("source").len(), 2);
        assert_eq!(
            graph.slot_for("chain-A").map(|slot| slot.name.as_str()),
            Some("Sticks")
        );

        // A slot removed from under a chain leaves it looking like its molecule rather
        // than like a fresh one: the index falls back to the first slot, not to the
        // default style.
        name_slot(&mut graph, Some(7));
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::Cartoon
        );

        // A slot beats a style the chain carried from before slots existed, and a wired
        // Style node still beats the slot.
        if let Some(node) = graph.node_mut("chain-A") {
            node.kind = NodeKind::Chain {
                chain: "A".into(),
                slot: Some(1),
                style: Some(Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                }),
            };
        }
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::BallAndStick
        );
        graph.links.push(crate::graph::Link {
            from: "style".into(),
            to: "chain-A".into(),
            socket: "Style".into(),
        });
        assert_eq!(
            graph.chain_style("chain-A").primary(),
            Representation::Cartoon
        );
    }

    #[test]
    fn a_render_has_nothing_to_configure_but_its_quality() {
        // One input, the view. A view already carries the geometry, the lighting and the
        // world, so copies of those on the render could only ever disagree with the
        // picture on screen - and there is no camera object to make the disagreement
        // meaningful.
        let graph = default_graph();
        let render = graph.node("render").expect("a Render node");
        let inputs: Vec<&str> = render
            .kind
            .inputs()
            .iter()
            .map(|socket| socket.name.as_str().to_owned().leak() as &str)
            .collect();
        assert_eq!(inputs, vec!["View"]);
    }

    #[test]
    fn a_render_with_no_view_says_so() {
        let mut graph = graph_with_id();
        graph.links.retain(|link| link.socket != "View");
        let evaluation = evaluate(&graph, &jobs());
        // The viewport is fine; the render is the thing that is unfinished.
        assert!(evaluation.plan.is_some());
        assert_eq!(
            evaluation.node_states["render"],
            NodeState::Incomplete {
                reason: "no view connected".into()
            }
        );
        assert!(evaluation.renders[0].plan.is_none());
    }

    #[test]
    fn two_render_nodes_are_allowed_but_two_viewports_are_not() {
        use crate::graph::{Link, RenderEngine};
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "draft",
            NodeKind::Render {
                engine: RenderEngine::Eevee,
                samples: 16,
                width: 640,
                height: 480,
                transparent: true,
                output: None,
            },
            [900.0, 400.0],
        ));
        graph.links.push(Link {
            from: "viewport".into(),
            to: "draft".into(),
            socket: "View".into(),
        });

        // A draft render and a final render is the reason Render is not limited to one.
        assert!(graph.validate().is_empty(), "{:?}", graph.validate());
        assert_eq!(graph.render_nodes().len(), 2);
        let evaluation = evaluate(&graph, &jobs());
        assert_eq!(evaluation.renders.len(), 2);
        assert!(evaluation
            .renders
            .iter()
            .all(|target| target.plan.is_some()));

        // The Viewport is still exactly one.
        graph
            .nodes
            .push(Node::new("second", NodeKind::Viewport, [1200.0, 0.0]));
        assert!(matches!(
            graph.output_node(),
            Err(crate::graph::GraphError::ManyOutputs(2))
        ));
    }

    #[test]
    fn a_style_is_built_from_the_network_wired_into_it() {
        let graph = graph_with_id();
        let style = graph.style_of("style").expect("the style resolves");
        // Straight off the node: a style is picked, not assembled, unless it is wired.
        assert_eq!(style.primary(), Representation::Cartoon);
        assert_eq!(style.scheme, crate::geometry::ColorScheme::Secondary);
        assert_eq!(style.material, crate::geometry::Material::default());
        assert!(style.solid.is_none());
    }

    #[test]
    fn a_solid_colour_replaces_the_colouring_rule() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "orange",
            NodeKind::SolidColour {
                color: [1.0, 0.216, 0.0],
            },
            [40.0, 420.0],
        ));
        graph.links.push(Link {
            from: "orange".into(),
            to: "style".into(),
            socket: "Colour".into(),
        });

        let style = graph.style_of("style").expect("the style resolves");
        assert_eq!(style.solid, Some([1.0, 0.216, 0.0]));

        // And it reaches the plan, which is what actually gets drawn.
        let plan = evaluate(&graph, &jobs()).plan.expect("a plan");
        assert_eq!(
            plan.parts[0].assignments[1].style.solid,
            Some([1.0, 0.216, 0.0])
        );
    }

    #[test]
    fn a_surface_wired_to_a_style_reaches_the_scene() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "surface",
            NodeKind::Material {
                material: crate::geometry::Material {
                    roughness: 0.9,
                    specular: 0.1,
                    metallic: 0.0,
                },
            },
            [40.0, 380.0],
        ));
        graph.links.push(Link {
            from: "surface".into(),
            to: "style".into(),
            socket: "Surface".into(),
        });
        let plan = evaluate(&graph, &jobs()).plan.expect("a plan");
        assert_eq!(plan.parts[0].assignments[1].style.material.roughness, 0.9);
        // Both renderers shade with one material, taken from the first chain.
        assert_eq!(plan.style.material.roughness, 0.9);
    }

    #[test]
    fn a_per_chain_surface_is_reported_as_unrenderable() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "chain-B",
            NodeKind::Chain {
                chain: "B".into(),
                slot: None,
                style: None,
            },
            [340.0, 200.0],
        ));
        graph.nodes.push(Node::new(
            "glossy-style",
            NodeKind::Style {
                style: Style {
                    material: crate::geometry::Material {
                        roughness: 0.05,
                        specular: 0.9,
                        metallic: 0.0,
                    },
                    ..Default::default()
                },
            },
            [40.0, 480.0],
        ));
        for (from, to, socket) in [
            ("source", "chain-B", "Molecule"),
            ("glossy-style", "chain-B", "Style"),
            ("chain-B", "viewport", "Geometry"),
        ] {
            graph.links.push(Link {
                from: from.into(),
                to: to.into(),
                socket: socket.into(),
            });
        }

        // Chain A's surface is the scene's, so B's is the one that cannot be honoured.
        assert_eq!(
            graph.unrendered_materials("viewport"),
            vec!["chain-B".to_string()]
        );

        if let Some(node) = graph.node_mut("glossy-style") {
            node.kind = NodeKind::Style {
                style: Style::default(),
            };
        }
        assert!(graph.unrendered_materials("viewport").is_empty());
    }

    #[test]
    fn a_molecule_node_takes_a_file_or_an_id() {
        let mut graph = graph_with_id();
        assert_eq!(
            evaluate(&graph, &jobs()).plan.expect("a plan").parts[0]
                .structure
                .clone(),
            StructureRef::Pdb {
                version: Default::default(),
                id: "1ubq".into(),
                assembly: true,
            }
        );

        // A file wins, and the node says the id is being ignored rather than quietly
        // picking one.
        if let Some(node) = graph.node_mut("source") {
            node.kind = NodeKind::Molecule {
                version: Default::default(),
                pdb_id: "1ubq".into(),
                assembly: None,
                path: Some(PathBuf::from("/tmp/6vxx.cif")),
                options: SceneOptions::default(),
                chains: vec!["A".into()],
                expanded: true,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            };
        }
        let evaluation = evaluate(&graph, &jobs());
        assert_eq!(
            evaluation.plan.expect("a plan").parts[0].structure.clone(),
            StructureRef::File {
                version: Default::default(),
                path: PathBuf::from("/tmp/6vxx.cif"),
                assembly: false,
            }
        );
        assert_eq!(
            evaluation.node_states["source"],
            NodeState::Warned {
                message: "using the file; 1UBQ is ignored".into()
            }
        );
        assert!(evaluation.problems.is_empty(), "{:?}", evaluation.problems);
    }

    #[test]
    fn an_empty_molecule_node_says_so_without_talking_about_pdb_ids() {
        let mut graph = graph_with_id();
        if let Some(node) = graph.node_mut("source") {
            node.kind = NodeKind::Molecule {
                version: Default::default(),
                pdb_id: "1ub".into(),
                assembly: None,
                path: None,
                options: SceneOptions::default(),
                chains: vec!["A".into()],
                expanded: true,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            };
        }
        // A half-typed id is a different complaint from an empty node.
        assert_eq!(
            evaluate(&graph, &jobs()).node_states["source"],
            NodeState::Incomplete {
                reason: "a PDB id is four characters, like 1ubq".into()
            }
        );
    }

    #[test]
    fn whatever_a_render_feeds_is_not_marked_unused() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "picture",
            NodeKind::Image { path: None },
            [900.0, 250.0],
        ));
        graph.links.push(Link {
            from: "render".into(),
            to: "picture".into(),
            socket: "Image".into(),
        });

        // Unused draws dimmed, and a dimmed image node shows the editor's grid through
        // the picture - which is what it looked like before this.
        assert_eq!(
            evaluate(&graph, &jobs()).node_states["picture"],
            NodeState::Ready
        );
    }

    #[test]
    fn a_viewport_draws_every_molecule_wired_to_it() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        // A second molecule from a file, with its own chain node and its own style.
        graph.nodes.push(Node::new(
            "ligand",
            NodeKind::Molecule {
                version: Default::default(),
                pdb_id: String::new(),
                assembly: None,
                path: Some(PathBuf::from("/tmp/ligand.cif")),
                options: SceneOptions::default(),
                chains: vec!["X".into()],
                expanded: true,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            },
            [40.0, 620.0],
        ));
        graph.nodes.push(Node::new(
            "ligand-chain",
            NodeKind::Chain {
                chain: "X".into(),
                slot: None,
                style: None,
            },
            [340.0, 620.0],
        ));
        graph.nodes.push(Node::new(
            "ligand-style",
            NodeKind::Style {
                style: Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                },
            },
            [40.0, 760.0],
        ));
        for (from, to, socket) in [
            ("ligand", "ligand-chain", "Molecule"),
            ("ligand-style", "ligand-chain", "Style"),
            ("ligand-chain", "viewport", "Geometry"),
        ] {
            graph.links.push(Link {
                from: from.into(),
                to: to.into(),
                socket: socket.into(),
            });
        }

        assert!(graph.validate().is_empty(), "{:?}", graph.validate());
        let plan = evaluate(&graph, &jobs()).plan.expect("a plan");
        // Two molecules, two parts, each with its own chains' styles.
        assert_eq!(plan.parts.len(), 2);
        assert_eq!(
            plan.parts[0].structure,
            StructureRef::Pdb {
                version: Default::default(),
                id: "1ubq".into(),
                assembly: true,
            }
        );
        assert_eq!(
            plan.parts[1].structure,
            StructureRef::File {
                version: Default::default(),
                path: PathBuf::from("/tmp/ligand.cif"),
                assembly: false,
            }
        );
        assert_eq!(
            plan.parts[0].assignments[1].style.primary(),
            Representation::Cartoon
        );
        assert_eq!(
            plan.parts[1].assignments[1].style.primary(),
            Representation::Spacefill
        );
    }

    #[test]
    fn one_unresolved_molecule_holds_up_the_whole_frame() {
        use crate::graph::Link;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "empty",
            NodeKind::Molecule {
                version: Default::default(),
                pdb_id: String::new(),
                assembly: None,
                path: None,
                options: SceneOptions::default(),
                chains: vec!["A".into()],
                expanded: true,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            },
            [40.0, 620.0],
        ));
        graph.nodes.push(Node::new(
            "empty-chain",
            NodeKind::Chain {
                chain: "A".into(),
                slot: None,
                style: None,
            },
            [340.0, 620.0],
        ));
        for (from, to, socket) in [
            ("empty", "empty-chain", "Molecule"),
            ("empty-chain", "viewport", "Geometry"),
        ] {
            graph.links.push(Link {
                from: from.into(),
                to: to.into(),
                socket: socket.into(),
            });
        }

        // Drawing the half that resolved and saying nothing would read as the graph being
        // wrong rather than as a molecule still being unfilled.
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(
            evaluation
                .problems
                .iter()
                .any(|problem| problem.contains("no molecule yet")),
            "{:?}",
            evaluation.problems
        );
    }

    #[test]
    fn a_collapsed_node_survives_a_round_trip() {
        let mut graph = graph_with_id();
        graph.node_mut("render").expect("the render").collapsed = true;
        let text = serde_json::to_string(&graph).expect("serialise");
        // The editor reads this shape, so it is an interface and worth pinning.
        assert!(text.contains(r#""type":"render""#), "{text}");
        assert!(text.contains(r#""type":"chain""#), "{text}");
        assert!(text.contains(r#""collapsed":true"#), "{text}");

        let back: Graph = serde_json::from_str(&text).expect("deserialise");
        assert!(back.node("render").expect("the render").collapsed);
        assert!(!back.node("viewport").expect("the viewport").collapsed);
    }

    #[test]
    fn the_default_graph_waits_for_a_fetch() {
        let graph = default_graph();
        // Valid, but with nothing to draw: chains are nodes, and a fetch is what makes
        // them. This is the state the app opens in.
        let problems = graph.validate();
        assert!(problems.is_empty(), "{problems:?}");
        assert!(graph
            .nodes
            .iter()
            .all(|node| !matches!(node.kind, crate::graph::NodeKind::Chain { .. })));

        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(
            evaluation
                .problems
                .iter()
                .any(|problem| problem.contains("no chain connected")),
            "{:?}",
            evaluation.problems
        );
    }

    #[test]
    fn filling_in_the_id_completes_the_plan() {
        let graph = graph_with_id();
        let evaluation = evaluate(&graph, &jobs());
        let plan = evaluation.plan.expect("a plan");
        assert_eq!(
            plan.parts[0].structure,
            StructureRef::Pdb {
                version: Default::default(),
                id: "1ubq".into(),
                assembly: true,
            }
        );
        // A placeholder that draws nothing, then one assignment per chain node.
        assert_eq!(plan.parts[0].assignments.len(), 2);
        assert!(plan.parts[0].assignments[0]
            .style
            .representations
            .is_empty());
        assert_eq!(plan.parts[0].assignments[1].chain.as_deref(), Some("A"));
        assert_eq!(
            plan.parts[0].assignments[1].style.primary(),
            Representation::Cartoon
        );
        assert_eq!(
            plan.parts[0].assignments[1].style.scheme,
            crate::geometry::ColorScheme::Secondary
        );
        // A key and a fill: one light leaves the far half of every structure on the
        // ambient term alone, which reads as a structure with a dark side.
        assert_eq!(plan.style.lights.len(), 2);
        assert!(
            plan.style.lights.iter().all(|light| light.follow_view),
            "the shipped rig follows the camera, as PyMOL's does"
        );
        assert!(evaluation.problems.is_empty());
    }

    #[test]
    fn the_world_node_drives_the_style() {
        let mut graph = graph_with_id();
        if let Some(node) = graph.node_mut("world") {
            node.kind = NodeKind::World {
                background: [0.0, 0.0, 0.0],
                ambient: 0.1,
                ambient_occlusion: false,
                outline: true,
            };
        }
        let plan = evaluate(&graph, &jobs()).plan.expect("a plan");
        assert_eq!(plan.style.background, [0.0, 0.0, 0.0]);
        assert!(!plan.style.ambient_occlusion);
        assert!(plan.style.outline);
    }

    #[test]
    fn disconnecting_the_lights_falls_back_to_the_default_rig() {
        // An unlit graph is an unfinished one, not a request for a black frame - and the
        // stand-in is one light, the same as the graph ships with, so deleting the light
        // does not brighten the scene.
        let mut graph = graph_with_id();
        graph.links.retain(|link| link.socket != "Lights");
        let plan = evaluate(&graph, &jobs()).plan.expect("a plan");
        assert_eq!(plan.style.lights.len(), 1);
    }

    #[test]
    fn a_prediction_reports_progress_rather_than_a_plan() {
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "seq",
            NodeKind::SequenceInput {
                fasta: "MKTAYIAK".into(),
            },
            [0.0, 0.0],
        ));
        graph.nodes.push(Node::new(
            "predict",
            NodeKind::Predict {
                model_id: "boltz2".into(),
                provider_id: "local".into(),
                inputs: BTreeMap::new(),
                job_id: Some("job-1".into()),
            },
            [0.0, 0.0],
        ));
        graph.links.push(Link {
            from: "seq".into(),
            to: "predict".into(),
            socket: "Sequence".into(),
        });
        graph.links.push(Link {
            from: "predict".into(),
            to: "source".into(),
            socket: "Structure".into(),
        });

        // Running.
        let mut statuses = jobs();
        statuses.insert(
            "job-1".into(),
            JobStatus {
                job_id: "job-1".into(),
                state: JobState::Running,
                structure: None,
                error: None,
            },
        );
        let evaluation = evaluate(&graph, &statuses);
        assert!(evaluation.plan.is_none());
        assert_eq!(
            evaluation.node_states["predict"],
            NodeState::Running {
                job_id: "job-1".into()
            }
        );
        assert!(evaluation.problems.iter().any(|p| p.contains("waiting")));

        // Finished.
        statuses.insert(
            "job-1".into(),
            JobStatus {
                job_id: "job-1".into(),
                state: JobState::Succeeded,
                structure: Some(PathBuf::from("/tmp/model.cif")),
                error: None,
            },
        );
        let evaluation = evaluate(&graph, &statuses);
        assert_eq!(
            evaluation.plan.expect("a plan").parts[0].structure.clone(),
            StructureRef::Job {
                job_id: "job-1".into()
            }
        );
        assert_eq!(evaluation.node_states["seq"], NodeState::Ready);
    }

    #[test]
    fn a_failed_prediction_says_why() {
        let mut graph = graph_with_id();
        graph.links.retain(|link| link.socket != "Structure");
        graph.nodes.push(Node::new(
            "predict",
            NodeKind::Predict {
                model_id: "boltz2".into(),
                provider_id: "local".into(),
                inputs: BTreeMap::from([("sequence".into(), serde_json::json!("MKT"))]),
                job_id: Some("job-2".into()),
            },
            [0.0, 0.0],
        ));
        graph.links.push(Link {
            from: "predict".into(),
            to: "source".into(),
            socket: "Structure".into(),
        });

        let statuses = BTreeMap::from([(
            "job-2".to_string(),
            JobStatus {
                job_id: "job-2".into(),
                state: JobState::Failed,
                structure: None,
                error: Some("out of memory".into()),
            },
        )]);
        let evaluation = evaluate(&graph, &statuses);
        assert!(evaluation.plan.is_none());
        assert_eq!(
            evaluation.node_states["predict"],
            NodeState::Failed {
                message: "out of memory".into()
            }
        );
        assert!(evaluation
            .problems
            .iter()
            .any(|p| p.contains("out of memory")));
    }

    #[test]
    fn a_cycle_is_reported_rather_than_recursed_into() {
        // Without the check this is a stack overflow, which takes the app down instead of
        // showing an error.
        // A molecule feeding its own structure input.
        let mut graph = graph_with_id();
        graph.links.push(Link {
            from: "source".into(),
            to: "source".into(),
            socket: "Structure".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(evaluation.problems.iter().any(|p| p.contains("cycle")));
    }

    #[test]
    fn a_wrongly_typed_link_is_refused() {
        let mut graph = graph_with_id();
        graph.links.push(Link {
            from: "world".into(),
            to: "source".into(),
            socket: "Structure".into(),
        });
        let problems = graph.validate();
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, GraphError::TypeMismatch { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn nodes_nothing_uses_are_marked_unused() {
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "spare",
            NodeKind::Style {
                style: Style {
                    representations: vec![Representation::Spacefill],
                    ..Default::default()
                },
            },
            [0.0, 0.0],
        ));
        let evaluation = evaluate(&graph, &jobs());
        assert_eq!(evaluation.node_states["spare"], NodeState::Unused);
        assert_eq!(evaluation.node_states["style"], NodeState::Ready);
    }

    #[test]
    fn the_graph_round_trips_through_json() {
        // The editor reads and writes this shape, so its field names are an interface.
        // A `Light` flattened into a `Node` put two fields called `id` in one object,
        // which serde resolves by dropping one - and the light silently became the node.
        let graph = graph_with_id();
        let json = serde_json::to_string(&graph).expect("serialises");
        let parsed: Graph = serde_json::from_str(&json).expect("deserialises");

        assert_eq!(parsed.nodes.len(), graph.nodes.len());
        assert_eq!(parsed.links.len(), graph.links.len());

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let source = value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == "source")
            .expect("the source node");
        assert_eq!(source["type"], "molecule");
        assert!(
            source.get("pdb_id").is_some(),
            "the id field must not collide"
        );

        let light = value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["type"] == "light")
            .expect("a light node");
        assert_ne!(
            light["id"], light["light"]["id"],
            "the node's id and the light's id are different things"
        );
        assert!(light["light"]["energy"].is_number());

        // And the round trip still evaluates.
        let mut parsed = parsed;
        if let Some(node) = parsed.node_mut("source") {
            node.kind = NodeKind::Molecule {
                version: Default::default(),
                pdb_id: "1ubq".into(),
                assembly: None,
                path: None,
                options: SceneOptions::default(),
                chains: Vec::new(),
                expanded: false,
                slots: Vec::new(),
                active_slot: 0,
                features: Vec::new(),
                objects: Vec::new(),
            };
        }
        assert!(evaluate(&parsed, &jobs()).plan.is_some());
    }

    #[test]
    fn an_unread_node_is_mentioned_but_does_not_stop_the_viewport() {
        let mut saved = serde_json::to_value(graph_with_id()).unwrap();
        saved["nodes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"id": "future", "type": "hologram"}));
        let graph: Graph = serde_json::from_value(saved).unwrap();
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_some(), "{:?}", evaluation.problems);
        assert!(
            evaluation
                .problems
                .iter()
                .any(|problem| problem.contains("newer version")),
            "{:?}",
            evaluation.problems
        );
        // It is not a node of this graph, so it gets no state.
        assert!(!evaluation.node_states.contains_key("future"));
    }

    #[test]
    fn dangling_links_do_not_stop_the_plan() {
        let mut graph = graph_with_id();
        graph.links.push(Link {
            from: "deleted-chain".into(),
            to: "viewport".into(),
            socket: "Geometry".into(),
        });
        graph.links.push(Link {
            from: "source".into(),
            to: "deleted-chain-2".into(),
            socket: "Molecule".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        // The chain that is there is still drawn; the missing ones are named.
        let plan = evaluation.plan.expect("a plan");
        assert_eq!(plan.parts.len(), 1);
        assert!(evaluation
            .problems
            .iter()
            .any(|problem| problem.contains("deleted-chain")));
    }

    #[test]
    fn a_chain_with_no_molecule_says_which() {
        let mut graph = graph_with_id();
        graph.links.retain(|link| link.socket != "Molecule");
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(
            evaluation
                .problems
                .iter()
                .any(|problem| problem.contains("chain A is not connected to a molecule")),
            "{:?}",
            evaluation.problems
        );
        // Whichever render follows the view is blocked too, and says so on itself.
        assert!(evaluation.renders[0].plan.is_none());
        assert!(evaluation.renders[0].problem.is_some());
    }

    #[test]
    fn something_other_than_a_chain_on_the_geometry_socket_is_reported() {
        let mut graph = graph_with_id();
        graph.links.push(Link {
            from: "source".into(),
            to: "viewport".into(),
            socket: "Geometry".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(evaluation
            .problems
            .iter()
            .any(|problem| problem.contains("not a Chain")));
    }

    #[test]
    fn a_cycle_through_the_outputs_is_reported_not_recursed() {
        // A render's image fed back into the viewport: wrongly typed, and a loop through
        // the one place `resolve` recurses. It must come back as a problem, not overflow.
        let mut graph = graph_with_id();
        graph.links.push(Link {
            from: "render".into(),
            to: "viewport".into(),
            socket: "Geometry".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(evaluation.renders.is_empty());
        assert!(evaluation.problems.iter().any(|p| p.contains("cycle")));
    }

    #[test]
    fn a_render_watching_another_render_is_not_ready() {
        use crate::graph::RenderEngine;
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "second",
            NodeKind::Render {
                engine: RenderEngine::Eevee,
                samples: 1,
                width: 10,
                height: 10,
                transparent: false,
                output: None,
            },
            [0.0, 0.0],
        ));
        graph.links.push(Link {
            from: "render".into(),
            to: "second".into(),
            socket: "View".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        let second = evaluation
            .renders
            .iter()
            .find(|target| target.node_id == "second")
            .expect("the second render is resolved");
        assert!(second.plan.is_none());
    }

    #[test]
    fn a_prediction_with_no_job_and_no_sequence_asks_for_one() {
        let mut graph = graph_with_id();
        graph.nodes.push(Node::new(
            "predict",
            NodeKind::Predict {
                model_id: "boltz2".into(),
                provider_id: "local".into(),
                inputs: BTreeMap::new(),
                job_id: None,
            },
            [0.0, 0.0],
        ));
        graph.links.push(Link {
            from: "predict".into(),
            to: "source".into(),
            socket: "Structure".into(),
        });
        let evaluation = evaluate(&graph, &jobs());
        assert_eq!(
            evaluation.node_states["predict"],
            NodeState::Incomplete {
                reason: "no sequence to predict from".into()
            }
        );
        // A succeeded job that left no structure is a failure, not a hang.
        if let Some(node) = graph.node_mut("predict") {
            if let NodeKind::Predict { job_id, .. } = &mut node.kind {
                *job_id = Some("job-x".into());
            }
        }
        let statuses = BTreeMap::from([(
            "job-x".to_string(),
            JobStatus {
                job_id: "job-x".into(),
                state: JobState::Succeeded,
                structure: None,
                error: None,
            },
        )]);
        assert!(matches!(
            evaluate(&graph, &statuses).node_states["predict"],
            NodeState::Failed { .. }
        ));
    }

    #[test]
    fn an_empty_graph_evaluates_to_a_reason() {
        let evaluation = evaluate(&Graph::default(), &jobs());
        assert!(evaluation.plan.is_none());
        assert!(evaluation.renders.is_empty());
        assert!(!evaluation.problems.is_empty());
    }

    #[test]
    fn two_viewports_are_refused() {
        let mut graph = default_graph();
        graph
            .nodes
            .push(Node::new("second", NodeKind::Viewport, [0.0, 0.0]));
        let evaluation = evaluate(&graph, &jobs());
        assert!(evaluation.plan.is_none());
        assert!(evaluation
            .problems
            .iter()
            .any(|p| p.contains("only be one")));
    }
}
