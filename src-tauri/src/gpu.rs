//! The structure analyses, run on the graphics card.
//!
//! What the analyses spend their time on is the same question asked of every atom: what is
//! near it. Each atom's answer is its own, which is the shape of work a GPU is made for -
//! thousands of atoms at once rather than ten. Two kinds are done here:
//!
//! - **surface**: how much of each atom's probe sphere no other atom covers, for the
//!   buried-surface search behind the hydrophobic core;
//! - **pairs**: every pair of chosen atoms within a distance, for the contacts - what then
//!   makes a pair a hydrogen bond (its two angles, its donor and acceptor) is decided on
//!   the processor from this short list;
//!
//! The neighbours are found through a grid sorted on the processor and searched on the
//! card. Anything that goes wrong - no adapter, a device lost, a buffer too large - returns
//! `None`, and the caller does the same work on the processor; the two give the same
//! answer.

use std::sync::OnceLock;
use wgpu::util::DeviceExt;

/// The device and its compiled pipelines, made on first use.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::ComputePipeline,
    pairs: wgpu::ComputePipeline,
    /// The largest storage buffer the device will bind.
    max_binding: u64,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Whether the analyses may use the card at all. Off when `GEIS_NO_GPU` is set, which is
/// how the two paths are compared against each other.
fn enabled() -> bool {
    std::env::var_os("GEIS_NO_GPU").is_none()
}

fn gpu() -> Option<&'static Gpu> {
    if !enabled() {
        return None;
    }
    GPU.get_or_init(|| pollster::block_on(open()).ok()).as_ref()
}

/// The name of the card the analyses run on, or None when they run on the processor.
pub fn adapter_name() -> Option<String> {
    gpu()?;
    ADAPTER.get().cloned()
}
static ADAPTER: OnceLock<String> = OnceLock::new();

async fn open() -> Result<Gpu, String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .map_err(|error| error.to_string())?;
    let _ = ADAPTER.set(adapter.get_info().name);
    // The largest buffers the card has: a capsid's atoms are tens of megabytes.
    let limits = adapter.limits();
    let max_binding = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Geis analyses"),
            required_limits: limits,
            ..Default::default()
        })
        .await
        .map_err(|error| error.to_string())?;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("analyses"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    Ok(Gpu {
        surface: pipeline("surface"),
        pairs: pipeline("pairs"),
        device,
        queue,
        max_binding,
    })
}

/// Threads a workgroup runs; the shader's `@workgroup_size` says the same.
const GROUP: u32 = 64;
/// Workgroups along x before a dispatch wraps to y, under the 65,535 the API allows.
const ROW: u32 = 32_768;

const SHADER: &str = r#"
struct Grid {
    origin: vec3<f32>,
    cell: f32,
    dims: vec3<u32>,
    count: u32,
    cutoff: f32,
    mode: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var<uniform> grid: Grid;
// Atoms in grid order: x, y, z and, for the surface, the reach of their probe sphere.
@group(0) @binding(1) var<storage, read> atoms: array<vec4<f32>>;
// Where each cell's atoms start in that order; one more entry than there are cells.
@group(0) @binding(2) var<storage, read> starts: array<u32>;
// Per atom in grid order: its atom index, its residue, its chain, and a side or role.
@group(0) @binding(3) var<storage, read> info: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read_write> out: array<u32>;
@group(0) @binding(5) var<storage, read> points: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read_write> counter: atomic<u32>;

fn index_of(id: vec3<u32>) -> u32 {
    return id.x + id.y * (32768u * 64u);
}

fn cell_of(p: vec3<f32>) -> vec3<i32> {
    let c = vec3<i32>(floor((p - grid.origin) / grid.cell));
    return clamp(c, vec3<i32>(0), vec3<i32>(grid.dims) - vec3<i32>(1));
}

fn cell_index(c: vec3<i32>) -> u32 {
    return u32(c.x) + grid.dims.x * (u32(c.y) + grid.dims.y * u32(c.z));
}

fn inside(c: vec3<i32>) -> bool {
    return all(c >= vec3<i32>(0)) && all(c < vec3<i32>(grid.dims));
}

// The open part of each atom's probe sphere, as the number of its 96 points no
// neighbour covers. Only the atoms with info.w = 1 are measured.
@compute @workgroup_size(64)
fn surface(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= grid.count) { return; }
    if (info[i].w == 0u) { out[i] = 0u; return; }
    let here = atoms[i];
    var near: array<vec4<f32>, 192>;
    var found = 0u;
    let home = cell_of(here.xyz);
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = home + vec3<i32>(dx, dy, dz);
                if (!inside(c)) { continue; }
                let k = cell_index(c);
                for (var j = starts[k]; j < starts[k + 1u]; j++) {
                    if (j == i) { continue; }
                    let there = atoms[j];
                    if (distance(here.xyz, there.xyz) < here.w + there.w && found < 192u) {
                        near[found] = vec4<f32>(there.xyz, there.w * there.w);
                        found++;
                    }
                }
            }
        }
    }
    var open = 0u;
    for (var p = 0u; p < 96u; p++) {
        let at = here.xyz + points[p].xyz * here.w;
        var covered = false;
        for (var n = 0u; n < found; n++) {
            let d = at - near[n].xyz;
            if (dot(d, d) < near[n].w) { covered = true; break; }
        }
        if (!covered) { open++; }
    }
    out[i] = open;
}

