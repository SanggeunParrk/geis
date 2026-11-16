// The typed edge of the Rust core. Every call the UI makes goes through here, so the
// shape of the backend is written down in exactly one place.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";


export type Task =
  | "structure_prediction"
  | "sequence_design"
  | "docking"
  | "embedding"
  | "msa"
  | "custom";

export type GpuNeed = "required" | "recommended" | "none";
export type RuntimeKind = "python" | "docker" | "binary" | "http";
export type WeightsPolicy = "auto" | "manual" | "none";
export type ProviderKindName = "local" | "ssh" | "slurm" | "http";

export type JobState =
  | "queued"
  | "staging"
  | "pending"
  | "running"
  | "fetching"
  | "succeeded"
  | "failed"
  | "cancelled";

export interface Gpu {
  backend: "cuda" | "rocm" | "metal" | "unknown";
  name: string;
  memory_gb: number | null;
  driver: string | null;
}

export interface Tool {
  name: string;
  path: string;
  version: string | null;
}

export interface HostProbe {
  os: string;
  arch: string;
  hostname: string;
  cpu: string;
  cores: number;
  memory_gb: number;
  gpus: Gpu[];
  tools: Tool[];
  scratch: string | null;
  scratch_free_gb: number | null;
  complete: boolean;
  probed_at: string;
}

export type ProviderStatus =
  | { state: "unknown" }
  | { state: "ready"; summary: string }
  | { state: "degraded"; summary: string; warnings: string[] }
  | { state: "unreachable"; error: string };

export interface ExecOptions {
  scratch?: string | null;
  weights_dir?: string | null;
  env?: Record<string, string>;
  prelude?: string[];
  max_parallel?: number;
}

export type ProviderConfig =
  | ({ kind: "local" } & ExecOptions)
  | ({ kind: "ssh"; host: string; remote_dir: string; ssh_options?: string[] } & ExecOptions)
  | ({
      kind: "slurm";
      host: string;
      remote_dir: string;
      ssh_options?: string[];
      partition?: string | null;
      account?: string | null;
      gpus?: string | null;
      cpus_per_task?: number | null;
      mem?: string | null;
      time_limit?: string | null;
      extra_sbatch?: string[];
    } & ExecOptions)
  | {
      kind: "http";
      endpoint: string;
      auth_header?: string | null;
      auth_format?: string | null;
      secret_ref?: string | null;
      headers?: Record<string, string>;
      max_parallel?: number;
    };

export interface ProviderRecord {
  id: string;
  name: string;
  config: ProviderConfig;
  enabled: boolean;
  created_at: string;
  probe: HostProbe | null;
  status: ProviderStatus;
}

export interface InputSpec {
  type: "fasta" | "file" | "dir" | "int" | "float" | "bool" | "enum" | "text";
  required?: boolean;
  default?: unknown;
  min?: number | null;
  max?: number | null;
  choices?: string[] | null;
  extensions?: string[] | null;
  label?: string | null;
  help?: string | null;
}

export interface OutputSpec {
  type: "structure" | "json" | "csv" | "image" | "log" | "any";
  pattern: string;
  label?: string | null;
  primary?: boolean;
}

export interface ModelCard {
  id: string;
  name: string;
  task: Task;
  version: string | null;
  license: string | null;
  homepage: string | null;
  citation: string | null;
  description: string | null;
  requirements: {
    gpu: GpuNeed;
    vram_gb: number | null;
    runtime: RuntimeKind;
    weights: WeightsPolicy;
    disk_gb: number | null;
    needs_binaries: string[];
    notes: string | null;
  };
  inputs: Record<string, InputSpec>;
  outputs: Record<string, OutputSpec>;
  eligible_providers: string[];
  blocked_reason: string | null;
  /** `model` is pointed at and run; `extension` is installed by this app first. */
  kind?: "model" | "extension";
  install?: { steps: string[]; uninstall: string[]; download_mb: number | null; terms: string | null } | null;
}

/** Where an extension stands on one provider, by this app's own record of its jobs. */
export interface ExtensionState {
  provider_id: string;
  state: "installed" | "installing" | "removing" | "failed" | "absent";
  job_id: string | null;
  at: string | null;
}

export interface JobRecord {
  id: string;
  name: string;
  model_id: string;
  model_name: string;
  provider_id: string;
  task: string;
  state: JobState;
  world_id: string | null;
  local_dir: string;
  external_id: string | null;
  exit_code: number | null;
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
}

export interface StoredArtifact {
  name: string;
  kind: string;
  path: string;
  bytes: number;
  primary: boolean;
}

export interface JobDetail {
  job: JobRecord;
  artifacts: StoredArtifact[];
  is_running: boolean;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  jobs_dir: string;
  user_models_dir: string;
  bundled_models_dir: string | null;
  model_count: number;
  manifest_problems: { path: string; message: string }[];
}

export type Representation = "cartoon" | "spacefill" | "ball-and-stick" | "sticks" | "backbone";

export const REPRESENTATION_LABELS: Record<Representation, string> = {
  cartoon: "Cartoon",
  spacefill: "Spacefill",
  "ball-and-stick": "Ball and stick",
  sticks: "Sticks",
  backbone: "Backbone",
};

export type ColorScheme =
  | "element"
  | "chain"
  | "residue-kind"
  | "secondary"
  | "confidence"
  /** PyMOL's `spectrum b`: a ramp over the B-factors the structure actually has. */
  | "b-factor"
  /** PyMOL's `spectrum count, rainbow, byres=1`: one sweep, N to C, over everything. */
  | "rainbow"
  /** PyMOL's `util.chainbow`: the same sweep restarted for each chain. */
  | "chainbow";

