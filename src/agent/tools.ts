// What an agent can do.
//
// One list, each entry a description the model reads, a JSON Schema for its arguments, and
// a function that does it. The functions are not implemented here: they are handed in by
// the screen, which owns the graph, the viewport and the selection. That split is what
// keeps this file readable as *what an agent may do*, which is the thing worth being able
// to read at a glance - and it is the whole of the answer to "what can it touch?". There
// is no tool for the app's own files, settings or code, and no tool that runs anything on
// the machine. That is not a permission that can be granted; it is an absence.
//
// `writes` is the only other axis: whether a tool changes what is on screen. The user's
// policy decides what happens to those - run them, ask first, or refuse.

import { plain } from "../ui/dom";
import type { AgentToolSpec, SelectSpec } from "../ipc";

/** What the screen must provide for the tools to work. Everything returns plain data. */
export interface AgentHost {
  listMolecules: () => Promise<unknown>;
  readGraph: () => Promise<unknown>;
  chainSequence: (chain: string, molecule?: string) => Promise<unknown>;
  currentSelection: () => Promise<unknown>;
  listDocs: () => Promise<unknown>;
  readDoc: (name: string) => Promise<unknown>;

  openPdb: (id: string) => Promise<unknown>;
  select: (spec: SelectSpec, molecule?: string) => Promise<unknown>;
  clearSelection: () => Promise<unknown>;
  setStyle: (args: StyleArgs) => Promise<unknown>;
  addSlot: (args: StyleArgs & { name: string }) => Promise<unknown>;
  assignSlot: (chain: string, slot: number | string, molecule?: string) => Promise<unknown>;
  colourChain: (chain: string, colour: string, molecule?: string) => Promise<unknown>;
  setMoleculeOptions: (args: { molecule?: string; hydrogens?: boolean; waters?: boolean; ligands?: boolean }) => Promise<unknown>;
  resetView: () => Promise<unknown>;
  showImage: (url: string, title?: string) => Promise<unknown>;

  styleResidues: (args: { molecule?: string; chain?: string; from?: number; to?: number; show?: unknown; colour?: unknown; hide?: unknown }) => Promise<unknown>;
  clearResidueStyles: (molecule?: string) => Promise<unknown>;
  listLinks: (molecule?: string) => Promise<unknown>;
  listLigands: (molecule?: string) => Promise<unknown>;
  measureDistance: (from: ResidueRef, to: ResidueRef, molecule?: string) => Promise<unknown>;
  setViewMode: (mode: string) => Promise<unknown>;
  setSelectLevel: (level: string) => Promise<unknown>;
  focus: (on: string) => Promise<unknown>;
  setWorld: (args: { background?: string; ambient?: number; ambient_occlusion?: boolean; outline?: boolean }) => Promise<unknown>;
  setLight: (light: string | number, energy: number) => Promise<unknown>;
  removeMolecule: (molecule?: string) => Promise<unknown>;
  openFile: (path: string) => Promise<unknown>;
  showPane: (pane: string, open: boolean) => Promise<unknown>;
  render: () => Promise<unknown>;
  saveViewportImage: (path: string, width?: number) => Promise<unknown>;
  undo: () => Promise<unknown>;
  redo: () => Promise<unknown>;
}

/** One residue, the way people name one: its chain and its number in that chain. */
export interface ResidueRef {
  chain: string;
  number: number;
}

export interface StyleArgs {
  molecule?: string;
  /** A slot's name or its position. The molecule's active slot when left out. */
  slot?: number | string;
  representations?: string[];
  scheme?: string;
  ramp?: string;
  /** A flat colour, `#rrggbb`. Clears the scheme when given; `null` goes back to it. */
  colour?: string | null;
  heteroatoms_by_element?: boolean;
}

export interface Tool {
  spec: AgentToolSpec;
  /** Whether it changes what is on screen, which is what the policy gates. */
  writes: boolean;
  run: (args: Record<string, unknown>) => Promise<unknown>;
}

