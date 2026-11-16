//! Parses the output of `assets/probe.sh` into something the UI can render.
//!
//! The script runs identically on this machine and on a cluster login node, so this
//! parser is the single place host capability is understood.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROBE_SCRIPT: &str = include_str!("../../assets/probe.sh");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackend {
    Cuda,
    Rocm,
    Metal,
    Unknown,
}

impl GpuBackend {
    fn parse(value: &str) -> Self {
        match value {
            "cuda" => Self::Cuda,
            "rocm" => Self::Rocm,
            "metal" => Self::Metal,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gpu {
    pub backend: GpuBackend,
    pub name: String,
    /// GiB. `None` when the tool did not report it (common on ROCm).
    pub memory_gb: Option<u32>,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostProbe {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub cpu: String,
    pub cores: u32,
    pub memory_gb: u32,
    pub gpus: Vec<Gpu>,
    pub tools: Vec<Tool>,
    pub scratch: Option<String>,
    pub scratch_free_gb: Option<u32>,
    /// True only if the script ran to completion. A truncated probe (ssh dropped, shell
    /// rc file wrote to stdout, ...) must not be mistaken for "this host has no GPU".
    pub complete: bool,
    pub probed_at: String,
}

impl HostProbe {
    pub fn parse(stdout: &str) -> Self {
        let mut probe = HostProbe {
            probed_at: crate::util::now_iso(),
            ..Default::default()
        };
        let mut scalars: BTreeMap<&str, &str> = BTreeMap::new();

        for line in stdout.lines() {
            let line = line.trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "gpu" => {
                    if let Some(gpu) = parse_gpu(value) {
                        probe.gpus.push(gpu);
                    }
                }
                "tool" => {
                    if let Some(tool) = parse_tool(value) {
                        probe.tools.push(tool);
                    }
                }
                _ => {
                    scalars.insert(key, value);
                }
            }
        }

        probe.os = scalars.get("os").unwrap_or(&"unknown").to_string();
        probe.arch = scalars.get("arch").unwrap_or(&"unknown").to_string();
        probe.hostname = scalars.get("host").unwrap_or(&"unknown").to_string();
        probe.cpu = scalars.get("cpu").unwrap_or(&"unknown").to_string();
        probe.cores = scalars
            .get("cores")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        probe.memory_gb = scalars
            .get("mem_bytes")
            .and_then(|v| v.parse::<u64>().ok())
            .map(bytes_to_gb)
            .unwrap_or(0);
        probe.scratch = scalars.get("scratch").map(|v| v.to_string());
        probe.scratch_free_gb = scalars
            .get("scratch_free_bytes")
            .and_then(|v| v.parse::<u64>().ok())
            .map(bytes_to_gb);
        probe.complete = scalars.contains_key("probe_done");
        probe
    }

    pub fn tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tool(name).is_some()
    }

    pub fn has_gpu(&self) -> bool {
        !self.gpus.is_empty()
    }

    /// Largest single-device memory, which is what a model actually has to fit into.
    pub fn max_vram_gb(&self) -> Option<u32> {
        self.gpus.iter().filter_map(|g| g.memory_gb).max()
    }

    pub fn summary(&self) -> String {
        let gpu = if self.gpus.is_empty() {
            "no GPU".to_string()
        } else if self.gpus.len() == 1 {
            self.gpus[0].name.clone()
        } else {
            format!("{}x {}", self.gpus.len(), self.gpus[0].name)
        };
        format!(
            "{} {} - {} cores, {} GiB RAM - {}",
            self.os, self.arch, self.cores, self.memory_gb, gpu
        )
    }
}

fn bytes_to_gb(bytes: u64) -> u32 {
    (bytes / 1_073_741_824) as u32
}

fn dash_to_none(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "-" {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_gpu(value: &str) -> Option<Gpu> {
    let mut parts = value.split('|');
    let backend = GpuBackend::parse(parts.next()?.trim());
    let name = parts.next()?.trim().to_string();
    let memory_gb = parts
        .next()
        .and_then(dash_to_none)
        .and_then(|v| v.parse().ok());
    let driver = parts.next().and_then(dash_to_none);
    if name.is_empty() {
        return None;
    }
    Some(Gpu {
        backend,
        name,
        memory_gb,
        driver,
    })
}

fn parse_tool(value: &str) -> Option<Tool> {
    let mut parts = value.splitn(3, '|');
    let name = parts.next()?.trim().to_string();
    let path = parts.next()?.trim().to_string();
    let version = parts.next().and_then(dash_to_none);
    if name.is_empty() || path.is_empty() {
        return None;
    }
    Some(Tool {
        name,
        path,
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
probe_schema=1
os=Linux
arch=x86_64
host=login01
cpu=AMD EPYC 7763
cores=128
mem_bytes=1099511627776
gpu=cuda|NVIDIA A100-SXM4-80GB|80|535.104.05
gpu=cuda|NVIDIA A100-SXM4-80GB|80|535.104.05
tool=python3|/usr/bin/python3|Python 3.11.2
tool=sbatch|/usr/bin/sbatch|slurm 23.02.6
scratch=/scratch/me
scratch_free_bytes=10995116277760
probe_done=1
";

    #[test]
    fn parses_a_cluster_login_node() {
        let probe = HostProbe::parse(SAMPLE);
        assert!(probe.complete);
        assert_eq!(probe.cores, 128);
        assert_eq!(probe.memory_gb, 1024);
        assert_eq!(probe.gpus.len(), 2);
        assert_eq!(probe.max_vram_gb(), Some(80));
        assert!(probe.has_tool("sbatch"));
        assert!(!probe.has_tool("blender"));
        assert_eq!(probe.scratch_free_gb, Some(10240));
    }

    #[test]
    fn truncated_output_is_not_reported_as_complete() {
        let probe = HostProbe::parse("os=Linux\narch=x86_64\n");
        assert!(!probe.complete);
        assert!(!probe.has_gpu());
    }

    #[test]
    fn apple_unified_memory_counts_as_vram() {
        let probe = HostProbe::parse("gpu=metal|Apple M2 Pro (unified)|16|-\nprobe_done=1\n");
        assert_eq!(probe.gpus[0].backend, GpuBackend::Metal);
        assert_eq!(probe.max_vram_gb(), Some(16));
        assert!(probe.gpus[0].driver.is_none());
    }

    #[test]
    fn shell_noise_before_output_is_ignored() {
        // Login shells that echo MOTDs into non-interactive ssh sessions are a fact of
        // cluster life; the parser must skip anything that is not key=value.
        let noisy = format!("Welcome to cluster!\nModule load done.\n{SAMPLE}");
        let probe = HostProbe::parse(&noisy);
        assert!(probe.complete);
        assert_eq!(probe.gpus.len(), 2);
    }
}
