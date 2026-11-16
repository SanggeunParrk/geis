//! The node graph.
//!
//! Everything the app does is a graph: where a structure comes from, how it is drawn, how
//! it is lit, and what is done with the result. That includes prediction - a model is a
//! source node like any other, so "predict then look at it" is two connected nodes rather
//! than two separate screens.
//!
//! The graph lives in the core rather than in the editor for the same reason geometry
//! does: it has to be evaluated by whatever consumes it. The viewport, a Blender render
//! and a batch job all evaluate the same graph.

use crate::compute::job::JobRequest;
use crate::geometry::{Categorical, ColorScheme, Light, Material, SceneOptions, SceneStyle, Style};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// What flows along a link.
///
/// Typed because the editor should refuse a connection that cannot mean anything, rather
/// than accepting it and failing at evaluation - which is the difference between a node
/// editor that teaches you the pipeline and one that hides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocketKind {
    /// One or more sequences.
    Sequence,
    /// A loaded structure.
    Structure,
    /// Geometry ready to draw: a structure plus how to draw it.
    Geometry,
    /// A colouring rule.
    Colour,
    Light,
    World,
    Material,
    /// A rendered image on disk.
    Image,
    /// How a molecule, or one of its chains, is drawn.
    Style,
    /// Everything the viewport is drawing.
    View,
}

impl SocketKind {
    /// What to call this kind on screen.
    ///
    /// The serialised name is kebab-case for the wire; showing it to a person puts
    /// `solid-colour` in a row where a title belongs.
    pub fn label(self) -> &'static str {
        match self {
            SocketKind::Sequence => "Sequence",
            SocketKind::Structure => "Structure",
            SocketKind::Geometry => "Geometry",
            SocketKind::Colour => "Colour",
            SocketKind::Light => "Light",
            SocketKind::World => "World",
            SocketKind::Material => "Surface",
            SocketKind::Image => "Image",
            SocketKind::Style => "Style",
            SocketKind::View => "View",
        }
    }

    /// The colour the editor draws this socket and its links in. Sockets are read by
    /// colour before they are read by name, which is why every graph tool colour-codes
    /// them.
    pub fn color(self) -> [u8; 3] {
        match self {
            SocketKind::Sequence => [0x94, 0xd8, 0x2d],  // lime
            SocketKind::Structure => [0x33, 0x9a, 0xf0], // blue
            SocketKind::Geometry => [0x20, 0xc9, 0x97],  // teal
            SocketKind::Colour => [0xf0, 0x65, 0x95],    // pink
            SocketKind::Light => [0xfc, 0xc4, 0x19],     // yellow
            SocketKind::World => [0x86, 0x8e, 0x96],     // gray
            SocketKind::Material => [0xcc, 0x5d, 0xe8],  // grape
            SocketKind::Image => [0xff, 0x92, 0x2b],     // orange
            SocketKind::Style => [0x94, 0x77, 0xf0],     // violet
            // What the viewport is showing, for a render that follows it.
            SocketKind::View => [0xff, 0x92, 0x2b], // orange, like the image it makes
        }
    }
}

/// One input socket on a node.
#[derive(Debug, Clone, Serialize)]
pub struct Socket {
    pub name: String,
    pub kind: SocketKind,
    /// Whether several links may arrive here. Lights do; a structure does not.
    pub multiple: bool,
    pub required: bool,
}

fn socket(name: &str, kind: SocketKind) -> Socket {
    Socket {
        name: name.into(),
        kind,
        multiple: false,
        required: true,
    }
}

fn optional(name: &str, kind: SocketKind) -> Socket {
    Socket {
        required: false,
        ..socket(name, kind)
    }
}

fn many(name: &str, kind: SocketKind) -> Socket {
    Socket {
        multiple: true,
        required: false,
        ..socket(name, kind)
    }
}

/// One style in a molecule's slot list - a Blender material slot.
///
/// Named, because a bundle of styles is only usable if its members can be told apart:
/// "Cartoon teal" and "Ligand sticks" say what a slot is for in a way that slot 0 and
/// slot 1 never will.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleSlot {
    pub name: String,
    pub style: Style,
}

impl StyleSlot {
    pub fn new(name: impl Into<String>, style: Style) -> Self {
        Self {
            name: name.into(),
            style,
        }
    }
}

/// What a node is and what it is configured to do.
/// Which version of an entry a molecule is read as, beyond assembly or not: the assembly
/// by id, the model by number, the alternate-location label. Each unsaid is the default -
/// the first assembly, the first model, each residue's most occupied conformer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assembly_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
}