export const COLOR_SCHEME_LABELS: Record<ColorScheme, string> = {
  element: "Element (CPK)",
  chain: "Chain",
  "residue-kind": "Residue type",
  secondary: "Secondary structure",
  confidence: "Confidence / B-factor",
  "b-factor": "B-factor (spectrum)",
  rainbow: "Rainbow (N to C)",
  chainbow: "Chainbow (per chain)",
};

/** Which ramp a continuous scheme uses. */
export type Ramp = "rainbow" | "viridis" | "blue-white-red";

export const RAMP_LABELS: Record<Ramp, string> = {
  rainbow: "Rainbow",
  viridis: "Viridis",
  "blue-white-red": "Blue-white-red",
};

/**
 * Schemes that only mean something computed over a whole molecule.
 *
 * Chainbow needs each chain to know where its own ends are and a colour per chain needs
 * the chain list to hand colours out from, so both belong to the molecule group and are
 * shared by its chains. A lone chain styled `chainbow` is just a rainbow.
 */
export const MOLECULE_WIDE_SCHEMES: ColorScheme[] = ["chain", "chainbow", "rainbow"];

/** Schemes that are a ramp, and so have a palette to choose. */
export const RAMP_SCHEMES: ColorScheme[] = ["b-factor", "rainbow", "chainbow"];

export type ContactKind = "polar" | "salt-bridge" | "pi-stacking" | "hydrophobic";

/** A contact between two atoms that is not a bond, with where its ends are. */
/** The analyses made of every structure as it is opened: four kinds of contact, four sets of residues. */
export type PresetId = ContactKind | "core" | "site" | "interface" | "ligands";

export interface ContactLine {
  residues: [number, number];
  names: [string, string];
  distance: number;
  from: [number, number, number];
  to: [number, number, number];
}

/** A named set of instructions for the agent, called on with `/name`. */
export interface AgentSkill {
  name: string;
  description: string;
  body: string;
}

/** A camera kept in a world: a named place to look from, as Blender keeps camera objects. */
export interface SavedCamera {
  id: string;
  name: string;
  eye: [number, number, number];
  center: [number, number, number];
  up: [number, number, number];
  radius: number;
}

export interface WorldView {
  cameras?: SavedCamera[];
  /** The camera last looked through. */
  active?: string | null;
}

/** The categorical palettes the core knows, by the names it reads. */
export type Categorical = "okabe-ito" | "vivid" | "tol-bright" | "tol-muted" | "tableau10" | "set2" | "soft";

export interface SceneOptions {
  show_hydrogens: boolean;
  show_waters: boolean;
  show_ligands: boolean;
}

export type LightKind = "sun" | "point" | "area";

export interface Light {
  id: string;
  name: string;
  kind: LightKind;
  /** Linear RGB 0..1. */
  color: [number, number, number];
  /** Irradiance for a sun, watts for the others. */
  energy: number;
  /** Unit vector from the subject towards the light, in world space. */
  direction: [number, number, number];
  /** Distance in units of the structure's radius; point and area lights only. */
  distance: number;
  /** Angular diameter in degrees for a sun, emitter size for an area light. */
  softness: number;
  /**
   * Whether `direction` is read relative to the camera rather than to the world.
   *
   * On by default, as PyMOL's lights are: a light fixed in the world means half of every
   * orbit is spent looking at the unlit side.
   */
  follow_view: boolean;
  enabled: boolean;
}

export interface Material {
  roughness: number;
  specular: number;
  metallic: number;
}

/**
 * How the scene is lit and coloured. The frontend owns this and sends it with each
 * request, so the viewport and a Blender render are driven by the same numbers.
 */
export interface SceneStyle {
  /** sRGB 0..1, as picked in the UI. */
  background: [number, number, number];
  lights: Light[];
  material: Material;
  ambient: number;
  categorical: Categorical;
  continuous_colors: boolean;
  ambient_occlusion: boolean;
  outline: boolean;
}

// --- the node graph --------------------------------------------------------

export type SocketKind =
  | "sequence"
  | "structure"
  | "geometry"
  | "colour"
  | "light"
  | "world"
  | "material";

/**
 * A node as the core describes it: title, sockets and their colours.
 *
 * The editor draws from this rather than from a table of its own. Socket types decide
 * which links are legal, and a second copy of that table would drift from the one that
 * enforces it.
 */
export interface NodeDescription {
  id: string;
  title: string;
  type_name: string;
  /** The family this node belongs to, and the colour its header is tinted with. */
  category: string;
  accent: [number, number, number];
  inputs: SocketDescription[];
  output: SocketDescription | null;
  position: [number, number];
}

export interface SocketDescription {
  name: string;
  kind: SocketKind;
  /** What to call the kind on screen; `kind` itself is the kebab-case wire tag. */
  kind_label: string;
  color: [number, number, number];
  multiple: boolean;
  required: boolean;
}

export type NodeState =
  | { state: "ready" }
  /** Usable, but something in it is being ignored. */
  | { state: "warned"; message: string }
  | { state: "unused" }
  | { state: "running"; job_id: string }
  | { state: "incomplete"; reason: string }
  | { state: "failed"; message: string };

/**
 * One node. The shape is serde's: a `type` tag plus that variant's own fields, flattened
 * alongside the node's id and position.
 */
/** Which Blender engine a Render node uses. */
export type RenderEngine = "cycles" | "eevee";

export const RENDER_ENGINE_LABELS: Record<RenderEngine, string> = {
  cycles: "Cycles (path traced)",
  eevee: "EEVEE (fast)",
};