export const REPRESENTATIONS = ["cartoon", "sticks", "spacefill", "ball-and-stick", "backbone"];
export const SCHEMES = ["element", "chain", "residue-kind", "secondary", "confidence", "b-factor", "rainbow", "chainbow"];
export const RAMPS = ["rainbow", "viridis", "blue-white-red"];

const molecule = {
  type: "string",
  description: "Which molecule, by node id. Only needed when more than one is open.",
};

const styleProperties = {
  molecule,
  slot: {
    type: ["string", "number"],
    description: "The slot's name or its position. The molecule's active slot when left out.",
  },
  representations: {
    type: "array",
    items: { type: "string", enum: REPRESENTATIONS },
    description: "How it is drawn. Several at once is allowed: a cartoon with sticks is one style.",
  },
  scheme: { type: "string", enum: SCHEMES, description: "The colour rule." },
  ramp: { type: "string", enum: RAMPS, description: "Which colours a continuous scheme runs through." },
  colour: {
    type: ["string", "null"],
    description: "A flat colour as #rrggbb, used instead of the scheme. null goes back to the scheme.",
  },
  heteroatoms_by_element: {
    type: "boolean",
    description: "Keep oxygen red, nitrogen blue and sulfur yellow whatever the scheme says.",
  },
};

/** The conditions a selection is built from - BioMol's, and the core's `Where`. */
const WHERE = {
  type: "object",
  description: "Conditions, all of which must hold. Leave out what does not matter.",
  properties: {
    chains: { type: "array", items: { type: "string" }, description: "Chain ids, `A_1` or the author's letter." },
    entity_type: { type: "array", items: { type: "string", enum: ["polymer", "non-polymer", "water", "branched"] } },
    residue_numbers: {
      type: "array",
      description: "Inclusive ranges of author residue numbers.",
      items: {
        type: "object",
        properties: { from: { type: "number" }, to: { type: "number" } },
        required: ["from", "to"],
      },
    },
    residue_names: { type: "array", items: { type: "string" }, description: "Three-letter codes, e.g. HIS." },
    secondary: { type: "array", items: { type: "string", enum: ["H", "E", "C"] } },
    hetero: { type: "boolean", description: "true for ligands and waters, false for the polymer." },
    atom_names: { type: "array", items: { type: "string" }, description: "e.g. CA, NZ." },
    elements: { type: "array", items: { type: "string" }, description: "e.g. FE, S." },
  },
};

function object(properties: Record<string, unknown>, required: string[] = []): Record<string, unknown> {
  return { type: "object", properties, required };
}

/**
 * A tool's result as the text the model is sent. Always a string: `JSON.stringify` gives
 * back undefined for a tool that returned nothing, and a tool message with no content is
 * one the provider refuses, which ends the whole conversation rather than the one call.
 */
const text = (value: unknown): string => (typeof value === "string" ? value : (JSON.stringify(value) ?? "done"));