// Every pair of atoms within the cutoff, at least 2 A apart, in different residues that
// are not neighbours along one chain. Each pair is written once, from the atom with the
// lower index: out[0] counts them, and they follow as atom, atom, distance bits.
@compute @workgroup_size(64)
fn pairs(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = index_of(id);
    if (i >= grid.count) { return; }
    let here = atoms[i];
    let me = info[i];
    let home = cell_of(here.xyz);
    let capacity = (arrayLength(&out) - 1u) / 3u;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = home + vec3<i32>(dx, dy, dz);
                if (!inside(c)) { continue; }
                let k = cell_index(c);
                for (var j = starts[k]; j < starts[k + 1u]; j++) {
                    let them = info[j];
                    if (them.x <= me.x || them.y == me.y) { continue; }
                    if (them.z == me.z && (them.y == me.y + 1u || me.y == them.y + 1u)) { continue; }
                    let d = distance(here.xyz, atoms[j].xyz);
                    if (d <= grid.cutoff && d >= 2.0) {
                        let slot = atomicAdd(&counter, 1u);
                        if (slot < capacity) {
                            out[1u + slot * 3u] = me.x;
                            out[2u + slot * 3u] = them.x;
                            out[3u + slot * 3u] = bitcast<u32>(d);
                        }
                    }
                }
            }
        }
    }
}

"#;

/// The grid uniform, laid out as the shader's `Grid`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GridUniform {
    origin: [f32; 3],
    cell: f32,
    dims: [u32; 3],
    count: u32,
    cutoff: f32,
    mode: u32,
    pad: [u32; 2],
}

// SAFETY: plain 4-byte fields with no padding between them, so every bit pattern is a
// valid value and the struct is its bytes.
unsafe impl bytemuck::Zeroable for GridUniform {}
unsafe impl bytemuck::Pod for GridUniform {}

/// Atoms sorted into grid cells.
struct Sorted {
    uniform: GridUniform,
    /// For each slot in grid order, the input position it came from.
    order: Vec<u32>,
    starts: Vec<u32>,
}