export type GraphNode = {
  id: string;
  position: [number, number];
  /** Folded to its header. Part of the graph, like the position. */
  collapsed?: boolean;
} & (
  | {
      type: "molecule";
      pdb_id: string;
      path: string | null;
      /** Shown as its biological assembly; unsaid, an entry is and a file is not. */
      assembly?: boolean;
      /** Which assembly, model and alternate locations, where not the defaults. */
      version?: Version;
      /** Chain ids, filled in by the core once the structure is opened. */
      chains: string[];
      /** Whether the group is showing its chains. */
      expanded: boolean;
      /** Which atoms the molecule has. A property of the molecule, not of its style. */
      options: SceneOptions;
      /** The molecule's styles, in slot order - Blender's material slots. */
      slots: StyleSlot[];
      /** Which slot the panel is editing. Not which slot is drawn - that is per chain. */
      active_slot: number;
      /** The molecule's residue-level features, laid over its chain styles. */
      features?: Feature[];
      /** Named parts of the molecule, picked out and kept - PyMOL's named selections. */
      objects?: NamedObject[];
    }
  | { type: "sequence-input"; fasta: string }
  | {
      type: "predict";
      model_id: string;
      provider_id: string;
      inputs: Record<string, unknown>;
      job_id: string | null;
    }
  | {
      type: "chain";
      chain: string;
      /** Which of the molecule's style slots this chain draws with. */
      slot: number | null;
      /** A style of the chain's own, from before styles lived in slots. */
      style: Style | null;
    }
  | { type: "style"; style: Style }
  | {
      type: "colouring";
      scheme: ColorScheme;
      categorical: Categorical;
      continuous: boolean;
    }
  | { type: "solid-colour"; color: [number, number, number] }
  | { type: "light"; light: Light }
  | {
      type: "world";
      background: [number, number, number];
      ambient: number;
      ambient_occlusion: boolean;
      outline: boolean;
    }
  | { type: "material"; material: Material }
  | { type: "viewport" }
  | {
      type: "render";
      engine: RenderEngine;
      samples: number;
      width: number;
      height: number;
      transparent: boolean;
      /** Where the last render of this node went. */
      output: string | null;
    }
  | { type: "image"; path: string | null }
);

export interface GraphLink {
  from: string;
  to: string;
  socket: string;
}

export interface Graph {
  nodes: GraphNode[];
  links: GraphLink[];
}

export interface NodeTemplate {
  type_name: string;
  title: string;
  group: string;
  /** A ready-to-insert node, minus its id and position. */
  kind: Record<string, unknown>;
}

export type StructureRef =
  | { source: "file"; path: string }
  | { source: "pdb"; id: string }
  | { source: "job"; job_id: string };

/**
 * How a molecule, or one chain of it, is drawn - the analogue of a Blender material.
 */
export interface Style {
  /** Several at once: a cartoon with its side chains as sticks is one style, not two. */
  representations: Representation[];
  scheme: ColorScheme;
  categorical: Categorical;
  continuous: boolean;
  /** A flat colour, used instead of the scheme when set. */
  solid: [number, number, number] | null;
  /** Which ramp the continuous schemes use. */
  ramp: Ramp;
  /** The surface. Part of the style, as a Blender material's surface is. */
  material: Material;
  /** Colour everything but carbon by element, whatever the scheme says. */
  heteroatoms_by_element: boolean;
}

/** A named flat colour. */
export interface NamedColour {
  name: string;
  srgb: [number, number, number];
  linear: [number, number, number];
}

/**
 * One style in a molecule's slot list - a Blender material slot.
 *
 * The style lives on the molecule and the chains name a slot, so one slot used by three
 * chains is one style to edit.
 */
export interface StyleSlot {
  name: string;
  style: Style;
}

/** A style with a name - a Blender material, for a molecule. */
export interface NamedStyle {
  name: string;
  style: Style;
  /** Shipped with the app, so it cannot be renamed or deleted. */
  builtin: boolean;
}

export interface StyleLibrary {
  styles: NamedStyle[];
  colours: NamedColour[];
}

/** A style and the chain it applies to. `null` means every chain without one of its own. */
export interface Assignment {
  chain: string | null;
  style: Style;
}

/** One molecule in the scene: where it comes from, and how its chains are drawn. */
/**
 * One feature of a molecule: something said about a run of its residues, by chain and
 * author numbers. Drawn in extra representations, hidden, or given a colour of its own.
 */
export interface Feature {
  chain: string;
  from: number;
  to: number;
  show?: Representation[];
  hide?: boolean;
  /** Linear RGB, like a style's `solid`. */
  colour?: [number, number, number] | null;
}

/** A named set of residues of one molecule, as runs of author numbers within chains. */
export interface NamedObject {
  name: string;
  runs: { chain: string; from: number; to: number }[];
}

export interface PlanPart {
  structure: StructureRef;
  options: SceneOptions;
  assignments: Assignment[];
  features?: Feature[];
}

export interface GraphPlan {
  /** One per Molecule group feeding the output. A viewport draws a scene. */
  parts: PlanPart[];
  style: SceneStyle;
}

/** One Render node's resolved pipeline. */
export interface RenderTarget {
  node_id: string;
  plan: GraphPlan | null;
  problem: string | null;
}

export interface GraphEvaluation {
  plan: GraphPlan | null;
  renders: RenderTarget[];
  node_states: Record<string, NodeState>;
  problems: string[];
}

export interface GraphResult {
  evaluation: GraphEvaluation;
  nodes: NodeDescription[];
  /** One per molecule in the plan, in the plan's order. */
  structures: StructureInfo[];
  submitted: { node_id: string; job_id: string } | null;
  /** Chains each opened structure turned out to have, for its Molecule group to record. */
  chains: { node_id: string; chains: string[] }[];
}

export interface SwatchGroup {
  name: string;
  colors: string[];
}

export interface ScenePart {
  structure_id: string;
  options: SceneOptions;
  assignments: Assignment[];
  features?: Feature[];
  /** Where this part stands, for a viewport holding several worlds at once. */
  place?: [number, number, number];
  /** The radius it is sized to when placed, so a row of worlds is framed alike. */
  size?: number;
}