/** Every tool, in the order an agent tends to need them. */
export function buildTools(host: AgentHost): Tool[] {
  const tool = (
    name: string,
    description: string,
    parameters: Record<string, unknown>,
    writes: boolean,
    run: (args: Record<string, unknown>) => Promise<unknown>,
  ): Tool => ({ spec: { name, description, parameters }, writes, run });

  return [
    // --- reading ---------------------------------------------------------------------
    tool(
      "list_docs",
      "The documents about this app that you can read: the node graph, selection, conventions.",
      object({}),
      false,
      () => host.listDocs(),
    ),
    tool(
      "read_doc",
      "Read one of those documents in full. Read `graph` before changing styles and `selection` before selecting.",
      object({ name: { type: "string", description: "Its name from list_docs." } }, ["name"]),
      false,
      (args) => host.readDoc(plain(args.name, "")),
    ),
    tool(
      "list_molecules",
      "The molecules that are open: node id, PDB entry, chains with their ids, entity types and residue counts.",
      object({}),
      false,
      () => host.listMolecules(),
    ),
    tool(
      "read_graph",
      "The node graph: every node with the fields that matter, and every link. Read this before changing anything.",
      object({}),
      false,
      () => host.readGraph(),
    ),
    tool(
      "chain_sequence",
      "One chain's sequence as one-letter codes, with its secondary structure and its first residue number.",
      object({ chain: { type: "string", description: "A chain id, `A_1` or the author's letter." }, molecule }, ["chain"]),
      false,
      (args) => host.chainSequence(plain(args.chain, ""), args.molecule as string | undefined),
    ),
    tool(
      "current_selection",
      "What is selected right now: the level, how many chains, residues and atoms, and a label for it.",
      object({}),
      false,
      () => host.currentSelection(),
    ),

    // --- changing --------------------------------------------------------------------
    tool(
      "colour_chain",
      "Give one chain its own colour. The usual way to answer 'make chain A magenta' - it does the slot work for you.",
      object(
        {
          chain: { type: "string", description: "A chain id, `A_1` or the author's letter." },
          colour: { type: "string", description: "A colour as #rrggbb, e.g. #ff00ff for magenta." },
          molecule,
        },
        ["chain", "colour"],
      ),
      true,
      (args) => host.colourChain(plain(args.chain, ""), plain(args.colour, ""), args.molecule as string | undefined),
    ),
    tool(
      "open_pdb",
      "Fetch a NEW structure from the PDB and add it to the graph. Only for four-character entry ids like 4hhb - never for a chain, which belongs to a molecule that is already open.",
      object({ id: { type: "string", description: "A PDB entry id, four characters, e.g. 4hhb." } }, ["id"]),
      true,
      (args) => host.openPdb(plain(args.id, "")),
    ),
    tool(
      "select",
      "Select part of a structure by condition. Selecting outlines it and points the panels at it; it does not change how anything is drawn.",
      object(
        {
          level: { type: "string", enum: ["chain", "residue", "atom"], description: "What the selection is made of." },
          where: WHERE,
          molecule,
        },
        ["level"],
      ),
      true,
      (args) =>
        host.select(
          { level: args.level as SelectSpec["level"], where: (args.where ?? {}) },
          args.molecule as string | undefined,
        ),
    ),
    tool("clear_selection", "Select nothing.", object({}), true, () => host.clearSelection()),
    tool(
      "set_style",
      "Change one of a molecule's style slots. Every chain on that slot changes with it, which is usually what is wanted.",
      object(styleProperties),
      true,
      (args) => host.setStyle(args),
    ),
    tool(
      "add_style_slot",
      "Add a style slot to a molecule, so some chains can be drawn differently from the rest. Assign it with assign_slot.",
      object({ ...styleProperties, name: { type: "string", description: "What to call it, e.g. Sticks." } }, ["name"]),
      true,
      (args) => host.addSlot({ ...(args as StyleArgs), name: plain(args.name, "Style") }),
    ),
    tool(
      "assign_slot",
      "Draw one chain with one of its molecule's style slots.",
      object(
        {
          chain: { type: "string", description: "A chain id, `A_1` or the author's letter." },
          slot: { type: ["string", "number"], description: "The slot's name or its position." },
          molecule,
        },
        ["chain", "slot"],
      ),
      true,
      (args) => host.assignSlot(plain(args.chain, ""), args.slot as string | number, args.molecule as string | undefined),
    ),
    tool(
      "set_molecule_options",
      "Which atoms a molecule draws at all: hydrogens, waters, ligands.",
      object({
        molecule,
        hydrogens: { type: "boolean" },
        waters: { type: "boolean" },
        ligands: { type: "boolean" },
      }),
      true,
      (args) => host.setMoleculeOptions(args),
    ),
    tool("reset_view", "Frame everything in the viewport again.", object({}), true, () => host.resetView()),
    tool(
      "style_residues",
      "Style a run of residues within a chain, on top of the chain's own style: draw them in extra representations (`show`, e.g. [\"sticks\"]), give them a flat `colour` (hex), or `hide` them. `from` and `to` are author residue numbers.",
      object(
        {
          molecule,
          chain: { type: "string" },
          from: { type: "integer" },
          to: { type: "integer" },
          show: { type: "array", items: { type: "string", enum: REPRESENTATIONS } },
          colour: { type: "string" },
          hide: { type: "boolean" },
        },
        ["chain", "from"],
      ),
      true,
      (args) => host.styleResidues(args),
    ),
    tool(
      "clear_residue_styles",
      "Remove every residue-level style from a molecule, leaving its chains' styles.",
      object({ molecule }),
      true,
      (args) => host.clearResidueStyles(args.molecule as string | undefined),
    ),
    tool(
      "list_links",
      "The bonds between residues that are not the backbone: disulfide bridges, metal coordination, covalent links to ligands and glycans. Each with the two residues it joins.",
      object({ molecule }),
      false,
      (args) => host.listLinks(args.molecule as string | undefined),
    ),
    tool(
      "list_ligands",
      "Everything in the molecule that is not polymer or water: ligands, ions, cofactors - with component id, chain and residue number.",
      object({ molecule }),
      false,
      (args) => host.listLigands(args.molecule as string | undefined),
    ),
    tool(
      "measure_distance",
      "The distance in angstroms between two residues (alpha carbon for an amino acid, C4' for a nucleotide, centre for a ligand). Each end is {chain, number}.",
      object(
        {
          from: object({ chain: { type: "string" }, number: { type: "integer" } }, ["chain", "number"]),
          to: object({ chain: { type: "string" }, number: { type: "integer" } }, ["chain", "number"]),
          molecule,
        },
        ["from", "to"],
      ),
      false,
      (args) => host.measureDistance(args.from as ResidueRef, args.to as ResidueRef, args.molecule as string | undefined),
    ),
    tool(
      "set_view_mode",
      "Switch the viewport between `structure` (the molecule as styled) and `graph` (each chemical component a labelled sphere joined by its bonds, over a ghost of the structure).",
      object({ mode: { type: "string", enum: ["structure", "graph"] } }, ["mode"]),
      true,
      (args) => host.setViewMode(plain(args.mode, "")),
    ),
    tool(
      "set_select_level",
      "What a click in the viewport selects: `atom`, `residue` or `chain`.",
      object({ level: { type: "string", enum: ["atom", "residue", "chain"] } }, ["level"]),
      true,
      (args) => host.setSelectLevel(plain(args.level, "")),
    ),
    tool(
      "focus",
      "Move the camera: `selection` zooms to what is selected, `all` frames everything.",
      object({ on: { type: "string", enum: ["selection", "all"] } }, ["on"]),
      true,
      (args) => host.focus(plain(args.on, "")),
    ),
    tool(
      "set_world",
      "The scene around the molecule: background colour (hex), ambient light 0..2, ambient occlusion on or off, dark outlines on or off.",
      object({
        background: { type: "string" },
        ambient: { type: "number" },
        ambient_occlusion: { type: "boolean" },
        outline: { type: "boolean" },
      }),
      true,
      (args) => host.setWorld(args),
    ),
    tool(
      "set_light",
      "How bright one light is. `light` is its name or position as read_graph lists them; `energy` is its strength (a sun is about 3).",
      object({ light: { type: "string" }, energy: { type: "number" } }, ["light", "energy"]),
      true,
      (args) => host.setLight(args.light as string | number, Number(args.energy)),
    ),
    tool(
      "remove_molecule",
      "Take a molecule, and its chains, out of the world.",
      object({ molecule }),
      true,
      (args) => host.removeMolecule(args.molecule as string | undefined),
    ),
    tool(
      "open_file",
      "Open a structure file on this machine by its full path (.pdb, .cif, .mmcif, .ent). Only a path the user gave you.",
      object({ path: { type: "string" } }, ["path"]),
      true,
      (args) => host.openFile(plain(args.path, "")),
    ),
    tool(
      "show_pane",
      "Open or close one of the panes beside the viewport: `graph` (the node graph), `sequence`, `text` (FASTA).",
      object({ pane: { type: "string", enum: ["graph", "sequence", "text"] }, open: { type: "boolean" } }, ["pane", "open"]),
      true,
      (args) => host.showPane(plain(args.pane, ""), args.open !== false),
    ),
    tool(
      "render",
      "Render the scene with Blender, as it is framed now. Takes a while; the picture appears in the world as an image.",
      object({}),
      true,
      () => host.render(),
    ),
    tool(
      "save_viewport_image",
      "Save what the viewport shows as a PNG at a full path the user gave you. `width` in pixels draws it again at that size, at the viewport's shape; left out, it is the size on screen.",
      object({ path: { type: "string" }, width: { type: "number" } }, ["path"]),
      true,
      (args) => host.saveViewportImage(plain(args.path, ""), typeof args.width === "number" ? args.width : undefined),
    ),
    tool("undo", "Undo the last change to the scene.", object({}), true, () => host.undo()),
    tool("redo", "Redo the change that was just undone.", object({}), true, () => host.redo()),
    tool(
      "show_image",
      "Show a picture from the web in an image pane beside the viewport - a figure from a paper, a diagram. `url` must be the https address of the image file itself (png, jpg, gif, webp), not of the page it is on.",
      object({ url: { type: "string" }, title: { type: "string" } }, ["url"]),
      true,
      (args) => host.showImage(plain(args.url, ""), args.title === undefined ? undefined : plain(args.title)),
    ),
  ];
}