/// Sorts points into a grid of cells `cell` wide. The grid is coarsened if it would have
/// more cells than is sensible to hold - a few far-flung atoms should not cost a gigabyte.
fn sort_into_grid(points: &[[f32; 3]], cell: f32) -> Option<Sorted> {
    if points.is_empty() || cell.is_nan() || cell <= 0.0 {
        return None;
    }
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for point in points {
        for axis in 0..3 {
            if !point[axis].is_finite() {
                return None;
            }
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    let mut cell = cell;
    let dims = loop {
        let dims = [0, 1, 2]
            .map(|axis| (((high[axis] - low[axis]) / cell).floor() as u32).saturating_add(1));
        let cells = dims.iter().map(|&d| u64::from(d)).product::<u64>();
        if cells <= 16_000_000 {
            break dims;
        }
        cell *= 1.5;
    };
    let cells = (dims[0] * dims[1] * dims[2]) as usize;
    let key = |point: &[f32; 3]| {
        let c = [0, 1, 2]
            .map(|axis| (((point[axis] - low[axis]) / cell).floor() as u32).min(dims[axis] - 1));
        (c[0] + dims[0] * (c[1] + dims[1] * c[2])) as usize
    };
    let mut starts = vec![0_u32; cells + 1];
    let keys: Vec<usize> = points.iter().map(key).collect();
    for &k in &keys {
        starts[k + 1] += 1;
    }
    for k in 0..cells {
        starts[k + 1] += starts[k];
    }
    let mut fill = starts.clone();
    let mut order = vec![0_u32; points.len()];
    for (index, &k) in keys.iter().enumerate() {
        order[fill[k] as usize] = index as u32;
        fill[k] += 1;
    }
    Some(Sorted {
        uniform: GridUniform {
            origin: low,
            cell,
            dims,
            count: points.len() as u32,
            cutoff: 0.0,
            mode: 0,
            pad: [0; 2],
        },
        order,
        starts,
    })
}

impl Gpu {
    fn storage(&self, label: &str, bytes: &[u8]) -> Option<wgpu::Buffer> {
        // An empty binding is not allowed; four bytes stand in for nothing.
        let bytes = if bytes.is_empty() {
            &[0_u8; 16][..]
        } else {
            bytes
        };
        if bytes.len() as u64 > self.max_binding {
            return None;
        }
        Some(
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytes,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                }),
        )
    }

    /// Runs one entry point over `count` threads and reads back `out`.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        pipeline: &wgpu::ComputePipeline,
        uniform: GridUniform,
        atoms: &[[f32; 4]],
        starts: &[u32],
        info: &[[u32; 4]],
        points: &[[f32; 4]],
        out_words: usize,
        counter: bool,
    ) -> Option<(Vec<u32>, u32)> {
        let device = &self.device;
        let out_bytes = (out_words.max(4) * 4) as u64;
        if out_bytes > self.max_binding {
            return None;
        }
        let grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let atoms = self.storage("atoms", bytemuck::cast_slice(atoms))?;
        let starts = self.storage("starts", bytemuck::cast_slice(starts))?;
        let info = self.storage("info", bytemuck::cast_slice(info))?;
        let points = self.storage("points", bytemuck::cast_slice(points))?;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let count = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("counter"),
            contents: &[0_u8; 4],
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: atoms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: starts.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: info.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: out.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: points.as_entire_binding(),
            },
        ];
        if counter {
            entries.push(wgpu::BindGroupEntry {
                binding: 6,
                resource: count.as_entire_binding(),
            });
        }
        // Bindings the entry point does not use are not in its layout, and naming them is
        // an error: each keeps only what its own layout has.
        let layout = pipeline.get_bind_group_layout(0);
        let used: Vec<wgpu::BindGroupEntry> = entries
            .into_iter()
            .filter(|entry| match entry.binding {
                5 => std::ptr::eq(pipeline, &self.surface),
                0..=4 => true,
                _ => counter,
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("analysis"),
            layout: &layout,
            entries: &used,
        });

        let read_out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read out"),
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read counter"),
            size: 4,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("analysis"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("analysis"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let groups = uniform.count.div_ceil(GROUP).max(1);
            pass.dispatch_workgroups(groups.min(ROW), groups.div_ceil(ROW), 1);
        }
        encoder.copy_buffer_to_buffer(&out, 0, &read_out, 0, out_bytes);
        encoder.copy_buffer_to_buffer(&count, 0, &read_count, 0, 4);
        self.queue.submit([encoder.finish()]);

        let read = |buffer: &wgpu::Buffer| -> Option<Vec<u32>> {
            let slice = buffer.slice(..);
            let (send, receive) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
            device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
            receive.recv().ok()?.ok()?;
            let words = bytemuck::cast_slice::<u8, u32>(&slice.get_mapped_range().ok()?).to_vec();
            buffer.unmap();
            Some(words)
        };
        let words = read(&read_out)?;
        let counted = if counter {
            read(&read_count)?.first().copied().unwrap_or(0)
        } else {
            0
        };
        Some((words, counted))
    }
}

