//! Worlds: a session you can come back to.
//!
//! A world is what PyMOL calls a session - the molecules that are open, the graph that
//! draws them, the styles, and where the camera was left - under a name you chose. The app
//! opens on a shelf of them rather than on an empty viewer, because a workbench that
//! forgets what you were doing between launches is a workbench you re-set-up every morning.
//!
//! Stored as one JSON list in the settings table rather than a file each: a world is a
//! graph and a camera, which is kilobytes, and keeping them together means renaming one or
//! reordering the shelf is a single write. The one thing kept outside is the picture on the
//! card, which is a PNG and would bloat every read of the list.

use serde::{Deserialize, Serialize};

pub const SETTING: &str = "worlds";
pub const ACTIVE: &str = "active_world";

/// Where the camera was left, in the form the viewer describes itself with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub eye: [f32; 3],
    pub center: [f32; 3],
    pub up: [f32; 3],
    pub radius: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub id: String,
    pub name: String,
    /// Milliseconds since the epoch, for ordering the shelf by when it was last opened.
    pub created: i64,
    pub opened: i64,
    /// Everything the viewer draws.
    pub graph: crate::graph::Graph,
    /// Where the camera was when it was last left. None until it has been opened once.
    #[serde(default)]
    pub camera: Option<Camera>,
}

impl World {
    pub fn new(name: String, graph: crate::graph::Graph) -> Self {
        let now = now_ms();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            created: now,
            opened: now,
            graph,
            camera: None,
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

/// What the shelf shows: a world without its graph, which the shelf never needs.
#[derive(Debug, Clone, Serialize)]
pub struct WorldCard {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub opened: i64,
    /// How much is in it, for the card's one line of detail.
    pub molecules: usize,
    pub chains: usize,
    /// A fingerprint of the graph, so the shelf can tell whether it has to build again.
    ///
    /// The counts above are a summary and a summary cannot answer that question: recolouring
    /// a chain changes what the row shows without changing how many chains there are.
    pub stamp: u64,
}

/// The graph, boiled down to one number.
fn stamp(graph: &crate::graph::Graph) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(graph)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

pub fn card(world: &World) -> WorldCard {
    let molecules = world
        .graph
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, crate::graph::NodeKind::Molecule { .. }))
        .count();
    let chains = world
        .graph
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, crate::graph::NodeKind::Chain { .. }))
        .count();
    WorldCard {
        id: world.id.clone(),
        name: world.name.clone(),
        created: world.created,
        opened: world.opened,
        molecules,
        chains,
        stamp: stamp(&world.graph),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_counts_what_is_in_the_world_without_carrying_it() {
        let graph = crate::graph::default_graph();
        let world = World::new("Test".into(), graph);
        let card = card(&world);
        assert_eq!(card.name, "Test");
        assert_eq!(card.id, world.id);
        // The default graph has no molecule until one is opened.
        assert_eq!(card.molecules, 0);
        assert_eq!(card.opened, card.created);
    }

    #[test]
    fn a_card_is_stamped_by_what_the_graph_holds_rather_than_by_its_name() {
        let world = World::new("Test".into(), crate::graph::default_graph());
        let before = card(&world).stamp;
        // Renaming is not a change the shelf has to redraw geometry for.
        let mut renamed = world.clone();
        renamed.name = "Renamed".into();
        assert_eq!(card(&renamed).stamp, before);
        // Adding a node is.
        let mut grown = world.clone();
        grown.graph.nodes.push(crate::graph::Node::new(
            "extra",
            crate::graph::NodeKind::Chain {
                chain: "A".into(),
                slot: None,
                style: None,
            },
            [0.0, 0.0],
        ));
        assert_ne!(card(&grown).stamp, before);
    }
}
