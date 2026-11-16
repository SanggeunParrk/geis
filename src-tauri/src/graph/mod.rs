//! The node graph: the app's central abstraction.
//!
//! Where a structure comes from, how it is drawn, how it is lit, and what is done with the
//! result are all nodes on one canvas. Prediction is a source node like any other, so
//! "predict, then look at it" is a connection rather than two separate screens.

pub mod eval;
pub mod model;

pub use eval::{
    default_graph, evaluate, resolve, Evaluation, JobStatus, NodeState, Plan, RenderTarget,
    Resolution, StructureRef,
};
pub use model::{
    Graph, GraphError, Link, Node, NodeKind, RenderEngine, Socket, SocketKind, StyleSlot,
};