/// The open area of each atom's probe sphere, as in `contacts::accessible_surface`: `atoms`
/// are positions with the reach of each sphere (its radius plus the probe's), `measured`
/// says which are asked about, `sphere` is the unit points tested. Returns, per atom, how
/// many of the points are open, or None to do it on the processor.
pub fn open_points(
    atoms: &[([f32; 3], f32)],
    measured: &[bool],
    sphere: &[[f32; 3]],
) -> Option<Vec<u32>> {
    let gpu = gpu()?;
    if sphere.len() != 96 || atoms.len() != measured.len() {
        return None;
    }
    let widest = atoms
        .iter()
        .map(|&(_, reach)| reach)
        .fold(0.0_f32, f32::max)
        .max(1.0);
    let positions: Vec<[f32; 3]> = atoms.iter().map(|&(point, _)| point).collect();
    let sorted = sort_into_grid(&positions, widest * 2.0)?;
    let gathered: Vec<[f32; 4]> = sorted
        .order
        .iter()
        .map(|&index| {
            let (point, reach) = atoms[index as usize];
            [point[0], point[1], point[2], reach]
        })
        .collect();
    let info: Vec<[u32; 4]> = sorted
        .order
        .iter()
        .map(|&index| [index, 0, 0, u32::from(measured[index as usize])])
        .collect();
    let points: Vec<[f32; 4]> = sphere.iter().map(|p| [p[0], p[1], p[2], 0.0]).collect();
    let (words, _) = gpu.run(
        &gpu.surface,
        sorted.uniform,
        &gathered,
        &sorted.starts,
        &info,
        &points,
        atoms.len(),
        false,
    )?;
    let mut open = vec![0_u32; atoms.len()];
    for (slot, &index) in sorted.order.iter().enumerate() {
        open[index as usize] = words[slot];
    }
    Some(open)
}

/// One atom as the pair search takes it.
#[derive(Clone, Copy)]
pub struct Site {
    pub atom: u32,
    pub position: [f32; 3],
    pub residue: u32,
    pub chain: u32,
}

fn gathered(sites: &[Site], sorted: &Sorted) -> (Vec<[f32; 4]>, Vec<[u32; 4]>) {
    let atoms = sorted
        .order
        .iter()
        .map(|&index| {
            let p = sites[index as usize].position;
            [p[0], p[1], p[2], 0.0]
        })
        .collect();
    let info = sorted
        .order
        .iter()
        .map(|&index| {
            let site = sites[index as usize];
            [site.atom, site.residue, site.chain, 0]
        })
        .collect();
    (atoms, info)
}

/// Every pair of sites within `cutoff`, as `contacts::atom_pairs` finds them: atom, atom
/// (the lower first), distance; sorted, so the order does not depend on the card.
pub fn pairs(sites: &[Site], cutoff: f32) -> Option<Vec<(usize, usize, f32)>> {
    let gpu = gpu()?;
    let positions: Vec<[f32; 3]> = sites.iter().map(|site| site.position).collect();
    let mut sorted = sort_into_grid(&positions, cutoff)?;
    sorted.uniform.cutoff = cutoff;
    let (atoms, info) = gathered(sites, &sorted);
    // A guess at how many pairs there are, and once more at the size counted if it was short.
    let mut capacity = sites.len() * 4 + 1024;
    for _ in 0..2 {
        let (words, counted) = gpu.run(
            &gpu.pairs,
            sorted.uniform,
            &atoms,
            &sorted.starts,
            &info,
            &[],
            1 + capacity * 3,
            true,
        )?;
        let counted = counted as usize;
        if counted <= capacity {
            let mut found: Vec<(usize, usize, f32)> = (0..counted)
                .map(|slot| {
                    let at = 1 + slot * 3;
                    (
                        words[at] as usize,
                        words[at + 1] as usize,
                        f32::from_bits(words[at + 2]),
                    )
                })
                .collect();
            found.sort_unstable_by_key(|&(a, b, _)| (a, b));
            return Some(found);
        }
        capacity = counted + 1024;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_puts_every_point_in_exactly_one_cell() {
        let points = [
            [0.0, 0.0, 0.0],
            [5.0, 0.0, 0.0],
            [0.1, 0.1, 0.1],
            [9.9, 9.9, 9.9],
        ];
        let sorted = sort_into_grid(&points, 4.0).unwrap();
        assert_eq!(*sorted.starts.last().unwrap() as usize, points.len());
        let mut seen = sorted.order.clone();
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3]);
        assert_eq!(sorted.uniform.dims, [3, 3, 3]);
    }

    #[test]
    fn a_grid_of_nothing_or_of_nonsense_is_no_grid() {
        assert!(sort_into_grid(&[], 4.0).is_none());
        assert!(sort_into_grid(&[[f32::NAN, 0.0, 0.0]], 4.0).is_none());
    }
}
