pub mod manifest;
pub mod registry;
pub mod template;

pub use manifest::{
    GpuNeed, InputKind, InputSpec, InstallSpec, Kind, Maintenance, Manifest, OutputKind,
    OutputSpec, Requirements, RuntimeKind, Task, WeightsPolicy,
};
pub use registry::Registry;