impl Version {
    pub fn is_default(&self) -> bool {
        *self == Version::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum NodeKind {
    /// A molecule, fetched from the PDB or read from a file.
    ///
    /// One node rather than two, because it answers one question - which molecule - and a
    /// graph in which `1ubq` and `1ubq.cif` are different node types makes the user answer
    /// it twice. A node holding both says so rather than choosing silently.
    ///
    /// The field is `pdb_id` rather than `id` because `Node` already has an `id` and the
    /// kind is flattened into it - two fields called `id` in one object is a collision
    /// that serde resolves by silently dropping one.
    /// A molecule is a source of structure and nothing else. Its chains become nodes of
    /// their own when it is fetched - see `Chain` - because a chain is a thing you style,
    /// move and look at, and a socket on somebody else's node is none of those.
    Molecule {
        #[serde(default)]
        pdb_id: String,
        #[serde(default)]
        path: Option<PathBuf>,
        /// Whether it is shown as its biological assembly rather than as deposited.
        /// Unsaid, an entry fetched from the PDB is its assembly and a file is as it is
        /// written; the Fetch dialog's checkbox and the Structure tab say otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assembly: Option<bool>,
        /// Which assembly, model and conformers, where not the defaults.
        #[serde(default, skip_serializing_if = "Version::is_default")]
        version: Version,
        /// Which atoms the molecule has: hydrogens, waters, ligands. A property of the
        /// molecule, not of how it is drawn - an object's contents are not part of its
        /// material.
        #[serde(default)]
        options: SceneOptions,
        /// Chain ids, in the order the structure reports them.
        #[serde(default)]
        chains: Vec<String>,
        /// Whether the group is showing its chains.
        #[serde(default)]
        expanded: bool,
        /// The molecule's styles, in slot order - Blender's material slots.
        ///
        /// The styles live on the molecule rather than on each chain because that is what
        /// makes a bundle of styles a thing you can keep: one slot named "Cartoon teal"
        /// used by three chains is one style to edit, and editing it moves all three. A
        /// chain names the slot it draws with (see `Chain::slot`), the way a mesh face
        /// carries a material index rather than a material.
        #[serde(default)]
        slots: Vec<StyleSlot>,
        /// Which slot the panel is editing. Not which slot is drawn - that is per chain.
        #[serde(default)]
        active_slot: usize,
        /// The molecule's residue-level features: what is said about runs of residues,
        /// below the level a style reaches. The BioMol's own record of how it looks.
        #[serde(default)]
        features: Vec<crate::geometry::Feature>,
        /// Named parts of the molecule - PyMOL's named selections: a set of residues the
        /// user picked out and gave a name, kept so it can be picked again, styled and
        /// measured as a thing of its own. Kept by the app's own screens; the core only
        /// carries them, so they are whatever JSON was put there.
        #[serde(default)]
        objects: Vec<serde_json::Value>,
    },
    /// Sequences typed in, for feeding a prediction.
    SequenceInput {
        fasta: String,
    },
    /// A prediction. The pipeline's whole point: a model is a source of structure.
    Predict {
        model_id: String,
        provider_id: String,
        #[serde(default)]
        inputs: BTreeMap<String, serde_json::Value>,
        /// Set once a job has been submitted for this node, so re-evaluating does not
        /// launch a second one.
        #[serde(default)]
        job_id: Option<String>,
    },
    /// How a molecule is drawn.
    ///
    /// The analogue of Blender's Material Output: the node a style network ends at. The
    /// representation and what is visible are decided here; the colour and the surface
    /// arrive on sockets, so they can be built up rather than typed in - which is the
    /// point of defining a material as a graph at all.
    Style {
        style: Style,
    },
    /// Colour from a property of the structure: chain, element, secondary structure,
    /// confidence. A rule, not a colour.
    Colouring {
        scheme: ColorScheme,
        #[serde(default)]
        categorical: Categorical,
        #[serde(default)]
        continuous: bool,
    },
    /// One flat colour. What a named colour like PyMOL's `orange` actually is.
    SolidColour {
        color: [f32; 3],
    },
    /// Not flattened, for the same reason: `Light` has its own `id`.
    Light {
        light: Light,
    },
    World {
        background: [f32; 3],
        ambient: f32,
        #[serde(default = "yes")]
        ambient_occlusion: bool,
        #[serde(default)]
        outline: bool,
    },
    Material {
        material: Material,
    },
    /// What the viewport shows. Exactly one graph output, like a shader's Material Output.
    Viewport,
    /// A Blender render of the same scene. An output like `Viewport`, but one that writes
    /// a file rather than drawing a frame - so a graph can preview and publish the same
    /// pipeline, at different quality, without being rewired.
    Render {
        #[serde(default)]
        engine: RenderEngine,
        #[serde(default = "default_samples")]
        samples: u32,
        #[serde(default = "default_width")]
        width: u32,
        #[serde(default = "default_height")]
        height: u32,
        #[serde(default)]
        transparent: bool,
        /// Where the last render of this node went, so the node can show it.
        #[serde(default)]
        output: Option<PathBuf>,
    },
    /// One chain of a molecule, with its own style.
    ///
    /// Made for each chain when a molecule is fetched. A figure of a complex is mostly
    /// decisions about individual chains, so each is a node: it can be styled, selected,
    /// moved, and left out by unwiring it.
    Chain {
        /// Author chain id, as the structure reports it.
        chain: String,
        /// Which of the molecule's style slots this chain draws with.
        ///
        /// An index rather than a style, so several chains can share one and be restyled
        /// together. `None` means the first slot, so a chain added to a molecule that
        /// already has styles is not invisible.
        #[serde(default)]
        slot: Option<usize>,
        /// A style of the chain's own, from before styles lived in slots.
        ///
        /// Kept so a saved graph still looks the way it did; a slot wins over it, and the
        /// panel writes slots. The `Style` socket overrides both - the way a Blender
        /// object's material slot can name an existing material or be driven by a node
        /// network.
        #[serde(default)]
        style: Option<Style>,
    },
    /// A rendered image, with somewhere to copy or save it from.
    ///
    /// A node rather than a panel because a render is a result like any other, and two
    /// renders produce two of them: an image that lives in a side panel has nothing to say
    /// about which settings made it, and the next render overwrites it.
    Image {
        #[serde(default)]
        path: Option<PathBuf>,
    },
}

/// Which Blender engine a Render node uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderEngine {
    /// Path traced, and the reason to render offline at all.
    #[default]
    Cycles,
    /// Rasterised. Seconds rather than minutes, for checking a composition.
    Eevee,
}

impl RenderEngine {
    /// The name Blender's `render.engine` takes; see `assets/render_scene.py`.
    pub fn as_str(self) -> &'static str {
        match self {
            RenderEngine::Cycles => "cycles",
            RenderEngine::Eevee => "eevee",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RenderEngine::Cycles => "Cycles (path traced)",
            RenderEngine::Eevee => "EEVEE (fast)",
        }
    }
}