/**
 * The system prompt: what the agent is, how it is to work, and what it has to hand.
 *
 * One prompt, for every model - the one running on this machine, Claude, ChatGPT. It is
 * written as rules, a table from request to tool, and worked examples rather than as
 * prose, for the smallest model that will read it: a few billion parameters given advice
 * guess at ids, call three tools in one reply, and answer "done" having called none. A
 * large model loses nothing by being told the same things plainly, and the two then
 * behave alike, which matters more than either being addressed in its own register.
 *
 * Every line is there because some model got that thing wrong without it: it invented a
 * node id, it wrote the colour as a word, it described a change instead of making it, it
 * answered in English to a question in Korean, it said it did not know rather than
 * looking. Keep it short - what is added here is taken from the room a small model's
 * conversation has.
 */
/**
 * The tools a model on this machine is given: what most requests are made of.
 *
 * Every tool's description and schema is read on every request, and the full set is about
 * nine thousand tokens of them - a third of a small model's window gone before the user
 * has said anything, and thirty-odd choices where a few-billion-parameter model picks
 * reliably among ten. The rest are for the hosted models, which have the room.
 */
export const LOCAL_TOOLS: ReadonlySet<string> = new Set([
  "list_molecules",
  "open_pdb",
  "colour_chain",
  "set_style",
  "style_residues",
  "set_molecule_options",
  "select",
  "clear_selection",
  "chain_sequence",
  "list_ligands",
  "focus",
  "set_view_mode",
  "remove_molecule",
  "undo",
]);