export interface SceneRequest {
  parts: ScenePart[];
  style: SceneStyle;
  /** Build it for a preview rather than for study: the coarsest ribbon that still reads. */
  coarse?: boolean;
}

export interface ChainInfo {
  /** `asym_id_oper_id`, e.g. `A_1`: the archive's own chain, and the assembly operator. */
  id: string;
  /** The author chain id - the letter in the paper. */
  auth_id: string;
  /** `polymer`, `non-polymer`, `water` or `branched`. */
  entity_type: string;
  residue_count: number;
}

export interface StructureInfo {
  id: string;
  /** The PDB id for an archive entry, the data block name otherwise. */
  entry_id: string;
  title: string;
  source: string;
  path: string | null;
  atom_count: number;
  bond_count: number;
  residue_count: number;
  chains: ChainInfo[];
  b_factor_min: number;
  b_factor_max: number;
  /** True when the B-factor column holds pLDDT, so the field can be named correctly. */
  looks_like_plddt: boolean;
  /** Something about how it was opened worth a line: the assembly was too large, say. */
  note?: string | null;
  /** What else the file can be read as, and what this is. */
  versions?: Versions;
}

/** An entry's assemblies, models and alternate locations; see `Versions` in the core. */
export interface Versions {
  assemblies: { id: string; details: string; count: number | null }[];
  models: string[];
  alternates: string[];
  alternate_residues: number;
  /** The assembly this is, or null for the deposited coordinates. */
  assembly: string | null;
  model: string | null;
  /** The label kept where a residue has it, or null for each residue's most occupied. */
  alternate: string | null;
}

/** A molecule's version as the core's `ReadingRequest` takes it. */
function readingOf(version: Version): { assembly_id: string | null; model: string | null; alternate: string | null } {
  return { assembly_id: version.assembly_id ?? null, model: version.model ?? null, alternate: version.alternate ?? null };
}

/** Which version of an entry a molecule is read as, where not the default. */
export interface Version {
  assembly_id?: string;
  model?: string;
  alternate?: string;
}

/** What the viewport was pointing at, at every level. */
export interface PickTarget {
  /** `A_1`. */
  chain: string;
  /** The author chain id, for the label people read. */
  auth_chain: string;
  residue_index: number;
  residue_number: number;
  residue_name: string;
  atom_index: number | null;
  atom_name: string | null;
  element: string | null;
}

/** Which atoms and residues of a structure belong to a set of chains. */
export interface SelectionMask {
  /** One byte per atom, 1 when selected. */
  atoms: number[];
  residues: number[];
}

/** BioMol's three levels. */
export type StructureLevel = "chain" | "residue" | "atom";

/** A selection as the core has it: a level and indices. BioMol's `View`. */
export interface CoreView {
  level: StructureLevel;
  indices: number[];
}

/** The conditions a selection is built from, all ANDed - BioMol's `select(**kwargs)`. */
export interface SelectWhere {
  chains?: string[];
  entity_type?: string[];
  residue_indices?: number[];
  residue_numbers?: { from: number; to: number }[];
  residue_names?: string[];
  hetero?: boolean | null;
  secondary?: string[];
  atom_names?: string[];
  elements?: string[];
  atom_indices?: number[];
}

export interface SelectSpec {
  level: StructureLevel;
  where?: SelectWhere;
}

export interface ViewSummary {
  level: StructureLevel;
  chains: number;
  residues: number;
  atoms: number;
  label: string;
  chain_ids: string[];
  /** For an atom view, the residues its atoms belong to; empty otherwise. */
  residue_indices: number[];
}

/** One residue of a chain, for the BioMol panel's tree. */
export interface ResidueRow {
  index: number;
  name: string;
  code: string;
  number: number;
  insertion_code: string | null;
  secondary: string;
  kind: "amino" | "nucleotide" | "water" | "ligand";
  atom_count: number;
}

export interface AtomRow {
  index: number;
  name: string;
  element: string;
  b_factor: number;
}

/** A residue opened in the map: its atoms and the bonds among them. */
export interface ResidueAtoms {
  atoms: AtomRow[];
  bonds: [number, number][];
}

// --- worlds ------------------------------------------------------------------------

/** Where a world's camera was left. */
export interface WorldCamera {
  eye: [number, number, number];
  center: [number, number, number];
  up: [number, number, number];
  radius: number;
}

/** A world on the shelf, without its graph. */
export interface WorldCard {
  id: string;
  name: string;
  created: number;
  opened: number;
  molecules: number;
  chains: number;
  /** A fingerprint of the graph, so the shelf knows whether it has to build the row again. */
  stamp: number;
}

/** A world in full: the session itself. */
export interface World {
  id: string;
  name: string;
  created: number;
  opened: number;
  graph: Graph;
  camera: WorldCamera | null;
}

// --- agents ------------------------------------------------------------------------

export type AgentKind = "local" | "anthropic" | "openai" | "cli";

export interface AgentConnection {
  id: string;
  name: string;
  kind: AgentKind;
  endpoint: string;
  model: string;
  secret_ref: string | null;
  /** For `cli`: the program to run, e.g. `claude`. */
  command: string | null;
  /** For `cli`: the arguments before the prompt, which goes in on stdin. */
  args: string[];
  /** For `local`: the downloaded model Geis runs itself. The endpoint is then worked out. */
  engine_model: string | null;
  /** For `cli`: which of the provider's models to ask for; null leaves it to the tool. */
  choice?: string | null;
  /** For `cli`: how hard it is to think, `low` to `max`; null leaves it to the tool. */
  effort?: string | null;
}

/** How much an agent may do without being asked. */
export type AgentPolicy = "auto" | "ask" | "read-only";