fn default_samples() -> u32 {
    64
}
fn default_width() -> u32 {
    1600
}
fn default_height() -> u32 {
    1200
}

fn yes() -> bool {
    true
}

impl NodeKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::Molecule { .. } => "molecule",
            NodeKind::Chain { .. } => "chain",
            NodeKind::SequenceInput { .. } => "sequence-input",
            NodeKind::Predict { .. } => "predict",
            NodeKind::Style { .. } => "style",
            NodeKind::Colouring { .. } => "colouring",
            NodeKind::SolidColour { .. } => "solid-colour",
            NodeKind::Light { .. } => "light",
            NodeKind::World { .. } => "world",
            NodeKind::Material { .. } => "material",
            NodeKind::Viewport => "viewport",
            NodeKind::Render { .. } => "render",
            NodeKind::Image { .. } => "image",
        }
    }

    pub fn title(&self) -> String {
        match self {
            NodeKind::Molecule {
                path: Some(path), ..
            } => path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Molecule")
                .to_string(),
            NodeKind::Molecule { pdb_id, .. } if !pdb_id.trim().is_empty() => {
                pdb_id.trim().to_uppercase()
            }
            NodeKind::Molecule { .. } => "Molecule".into(),
            NodeKind::Chain { chain, .. } => format!("Chain {chain}"),
            NodeKind::SequenceInput { .. } => "Sequence".into(),
            NodeKind::Predict { model_id, .. } => format!("Predict: {model_id}"),
            // Named by what it draws, so a style that draws two says so.
            NodeKind::Style { style } => match style.representations.len() {
                0 => "Style".into(),
                1 => style.primary().label().to_string(),
                count => format!("{} +{}", style.primary().label(), count - 1),
            },
            NodeKind::Colouring { scheme, .. } => scheme.label().to_string(),
            NodeKind::SolidColour { .. } => "Colour".into(),
            NodeKind::Light { light } => light.name.clone(),
            NodeKind::World { .. } => "World".into(),
            NodeKind::Material { .. } => "Material".into(),
            NodeKind::Viewport => "Viewport".into(),
            NodeKind::Render { engine, .. } => match engine {
                RenderEngine::Cycles => "Render (Cycles)".into(),
                RenderEngine::Eevee => "Render (EEVEE)".into(),
            },
            NodeKind::Image { path } => path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
                .unwrap_or_else(|| "Image".into()),
        }
    }

    pub fn inputs(&self) -> Vec<Socket> {
        match self {
            NodeKind::SequenceInput { .. }
            | NodeKind::Colouring { .. }
            | NodeKind::SolidColour { .. }
            | NodeKind::Light { .. }
            | NodeKind::World { .. }
            | NodeKind::Material { .. } => Vec::new(),
            // A molecule takes a structure from upstream when a prediction feeds it, and
            // nothing else: its chains are nodes.
            NodeKind::Molecule { .. } => vec![optional("Structure", SocketKind::Structure)],
            // The molecule link is how the graph knows which structure a chain reads,
            // but it is internal to the group: the frame around them says it, so the
            // editor hides the row. See `hiddenSockets` there.
            NodeKind::Chain { .. } => vec![
                socket("Molecule", SocketKind::Structure),
                optional("Style", SocketKind::Style),
            ],
            NodeKind::Image { .. } => vec![optional("Image", SocketKind::Image)],
            // A style ends a small network of its own, the way a Material Output does.
            NodeKind::Style { .. } => vec![
                optional("Colour", SocketKind::Colour),
                optional("Surface", SocketKind::Material),
            ],
            NodeKind::Predict { .. } => vec![socket("Sequence", SocketKind::Sequence)],

            // A Render takes exactly what the Viewport takes, because it is the same
            // scene: anything else and the render would stop predicting the preview.
            NodeKind::Viewport => vec![
                // Several, because a viewport draws a scene: two molecules side by side,
                // or a complex and its ligand from separate files.
                many("Geometry", SocketKind::Geometry),
                many("Lights", SocketKind::Light),
                optional("World", SocketKind::World),
            ],
            // One input: the view. There is no camera object here - the viewport *is* the
            // camera - and a view already carries the geometry, the lighting and the
            // world, so giving a render its own copies of those would be three more
            // sockets that can only ever disagree with the picture on screen. A render is
            // the viewport at higher quality, and its own fields say how much.
            NodeKind::Render { .. } => vec![socket("View", SocketKind::View)],
        }
    }

    pub fn output(&self) -> Option<SocketKind> {
        match self {
            NodeKind::Predict { .. } => Some(SocketKind::Structure),
            NodeKind::Molecule { .. } => Some(SocketKind::Structure),
            // A chain plus its style is geometry, which is what an output draws.
            NodeKind::Chain { .. } => Some(SocketKind::Geometry),
            NodeKind::SequenceInput { .. } => Some(SocketKind::Sequence),
            NodeKind::Style { .. } => Some(SocketKind::Style),
            NodeKind::Colouring { .. } | NodeKind::SolidColour { .. } => Some(SocketKind::Colour),
            NodeKind::Light { .. } => Some(SocketKind::Light),
            NodeKind::World { .. } => Some(SocketKind::World),
            NodeKind::Material { .. } => Some(SocketKind::Material),
            NodeKind::Viewport => Some(SocketKind::View),
            // A render hands on the image it made, which is what an Image node receives.
            NodeKind::Render { .. } => Some(SocketKind::Image),
            NodeKind::Image { .. } => None,
        }
    }

    /// Which family of nodes this belongs to, and the colour that says so.
    ///
    /// Blender tints a node's header by category - input, shader, colour, converter,
    /// output - and it is the fastest way to read a graph you did not build. The colours
    /// follow the sockets so that "the teal family makes teal things".
    pub fn category(&self) -> (&'static str, [u8; 3]) {
        match self {
            NodeKind::Molecule { .. } | NodeKind::SequenceInput { .. } => {
                ("Source", SocketKind::Structure.color())
            }
            NodeKind::Chain { .. } => ("Chain", SocketKind::Geometry.color()),
            NodeKind::Predict { .. } => ("Compute", SocketKind::Sequence.color()),
            NodeKind::Style { .. } => ("Style", SocketKind::Style.color()),
            NodeKind::Colouring { .. } | NodeKind::SolidColour { .. } => {
                ("Colour", SocketKind::Colour.color())
            }
            NodeKind::Material { .. } => ("Surface", SocketKind::Material.color()),
            NodeKind::Light { .. } => ("Light", SocketKind::Light.color()),
            NodeKind::World { .. } => ("World", SocketKind::World.color()),
            NodeKind::Viewport | NodeKind::Render { .. } | NodeKind::Image { .. } => {
                ("Output", SocketKind::Image.color())
            }
        }
    }

    /// Whether this kind ends a pipeline rather than feeding one.
    pub fn is_output(&self) -> bool {
        matches!(
            self,
            NodeKind::Viewport | NodeKind::Render { .. } | NodeKind::Image { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    #[serde(flatten)]
    pub kind: NodeKind,
    /// Editor position. Stored with the graph because where a node sits is part of how a
    /// pipeline reads.
    #[serde(default)]
    pub position: [f32; 2],
    /// Collapsed to its header. Stored for the same reason as the position: once every
    /// step is a node, a big graph is only readable if most of it can be folded away.
    #[serde(default)]
    pub collapsed: bool,
}

impl Node {
    /// An expanded node at a position. A constructor rather than struct literals, so a
    /// field added to `Node` later does not have to be threaded through every call site.
    pub fn new(id: impl Into<String>, kind: NodeKind, position: [f32; 2]) -> Self {
        Node {
            id: id.into(),
            kind,
            position,
            collapsed: false,
        }
    }
}

/// A connection from one node's output to a named input on another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    pub socket: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Nodes this build could not read, kept verbatim so saving writes them back.
    ///
    /// A world saved by a newer version can hold a node type this one has never heard of,
    /// or a node whose fields have changed shape. Failing the whole graph over one such
    /// node threw the world off the shelf - and the next save made that permanent. Setting
    /// it aside instead loses nothing: the rest of the graph opens, and a later build that
    /// knows the node reads it back from here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<serde_json::Value>,
    /// Links to or from an unread node, kept beside it for the same reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread_links: Vec<Link>,
}

impl<'de> Deserialize<'de> for Graph {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Saved {
            #[serde(default)]
            nodes: Vec<serde_json::Value>,
            #[serde(default)]
            links: Vec<Link>,
            #[serde(default)]
            unread: Vec<serde_json::Value>,
            #[serde(default)]
            unread_links: Vec<Link>,
        }
        let saved = Saved::deserialize(deserializer)?;

        let mut graph = Graph::default();
        // What an earlier build set aside is tried again: this build may know it.
        for value in saved.nodes.into_iter().chain(saved.unread) {
            match Node::deserialize(&value) {
                Ok(node) => graph.nodes.push(node),
                Err(error) => {
                    eprintln!("[graph] a node could not be read and is kept aside: {error}");
                    graph.unread.push(value);
                }
            }
        }
        let aside: HashSet<&str> = graph
            .unread
            .iter()
            .filter_map(|value| value.get("id").and_then(|id| id.as_str()))
            .collect();
        let (unread_links, links): (Vec<Link>, Vec<Link>) = saved
            .links
            .into_iter()
            .chain(saved.unread_links)
            .partition(|link| {
                aside.contains(link.from.as_str()) || aside.contains(link.to.as_str())
            });
        graph.links = links;
        graph.unread_links = unread_links;
        Ok(graph)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GraphError {
    #[error("the graph has no Viewport node, so nothing is being shown")]
    NoOutput,
    #[error("the graph has {0} Viewport nodes; there can only be one")]
    ManyOutputs(usize),
    #[error("{node}: {socket} is not connected")]
    Missing { node: String, socket: String },
    #[error("the graph has a cycle through {0}")]
    Cycle(String),
    #[error("no node with id {0}")]
    UnknownNode(String),
    #[error("two nodes share the id {0}")]
    DuplicateId(String),
    #[error("{from} has no output, so it cannot connect to {to}")]
    NoOutputSocket { from: String, to: String },
    #[error("{from} outputs {from_kind:?}, which cannot connect to {to}'s {socket}")]
    TypeMismatch {
        from: String,
        from_kind: SocketKind,
        to: String,
        socket: String,
    },
}

impl Graph {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// Nodes linked into `socket` of `node`, in graph order.
    pub fn sources(&self, node: &str, socket: &str) -> Vec<&Node> {
        self.links
            .iter()
            .filter(|link| link.to == node && link.socket == socket)
            .filter_map(|link| self.node(&link.from))
            .collect()
    }

    /// Nodes fed by `node`'s output, in graph order.
    pub fn consumers(&self, node: &str) -> Vec<&Node> {
        self.links
            .iter()
            .filter(|link| link.from == node)
            .filter_map(|link| self.node(&link.to))
            .collect()
    }

    pub fn output_node(&self) -> Result<&Node, GraphError> {
        let outputs: Vec<&Node> = self
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Viewport))
            .collect();
        match outputs.len() {
            0 => Err(GraphError::NoOutput),
            1 => Ok(outputs[0]),
            count => Err(GraphError::ManyOutputs(count)),
        }
    }

    /// Every Render node, in graph order. Each is its own output, evaluated separately.
    pub fn render_nodes(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Render { .. }))
            .collect()
    }

    /// Checks the graph before anything tries to evaluate it.
    ///
    /// Reported as a list rather than the first failure: a half-built graph usually has
    /// several loose ends, and fixing them one round trip at a time is miserable.
    pub fn validate(&self) -> Vec<GraphError> {
        let mut problems = Vec::new();

        match self.output_node() {
            Ok(_) => {}
            Err(error) => problems.push(error),
        }

        // Every lookup goes by id and finds the first match, so a second node with the
        // same id is invisible to evaluation while still being drawn in the editor.
        let mut ids: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id.as_str()) {
                problems.push(GraphError::DuplicateId(node.id.clone()));
            }
        }
        for link in &self.links {
            let Some(from) = self.node(&link.from) else {
                problems.push(GraphError::UnknownNode(link.from.clone()));
                continue;
            };
            let Some(to) = self.node(&link.to) else {
                problems.push(GraphError::UnknownNode(link.to.clone()));
                continue;
            };
            let Some(produced) = from.kind.output() else {
                problems.push(GraphError::NoOutputSocket {
                    from: link.from.clone(),
                    to: link.to.clone(),
                });
                continue;
            };
            let accepted = to
                .kind
                .inputs()
                .into_iter()
                .find(|input| input.name == link.socket);
            match accepted {
                Some(input) if input.kind == produced => {}
                _ => problems.push(GraphError::TypeMismatch {
                    from: link.from.clone(),
                    from_kind: produced,
                    to: link.to.clone(),
                    socket: link.socket.clone(),
                }),
            }
        }

        for node in &self.nodes {
            for input in node.kind.inputs() {
                if input.required && self.sources(&node.id, &input.name).is_empty() {
                    problems.push(GraphError::Missing {
                        node: node.id.clone(),
                        socket: input.name,
                    });
                }
            }
        }

        if let Some(cycle) = self.find_cycle() {
            problems.push(GraphError::Cycle(cycle));
        }
        problems
    }

    /// Depth-first search for a back edge.
    ///
    /// Worth doing explicitly: a cycle in an evaluator that follows links recursively is a
    /// stack overflow, which takes the whole app down rather than showing an error.
    fn find_cycle(&self) -> Option<String> {
        let mut state: HashMap<&str, u8> = HashMap::new();
        for node in &self.nodes {
            if let Some(found) = self.visit(&node.id, &mut state) {
                return Some(found);
            }
        }
        None
    }

    fn visit<'a>(&'a self, id: &'a str, state: &mut HashMap<&'a str, u8>) -> Option<String> {
        match state.get(id) {
            Some(1) => return Some(id.to_string()), // on the current path: a back edge
            Some(2) => return None,                 // already cleared
            _ => {}
        }
        state.insert(id, 1);
        for link in self.links.iter().filter(|link| link.to == id) {
            if let Some(found) = self.visit(&link.from, state) {
                return Some(found);
            }
        }
        state.insert(id, 2);
        None
    }

    /// Builds the request a prediction node describes.
    pub fn job_request(&self, node: &Node) -> Option<JobRequest> {
        let NodeKind::Predict {
            model_id,
            provider_id,
            inputs,
            ..
        } = &node.kind
        else {
            return None;
        };

        let mut inputs = inputs.clone();
        // A sequence arriving on the socket wins over one typed into the node, because the
        // link is the more explicit statement of intent.
        if let Some(source) = self.sources(&node.id, "Sequence").first() {
            if let NodeKind::SequenceInput { fasta } = &source.kind {
                inputs.insert("sequence".into(), serde_json::json!(fasta));
            }
        }

        Some(JobRequest {
            model_id: model_id.clone(),
            provider_id: provider_id.clone(),
            world_id: None,
            name: Some(node.kind.title()),
            inputs,
        })
    }

    /// The style the Viewport's World, Material and Light nodes describe.
    pub fn style(&self) -> SceneStyle {
        match self.output_node() {
            Ok(node) => self.style_for(&node.id),
            Err(_) => SceneStyle::default(),
        }
    }

    /// The style feeding one particular output.
    ///
    /// Taken per output rather than per graph because a Render node has its own World,
    /// Material and Lights: a figure lit for print and a preview lit for reading the
    /// structure are different scenes, and both have to be expressible.
    pub fn style_for(&self, output_id: &str) -> SceneStyle {
        let output = match self.node(output_id) {
            Some(node) => node,
            None => return SceneStyle::default(),
        };
        let mut style = SceneStyle::default();

        if let Some(world) = self.sources(&output.id, "World").first() {
            if let NodeKind::World {
                background,
                ambient,
                ambient_occlusion,
                outline,
            } = &world.kind
            {
                style.background = *background;
                style.ambient = *ambient;
                style.ambient_occlusion = *ambient_occlusion;
                style.outline = *outline;
            }
        }

        let lights: Vec<Light> = self
            .sources(&output.id, "Lights")
            .into_iter()
            .filter_map(|node| match &node.kind {
                NodeKind::Light { light } => Some(light.clone()),
                _ => None,
            })
            .collect();
        // An unlit graph is almost always an unfinished one rather than a deliberate
        // choice, so a light stands in until one is connected. One, not the whole default
        // rig: the graph ships with a single key light, and deleting it should not make the
        // scene brighter than it was.
        style.lights = if lights.is_empty() {
            style.lights.into_iter().take(1).collect()
        } else {
            lights
        };

        // The palette and the surface are part of the scene style too: the legend and the
        // background-aware CPK colours are drawn from the palette, and the renderers shade
        // the whole scene with one material. Both come from the unassigned style of
        // whatever the output draws - per-chain palettes are in the assignments, and a
        // per-chain surface is not something either renderer can honour.
        if let Some(chain) = self.sources(&output.id, "Geometry").first() {
            let assigned = self.chain_style(&chain.id);
            style.categorical = assigned.categorical;
            style.continuous_colors = assigned.continuous;
            style.material = assigned.material;
        }
        style
    }

    /// One Style node, resolved with whatever is wired into it.
    ///
    /// The node holds the representation; the colour and the surface come off sockets, so
    /// a style is built up from parts the way a Blender material is. What is on the node
    /// stands in for a socket nothing is connected to.
    pub fn style_of(&self, style_node_id: &str) -> Option<Style> {
        let node = self.node(style_node_id)?;
        let NodeKind::Style { style } = &node.kind else {
            return None;
        };
        let mut resolved = style.clone();

        match self
            .sources(style_node_id, "Colour")
            .first()
            .map(|n| &n.kind)
        {
            Some(NodeKind::Colouring {
                scheme,
                categorical,
                continuous,
            }) => {
                resolved.scheme = *scheme;
                resolved.categorical = *categorical;
                resolved.continuous = *continuous;
                resolved.solid = None;
            }
            Some(NodeKind::SolidColour { color }) => resolved.solid = Some(*color),
            _ => {}
        }
        if let Some(NodeKind::Material { material }) = self
            .sources(style_node_id, "Surface")
            .first()
            .map(|n| &n.kind)
        {
            resolved.material = *material;
        }
        Some(resolved)
    }

    /// Which atoms a Molecule group includes.
    /// A molecule's residue-level features.
    pub fn features(&self, molecule_id: &str) -> Vec<crate::geometry::Feature> {
        match self.node(molecule_id).map(|node| &node.kind) {
            Some(NodeKind::Molecule { features, .. }) => features.clone(),
            _ => Vec::new(),
        }
    }

    pub fn options(&self, molecule_id: &str) -> SceneOptions {
        match self.node(molecule_id).map(|node| &node.kind) {
            Some(NodeKind::Molecule { options, .. }) => *options,
            _ => SceneOptions::default(),
        }
    }

    /// The style on one Chain node.
    ///
    /// In order: a wired Style node, then the molecule's slot this chain names, then a
    /// style the chain carries itself (which is where styles lived before slots), then the
    /// app's default - an unstyled chain being invisible is a worse answer than one that
    /// looks ordinary.
    ///
    /// The order is the point. Choosing a slot is the easy path, wiring a node is how you
    /// override it, and that is exactly how a Blender material slot behaves.
    pub fn chain_style(&self, chain_id: &str) -> Style {
        if let Some(wired) = self
            .sources(chain_id, "Style")
            .first()
            .and_then(|node| self.style_of(&node.id))
        {
            return wired;
        }
        if let Some(slot) = self.slot_for(chain_id) {
            return slot.style.clone();
        }
        match self.node(chain_id).map(|node| &node.kind) {
            Some(NodeKind::Chain {
                style: Some(style), ..
            }) => style.clone(),
            _ => Style::default(),
        }
    }

    /// A molecule's style slots, or an empty list for anything else.
    pub fn slots(&self, molecule_id: &str) -> &[StyleSlot] {
        match self.node(molecule_id).map(|node| &node.kind) {
            Some(NodeKind::Molecule { slots, .. }) => slots,
            _ => &[],
        }
    }

    /// The slot a chain draws with, if its molecule has any.
    ///
    /// An index past the end of the list falls back to the first slot rather than to the
    /// default style: a slot removed from under a chain should leave it looking like the
    /// molecule it belongs to, not like a fresh one.
    pub fn slot_for(&self, chain_id: &str) -> Option<&StyleSlot> {
        let molecule = self.molecule_of(chain_id)?;
        let slots = self.slots(&molecule.id);
        if slots.is_empty() {
            return None;
        }
        let index = match self.node(chain_id).map(|node| &node.kind) {
            Some(NodeKind::Chain { slot, .. }) => slot.unwrap_or(0),
            _ => 0,
        };
        slots.get(index).or_else(|| slots.first())
    }

    /// The Molecule a Chain node reads from.
    pub fn molecule_of(&self, chain_id: &str) -> Option<&Node> {
        self.sources(chain_id, "Molecule")
            .into_iter()
            .find(|node| matches!(node.kind, NodeKind::Molecule { .. }))
    }

    /// Chain nodes reading from one molecule, in graph order.
    pub fn chains_of(&self, molecule_id: &str) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Chain { .. }))
            .filter(|node| {
                self.molecule_of(&node.id)
                    .map(|molecule| molecule.id == molecule_id)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Chains whose style asks for a surface the renderers cannot give them.
    ///
    /// Both renderers shade the whole scene with one material, so a chain assigned a
    /// different one is drawn with the first chain's. Reported rather than ignored:
    /// silently dropping half of what a node says is worse than saying it cannot be done.
    pub fn unrendered_materials(&self, output_id: &str) -> Vec<String> {
        let chains = self.sources(output_id, "Geometry");
        let Some(first) = chains.first() else {
            return Vec::new();
        };
        let reference = self.chain_style(&first.id).material;
        chains
            .iter()
            .filter(|node| self.chain_style(&node.id).material != reference)
            .filter(|node| matches!(node.kind, NodeKind::Chain { .. }))
            // By node, not by chain letter: two molecules both have a chain A, and the
            // warning belongs to the one that differs.
            .map(|node| node.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::default_graph;

    fn link(from: &str, to: &str, socket: &str) -> Link {
        Link {
            from: from.into(),
            to: to.into(),
            socket: socket.into(),
        }
    }

    fn chain(id: &str) -> Node {
        Node::new(
            id,
            NodeKind::Chain {
                chain: "A".into(),
                slot: None,
                style: None,
            },
            [0.0, 0.0],
        )
    }

    #[test]
    fn a_node_type_from_a_newer_version_does_not_lose_the_world() {
        // The world shelf drops any world whose graph fails to parse, and the next save
        // makes that permanent. One unknown node must cost that node at most.
        let mut saved = serde_json::to_value(default_graph()).unwrap();
        saved["nodes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "id": "glow",
                "type": "bloom-filter",
                "strength": 0.4,
                "position": [10.0, 20.0],
            }));
        saved["links"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "from": "glow", "to": "viewport", "socket": "Post"
            }));

        let graph: Graph = serde_json::from_value(saved).expect("the graph still loads");
        assert_eq!(graph.nodes.len(), default_graph().nodes.len());
        assert!(graph.node("viewport").is_some());
        // Its link is set aside with it, so the editor is not handed a dangling link and
        // validation does not complain about a node it cannot see.
        assert!(graph.links.iter().all(|link| link.from != "glow"));
        assert!(graph.validate().is_empty(), "{:?}", graph.validate());

        // And saving writes both back, so a newer build finds them where it left them.
        let written = serde_json::to_value(&graph).unwrap();
        assert_eq!(written["unread"][0]["type"], "bloom-filter");
        assert_eq!(written["unread"][0]["strength"], 0.4);
        assert_eq!(written["unread_links"][0]["from"], "glow");
        let again: Graph = serde_json::from_value(written).unwrap();
        assert_eq!(again.unread.len(), 1);
        assert_eq!(again.unread_links.len(), 1);
    }

    #[test]
    fn a_graph_with_nothing_set_aside_serialises_as_before() {
        // The editor reads this shape; the new fields only appear when they hold something.
        let text = serde_json::to_string(&default_graph()).unwrap();
        assert!(!text.contains("unread"), "{text}");
    }

    #[test]
    fn a_known_node_with_a_field_this_build_does_not_know_still_loads() {
        let saved = serde_json::json!({
            "nodes": [{"id": "viewport", "type": "viewport", "exposure": 1.5}],
            "links": [],
        });
        let graph: Graph = serde_json::from_value(saved).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.unread.is_empty());
    }

    #[test]
    fn a_malformed_known_node_is_kept_aside_rather_than_failing_the_graph() {
        // A World whose required field has gone, as a hand-edited or truncated save has.
        let saved = serde_json::json!({
            "nodes": [
                {"id": "viewport", "type": "viewport"},
                {"id": "world", "type": "world", "background": [1.0, 1.0, 1.0]},
            ],
            "links": [{"from": "world", "to": "viewport", "socket": "World"}],
        });
        let graph: Graph = serde_json::from_value(saved).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.unread.len(), 1);
        assert!(graph.links.is_empty());
        assert_eq!(graph.unread_links.len(), 1);
    }

    #[test]
    fn an_empty_object_is_an_empty_graph() {
        let graph: Graph = serde_json::from_str("{}").unwrap();
        assert!(graph.nodes.is_empty() && graph.links.is_empty());
        assert_eq!(graph.validate(), vec![GraphError::NoOutput]);
    }

    #[test]
    fn links_to_nodes_that_do_not_exist_are_reported_not_followed() {
        let mut graph = default_graph();
        graph.links.push(link("ghost", "viewport", "Geometry"));
        graph.links.push(link("world", "nowhere", "World"));
        let problems = graph.validate();
        assert!(problems.contains(&GraphError::UnknownNode("ghost".into())));
        assert!(problems.contains(&GraphError::UnknownNode("nowhere".into())));
        // Nothing that walks links trips over them.
        assert!(graph.sources("viewport", "Geometry").is_empty());
        assert!(graph
            .consumers("world")
            .iter()
            .all(|node| node.id != "nowhere"));
        let _ = graph.style();
        assert!(graph.unrendered_materials("viewport").is_empty());
    }

    #[test]
    fn two_nodes_with_one_id_are_reported() {
        // Lookups take the first; the second would be drawn in the editor and ignored by
        // everything else, which is a node that silently does nothing.
        let mut graph = default_graph();
        graph.nodes.push(chain("twin"));
        graph.nodes.push(chain("twin"));
        assert!(graph
            .validate()
            .contains(&GraphError::DuplicateId("twin".into())));
    }

    #[test]
    fn a_link_out_of_a_node_with_no_output_says_so() {
        let mut graph = default_graph();
        graph.nodes.push(Node::new(
            "picture",
            NodeKind::Image { path: None },
            [0.0, 0.0],
        ));
        graph.links.push(link("picture", "viewport", "Geometry"));
        let problems = graph.validate();
        assert!(
            problems.contains(&GraphError::NoOutputSocket {
                from: "picture".into(),
                to: "viewport".into(),
            }),
            "{problems:?}"
        );
        // The message must not claim the image outputs a structure.
        let message = problems
            .iter()
            .find(|problem| matches!(problem, GraphError::NoOutputSocket { .. }))
            .unwrap()
            .to_string();
        assert!(!message.contains("Structure"), "{message}");
    }

    #[test]
    fn a_link_into_a_socket_the_node_does_not_have_is_refused() {
        let mut graph = default_graph();
        graph.links.push(link("world", "viewport", "Sideways"));
        assert!(graph
            .validate()
            .iter()
            .any(|problem| matches!(problem, GraphError::TypeMismatch { .. })));
    }

    #[test]
    fn a_cycle_between_two_nodes_is_found() {
        let mut graph = default_graph();
        graph.nodes.push(chain("a"));
        graph.nodes.push(chain("b"));
        graph.links.push(link("a", "b", "Molecule"));
        graph.links.push(link("b", "a", "Molecule"));
        assert!(graph
            .validate()
            .iter()
            .any(|problem| matches!(problem, GraphError::Cycle(_))));
    }

    #[test]
    fn a_long_chain_without_a_cycle_is_not_called_one() {
        // A diamond: two paths into one node is not a cycle, and a visit-once search that
        // confused "seen" with "on the path" would say it was.
        let mut graph = default_graph();
        for id in ["top", "left", "right", "bottom"] {
            graph.nodes.push(chain(id));
        }
        graph.links.push(link("top", "left", "Molecule"));
        graph.links.push(link("top", "right", "Molecule"));
        graph.links.push(link("left", "bottom", "Molecule"));
        graph.links.push(link("right", "bottom", "Style"));
        assert!(!graph
            .validate()
            .iter()
            .any(|problem| matches!(problem, GraphError::Cycle(_))));
    }

    #[test]
    fn style_lookups_on_missing_or_wrong_nodes_answer_with_defaults() {
        let graph = default_graph();
        assert!(graph.style_of("nope").is_none());
        assert!(graph.style_of("world").is_none(), "a world is not a style");
        assert_eq!(graph.chain_style("nope"), Style::default());
        assert!(graph.slots("world").is_empty());
        assert!(graph.slot_for("nope").is_none());
        assert!(graph.molecule_of("nope").is_none());
        assert!(graph.features("nope").is_empty());
        assert_eq!(graph.options("world"), SceneOptions::default());
        let _ = graph.style_for("nope");
        assert!(graph.job_request(graph.node("world").unwrap()).is_none());
    }

    #[test]
    fn titles_never_come_out_empty() {
        let empty_style = NodeKind::Style {
            style: Style {
                representations: Vec::new(),
                ..Default::default()
            },
        };
        assert_eq!(empty_style.title(), "Style");
        let molecule = |pdb_id: &str, path: Option<PathBuf>| NodeKind::Molecule {
            version: Default::default(),
            pdb_id: pdb_id.into(),
            assembly: None,
            path,
            options: SceneOptions::default(),
            chains: Vec::new(),
            expanded: false,
            slots: Vec::new(),
            active_slot: 0,
            features: Vec::new(),
            objects: Vec::new(),
        };
        assert_eq!(molecule("   ", None).title(), "Molecule");
        assert_eq!(molecule(" 1ubq ", None).title(), "1UBQ");
        // A path with no file name - the root - still gets a name.
        assert_eq!(molecule("", Some(PathBuf::from("/"))).title(), "Molecule");
    }
}