export function systemPrompt(
  docs: { name: string; title: string; summary: string }[],
  policy: string,
  /** The tools this model has, when that is not all of them. */
  available?: ReadonlySet<string>,
): string {
  const has = (name: string) => !available || available.has(name);
  // A line of the table is kept if the model has any tool it points to.
  const offered = (line: string) => {
    const named = [...line.matchAll(/-> ([a-z_]+)/g)].map((match) => match[1]);
    return named.length === 0 || named.some(has);
  };
  const docIndex = docs.map((doc) => `- \`${doc.name}\` - ${doc.title}: ${doc.summary}`).join("\n");
  const permission =
    policy === "read-only"
      ? "You may read but not change anything; every tool that changes something will be refused. Say what you would have done."
      : policy === "ask"
        ? "Each change is put to the user for approval before it runs, so make them one at a time and say what each is for."
        : "You may make changes without asking.";
  return [
    "You are the assistant inside Geis, a protein-structure viewer. The user is looking at a molecule on screen. You change what they see by calling tools. You cannot change anything by describing it.",
    "",
    "# Rules",
    "1. To change or look up anything, call a tool. Never say something was done unless a tool result says so.",
    "2. One tool call per reply. Wait for its result, then call the next tool or answer.",
    "3. Never invent an id. Chain ids look like `A_1` (the letter `A` also works). Molecule and node ids come only from list_molecules or read_graph.",
    "4. Colours are always hex: red #ff0000, orange #ff8800, yellow #ffff00, green #00ff00, cyan #00ffff, blue #0000ff, purple #8000ff, magenta #ff00ff, pink #ff80c0, white #ffffff, grey #808080, black #000000.",
    "5. If you do not know which chain or molecule is meant, call list_molecules. If the request is still unclear, ask one short question.",
    "6. If a tool returns an error, read it - it says what to do instead - and try once more. Do not repeat the same call.",
    "7. When finished, answer in one or two sentences, in the language the user wrote in. Do not repeat tool output.",
    `8. ${permission}`,
    "9. You cannot read or write files, change settings, or run programs. If asked, say so.",
    "10. If you have web tools of your own (WebSearch, WebFetch, web_search), use them for anything outside this app: the paper a structure comes from, what a protein does, a figure. If you have none, say you cannot look things up. To show a picture you found, call show_image with the address of the image file itself.",
    "",
    "# Which tool",
    "- colour one chain -> colour_chain {chain, colour}",
    "- draw as cartoon / sticks / spacefill / ball-and-stick / backbone -> set_style {representations: [..]}",
    "- colour by a rule: element / chain / residue-kind / secondary / confidence / b-factor / rainbow / chainbow -> set_style {scheme}",
    "- one flat colour for everything -> set_style {colour}",
    "- show or hide hydrogens / waters / ligands -> set_molecule_options {hydrogens, waters, ligands}",
    "- load a structure by its four-character PDB id -> open_pdb {id}",
    "- select or highlight part of it -> select {level, where}",
    "- what is open, which chains exist -> list_molecules",
    "- a chain's sequence -> chain_sequence {chain}",
    "- sticks / a colour / hiding for some residues only (an active site, a loop) -> style_residues {chain, from, to, show, colour}",
    "- show a picture from the web (a paper's figure) in a pane -> show_image {url}",
    "- graph view / structure view -> set_view_mode {mode}",
    "- zoom to the selection, or frame everything -> focus {on}",
    "- background colour, outlines, ambient occlusion -> set_world; a light's brightness -> set_light",
    "- disulfides, metal sites, covalent links -> list_links; ligands, ions, cofactors -> list_ligands",
    "- how far apart two residues are -> measure_distance {from, to}",
    "- remove a molecule -> remove_molecule; open a file by path -> open_file {path}",
    "- open or close the node graph / sequence pane -> show_pane {pane, open}",
    "- a Blender render -> render; save the viewport as a PNG -> save_viewport_image {path}",
    "- take back the last change -> undo (and redo)",
    "- anything else, or you are unsure of the arguments -> read_doc first",
    ...(has("read_doc") ? [] : ["- anything none of your tools does: say so in one sentence. Do not pretend."]),
    "",
    "# Examples",
    "User: make chain A red",
    'Call: colour_chain {"chain": "A", "colour": "#ff0000"}',
    "Then: Chain A is now red.",
    "",
    "User: 스틱으로 보여줘",
    'Call: set_style {"representations": ["sticks"]}',
    "Then: 스틱 모델로 바꿨습니다.",
    "",
    "User: open 1ubq",
    'Call: open_pdb {"id": "1ubq"}',
    "Then: Ubiquitin (1UBQ) is open.",
    "",
    "User: select the helices of chain B",
    'Call: select {"level": "residue", "where": {"chains": ["B"], "secondary": ["H"]}}',
    "Then: The helices of chain B are selected.",
    "",
    "User: what is this?",
    "Call: list_molecules {}",
    "Then: (say what is open, from the result)",
    "",
    ...(has("read_doc") ? ["# Documents (read_doc)", docIndex] : []),
  ]
    .filter(offered)
    .join("\n");
}

export { text as toolText };