export interface AgentConfig {
  connections: AgentConnection[];
  policy: AgentPolicy;
  active: string | null;
}

export interface AgentToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

export interface AgentMessage {
  /** `user`, `assistant` or `tool`. */
  role: string;
  content: string;
  /** An assistant turn's reasoning, when the model does that. Not sent back to the model. */
  thinking?: string;
  tool_calls: AgentToolCall[];
  tool_call_id: string | null;
  /**
   * The turn as its provider wrote it, handed back untouched on the next request. Opaque
   * here: Claude's thinking blocks are signed, and only the core's adapter reads them.
   */
  provider?: unknown;
}

export interface AgentToolSpec {
  name: string;
  description: string;
  /** A JSON Schema object for the arguments. */
  parameters: Record<string, unknown>;
}

export interface ChatRequest {
  system: string;
  messages: AgentMessage[];
  tools: AgentToolSpec[];
  max_tokens?: number | null;
  temperature?: number | null;
}

export interface ChatReply {
  text: string;
  /** What a reasoning model worked through before answering. Shown folded away. */
  thinking: string;
  tool_calls: AgentToolCall[];
  /** `end` when the model is done, `tool` when it is waiting for results. */
  stop_reason: string;
  usage: { input: number; output: number };
  model: string;
  /** See `AgentMessage.provider`. */
  provider?: unknown;
}

/** A local model on offer, and what this machine has of it. */
export interface LocalModelState {
  id: string;
  name: string;
  summary: string;
  file: string;
  url: string;
  /** Bytes the whole file takes. */
  size: number;
  licence: string;
  /** How well it calls a tool with correct arguments. */
  tool_use: "good" | "fair" | "poor";
  /** Whether it reasons before answering: slower, and much better at a task with steps. */
  thinks: boolean;
  ram_gb: number;
  recommended: boolean;
  installed: boolean;
  /** Bytes on disk: the whole file, or how far a part-finished download got. */
  have: number;
  path: string | null;
}

export interface LocalModelProgress {
  id: string;
  received: number;
  total: number;
  /** `downloading`, `done`, `cancelled` or `failed`. */
  state: string;
  error: string | null;
}

/** The local inference engine: whether it is here, and whether it is running. */
export interface EngineState {
  /** False on a platform there is no build for. */
  supported: boolean;
  build: string;
  installed: boolean;
  path: string | null;
  size: number;
  running: boolean;
  port: number | null;
  /** The model id it was started with. */
  model: string | null;
  /** The window it was started with, in tokens; 0 before it has been started. */
  context?: number;
}

/** A world's conversation with its agent, as it is kept between sessions. */
export interface WorldChat {
  messages: AgentMessage[];
  usage?: { input: number; output: number };
}

/** Where a subscription connection stands: is its tool here, and is it signed in. */
export interface AgentAccount {
  found: string | null;
  version: string | null;
  signed_in: boolean | null;
  /** What it is signed in as, e.g. `Claude subscription (team)`. Never an address. */
  account: string | null;
  /** The command that installs the tool. */
  install: string;
  /** The command that signs it in, as typed in a terminal. */
  login: string;
}

/** A piece of an answer as it arrives. */
export type AgentDelta =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool"; id: string; name: string }
  /** Which model is actually answering. */
  | { kind: "model"; model: string }
  /** What it is doing while there is nothing to show yet: "thinking". */
  | { kind: "status"; text: string };

/** One document an agent can read. The index carries an empty body. */
export interface AgentDoc {
  name: string;
  title: string;
  summary: string;
  body: string;
}

/** One chain of the molecule map: residues as parallel arrays. */
export interface MapChain {
  id: string;
  auth_id: string;
  entity_type: string;
  residue_indices: number[];
  codes: string;
  secondary: string;
  numbers: number[];
  b_factors: number[];
  names: string[];
  /** x, y, z per residue: where each component stands in the graph view. */
  centers: number[];
  /** One character per residue: a amino acid, n nucleotide, w water, i ion, l ligand. */
  kinds: string;
  /** Each residue's atoms: the first one's index, and how many. */
  first_atoms?: number[];
  atom_counts?: number[];
}

export interface MapLink {
  residues: [number, number];
  kind: string;
}

/** Everything the molecule map draws for one structure. */
export interface StructureMap {
  id: string;
  entry_id: string;
  title: string;
  atom_count: number;
  chains: MapChain[];
  links: MapLink[];
  /** Polymer bonds between consecutive residues, as pairs of residue indices. */
  backbone: [number, number][];
}

/** One chain's sequence, as two strings the same length. */
export interface ChainSequence {
  chain: string;
  /** One-letter codes, in residue order. */
  codes: string;
  /** `H` for helix, `E` for strand, `-` otherwise. One character per code. */
  secondary: string;
  first_number: number;
  /** The structure's index of each residue, one per code. */
  residue_indices: number[];
}

export interface BlenderInfo {
  path: string;
  version: string;
}

export interface RenderSettings {
  eye: [number, number, number];
  center: [number, number, number];
  up: [number, number, number];
  fov_degrees: number;
  radius: number;
  width: number;
  height: number;
  samples: number;
  engine: "cycles" | "eevee";
  transparent: boolean;
  draft: boolean;
}

export interface RenderRequestBody {
  parts: ScenePart[];
  style: SceneStyle;
  settings: RenderSettings;
  output?: string | null;
}

export interface RenderOutcome {
  image: string;
  triangles: number;
  seconds: number;
  engine: string;
  log: string;
}

export interface LegendEntry {
  color: [number, number, number];
  label: string;
}

