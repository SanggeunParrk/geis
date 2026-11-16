#!/bin/sh
# Host capability probe.
#
# Deliberately POSIX sh with no dependencies: the exact same script is run locally and
# piped to `ssh host sh -s` for remote hosts, so local and remote discovery can never
# drift apart. Output is line-oriented key=value; unknown or failed checks are simply
# absent rather than guessed.
#
#   os=Darwin
#   gpu=<backend>|<name>|<vram_gib or ->|<driver or ->
#   tool=<name>|<path>|<version or ->

emit() { printf '%s\n' "$1"; }

emit "probe_schema=1"
emit "os=$(uname -s 2>/dev/null || echo unknown)"
emit "arch=$(uname -m 2>/dev/null || echo unknown)"
emit "host=$(hostname 2>/dev/null || echo unknown)"
emit "shell=${SHELL:-unknown}"
emit "home=${HOME:-unknown}"

case "$(uname -s)" in
Darwin)
    emit "cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
    emit "cores=$(sysctl -n hw.ncpu 2>/dev/null || echo 0)"
    emit "mem_bytes=$(sysctl -n hw.memsize 2>/dev/null || echo 0)"
    ;;
Linux)
    emit "cpu=$(awk -F': ' '/^model name/{print $2; exit}' /proc/cpuinfo 2>/dev/null || echo unknown)"
    emit "cores=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
    kb=$(awk '/^MemTotal/{print $2; exit}' /proc/meminfo 2>/dev/null || echo 0)
    emit "mem_bytes=$((kb * 1024))"
    ;;
esac

# --- GPUs -------------------------------------------------------------------
# NVIDIA. One line per device; memory reported in MiB, converted to GiB.
if command -v nvidia-smi >/dev/null 2>&1; then
    nvidia-smi --query-gpu=name,memory.total,driver_version \
        --format=csv,noheader,nounits 2>/dev/null |
        while IFS=, read -r name mem driver; do
            name=$(printf '%s' "$name" | sed 's/^ *//; s/ *$//')
            mem=$(printf '%s' "$mem" | sed 's/[^0-9]//g')
            driver=$(printf '%s' "$driver" | sed 's/^ *//; s/ *$//')
            [ -n "$mem" ] && gib=$((mem / 1024)) || gib="-"
            emit "gpu=cuda|${name}|${gib}|${driver:--}"
        done
fi

# AMD.
if command -v rocm-smi >/dev/null 2>&1; then
    rocm-smi --showproductname --csv 2>/dev/null |
        awk -F, 'NR>1 && NF>1 {print $2}' |
        while read -r name; do
            [ -n "$name" ] && emit "gpu=rocm|${name}|-|-"
        done
fi

# Apple. On Apple silicon the GPU shares system memory, so VRAM is reported as unified
# memory - that is genuinely the budget a model has, and calling it "-" would be worse.
if [ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ]; then
    chip=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo "Apple Silicon")
    bytes=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
    emit "gpu=metal|${chip} (unified)|$((bytes / 1073741824))|-"
fi

# --- Tooling ----------------------------------------------------------------
probe_tool() {
    name=$1
    path=$(command -v "$name" 2>/dev/null) || return 0
    [ -n "$path" ] || return 0
    version=$("$name" --version 2>&1 | head -1 | tr -d '\r')
    case "$version" in
    *"nrecognized"* | *"nknown option"* | *usage* | *Usage*) version="-" ;;
    "") version="-" ;;
    esac
    emit "tool=${name}|${path}|${version}"
}

for t in python3 python conda mamba micromamba uv pip3 docker podman apptainer singularity \
    blender mmseqs jackhmmer hmmsearch sbatch squeue sinfo rsync git nvidia-smi rocm-smi; do
    probe_tool "$t"
done

# --- Scratch ----------------------------------------------------------------
scratch=${GEIS_SCRATCH:-${TMPDIR:-/tmp}}
emit "scratch=${scratch}"
avail=$(df -k "$scratch" 2>/dev/null | awk 'NR==2 {print $4}')
[ -n "$avail" ] && emit "scratch_free_bytes=$((avail * 1024))"

emit "probe_done=1"