export interface LogChunk {
  job_id: string;
  text: string;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),

  listProviders: () => invoke<ProviderRecord[]>("list_providers"),
  saveProvider: (draft: {
    id?: string;
    name: string;
    enabled?: boolean;
    config: ProviderConfig;
  }) => invoke<ProviderRecord>("save_provider", { draft }),
  deleteProvider: (id: string) => invoke<void>("delete_provider", { id }),
  probeProvider: (id: string) => invoke<HostProbe>("probe_provider", { id }),

  listModels: () => invoke<ModelCard[]>("list_models"),
  reloadModels: () => invoke<number>("reload_models"),

  submitJob: (request: {
    model_id: string;
    provider_id: string;
    world_id?: string | null;
    name?: string | null;
    inputs: Record<string, unknown>;
  }) => invoke<JobRecord>("submit_job", { request }),
  cancelJob: (id: string) => invoke<boolean>("cancel_job", { id }),
  listJobs: (worldId?: string, limit = 100) =>
    invoke<JobRecord[]>("list_jobs", { worldId: worldId ?? null, limit }),
  jobDetail: (id: string) => invoke<JobDetail>("job_detail", { id }),
  readJobLog: (id: string, from?: number) =>
    invoke<{ text: string; next_offset: number }>("read_job_log", {
      id,
      from: from ?? null,
    }),

  setSecret: (reference: string, value: string) =>
    invoke<void>("set_secret", { reference, value }),
  hasSecret: (reference: string) => invoke<boolean>("has_secret", { reference }),
  deleteSecret: (reference: string) => invoke<void>("delete_secret", { reference }),

  revealPath: (path: string) => invoke<void>("reveal_path", { path }),

  /** Whether Blender is available for offline rendering, and which one. */
  blenderInfo: () => invoke<BlenderInfo | null>("blender_info"),
  /** Path-traces the current view with Blender. */
  renderScene: (request: RenderRequestBody) =>
    invoke<RenderOutcome>("render_scene", { request }),
  /**
   * Renders one Render node in the graph.
   *
   * The node's inputs decide the scene and its own fields decide the quality; the camera
   * comes from the viewport so the render is the view on screen, at higher quality.
   */
  graphRender: (
    graph: Graph,
    nodeId: string,
    camera: {
      eye: [number, number, number];
      center: [number, number, number];
      up: [number, number, number];
      radius: number;
      /** The viewport's width/height, so the render frames what the viewport framed. */
      view_aspect: number;
    },
    output?: string | null,
  ) => invoke<RenderOutcome>("graph_render", { graph, nodeId, camera, output: output ?? null }),
  /** Copies a rendered image to wherever the user asked for it. */
  exportImage: (from: string, to: string) => invoke<string>("export_image", { from, to }),
  /**
   * Puts an image on the system clipboard.
   *
   * Through the core rather than the webview's clipboard API, which requires the write to
   * happen inside the user gesture that started it - and reports success either way.
   */
  copyImage: (path: string) => invoke<void>("copy_image", { path }),
  /** Writes a rendered frame to disk from a canvas data URL. */
  /** Downloads a picture from an https address into the app's cache; returns its path. */
  fetchImage: (url: string) => invoke<string>("fetch_image", { url }),
  saveImage: (path: string, dataUrl: string) =>
    invoke<string>("save_image", { path, dataUrl }),
  /**
   * A view the environment asked for: what to draw, and where to write it. One call for
   * both outputs, so a Blender render asked for on its own is still coloured as requested.
   */
  scriptedView: () =>
    invoke<{
      representation: string | null;
      color_scheme: string | null;
      screenshot: string | null;
      blender_render: string | null;
    } | null>("scripted_view"),

  /** A structure path passed on the command line, consumed once. */
  pendingOpen: () => invoke<string | null>("pending_open"),

  /** Parses a structure file and keeps it open in the core. */
  openStructure: (path: string, assembly = false, reading: Version = {}) =>
    invoke<StructureInfo>("open_structure", { path, assembly, reading: readingOf(reading) }),
  /** Downloads and caches an entry from the RCSB PDB by its four-character id. */
  /** As its first biological assembly unless `assembly` is false; see `fetch_structure`. */
  fetchStructure: (pdbId: string, assembly = true, reading: Version = {}) =>
    invoke<StructureInfo>("fetch_structure", { pdbId, assembly, reading: readingOf(reading) }),
  /**
   * Builds geometry for a representation. The response is an ArrayBuffer, not JSON: a
   * cartoon of a large assembly is hundreds of thousands of triangles.
   */
  buildScene: (request: SceneRequest) => invoke<ArrayBuffer>("build_scene", { request }),
  /** Legend rows matching the colours the geometry was built with. */
  sceneLegend: (structureId: string, colorScheme: ColorScheme, style: SceneStyle) =>
    invoke<LegendEntry[]>("scene_legend", { structureId, colorScheme, style }),
  /** Open Color's ladders plus the categorical sets, for the colour picker. */
  colorSwatches: () => invoke<SwatchGroup[]>("color_swatches"),
  /** The default lighting rig, so the UI starts from what the renderer would use. */
  defaultStyle: () => invoke<SceneStyle>("default_style"),

  /** The graph a new session starts with: the smallest complete pipeline. */
  /** The graph this session opens with: the last one saved, or the default pipeline. */
  graphLoad: () => invoke<Graph>("graph_load"),
  /** Everything the Add menu offers. */
  graphCatalog: () => invoke<NodeTemplate[]>("graph_catalog"),
  /** The sequences of an open structure, for the sequence strip. */
  /** What a picked atom or residue belongs to: chain, residue, atom. */
  pickTarget: (id: string, kind: "atom" | "residue", index: number) =>
    invoke<PickTarget | null>("pick_target", { id, kind, index }),
  /** The atoms and residues of some chains, for highlighting them in the viewport. */
  selectionMask: (
    id: string,
    chains: string[],
    level?: { residue?: number; atom?: number; residues?: number[] },
  ) =>
    invoke<SelectionMask>("selection_mask", {
      id,
      chains,
      residue: level?.residue ?? null,
      atom: level?.atom ?? null,
      residues: level?.residues ?? null,
    }),
  /** The screen layout - which areas, split how, showing what - as the frontend's JSON. */
  layoutLoad: () => invoke<string | null>("layout_load"),
  layoutSave: (layout: string) => invoke<void>("layout_save", { layout }),
  structureSequence: (id: string) => invoke<ChainSequence[]>("structure_sequence", { id }),
  /** Evaluates a selection spec to a view. */
  selectView: (id: string, spec: SelectSpec) => invoke<CoreView>("select_view", { id, spec }),
  /** The viewport's masks for a view. */
  viewMask: (id: string, view: CoreView) => invoke<SelectionMask>("view_mask", { id, view }),
  /** Counts and a label for a view, for the card. */
  describeView: (id: string, view: CoreView) => invoke<ViewSummary>("describe_view", { id, view }),
  /** Every world, newest use first. */
  worldsList: () => invoke<WorldCard[]>("worlds_list"),
  /** Which world is open. */
  worldActive: () => invoke<string>("world_active"),
  worldCreate: (name: string) => invoke<WorldCard>("world_create", { name }),
  worldRename: (id: string, name: string) => invoke<void>("world_rename", { id, name }),
  worldDelete: (id: string) => invoke<void>("world_delete", { id }),
  /** Opens a world: the graph the viewer reads and writes becomes this one's. */
  worldOpen: (id: string) => invoke<World>("world_open", { id }),
  /** One world in full, for the shelf's preview. */
  worldRead: (id: string) => invoke<World>("world_read", { id }),
  worldCamera: (id: string, camera: WorldCamera) => invoke<void>("world_camera", { id, camera }),
  /** The conversation held with the agent in a world. */
  worldChat: (id: string) => invoke<WorldChat>("world_chat", { id }),
  worldChatSave: (id: string, chat: WorldChat) => invoke<void>("world_chat_save", { id, chat }),
  /** A world's picture for the shelf, if one was taken. */
  worldThumbnail: (id: string) => invoke<{ path: string; written: number } | null>("world_thumbnail", { id }),
  worldThumbnailSave: (id: string, dataUrl: string) => invoke<void>("world_thumbnail_save", { id, dataUrl }),
  /** What the file viewer shows for a file: drawn as it is, turned into a page, or a picture of its first page. */
  filePreview: (path: string) => invoke<FilePreview>("file_preview", { path }),
  /** A file's bytes, for the viewer to draw itself: a PDF. */
  fileBytes: (path: string) => invoke<ArrayBuffer>("file_bytes", { path }),
  /** Writes the scene as a `.glb` 3D model, for slides. */
  exportGlb: (request: SceneRequest, path: string) =>
    invoke<{ path: string; triangles: number; bytes: number }>("export_glb", { request, path }),
  /** Stops every Blender render under way. */
  renderCancel: () => invoke<void>("render_cancel"),
  extensionStates: (id: string) => invoke<ExtensionState[]>("extension_states", { id }),
  extensionMaintain: (id: string, providerId: string, action: "install" | "remove") =>
    invoke<JobRecord>("extension_maintain", { id, providerId, action }),
  /** Polar contacts or salt bridges, in a structure or around some of its residues. */
  findContacts: (id: string, kind: ContactKind, residues?: number[] | null) =>
    invoke<ContactLine[]>("find_contacts", { id, kind, residues: residues ?? null, waters: false }),
  /** The residues of a structure's hydrophobic core, by index. */
  findCore: (id: string) => invoke<number[]>("find_core", { id }),
  /** One of the analyses every structure gets when it is opened; see `structure_preset`. */
  structurePreset: (id: string, preset: PresetId) => invoke<ArrayBuffer>("structure_preset", { id, preset }),
  /** Puts a cleared conversation where an undo can find it. */
  chatBinPut: (id: string, chat: WorldChat) => invoke<void>("chat_bin_put", { id, chat }),
  /** Takes a world's last cleared conversation back, if it is still there. */
  chatBinTake: (id: string) => invoke<WorldChat | null>("chat_bin_take", { id }),
  /** The user's own skills, from the app's `skills` folder. */
  agentSkills: () => invoke<AgentSkill[]>("agent_skills"),
  /** A world's saved cameras and which one is in use. */
  worldView: (id: string) => invoke<WorldView>("world_view", { id }),
  worldViewSave: (id: string, view: WorldView) => invoke<void>("world_view_save", { id, view }),
  preferences: () => invoke<Record<string, unknown>>("preferences"),
  preferencesSave: (preferences: Record<string, unknown>) => invoke<void>("preferences_save", { preferences }),
  /** A world's pane layout: `default` if never arranged, null for a world older than this. */
  worldLayout: (id: string) => invoke<string | null>("world_layout", { id }),
  worldLayoutSave: (id: string, layout: string) => invoke<void>("world_layout_save", { id, layout }),

  /** The agent connections and how much they are allowed to do. */
  agentConfig: () => invoke<AgentConfig>("agent_config"),
  agentSaveConfig: (config: AgentConfig) => invoke<AgentConfig>("agent_save_config", { config }),
  /** Which connections have a key (or need none). */
  agentReady: () => invoke<string[]>("agent_ready"),
  /** One turn: the request goes out with the key attached in the core. */
  agentComplete: (id: string, request: ChatRequest) => invoke<ChatReply>("agent_complete", { id, request }),
  /**
   * One turn, reported as it is written.
   *
   * The pieces come down a channel - the answer, the reasoning, and each tool call as it
   * is decided - so the panel can show the work instead of a wait and then a wall of text.
   */
  agentStream: (id: string, request: ChatRequest, onDelta: (delta: AgentDelta) => void) => {
    const channel = new Channel<AgentDelta>();
    channel.onmessage = onDelta;
    return invoke<ChatReply>("agent_stream", { id, request, onDelta: channel });
  },
  /** Whether a subscription connection's tool is installed and signed in. */
  agentAccount: (id: string) => invoke<AgentAccount>("agent_account", { id }),
  /** Starts the tool's own sign-in in the browser; resolves when it has finished. */
  agentSignIn: (id: string) => invoke<AgentAccount>("agent_sign_in", { id }),
  /** What models a connection offers, and whether it answers at all. */
  agentProbe: (id: string) => invoke<string[]>("agent_probe", { id }),
  /** The index of the documents an agent can read. */
  agentDocs: () => invoke<AgentDoc[]>("agent_docs"),
  agentDoc: (name: string) => invoke<AgentDoc>("agent_doc", { name }),
  /** The local models on offer, with what is already downloaded. */
  localModels: () => invoke<LocalModelState[]>("local_models"),
  /** Fetches a model's weights. Progress arrives as events; resumes a part-finished file. */
  localModelDownload: (id: string) => invoke<void>("local_model_download", { id }),
  localModelCancel: (id: string) => invoke<void>("local_model_cancel", { id }),
  localModelDelete: (id: string) => invoke<void>("local_model_delete", { id }),
  /** The local engine: llama.cpp, fetched and checked against its published digest. */
  engineState: () => invoke<EngineState>("engine_state"),
  engineInstall: () => invoke<EngineState>("engine_install"),
  /** Starts the engine on a downloaded model and points the local connection at it. */
  engineStart: (model: string) => invoke<EngineState>("engine_start", { model }),
  engineStop: () => invoke<EngineState>("engine_stop"),

  /** The molecule map's data for a structure, in one call. */
  structureMap: (id: string) => invoke<StructureMap>("structure_map", { id }),
  /** The residues of one chain, in structure order. */
  chainResidues: (id: string, chain: string) => invoke<ResidueRow[]>("chain_residues", { id, chain }),
  /** The atoms of one residue. */
  residueAtoms: (id: string, residue: number) => invoke<ResidueAtoms>("residue_atoms", { id, residue }),
  /** The named styles - builtin and the user's - and the named colours behind them. */
  styleLibrary: () => invoke<StyleLibrary>("style_library"),
  /** Saves a style under a name. Returns the user's styles afterwards. */
  styleSave: (name: string, style: Style) =>
    invoke<NamedStyle[]>("style_save", { name, style }),
  styleDelete: (name: string) => invoke<NamedStyle[]>("style_delete", { name }),
  /** Saves the working graph, so a session survives closing the window. */
  graphSave: (graph: Graph) => invoke<void>("graph_save", { graph }),
  /** Throws the saved graph away and returns the default pipeline. */
  graphReset: () => invoke<Graph>("graph_reset"),
  /**
   * Evaluates a graph and does what it says - submitting a prediction, fetching from the
   * PDB, opening a file - returning the plan and each node's state.
   *
   * `submit: false` evaluates without starting a prediction that has not run - for a
   * caller that only wants to draw the graph, not to act on it.
   */
  graphEvaluate: (graph: Graph, submit = true) =>
    invoke<GraphResult>("graph_evaluate", { graph, submit }),
  closeStructure: (id: string) => invoke<void>("close_structure", { id }),

  /** Reports to the terminal running the app; see the Rust side for why this exists. */
  uiLog: (level: string, message: string) =>
    invoke<void>("ui_log", { level, message }).catch(() => {}),
};

export const events = {
  onJobChanged: (handler: (job: JobRecord) => void): Promise<UnlistenFn> =>
    listen<JobRecord>("job:changed", (event) => handler(event.payload)),
  onJobLog: (handler: (chunk: LogChunk) => void): Promise<UnlistenFn> =>
    listen<LogChunk>("job:log", (event) => handler(event.payload)),
  /** The agent's settings or its engine changed - in this window or another. */
  onAgentsChanged: (handler: () => void): Promise<UnlistenFn> =>
    listen("agents:changed", () => handler()),
  onEngineProgress: (handler: (progress: LocalModelProgress) => void): Promise<UnlistenFn> =>
    listen<LocalModelProgress>("engine:progress", (event) => handler(event.payload)),
  onLocalModelProgress: (handler: (progress: LocalModelProgress) => void): Promise<UnlistenFn> =>
    listen<LocalModelProgress>("local-model:progress", (event) => handler(event.payload)),
  onProviderChanged: (handler: (id: string) => void): Promise<UnlistenFn> =>
    listen<string>("provider:changed", (event) => handler(event.payload)),
  /** A structure was opened from outside - Finder, the Dock - and waits in `pendingOpen`. */
  onOpenFile: (handler: () => void): Promise<UnlistenFn> => listen("open-file", () => handler()),
  /** How far a Blender render has got: 0 to 1 once sampling, and what it is doing. */
  onRenderProgress: (handler: (progress: { output: string; fraction: number | null; stage: string }) => void): Promise<UnlistenFn> =>
    listen<{ output: string; fraction: number | null; stage: string }>("render:progress", (event) => handler(event.payload)),
  /** An item of the application menu that is the app's own was chosen; see `menu.rs`. */
  onMenu: (handler: (id: string) => void): Promise<UnlistenFn> =>
    listen<string>("menu", (event) => handler(event.payload)),
};

/** The file viewer's reading of a file; see `file_preview` in the core. */
export type FilePreview =
  | { kind: "image"; path: string }
  | { kind: "pdf"; path: string }
  | { kind: "html"; html: string }
  | { kind: "text"; text: string; cut: boolean }
  | { kind: "picture"; path: string }
  | { kind: "other" };
