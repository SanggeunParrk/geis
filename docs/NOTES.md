# Working notes

The long form of the README as it stood before the first release: what each part does and
why it was built that way. The README is now the short version; `ARCHITECTURE.md` is the
reference.

An end-to-end workbench for structure prediction: worlds to look at results in, and a
compute layer that hides where the work actually happened.

Working title. `Geis` is the crate name, the npm package name, and the window title -
rename all three and nothing else breaks.

## Running it

    npm install
    npm test                          # 98 frontend tests
    cd src-tauri && cargo test        # 289 tests, no GPU, network or Blender needed
    cd .. && npm run tauri dev

Requires Rust (rustup) and Node. Everything else - SQLite, the pty layer - is vendored.

Nine of those Rust tests read real entries and two of them start Blender; without the
files they pass by skipping. To run them for real, put `1ubq.cif`, `1ubq.pdb`, `4hhb.cif`
and `6vxx.cif` from files.rcsb.org in a directory and point `GEIS_TEST_STRUCTURES` at it.

**Always launch through `npm run tauri dev`, not `src-tauri/target/debug/geis`.** A debug
build loads the Vite dev server rather than the bundled files, so running the binary on
its own opens a blank window. The app now prints an explanation when that happens instead
of leaving you guessing. For a standalone app, `npm run tauri build`.

## What works today

The compute layer, end to end. Settings -> Compute is a real control plane:

- **Provider discovery.** One POSIX probe script runs identically here and over ssh, so
  local and remote capability detection cannot drift apart. It reports CPU, memory, GPUs
  (CUDA / ROCm / Metal, with Apple unified memory counted as VRAM because that is
  genuinely the budget a model gets), scratch space, and which of ~20 tools are on PATH.
- **Four provider kinds.** `local` (pty on this machine), `ssh` (another host),
  `slurm` (sbatch through a login node, with the job's output tailed back live while it
  runs), `http` (a hosted API declared in TOML).
- **A job queue** with per-provider parallelism limits, cancellation that actually kills
  the remote process, artifact collection against declared output patterns, and a log
  that survives closing the app.
- **The console.** Every job runs under a pty and the stream is captured. A working job
  never mentions CUDA; a broken one is debuggable without leaving the app.

## The node graph

Everything the app does is a graph, and the graph is the interface: where a structure comes
from, how it is drawn, how it is coloured and lit, and what is done with the result are all
nodes on one plane. Prediction is a source node like any other, so "predict, then look at
it" is a connection rather than two screens.

    [PDB 1UBQ]---.
                  >--[Cartoon]---.
    [Secondary]--'                >--[Viewport]
    [Key light]--------------------|
    [World: white]----------------'

- **Sockets are typed** and the types come from the core, which is also what enforces
  them: the editor refuses a link it already knows is illegal, and evaluation refuses the
  rest. Colour, structure, geometry, light, world and material each have their own colour.
- **Evaluation is pure.** `graph::eval` decides *what* to do - which structure, drawn how,
  lit how - and never does it; fetching, prediction and geometry all happen in the command
  layer with that as the plan. It is why a cycle, a missing input, and a prediction that
  has not finished are all covered by tests that need no network, GPU or Blender.
- **A cycle is reported, not recursed into.** An evaluator that follows links without
  checking is a stack overflow, which takes the app down instead of showing an error.
- **Nodes carry state**: ready, unused (dimmed - still there, just not contributing),
  incomplete with a reason, running with its job id, or failed with the message.
- **Predictions run through the job queue.** A prediction node submits once, records its
  job id, and the graph re-evaluates when the job changes state - so a finished prediction
  appears in the viewport on its own.
- **Right-click is the menu.** On the plane: `Add node` opening into one submenu per group
  (Source, Compute, Draw, Scene, Output), then the layout actions. Grouped rather than flat
  because the catalogue grows with every model and every kind of output, and a list long
  enough to scroll is a list nobody reads - the search field at the top of the Add panel is
  there for when you already know the name, and its results open in a panel of their own so
  typing never destroys the field being typed into. On a node: collapse, duplicate (which
  brings the incoming links along), disconnect, frame and delete.
- **Folding.** Every step being a node means a working graph has more nodes than fit on a
  screen, and most of them are set once and never touched. A node folds to its header from
  the triangle, `H`, or its menu; `Shift H` folds the lot. A folded node shows one dot a
  side rather than a stack of sockets nobody can aim at, so it has to be expanded to be
  rewired - which is also true in Blender. `Tab` folds the whole graph pane away for a
  full-window viewport. Folding is stored with the graph, like the position: it is part of
  how a pipeline reads.
- **Rendering is a node too.** A Render node takes exactly what the Viewport takes -
  geometry, lights, world, material - plus an engine, samples and a size, and puts the
  image it produced on itself. So a graph can hold a draft EEVEE render and a final Cycles
  one, each with its own lighting, and neither is "the" render. Only the Viewport is limited
  to one; two Render nodes is the point. The camera comes from the viewport, so a render is
  the view on screen at higher quality rather than a second guess at the same framing, and
  the plan comes from the same `graph::resolve` the viewport drew from - resolving it a
  second way is how a render starts to disagree with the preview it was set up in.
- **A render never crops what the viewport showed.** The viewport's field of view is
  vertical and a render is rarely the same shape as the window it was framed in; Blender's
  `camera.angle` goes to the larger render dimension by default, so a 40 degree vertical
  view became a 40 degree *horizontal* one at 1600x1200 - a vertical field of 30.6 degrees,
  which cut the top and bottom off every render. The rule now is to take whichever field of
  view contains both axes of what was on screen, and it lives in
  `RenderSettings::camera_fov_y` where it is tested as an invariant rather than against
  numbers typed in by hand. A render shaped differently therefore shows *more*, which makes
  the subject smaller - so the Render node offers one button to match the viewport's shape
  exactly, and says so when the two differ enough to matter.
- **The image is a node.** Rendering fills in an Image node wired to the Render node's
  output - created on the first render, reused after that - with the picture, Copy, and
  Save as. A result that lives in the graph can say which settings made it; an image in a
  side panel is overwritten by the next render and has nothing to say about either.
- **The graph is saved.** Everything the app does is in the graph - which molecule, how
  each chain is drawn, the lighting, the render settings, which nodes are folded - so until
  this existed, closing the window threw all of it away. Written to the settings table on
  every change, debounced, because a crash is exactly when losing an afternoon's pipeline
  hurts. `Reset graph` in the plane's menu goes back to the default pipeline.
- **A Style is defined as a graph, not a form.** The Style node is the analogue of
  Blender's Material Output: it decides the representation and what is visible, and takes
  its colour and its surface on sockets. A `Colouring` node is a rule (by chain, element,
  secondary structure, confidence); a `Colour` node is one flat colour; a `Material` node
  is the surface. So a style is built up from parts and a part can be shared between
  styles, which is the reason to define a material as a graph at all. The surface moved
  *into* the style for the same reason a Blender material's surface belongs to the
  material: "cartoon, orange, matte" is one description of one thing.
- **The helix path is a helix.** A spline through alpha carbons at 100 degrees a residue
  is 3.6 points per turn and traces a rounded triangle - the radius from the axis dips
  between the carbons, and the ribbon reads as angular. Neither PyMOL nor Mol\* fixes this
  by smoothing: PyMOL's `smooth_cycles` pass is `SmoothLoops` and breaks at any residue
  *with* secondary structure. Since the axis is already fitted for the orientation, the
  path inside a helix is generated from it - the centre slides along the axis while the
  radius rotates about it - which is round by construction. A test samples it and requires
  the radius constant to 0.05 A. At a kink, where two fitted axes disagree, it falls back
  to the spline.
- **The tangent comes from the spline**, evaluated either side of each sample and
  differenced, as Mol\*'s `interpolatePointsAndTangents` does. Blending the per-residue
  tangents linearly instead leaves the cross-section tilted against its own curve, which
  reads as a kink at every residue.
- **A helix ribbon is broad *along* the axis.** PyMOL's `cartoon_round_helices` sets its
  orientation vector to `cross(axis, tangent)`, which is radial - and that vector is the
  ribbon's **normal**, its thin direction. Used as the width direction, the band runs
  radially and the helix comes out as a screw thread: an auger 2.3 A deep and half an
  angstrom thick. The broad direction is the axis with the path taken out of it, so the
  band's flat face points outward and the turns line up. This was invisible in every
  whole-protein view and obvious the moment one ideal helix was rendered on its own, which
  is now a fixture: `ideal_helix_structure` builds one and a test measures the mesh's
  spread in radius, because that number is 0.5 A when the ribbon is right and 2.8 A when
  it is an auger.
- **The cartoon follows Mol\*,** which is what the Protein Viewer extension for VS Code
  ([`ArianJamasb.protein-viewer`](https://github.com/molstar/VSCoding-Sequence)) draws
  with. Reading `polymer/trace-iterator.ts` and `curve-segment.ts`: a strand's control
  point is `(p1 + 2*p2 + p3)/4` and a helix's position is left alone; the face is matched
  in direction **once per segment**, slerped along the curve, then three-point averaged
  over the *sampled* normals. The per-pair flip is the trap - a helix's face turns a
  hundred degrees a residue, so a rule that flips each frame to agree with the one before
  it inverts every other frame, and the ribbon shreds while the geometry underneath is
  perfectly smooth. That, not the spline, was what made the helices ugly.
- **Round helices, PyMOL's way.** A ribbon through raw alpha carbons comes out faceted,
  because the carbonyl that orients it turns about 100 degrees a residue. The fix is not to
  move the atoms: a four-point weighted average of consecutive alpha carbons, weights
  (0.2130, 0.2870, 0.2870, 0.2130), lands on the helix axis - at 100 degrees a residue the
  radial parts cancel, since `0.2130*(cos 0 + cos 300) + 0.2870*(cos 100 + cos 200) = 0` -
  and the ribbon's flat face is set perpendicular to both that axis and the path, so the
  band spirals with its face outward and consecutive turns line up. Reading
  `RepCartoonComputeRoundHelices` in PyMOL's source settles what `cartoon_round_helices`
  actually does: the positions there are `const`. An earlier attempt here smoothed the
  positions onto the axis instead, which throws the helix's 2.3 A radius away and turns a
  broad spiralling ribbon into a tight corkscrew.
- **Ball-and-stick is proportioned for a chain, not a ligand.** The balls were 2.7 times the
  stick radius, which reads fine on a bound drug and reads as beads threaded on wire across
  a whole polymer. Nearer 1.5 now, pinned by a test.
- **A viewport draws a scene, not a molecule.** Its Geometry socket takes several links;
  each molecule is built with its own styles and the results are concatenated. Two
  structures from separate files, or a complex and a ligand, go in one frame. One
  unresolved molecule holds up the whole frame rather than drawing half a scene silently.
- **Double-click frames everything.** The camera is easy to lose, and hunting for a button
  to get back is worse than the gesture every map and 3D tool already uses.
- **Style presets, and named colours.** A dozen presets - cartoon by secondary structure,
  spacefill CPK, backbone trace, cartoon in orange, glossy marine - because the bundle
  wanted is usually one of a handful, and assembling it from three controls every time is
  friction. The flat colours are Catppuccin Frappe (MIT) - pastel, so that several of them
  sit in one figure without fighting; see docs/LICENSING.md.
- **The properties panel is organised by what is in the scene** - the molecule and its
  chains, the lighting, interactions - not by whatever is selected in the graph. The graph
  says how things are connected; the panel says what the scene is made of, which is the
  question you ask more often. An unassigned chain row reads `assign` and clicking it
  creates the Style node and wires it, which is the only way to assign a chain's style
  without laying out nodes by hand.
- **Blender's arrangement.** A full-height strip down the right side - outliner above,
  properties below - with the viewport and the node graph stacked in the column to its
  left. The strip runs to the bottom of the window; the style panel is at the bottom of
  the properties half, which is where a Blender material lives.
- **The outliner is a tree.** Disclosure triangles, one row per thing, everything in the
  graph appearing exactly once: the scene with each molecule and its chains' styles, the
  lighting, the outputs, the scene settings. The flat list of sections it replaced showed
  some things twice and others not at all.
- **The viewport shows the molecule.** No legend, no atom count, no triangle count in the
  corner. What is selected appears there when something is, and nothing is there when
  nothing is; a problem appears when there is one.
- **The sequence space** runs across the top: one row per chain of one-letter codes,
  coloured by secondary structure with the same three colours the ribbon uses. A fold is
  read in 3D; a mutation, a domain boundary and a missing loop are read along the sequence.
  It comes over the wire as two strings per chain rather than a list of residue objects -
  a ribosome is a hundred thousand residues, and `[{"code":...},...]` for each is megabytes
  of JSON to say what two strings say.
- **Nodes are tinted by family**, as Blender does: source blue, style teal, colour pink,
  surface grape, light yellow, output orange. The colour comes from the core and matches
  the sockets that family produces, so it is not chosen twice.
- **Arrange lays groups out as units.** Each group is condensed to one node as tall as its
  stacked members, the condensed graph is laid out, and the members go in the column to its
  left - which is where its inputs come from. Without this the members scatter into
  whichever column their links put them in and the frame drawn around them swallows the
  plane.
- **Everything but carbon keeps its element colour.** A chain painted one colour with its
  oxygens red and its nitrogens blue reads as chemistry; the same chain painted uniformly
  reads as a shape. So every scheme - chain, secondary structure, confidence, a flat colour
  - decides carbon, and the heteroatoms are themselves. Carbon is most of the atoms, so the
  chain's identity survives. Hydrogen follows the scheme too, being easier to read that way
  and usually hidden. One checkbox turns the exception off for a genuinely flat colour.
- **A render has one input: the view.** A view already carries the geometry, the lighting
  and the world, so copies of those on the Render node could only ever disagree with the
  picture on screen - and with no camera object, the disagreement would not even be
  meaningful. A render is the viewport at higher quality, and its own fields say how much.
- **The render follows the viewport.** There is no camera object here - the viewport *is*
  the camera - so a Render node takes the viewport's `View` and draws exactly what is on
  screen, which is both the short way to say "render this" and the only way to be sure the
  framing matches. Wired that way out of the box; unwire it and give the render its own
  geometry, lights and world when the figure differs on purpose.
- **Styles live in the molecule's slots, and a chain names one.** Exactly Blender's
  material slots: the object owns the materials, a face carries an index, and one material
  used by half the mesh is one thing to edit. So a molecule holds a named list of styles
  and each chain says which slot it draws with - restyling four chains at once is editing
  one slot, not four nodes. A wired `Style` socket still overrides the slot, which is the
  other half of a Blender slot: name an existing material, or drive it with a node
  network. The precedence is wired, then the slot, then a style the chain carried from
  before slots existed, then the default - an unstyled chain looking ordinary beats an
  unstyled chain being invisible.
- **The properties panel is Blender's, structurally.** Icon tabs down the panel's left
  edge (a region of its own, as `PROPERTIES_PT_navigation_bar` is), a breadcrumb naming
  what is being edited - `4HHB > Chain B > Cartoon` - with a pin that freezes the panel on
  it, and then a stack of collapsible panels with labels right-aligned in a 42% column,
  which is what `use_property_split` does. Which tabs exist follows what is active, so an
  empty tab is never shown; the tab you are on survives a change of selection, because
  selecting another chain while looking at Style should show that chain's style rather than
  throw you back.
- **The Style tab is the Material tab.** A slot list whose names are edited in place, a
  `+ - v ^ v` button column beside it, and underneath the library row that Blender's
  `template_ID` is: browse every registered style, duplicate the slot, or reset it. Then
  Assign, which is how a slot reaches geometry. The slot menu carries the rest - register
  this style in the library, assign it to every chain, remove unused slots - which is
  where the shelf of buttons that used to hang under the panel went.
- **A style's fields are written once.** `styleGroups` builds Draw, Colour and Surface, and
  both the Style node's face and the panel's own panels are built from it: Blender does the
  same thing through `template_node_view`, where the Surface panel *is* the node drawn in a
  panel. Two implementations of "the settings of a style" would drift, and the one that
  drifted would be the one you were not looking at.
- **A molecule is a group, not a node.** There is no molecule box: the frame *is* the
  molecule, carrying its name, its fold and its drag, with its chains and their styles
  inside it. The molecule feeding its own chains is internal to the frame, so that link is
  not drawn - the frame already says they belong together.
- **Columns are as wide as what is in them.** Every column used to be one node wide, so a
  group holding its own two-column layout overlapped whatever came next - the chains and
  the viewport ended up side by side, which is the opposite of what ranking them was for.
  Column offsets now accumulate from the widest node in each column, and a group reports
  the width of its inner layout.
- **A dev launch seeds a molecule.** An empty viewport is the wrong place to work on how
  molecules are drawn, so a development build with no molecule in the graph fetches 4HHB
  once - a tetramer of two different chains, so the style slots have something to be
  about. Gated on `import.meta.env.DEV`, skipped as soon as the graph has a molecule of
  its own, and never in a shipped build: opening somebody else's haemoglobin uninvited is
  not a default.
- **Fetching builds one slot, not a style per chain.** A fetched molecule arrives with a
  single `Cartoon` slot that every chain names, the way a Blender object arrives with one
  material slot rather than one per face. Chains used to each carry a copy, which meant
  restyling a tetramer was four identical edits.
- **The left of the screen is a desk of floating panes; the right column is fixed.** The
  reason for panes is what comes next - PAE plots, contact maps, logs beside the structure
  they are about - and the model is a browser's windows, not Blender's areas: each pane has
  a title bar you drag to move it and edges and corners you drag to resize it, panes
  overlap, and the one touched last is on top. The title's dropdown picks what a pane
  shows, `×` closes it, `+ Pane` or a right-click on the empty desk opens another where
  you clicked, Tab still folds the graph away; the arrangement is saved and comes back.
  Panes may hang off the sides and bottom of the desk, a grip's worth always left to pull
  them back by; they cannot be dragged off the top, since the title bar is that grip.
  And the desktop's snapping: drag a title to the top edge to fill the desk, to a side to
  take that half, to a corner for that quarter, with a ghost showing where it will land;
  corners are regions along both edges rather than points, so reaching the left edge in
  its top fifth is the top-left quadrant, as it is in Chrome. The title bar's buttons are
  `⊞` (a menu of the halves, quarters, fill and restore), `–` (fold to the title bar), `▢`
  (fill, or come back) and `×`; a double-click on the title also fills; dragging a filled
  pane un-fills it under the pointer - but only once the pointer has actually moved, since a
  click on a filled pane's title once shrank it. Borders are bright with a dark rim outside
  so overlapping panes stay distinct; the front pane's border takes the accent.
- **Panes hold tabs, as a browser window does.** Each pane's title bar is a tab strip: `+`
  opens another tab (viewport, sequence, node graph, image, text), a click shows one, `×`
  closes one and closing the last closes the pane, a drag along the strip onto another pane
  moves the tab there, a drag off the strip onto the desk makes a pane of it, and a
  right-click changes what it shows or detaches it. The singleton rule holds across tabs:
  asking for the graph in a tab pulls it out of wherever it was, and asking for one that is
  already in this pane just shows it. An earlier version left an Image tab where the
  singleton had been, so asking for the Agents panel while it was open looked like having
  pressed Image. Old saved layouts, with one
  editor per pane, are read as one tab per pane. The drag looks like Chrome's: a ghost of
  the tab follows the pointer, a slot opens in whichever strip it is over at the place it
  would take, a drop on a pane's body joins that pane, and a pane with one tab *is* its tab
  - dragging it drags the pane, dropping it on another merges them. The desk's menus close
  on a press anywhere else or Escape, since the menu module leaves closing to its host.
- **Snapping fills the free part of a tiled desk.** Two quarters across the top and a third
  pane dragged below them used to need the bottom edge found exactly; now, when other panes
  already sit in halves or quarters, dropping over the part still free takes the largest
  free tile under the pointer - Chrome's rule. Top and bottom halves joined the snap set.
- **A double-click on the title fills the free space first.** Not the desk: the pane grows
  from where it is until each side meets the desk's edge or the nearest edge of a pane that
  shares some of the other axis, so nothing else is covered or moved. A second double-click
  fills the desk; a third comes back to where it started. `freeSpaceAround` is the
  geometry, `cycleFit` the three-state gesture; the `▢` button still fills the desk in one.
- **The Agents panel is a model that can read this app and change it.** Three connections -
  a model on this machine (any OpenAI-compatible server: Ollama, llama.cpp, LM Studio),
  Claude, and ChatGPT - chosen at the top of the panel, with the endpoint, the model name
  and the key behind a Settings card and a Test button that asks the connection what models
  it has. The key lives in the OS keychain and is attached by the core at call time; it
  never reaches the webview. The three wire formats differ in every detail - Anthropic wants
  tools with an `input_schema` and a result as a `tool_result` block inside a *user* turn,
  OpenAI wants a `function` with its arguments as a string and a `tool` role - so the core
  adapts one neutral request and reply to each, and everything above the adapters is written
  once. The loop copies the history it is given, because the panel keeps what it draws: for
  one turn each tool exchange went out twice, which wastes the context and, on Anthropic,
  repeats a `tool_use` id the model has already answered.
- **Local models are downloaded from inside the app, not installed beside it.** Asking
  someone to install Ollama and pull a model before the panel does anything is asking them
  to leave; but shipping the weights is worse, because a structure viewer that downloads as
  five gigabytes is one nobody downloads. So the app carries the *list* and a button. The
  catalogue is short and curated - the choice that matters is not which of the thousands but
  which of these three will run on this machine - and each entry names its true size, its
  licence, how much memory it wants, and how well it calls a tool, because a model that
  calls tools badly is worse than none: it fails by doing the wrong thing rather than by
  saying it cannot. The download reports as it goes, can be stopped and picked up again from
  where it left off, and lands as one whole file: bytes accumulate in a `.part` that is
  renamed at the end, so a file that exists is a complete one. A server that ignores the
  range request and sends everything starts the part again rather than appending to it,
  which would have produced a corrupt file that looked finished.
- **And the engine that runs them is fetched too, so nothing outside the app is needed.**
  Weights are data; llama.cpp's server is what turns them into answers, and it speaks the
  same OpenAI-compatible API the Local connection already talks to - so once it is up,
  nothing above knows the difference between it and Ollama. It is fetched rather than
  shipped for two reasons: it is another project's executable, and putting one of those
  inside a signed and notarised app makes that signature ours to defend; and a build per
  platform would have to come from our own build machines, which we do not have for every
  platform. So an exact llama.cpp build is pinned, and what arrives is checked against the
  SHA-256 GitHub publishes for that file - a mismatch is deleted rather than kept, so a
  retry cannot resume onto a bad one. Pressing Use on a downloaded model starts the server
  on a free port with `--jinja`, which is the flag without which it ignores the model's
  chat template and cannot emit a tool call at all, waits for it to load, and repoints the
  Local connection at it. It is killed when the window closes, because a child process
  holding six gigabytes outlives its parent unless something says otherwise - and because
  of that, the connection stores the *model*, never the port. A port is a fact about a live
  process: the first version wrote it into the settings, so after a restart the panel went
  on asking an address nothing was listening on and could say only that it could not reach
  it. Now the address is worked out when it is used, and a question asked after a restart
  starts the engine again on its way through. Settings written by that first version are
  migrated: a dead `127.0.0.1` endpoint becomes the model it was serving. The id of the
  running server is written beside its log, so a run that was force-quit - which never gets
  to stop anything - is cleaned up by the next one rather than leaving several gigabytes
  resident with nothing to talk to it.
- **The screen is two fixed columns with the desk between them.** The outliner and the
  properties moved to the left; the conversation is the right column. Neither is on the desk,
  for the same reason: a panel you can lose behind a viewport is a panel you stop using.
  Each has a divider, the conversation folds to a strip when the picture matters more, and
  all of it is remembered. The strip was five pixels wide at first, because an element with
  `display: none` is not placed in the grid at all, so the column after the hidden divider
  moved up into its track. A saved layout that still names an `agents` pane drops that tab
  rather than failing to load, and a pane left with no tabs goes with it.
- **The app opens on a shelf of worlds, not on an empty viewer.** A world is what PyMOL
  calls a session - the molecules that are open, the graph that draws them, the styles, and
  where the camera was left - under a name you chose, and Worlds is now the way in; the
  menu's Viewer entry is gone, because a viewer with nothing in it was never the thing
  anyone wanted. The shelf is laid out the way a game lays out characters: one row, one
  selected, the selected one large and *alive*. The preview is the world actually drawn -
  the same evaluate-and-build the viewer does, from the camera that world was left at,
  turning slowly - because a saved session is not a document with a name, it is a picture
  you recognise, and a still picture says less than a turning structure. One WebGL context
  serves the whole shelf, since a browser will not hand out one per card. The name on the
  big card is the field that renames it; entering opens the viewer on that world; leaving
  writes back where the camera was left.
  A session saved before worlds existed becomes the first world rather than being left in a
  settings row nothing reads.
- **The shelf is one viewport with the worlds standing in a row.** Not the working
  viewport - a made-up one, for showing worlds rather than working in them - with every
  world built into it at its own place along a line, sized alike as a character select
  frames every character the same. An arrow, a scroll or a dot moves the camera and the
  lights along that row; nothing is loaded on the way, which is what makes it instant, and
  the neighbours at the edges are the worlds themselves rather than pictures of them. The
  lamps are made to follow the view for the shelf, so the light travels with the camera: a
  world lit from behind as you arrive at it is a world you cannot see. The arrows are round,
  large and at the edges of the picture, because they are how you move and a control you
  have to hunt for is a control that is not there. At most four molecules of each world are
  built - a row of worlds is a picture to recognise rather than a scene to study. The
  background is left out entirely, so the structures stand on the page rather than in a box
  that would make them look like photographs of the app. That needed one more change than it
  looked like: the canvas had always declared its pixels *not* premultiplied, which WebKit
  composites as if they were, so every pixel left transparent came out black. The shader
  multiplies the colour by the alpha itself now and the canvas says so, which changes
  nothing for an opaque frame and is the difference between a black box and the page showing
  through.
- **The world turns; the camera only travels.** Spinning the camera around the chosen world
  turned its neighbours too, which reads as the whole shelf lurching rather than one world
  being alive. So the turn is geometry, not view: the vertex shaders rotate the points
  within reach of the selected world's centre and leave the rest where they stand. It is a
  uniform, so nothing is rebuilt when the selection moves, and the row behaves like a
  character select - the one you are on is alive, the ones beside it wait their turn.
- **The one in the middle is big and its neighbours are small.** A row of equals is a list;
  a row with a front is a choice, which is what a character select understands. The size is
  a uniform in the same shaders that turn the chosen world - continuous in where the camera
  actually is, so a world grows on the way in rather than popping when it lands - and so it
  costs nothing and nothing is rebuilt when the choice moves.
- **Leaving a world does not rebuild the shelf.** Coming back used to cost a graph
  evaluation and a geometry build for every world, about half a second of stillness, to
  arrive at the same row. Each card now carries a fingerprint of its graph, and the row is
  kept when nothing it is made of has changed. Two things had been quietly changing it: the
  shelf was sorted by which world was used last, so leaving one moved it to the front and
  shuffled its neighbours - it is ordered by when worlds were made now, because a shelf you
  learn the shape of is worth more than a shelf that guesses - and a picture of the world
  was captured on the way out for a card that no longer shows one, which was a redraw, a
  read-back and a PNG of a retina canvas spent on nothing.
- **A preview is built cheap, and building it does not stop the window.** The row used to
  be seventeen megabytes of ribbon: four structures at full detail, handed across and drawn
  every frame, for pictures an inch across where nobody counts the strands. It asks for a
  preview ribbon now - six subdivisions to eight, a twelve-point profile to twenty - which
  is half the geometry. It was a sixth at first, and that was a mistake worth keeping a note
  of: a ribbon that coarse is visibly flat-sided, and a flat-sided ribbon turning slowly
  changes face in steps you can see. A preview may be cheap but it still has to turn
  smoothly, and smooth is a property of the geometry, not only of the clock. The other
  half was that building it ran on the main thread, because that is where a synchronous
  Tauri command runs, so the window sat frozen for half a second while it worked. Both
  together: the first press of Worlds went from a 507 ms stall to none at all, and entering
  a world stopped stalling too. A render still gets every subdivision - that is what a
  render is for.
- **A turn is updated and drawn in the same display callback.** The viewer owns the
  animation callback, applies the current pose, and draws it immediately. Scheduling a
  second animation frame from the shelf rendered the previous pose a display interval
  later. Rotation uses the elapsed animation timestamp, with an exact integral for its
  acceleration and deceleration, rather than averaging frame durations and changing the
  animation's clock. Long gaps are capped when the app resumes. The final fullscreen
  composite does not use default-framebuffer MSAA: it cannot smooth the geometry already
  drawn into textures, but adds a resolve before presentation. The existing offscreen
  antialiasing pass is retained.
- **The slide is a spring, and each world keeps its own turn.** Two things made moving
  along the row feel wrong rather than slow. The easing was a fraction of the remaining
  distance each frame, which leaves at once at its fastest and then crawls for most of a
  second, and pressing an arrow mid-slide restarted it from a standstill; it is a critically
  damped spring now - half the journey in 100 ms, settled by 450, no overshoot, and a press
  while it is still moving only changes where it is heading. And there was one turn angle
  shared by whichever world was chosen, so every press made the world arriving jump to
  however far the last one had got while the world leaving snapped upright. Each world has
  its own angle now: the one you leave stops where it was, the one you arrive at carries on
  from where it stopped, and both ease in and out of turning rather than being switched on.
- **The shelf refuses the mouse and only the Enter button goes in.** Dragging a world about
  suggested you were working in it, and you were not, so the made-up viewport ignores drags,
  double-clicks and pinches; the wheel is its own, for moving along the row. Clicking a
  world does not open it either - a screen you fall through by touching it is a screen you
  cannot browse. Leaving the shelf stops its frame loop, so coming back rebuilds the row
  rather than returning to a frozen picture of whatever was there before.
- **The agent is watched at work, not waited for.** A turn used to be a silence and then a
  wall of text. Claude Code and Codex both show the work instead - the reasoning as it is
  reasoned, the answer as it is written, each tool call the moment it is decided - and that
  is most of what makes an agent feel like something you are working with. Both APIs stream
  server-sent events and neither does it the same way: OpenAI sends one JSON object per
  `data:` line with the new characters in a `delta`, Anthropic sends typed events that open,
  fill and close each block. `agents/stream.rs` turns either into the same three pieces -
  text, thinking, a call being made - which travel down a Tauri channel to the one panel that
  asked, rather than as a broadcast that would reach windows that did not. The panel opens
  the thought while it is being had and folds it away when the answer starts; a `<think>`
  tag cut in half by the stream is held back until the rest of it arrives. A command-line
  agent, which has its own loop and cannot stream, hands over its answer in one piece at the
  end, so the caller sees one delta rather than none.
- **A second window cannot trust a command's reply, so the settings watch the world
  instead.** Closing the Agents settings used to close the whole app: the window-event
  handler ran for every window and asked for a kind of state that is not managed, and the
  panic took everything with it. Worse, pressing Use did nothing visible - the engine had
  started and the settings had been written, but the promise in that window never settled,
  because a reply to a second window is lost whenever anything else talks to the core while
  the command is running. So the long operations - install the engine, download a model,
  start one - are asked for and then *watched*: the window polls until the world shows the
  result, and says how long it has been waiting. Events are broadcast a beat after the
  command returns rather than inside it, for the same reason.
- **The settings are a window, not a card in the conversation.** Choosing an endpoint and
  downloading five gigabytes are not things anyone does *while* talking to an agent, and as
  a card they pushed the conversation off the screen while they did it. Pressing Settings
  opens a second window - the same bundle, told by its query string to render one panel and
  nothing else - which can sit beside the main one while a download runs. Its progress bar
  says how far in bytes and per cent, and hides itself when nothing is happening: it did not
  at first, because `display: grid` on the element beat the browser's own
  `[hidden] { display: none }`, so an idle bar sat at zero looking stuck.
- **The models on offer are the current generation, and they reason.** Qwen3.8 at 9B and 4B,
  both Apache-2.0 and both reasoning models, with the 27B for a machine that can hold it -
  and a two-generation-old 0.5B kept only so the machinery can be checked on a small
  download. A reasoning model's thinking is read out of `reasoning_content` or out of the
  `<think>` block llama.cpp leaves inline, kept apart from the answer, and shown folded
  under the turn: it is the most useful thing on screen when an agent does the wrong thing,
  and it is not the answer, so showing it as one would be wrong. It is not sent back to the
  model either, which the providers reject.
- **What an agent may do is a list, and that list is the whole answer.** `agent/tools.ts`
  holds every tool with the description the model reads, a JSON Schema for its arguments,
  and whether it changes anything: reading the docs, the molecules, the graph, a sequence
  and the selection; opening a PDB entry, selecting by condition, restyling a slot, adding
  one, assigning a chain to it, switching hydrogens, waters and ligands, and reframing the
  view, and `colour_chain`, which does the whole slot dance for "make chain A magenta"
  because that is one thought and a model made to work it out from slots usually gets it
  wrong - a small one answered it by trying to fetch a PDB entry called `A_1`. The errors
  push back the same way: a bad entry id now says that `A` or `A_1` is a chain of a molecule
  already open, and which tool to use instead.
  There is nothing in the list that touches Geis's own files, code or settings, and nothing
  that runs anything on the machine - that is an absence rather than a permission, and the
  agent is told so. The changes it makes go through the same graph, save and re-evaluate
  path a click does, so there is one way for the scene to change. The user's policy decides
  the rest: change freely (the default), ask before each change, or read only.
- **The documents the agent reads are written for it and shipped in the binary.**
  `docs/agents/` - the overview, the node graph, selection, and how to work - embedded with
  `include_str!` so they cannot drift from the build that ships them. Only the index, a line
  each, goes in the system prompt; the bodies are fetched by `read_doc` when the agent
  decides it needs one, because spending the context on four documents every turn is worse
  than spending a call on the one that matters.
- **Edges are magnetic.** Dragging or resizing a pane to within a few pixels of another
  pane's edge, or the desk's, pulls it exactly onto that edge, so two panes meet with no
  sliver between them and no overlap - the alignment every window manager does and the
  eye expects. A move shifts the whole pane by the closer of its two sides; a resize moves
  only the dragged side. Pure, in `magnetise`, with the reach given in pixels by the screen.
  Two refinements from how window managers actually do it. KWin's: another pane's edge
  attracts only when the two share some of the other axis, so a pane far below cannot pull
  one sideways. xfwm4's *edge resistance*: the desk's left, right and bottom edges catch a
  pane pushed into them and hold it there, and only a push a few dozen pixels further lets
  it hang off - so the boundary is felt however fast the pane arrives, where a pull-zone of
  a few pixels was simply skipped over. The pointer reaching the edge still tiles, as Aero
  Snap does; the three coexist in the order a desktop has them: resist, then tile.
  The rule is about the pane, not the grip: a one-tab pane dragged by its tab is the same
  move as one dragged by its title bar, and both go through one `dragPaneTo` - the first
  version applied the magnet only to the title bar, which is the kind of gap two code
  paths for one gesture always open.
- **The sequence strip selects as you drag.** The range is reported every time its far end
  changes, not on release, so the outline in the viewport grows under the pointer; the
  listener numbers its requests and applies only the latest mask, since a slow one from
  mid-drag must not overwrite the final range.
- **The selection is one thing: a BioMol view.** BioMol (`vendor/BioMol`, MIT, one of
  whose authors is this project's) keeps a structure as atom, residue and chain containers
  plus an `IndexTable` - forward parent arrays and CSR reverse maps - and a selection as a
  `View`: a level and indices, convertible to any other level through the table. Geis now
  has the same in Rust (`structure/index.rs`): `IndexTable::from_structure`, `View`, and a
  `Spec` of ANDed conditions named after BioMol's CIF features (`chains`, `residue_numbers`,
  `chem_comp_id`-style `residue_names`, `hetero`, `atom_names`, `elements`) that `evaluate`
  turns into a view. Every way of selecting - a click in the viewport at the level bar's
  level, a row in the BioMol tree, a drag on the strip, a chain or molecule in the outliner
  - produces a view and ends in one `applySelection`; every way of showing one - the
  outline (`view_mask`), the card (`describe_view`), the tree, the strip, the graph's
  selected node - is set there. Before this each path built its own mask and they disagreed
  in small ways. Shift adds a range, cmd/ctrl toggles, a modifier-click on the background
  keeps what is selected.
- **The BioMol panel is a layered map.** Two versions were rejected on sight - a tree with
  the sequence strip beneath ("a list is not a space") and a radial map of rings - and a
  third, hierarchical edge bundling, on description ("the edges are not the point; the
  hierarchy is"). What stands is an *icicle partition with an arc diagram*: the molecule as
  a bar, its chains as cells spanning the bar, their residues as cells spanning each chain,
  and an opened residue's atoms as cells spanning it, so a parent literally spans its
  children and belonging is read at a glance; order runs left to right, so the residue
  layer is the sequence, in the idiom of a genome browser. Links that are not the backbone
  hang beneath as arcs - low between neighbours, high across the molecule, coloured by kind;
  an open residue's bonds are small arcs between its atoms. Waters are left out, a
  one-residue ligand is widened to a floor so it can be seen and clicked. Zooming changes
  what is shown, not just how big - cells become letters, letters get a ruler of numbers -
  and a fit never lets the layers drop below a readable height; a long molecule is panned,
  as a sequence is scrolled. Every change of state is interpolated (`molmap/tween.ts`): a
  residue opens by widening and pushing its neighbours along, the camera glides into a
  chain, a selection pulses once, a hover glows in. It selects and shows through the same
  `View` as everything else - cell = chain, residue or atom; drag along the residue layer =
  range; shift adds, cmd/ctrl toggles - and hovering here outlines in the viewport while
  hovering there glows here. Canvas, not elements, because a structure has thousands of
  residues and the transitions have to hold sixty frames a second; the geometry
  (`molmap/layout.ts`) is pure and tested. The layout is deterministic, so the picture is
  the same every time and components can be drawn on it as brackets. A load that fails is
  not remembered as drawn and says so on the canvas: the panel spent a session blank and
  silent because one `structure_map` call had failed and the empty result was cached for as
  long as the same molecules stayed open.
- **Edges are magnetic.** Dragging or resizing a pane to within a few pixels of another
  pane's edge, or the desk's, pulls it exactly onto that edge, so two panes meet with no
  sliver between them and no overlap - the alignment every window manager does and the
  eye expects. A move shifts the whole pane by the closer of its two sides; a resize moves
  only the dragged side. Pure, in `magnetise`, with the reach given in pixels by the screen.
  Two refinements from how window managers actually do it. KWin's: another pane's edge
  attracts only when the two share some of the other axis, so a pane far below cannot pull
  one sideways. xfwm4's *edge resistance*: the desk's left, right and bottom edges catch a
  pane pushed into them and hold it there, and only a push a few dozen pixels further lets
  it hang off - so the boundary is felt however fast the pane arrives, where a pull-zone of
  a few pixels was simply skipped over. The pointer reaching the edge still tiles, as Aero
  Snap does; the three coexist in the order a desktop has them: resist, then tile.
  The rule is about the pane, not the grip: a one-tab pane dragged by its tab is the same
  move as one dragged by its title bar, and both go through one `dragPaneTo` - the first
  version applied the magnet only to the title bar, which is the kind of gap two code
  paths for one gesture always open.
- **The sequence strip selects as you drag.** The range is reported every time its far end
  changes, not on release, so the outline in the viewport grows under the pointer; the
  listener numbers its requests and applies only the latest mask, since a slow one from
  mid-drag must not overwrite the final range.
- **The selection is one thing: a BioMol view.** BioMol (`vendor/BioMol`, MIT, one of
  whose authors is this project's) keeps a structure as atom, residue and chain containers
  plus an `IndexTable` - forward parent arrays and CSR reverse maps - and a selection as a
  `View`: a level and indices, convertible to any other level through the table. Geis now
  has the same in Rust (`structure/index.rs`): `IndexTable::from_structure`, `View`, and a
  `Spec` of ANDed conditions named after BioMol's CIF features (`chains`, `residue_numbers`,
  `chem_comp_id`-style `residue_names`, `hetero`, `atom_names`, `elements`) that `evaluate`
  turns into a view. Every way of selecting - a click in the viewport at the level bar's
  level, a row in the BioMol tree, a drag on the strip, a chain or molecule in the outliner
  - produces a view and ends in one `applySelection`; every way of showing one - the
  outline (`view_mask`), the card (`describe_view`), the tree, the strip, the graph's
  selected node - is set there. Before this each path built its own mask and they disagreed
  in small ways. Shift adds a range, cmd/ctrl toggles, a modifier-click on the background
  keeps what is selected.
- **The BioMol panel is a map, not a list.** A first version was a tree with the sequence
  strip beneath, and it was rejected on sight: a list is not a space. The panel is now a
  2D canvas panned and zoomed like the node graph, with one *molecule map* per molecule: a
  centre disc for the molecule, a ring of arcs for its chains (angle in proportion to
  residues, a floor so a one-residue ligand is still there, waters left out as the viewport
  leaves them out), a ring of ticks for the residues coloured as the ribbon colours them,
  and chords across the middle for the links that are not the backbone - disulfides,
  covalent modifications, metals - which is where an interaction inferencer's results will
  go. Hierarchy runs outward along the radius and order runs round the angle, so the tree
  and the sequence are one picture; Circos and sunburst charts are the idiom, which no
  structure viewer uses and everyone already reads. Zooming changes what is shown, not just
  how big: ticks become letters, letters get a ruler of numbers. Every change of state is
  interpolated (`molmap/tween.ts`): the camera glides to a chain on double-click, a
  selection pulses once, a hover glows in. It selects and shows through the same `View` as
  everything else: arc = chain, tick = residue, drag along the ring = range, shift adds,
  cmd/ctrl toggles; hovering here outlines in the viewport and hovering there glows here.
  Canvas, not elements, because a structure has thousands of residues and the transitions
  have to hold sixty frames a second; the geometry (`molmap/layout.ts`) is pure and tested.
- **Edges are magnetic.** Dragging or resizing a pane to within a few pixels of another
  pane's edge, or the desk's, pulls it exactly onto that edge, so two panes meet with no
  sliver between them and no overlap - the alignment every window manager does and the
  eye expects. A move shifts the whole pane by the closer of its two sides; a resize moves
  only the dragged side. Pure, in `magnetise`, with the reach given in pixels by the screen.
  Two refinements from how window managers actually do it. KWin's: another pane's edge
  attracts only when the two share some of the other axis, so a pane far below cannot pull
  one sideways. xfwm4's *edge resistance*: the desk's left, right and bottom edges catch a
  pane pushed into them and hold it there, and only a push a few dozen pixels further lets
  it hang off - so the boundary is felt however fast the pane arrives, where a pull-zone of
  a few pixels was simply skipped over. The pointer reaching the edge still tiles, as Aero
  Snap does; the three coexist in the order a desktop has them: resist, then tile.
  The rule is about the pane, not the grip: a one-tab pane dragged by its tab is the same
  move as one dragged by its title bar, and both go through one `dragPaneTo` - the first
  version applied the magnet only to the title bar, which is the kind of gap two code
  paths for one gesture always open.
- **The sequence strip selects as you drag.** The range is reported every time its far end
  changes, not on release, so the outline in the viewport grows under the pointer; the
  listener numbers its requests and applies only the latest mask, since a slow one from
  mid-drag must not overwrite the final range.
- **The selection is one thing: a BioMol view.** BioMol (`vendor/BioMol`, MIT, one of
  whose authors is this project's) keeps a structure as atom, residue and chain containers
  plus an `IndexTable` - forward parent arrays and CSR reverse maps - and a selection as a
  `View`: a level and indices, convertible to any other level through the table. Geis now
  has the same in Rust (`structure/index.rs`): `IndexTable::from_structure`, `View`, and a
  `Spec` of ANDed conditions named after BioMol's CIF features (`chains`, `residue_numbers`,
  `chem_comp_id`-style `residue_names`, `hetero`, `atom_names`, `elements`) that `evaluate`
  turns into a view. Every way of selecting - a click in the viewport at the level bar's
  level, a row in the BioMol tree, a drag on the strip, a chain or molecule in the outliner
  - produces a view and ends in one `applySelection`; every way of showing one - the
  outline (`view_mask`), the card (`describe_view`), the tree, the strip, the graph's
  selected node - is set there. Before this each path built its own mask and they disagreed
  in small ways. Shift adds a range, cmd/ctrl toggles, a modifier-click on the background
  keeps what is selected.
- **The BioMol panel replaces the sequence panel.** A tree - molecule, chains (`A_1`, auth
  id, entity type, residue count), residues on opening a chain, atoms on opening a residue,
  each level fetched when opened since a structure has tens of thousands of atoms - with the
  sequence strip beneath it. Rows light up for the selection at their level; chains and
  residues that contain a finer selection are marked, so a residue picked in the viewport
  can be found in a folded tree; the strip paints the selected residues and scrolls its
  chain into view. This is where a selection will become a component and be given a style.
  Saved layouts naming the old `sequence` editor read as the BioMol panel. The outliner and properties are not a pane. They
  are a fixed column on the right with a draggable width, because they are the one place
  that always says what is selected, and a layout that could bury them is a layout you can
  get lost in. The viewport, the BioMol panel, the graph and the Agents panel are
  singletons - a WebGL context, the graph state, one conversation - so choosing one where
  it is not *moves* it, and the tab it left goes with it rather than leaving anything
  behind; images and text can be shown in as many panes as wanted. The desk lives in `panes.ts`,
  pure and tested; a move is a style change on the pane's element, and singleton editors
  are reparented only when they change panes, never rebuilt.
- **Sticks circumscribe their radius.** A stick is a prism, and a prism inscribed in its
  circle has flats inside the radius - 5% inside, at ten segments. The sphere at each joint
  is exact, so it showed through every flat as a sliver: a trace of a ball at every atom,
  plain once you zoomed in. The ring is now scaled by `1/cos(π/N)` so the flats sit *on*
  the circle, and has sixteen segments; the normals stay on the unit circle so the shading
  is still of a round stick. The Blender export's prisms do the same.
- **The corner card reads out what the pointer is over**, before any click: a pale
  `⌖ SER 44 · CB · D_1 (auth D)` line under the selection, or on its own when nothing is
  selected. The same pick the hover outline already made; the card just says it.
- **A drag across the sequence strip selects those residues.** The strip carries each
  residue's index in the structure, a drag along one row names a range, and the range is
  outlined in the viewport and written on the card - `K11 - A26 (16 res)`. Clicking the
  viewport background clears it, as it clears everything else.
- **The trace atom under a cartoon has a bond, not a sphere.** When sticks are drawn with
  a cartoon, the alpha carbon is kept so the side chain has somewhere to start - but a
  0.3 A sphere at the ribbon's axis pokes out of both faces of a ribbon thinner than that
  and reads as a ball sitting on the cartoon. The stick still starts at the CA; the sphere
  is gone.
- **A level bar under the viewport**, thin as VS Code's status bar: Chain, Residue, Atom.
  It decides what a click selects and outlines. Chain is what the graph has nodes for; at
  the other two the graph selection is still the chain, but the outline and the card's
  last line are the residue or the atom that was hit - `pick_target` already returned all
  three levels, so the bar only chooses which one is read. Hover follows the same level.
- **A bond is two flat halves, not a gradient.** The stick's colour was chosen per
  *vertex*, and a strip with vertices only at its two ends interpolates that choice across
  the whole bond. The halves are now `flat` varyings and the split is made per fragment; the
  Blender export builds two prisms per bond for the same reason, since Blender interpolates
  vertex colours across a face too.
- **Arrange wraps a tall rank.** Ten chains of one rank stacked in a single column is a
  tower taller than the screen; with four to a column they become three columns side by
  side, at the same depth, so producer-before-consumer still reads left to right. Members of
  a folded group are arranged too, so unfolding it later shows a tidy interior.
- **A bond carries its own radius.** The cylinder set had one radius for the whole scene,
  taken from the *unassigned* style - so a chain drawn as sticks beside a cartoon got 0.30 A
  spheres at its atoms on 0.22 A sticks: a ball at every joint, in the one representation
  whose point is not having balls. The radius now rides with each bond (seven floats per
  instance), in the viewport, in the Blender export and in the wire format. Verified two
  ways: the live scene's histogram is one value, 0.300, for all 4558 spheres and 4700
  cylinders; and a synthetic sphere and cylinder of equal radius measure 240 and 236 pixels
  across at the same depth. What still looks bulbous in an extreme close-up is perspective
  on the nearer atoms - Blender, from the same camera, shows the same thing.
- **Chains are label asyms, with the operator: `A_1`.** BioMol's `asym_id_oper_id`. The
  mmCIF `label_asym_id` is the archive's own identity for a chain, and it is what tells a
  haem or a run of waters apart from the polymer they were deposited beside - author chain
  A of 4HHB is asyms A (protein), E (haem) and K (water). The operator is `1` for the
  asymmetric unit and is the field that changes when assemblies arrive. The author id and
  the entity type (`polymer`, `non-polymer`, `water`, `branched`) travel with every chain,
  from `_struct_asym` and `_entity` when the file has them and inferred from the residues
  when it does not, so a PDB file describes its chains in the same words as its mmCIF twin.
  Consequences: 4HHB has ten chain nodes (four globins, four haems, two phosphates); water
  asyms get no node, since the molecule's own switch already decides whether they draw;
  and a graph saved before this change names chains `A`, not `A_1`, so the dev graph was
  reset rather than migrated.
- **The corner card is a line of facts.** `Chain B_1` on top, then `4HHB · auth B · polymer
  · 146 res`, and after a click in the viewport a third line: `ARG 40 · CZ (C)`. Nothing the
  title already said, nothing that needs a second look.
- **What the pointer is over is outlined before it is clicked**, in a paler shade of the
  selection colour, so a click is confirmed before it is made. The hover pick is throttled
  to one every 60 ms and only fires once the pointer has moved; the mask is rebuilt only
  when the pointer crosses onto a different chain and cached per chain after that. Both
  outlines share one mask channel at two levels - 255 for the selection, 128 for the hover -
  and the composite finds each border at its own threshold, selection drawn over hover.
  Verified by capture: 18,438 pixels take the hover colour while over a chain, zero after
  leaving it.
- **The viewport is clickable.** A click picks what is under it - a chain, for now, since
  that is the level the graph has nodes for - and a click on the background clears the
  selection. Verified in the app: clicks across 4HHB come back as chain B, chain C, chain
  A, a click on empty space clears the card and the graph selection, and an orbit drag
  leaves the selection alone. The same call already returns the residue and the atom, so
  picking at those levels is a matter of what the caller reads.
- **Picking is one extra draw into one pixel, on the click.** The geometry shaders already
  carry the id they would need to write, so a uniform switches their fragment output from
  shading to an encoded id; the click draws the scene into a 1x1 scissor and reads that
  pixel back. Two bits of tag and thirty of index, because a sphere reports an *atom* and
  a ribbon vertex reports a *residue* and the reader has to tell them apart. The
  alternative - an id channel in the G-buffer every frame - spends a render target on
  every frame to serve a gesture that happens a few times a minute.
- **Sticks are thicker.** 0.22 A half-thickness is a stick under half an angstrom across,
  which reads as a wireframe on a whole chain and is thin enough that its own antialiasing
  eats it at a distance; sticks are 0.30 now and the backbone 0.36. Ball-and-stick keeps
  its thinner stick, because there the ball sets the scale.
- **A panel redraw is only blocked by text entry.** The guard against pulling a widget out
  from under an edit was any focus inside the panel, so after clicking a tab or an outliner
  row the panel stopped updating: clearing the selection from the viewport then left the
  old selection on screen. Only INPUT, TEXTAREA and SELECT block it now.
- **The viewport says what is selected.** Picking a chain or a molecule in the outliner
  outlines it in the viewport, which is where you check that you clicked the thing you
  meant to - before this, the outliner highlighted a row and the picture was identical.
  Verified by capture: selecting chain B of 4HHB changes 25,038 pixels, all of them the
  highlight colour, and selecting the whole molecule changes 73,275.
- **The selection is a mask in the G-buffer, not a second pass.** Each geometry pass reads
  a 0/1 texture by atom (spheres, sticks) or by residue (the ribbon) and writes the answer
  into the normal target's spare alpha channel; the composite finds the border of that mask
  and draws the line. Textures rather than attributes because the selection changes on a
  click while the geometry does not - a mask is kilobytes, a spacefill scene is tens of
  megabytes - and one bit in an existing target rather than a new one because a third
  render target is a compatibility risk for a line.
- **Merged scenes now keep their ids apart.** `Scene::append` offsets the incoming atom and
  residue ids, and the header records where each part starts. This was a known gap and it
  is what a selection mask needs: two molecules both numbering their atoms from zero would
  have highlighted parts of the wrong one.
- **The properties panel follows the selection, within reason.** Blender keeps the tab you
  are on, and that is right *within one subject* - clicking another chain while looking at
  Style should show that chain's style. Across subjects it is not: clicking a chain while
  the Render tab was open left the render settings on screen, which reads as the click
  having been ignored. Tabs now belong to families (Structure/Chain/Style, Render/Output),
  and a selection outside the current family moves the tab.
- **The two right-hand panels have a divider.** The outliner and the properties were a
  fixed 0.9 / 1.4 split, so a long outliner and a long panel stack could not both be read.
- **An unnamed render is called `untitled.png`.** It used to be a UUID: unimpeachable as a
  key, useless as a filename, and what the Save-as dialog then offered. The counter appears
  only from the second one.
- **Bonds are read, not guessed.** A file names its chemical components and the Chemical
  Component Dictionary says exactly which atoms of each are bonded and with what order, so
  the bonds inside a residue are a lookup. The order is: the file's own `_chem_comp_bond`
  (which covers its ligands too), the built-in table for the standard residues, the polymer
  links between consecutive residues, `_struct_conn` for disulfides and covalent
  modifications and metal coordination, and distance *only* for a component nothing else
  knew - marked `Inferred`, because a guess should be able to say it is one. On 4HHB that
  leaves nothing to guess: 4704 bonds, over 4000 from templates, 570 peptide links, four
  haem coordinations, 0 inferred. This follows StructCooker, where bonds are edge features
  from the CCD and distance is used only to *validate* them - the same 0.5 to 1.3 times the
  covalent radii sum check is `bonds::implausible`, which reports rather than deletes.
- **A bond carries its order and its provenance.** Single, double, triple, aromatic, from
  the dictionary - things distance cannot tell you and a carbonyl needs. And where it came
  from, so "the file says" and "we guessed" are distinguishable.
- **Metal coordination is not a stick.** His NE2 to a haem iron is in the graph as a
  `Metal` bond and is not drawn as a cylinder, which is what PyMOL does too: a stick means
  a covalent bond. It is what an interaction view should draw as a dashed line.
- **There is a residue-level bond graph**, derived from the atom-level one: which residues
  are joined and how. The first of BioMol's views to arrive here - the peptide chain, a
  disulfide, a histidine holding a haem.
- **A cartoon over sticks drops the backbone sticks.** PyMOL calls this
  `cartoon_side_chain_helper` and turns it on in every preset that draws both
  (`preset.py`), for a good reason: the ribbon runs through where the backbone sticks are,
  so N, C and O end up inside it and only their outward halves show - a scatter of red and
  blue stubs poking out of the ribbon, which is exactly what "the bonds are broken" looks
  like. The backbone is now dropped from the sticks when the same style also draws a
  cartoon, keeping the residue's trace atom so each side chain still has something to grow
  out of. The trace atom rather than the name "CA", so the rule holds for nucleic acids
  too. Measured on 4HHB: 3.74M triangles before, 2.43M after - 1.3M triangles of buried
  backbone.
- **A new style is a cartoon.** `+` on the slot list and `+` in the library both start from
  the shipped Cartoon entry, by name rather than "the first built-in", instead of copying
  whatever happens to be selected - a new slot that silently inherits spacefill is a new
  slot you have to undo. Duplicate is the button for "a copy of this one".
- **Draw, Colour and Surface start folded**, and each opens on its own toggle: the tab's
  job is to say which style you are on, and three open panels of fields push the assign
  buttons off the bottom of a 272px column. The `fieldset` that makes a shipped style
  read-only wraps each panel's *rows* rather than the panels, because a disabled fieldset
  disables every control inside it - including the fold toggles, which made a shipped style
  impossible to even open and look at.
- **A sphere impostor is bounded by its silhouette, not by its centre.** Each atom is one
  camera-facing quad whose fragment shader intersects the view ray with the sphere, and the
  quad has to contain the sphere's silhouette. The silhouette is *not* centred on the
  projected centre: it is the circle where the tangent cone from the eye touches the
  sphere, which sits closer to the eye and, for an atom away from the middle of the screen,
  off to one side. A quad built in the image plane around the centre therefore clipped the
  left and right of every off-axis atom - worst where a molecule fills the frame, which is
  the ordinary way to look at one. The quad is now built in the silhouette's own plane,
  perpendicular to the eye-to-centre axis, at distance `|C| - r^2/|C|` with radius
  `r*sqrt(|C|^2 - r^2)/|C|`. Measured against the old shader on the same spacefill view:
  276 to 3159 pixels per frame change, with channel deltas up to 242 - slivers of sphere
  that were being cut away.
- **The lights follow the camera.** PyMOL specifies its lights in camera space, and the
  reason is that half of every orbit around a fixed rig is spent looking at the unlit side.
  `Light.follow_view` (on by default) reads the direction in the camera's own frame: the
  viewport skips the rotation it used to apply, and a Blender render rotates the direction
  into world space through the camera basis so the picture matches. Turn it off for a light
  that belongs to the scene, so two renders from different angles are lit alike. A fresh
  graph now ships a key *and* a fill, because one lamp leaves everything facing away from
  it on the ambient term alone.
- **The Style tab writes styles; it does not paint with them.** Editing a style there
  changes nothing on screen - it writes to the library. Assigning it to a chain or to a
  molecule is a separate button, and that is when the view changes. The alternative, which
  this replaced, was a panel where every stray drag on a slider repainted the scene, so
  there was nowhere to *compose* a style. A Style node in the graph still edits live, since
  a node is part of the pipeline by definition.
- **Draw, Colour and Surface are sub-panels of Styles.** They are what a style is made of,
  and a flat stack of four panels says they are four peers. Blender nests with
  `bl_parent_id`; this nests with the same look, drawn inside the parent's body.
- **A shipped style is read-only.** The values are visible and the controls are not usable -
  an HTML `fieldset[disabled]`, so it is the browser doing the disabling rather than a
  pointer-events trick - with a Duplicate button beside it. The core refuses to overwrite a
  built-in name anyway, so a panel that let you try would only produce an error.
- **Every control has a tooltip.** The instruction was no descriptions inside a node, and a
  tooltip is where the explanation goes instead: `hints.ts` keyed by the label a control
  already shows, so a field is explained by existing rather than by being registered twice.
  Verified in the running app - fields 20/20, panels 4/4, tabs 4/4, breadcrumbs 2/2,
  sockets 17/17, node headers 8/8, outliner rows 8/8, sequence residues 801/801.
- **A tab whose only panel is folded looks empty**, so the single-panel tabs start open.
  This cost two rounds of "the panel is missing its controls" before it was written down.
- **PyMOL's colouring commands, by their own definitions.** Read out of the installed
  PyMOL rather than from memory (`pymol/util.py`, `pymol/viewing.py`): `util.chainbow`
  runs `spectrum count, rainbow, byres=1` on each chain of each object, `util.cbc` walks
  `get_chains` handing out a colour cycle, and `spectrum` normalises the expression over
  its range and interpolates between palette anchors in RGB. So Geis now has
  **Rainbow (N to C)** - one sweep over the whole molecule - **Chainbow (per chain)** - the
  same sweep restarted for each chain - and **B-factor (spectrum)** - a ramp over the range
  the structure actually has, which is a different question from `Confidence`, where the
  same column is read as pLDDT and painted in AlphaFold's bands.
- **The rainbow is a hue sweep, not PyMOL's table.** PyMOL's `rainbow` palette is
  `blue cyan green yellow orange red`, and those six are pure hues - so the ramp is a sweep
  from hue 240 to hue 0, which lands on the same colours without copying anyone's colour
  values. It spaces them evenly, where PyMOL's equal-spaced anchors put orange at 0.8
  against a sweep's 0.875. Viridis and blue-white-red are the other two ramps, and the
  choice is a `Ramp` field on the style.
- **Chainbow and colour-by-chain are set on the molecule group.** They are only meaningful
  computed over a whole molecule: chainbow needs each chain to know where its own ends are,
  and a colour per chain needs the chain list to hand colours out from - a lone chain set
  to chainbow is just a rainbow. So the Structure tab has a Colour panel that writes the
  scheme into every slot the molecule's chains draw with, which is what running
  `util.chainbow` on an object does. `ColorScheme::is_molecule_wide` marks them, and the
  picker says so on the option itself.
- **A picked colour is converted to linear light.** Every float triple in the graph - a
  style's flat colour, a light's tint, the world background - is linear, because that is
  the space light is added in, and the core's named colours are converted with
  `color::to_linear`. The picker was dividing by 255 and stopping, so a colour chosen from
  a swatch was stored as its sRGB number in a linear field: mid grey went in at 0.50 where
  linear light wants 0.216, and everything picked came out pale. `hexToUnit`/`unitToHex`
  now carry the sRGB transfer function both ways, so the swatch `#40c057` stores
  `[0.051, 0.527, 0.095]` - exactly what the core makes of the same colour - and reads
  back as `#40c057`.
- **The colour picker is a fixed layer on the body.** The field sits inside a 208px node
  and a 272px panel, both of which clip their overflow, so a 268px popover positioned
  within one was simply cut away - the picker was there and could not be used.
- **Socket kinds have display names.** The row used to show the serialised tag, so an
  output read `geometry` in a place where a title belongs.
- **The breadcrumb replaced the panel's heading.** A heading reading `Chain - Chain A` says
  less than the path `4HHB > Chain B > Cartoon`, which is the only thing in the panel that
  says where the settings you are looking at actually live.
- **Arrange ranks a group's members among themselves**, on their own links: a style that
  feeds three chains goes in front of them, not stacked alongside as if the four were
  peers. The group's condensed size is measured from that inner layout, so whatever is
  placed after it clears the frame the group actually occupies.
- **Arrange lays members inside their group**, below the header.
  Beside it, they end up outside the frame that is meant to contain them. A folded
  member's links are re-pointed at the group first, or the group has no links in the
  condensed graph, ranks as a sink, and is placed to the *right* of the viewport it feeds.
- **Fit ignores what is folded away**, and it is the plane's double-click rather than a
  button - the same gesture the viewport uses.
- **A chain's molecule socket is not shown.** The link is how the graph knows which
  structure the chain reads, but it is internal to the group - the frame around them says
  it, and the row would be a socket answering a question nobody asked.
- **The style library can be added to**, not only edited: the slot menu registers the slot
  in front of you under a name, and the library row picks any registered style into a slot.
- **A chain has one style control.** Wired, the socket is the style and the slot picker is
  not shown; unwired, the slot is it. Connect or choose, never both.
- **A group folds.** Twenty chains are twenty nodes on the plane whether or not you are
  working on them, so the frame's label folds the whole group away and the links into it
  converge on the group itself rather than disappearing. Folding one node at a time is a
  different thing, and both exist.
- **Node colours are per family and distinct.** Style was the same teal as Geometry, so a
  Chain node and a Style node looked identical; Style is violet now. Source is blue, chain
  teal, colour pink, surface grape, light yellow, output orange.
- **A chain is a node.** Fetching a PDB entry builds the group: the molecule, a node for
  each of its chains, and a style for them to share, wired to every output. A chain is a
  thing you style, select and leave out, and a socket on somebody else's node is none of
  those. The molecule itself has no id field - a text box there could only change the
  molecule out from under its chains - so `Fetch from the PDB...` and `Open a file...` are
  the first two items in the plane's menu, and each brings the whole group with it.
  Opening the structure happens *during* the fetch, not on the next evaluation: the chains
  are only known once it is open and a chain node is what puts the molecule in the plan, so
  waiting would deadlock.
- **Representations are a set, not a choice.** A style draws cartoon *and* sticks if you
  tick both; the first one ticked decides the scene-wide settings. A viewer that makes you
  choose between the fold and the side chains is a viewer you fight, which is why PyMOL's
  `show` is additive. An atom drawn by two representations is counted once.
- **The Style node carries all of it**, the way a Principled BSDF carries its inputs:
  which library style, what to draw, and the surface - roughness, specular, metallic - as
  fields on the node. The Colour and Surface sockets override the fields beside them when
  something is wired in. Five presets ship, not eleven.
- **A style is a named thing, like a material.** The library holds eleven shipped styles
  and whatever the user saves, kept in the store between sessions. The bottom half of the
  right column manages them the way Blender's material properties do: the list, `use` to
  apply one to the selected style node, `x` to delete a user entry, `Save as` to put the
  current one back. A style is *copied* into a node rather than linked, so editing a node's
  style does not change the entry it came from - which is a real difference from Blender,
  and the reason `Save as` exists.
- **What atoms exist is the molecule's business, not the style's.** Hydrogens, waters and
  ligands moved from the Style node onto the Molecule group: an object's contents are not
  part of its material. The Style node now carries what it is and how it is drawn, and
  nothing else.
- **The right column is an outliner over properties**, as in Blender. The outliner is a
  tree of what the scene is made of - each molecule, its style slots by chain, the lighting,
  interactions - and the half below it is where the selected thing is edited. An unassigned
  chain row reads `assign`; clicking it creates the Style node and wires it.
- **A Style is the molecular material.** Representation, what is visible, and how it is
  coloured, as one assignable thing - the analogue of a Blender material. Separate Draw and
  Colour nodes made you wire two things to say one, and made "this chain as a cartoon, that
  one as sticks" impossible to express at all.
- **A molecule is a group, not a node.** A PDB entry is not one thing: it has chains, and a
  figure usually draws them differently. The chains are discovered when the structure opens
  and written back onto the node, which is what lets the group grow a Style socket per
  chain alongside the `All` socket. Connect one Style to `All` and the whole molecule uses
  it; connect another to `Chain A` and that chain overrides it. A chain with nothing
  connected falls back rather than vanishing. Collapsed, the group hides the chain sockets
  that are unwired - a display rule only, because hiding them from the *graph* would drop
  the assignments and silently change the picture.
- **Geometry is built chain by chain.** The scene builder resolves a style per chain, then
  computes one colour array per distinct colouring rather than per chain, so two chains
  sharing a style share the array and a whole-structure scheme like Chain still sees the
  whole structure. A bond needs both ends to want one, or a cartoon chain
  disulfide-bonded to a spacefill one sprouts a stick from a solid sphere.
- **A properties panel on the right**, as in Blender: the graph is the topology, the panel
  is the detail of the one thing selected, at a size you can read. It is the same controls
  the node body shows, built by the same code - two implementations of "the settings of a
  Style" would drift, and it would be the one you were not looking at.
- **One Molecule node, not a fetch node and a file node.** Which molecule is one question,
  and a graph in which `1ubq` and `1ubq.cif` are different node types makes the user answer
  it twice. Filling in both is not an error - the file wins - but the node says which one it
  is using rather than choosing silently, which is what the `warned` node state is for.
- **One light, not a three-point rig.** Three light nodes with seven controls each is
  twenty-one dials for lighting before anything has been looked at. The graph ships with
  one, the node carries four controls, and a second light is one node away. Deleting it
  falls back to a single light too, so removing it cannot brighten the scene. It is called
  Light rather than Key: "key" only means something next to a fill and a rim.
- **The light is aimed in the viewport, not by two angle fields.** Alt-drag moves the light
  selected in the graph: selection is where you say which thing you mean, the viewport is
  where you can see what moving it does. It is nudged along the camera's own right and up
  axes rather than rotated about them, so it follows the cursor whichever way the view is
  turned.
- **Nodes carry controls and state, no prose.** A node says what it is, what it is set to,
  and what is wrong with it. Explaining what a key light is belongs in this file.
- **Socket rows are part of the layout.** They used to be absolutely-positioned strips, so
  the names sat on top of the controls behind them. Outputs first, then inputs, with the
  dot hanging half outside the node's edge. Folded, the rows are gone and every link
  converges on one dot a side, level with the header.
- **Links are drawn from measured positions.** They used to be drawn from the same
  constants that laid out the socket rows, which sounds right and was wrong: the rows sit
  inside `.node-sockets`, which starts below the header, while the link maths measured from
  the node's top. Every link was drawn 32 pixels above its dot. Constants that have to
  agree with a stylesheet do not stay agreeing, so the dots are now read from the layout.
- **Numbers are dragged, Blender-style.** A value field is the whole row: drag across it to
  change it, hold shift for a tenth of the speed, click it to type an exact number, arrow
  keys to step. An `<input type="range">` cannot be typed into and has no fine mode.
- **Arrange** lays the graph out in columns. A node's column comes from the longest path
  from it *to the output*, counted backwards, so producers sit immediately left of what
  consumes them - ranking forwards from the sources instead exiles a light that feeds only
  the viewport to the far-left column, next to the structure sources. Within a column,
  nodes are ordered by the mean row of what feeds them. The arithmetic is in
  `src/ui/layout.ts`, apart from the DOM and tested on its own, cycles included.
- **Drawing reconciles rather than rebuilds.** Not an optimisation: a parameter change
  re-evaluates the graph, and re-evaluating used to replace the whole node layer, which
  destroyed the control the pointer was holding. No slider could be dragged at all. Bodies
  are now left alone while a widget is mid-drag or a field has the keyboard.

## The viewer

Written here, from scratch. There is no molecular library and no 3D engine in the
dependency list: the frontend's five runtime dependencies are the Tauri API and xterm
(which is the *terminal panel*, not the viewer), and the Rust side is SQLite, a pty, an
HTTP client and a keychain. The PDB and mmCIF readers, bond inference, the impostor
shaders, the cartoon spline, the tessellator and the 4x4 matrix maths are all in this
repository. Other viewers were read for reference - Mol* is the one worth studying - and
nothing was taken from them.

- **Readers for PDB and mmCIF**, written against what the archive actually contains:
  fixed-column PDB fields (so a residue number of -1 running into the insertion code
  column still parses), a real mmCIF tokeniser with `loop_`, quoting and semicolon text
  fields, first-model-only, alternate conformers collapsed, author numbering preferred
  over label numbering, and secondary structure from `HELIX`/`SHEET`,
  `_struct_conf` **and** `_struct_sheet_range`. The two readers are cross-checked against
  each other on real entries: same atom count, same coordinates, same inferred bonds.
- **Bond inference** on a uniform grid, so 200k atoms is linear rather than quadratic.
- **Representations**: cartoon, spacefill, ball-and-stick, sticks, backbone. Cartoon draws
  ligands as atoms alongside the ribbon, because a cartoon with the bound drug hidden
  answers the wrong question.
- **Geometry is built in Rust**, not in the renderer, and handed over as one binary
  payload. Two consumers need the same answer - the viewport and Blender - and a cartoon
  defined twice drifts. Atoms and bonds cross as instance lists (the viewer ray-casts
  impostors against them, Blender gets them tessellated); the ribbon crosses as triangles.
- **Colour schemes**: element (CPK), chain, residue type, secondary structure, and
  confidence - which uses AlphaFold's own pLDDT bands and colours, and switches to viridis
  when the values are true B-factors rather than confidences.
- **Palettes** from reusable sources: [Open Color](https://github.com/yeun/open-color)
  (MIT) for the swatch ladders and the vivid defaults, Okabe-Ito (Color Universal Design)
  as the colour-vision-safe default for chains, and viridis (matplotlib, CC0) for
  continuous values. CPK carbon and hydrogen darken on a light background, because the
  convention's pale carbon is invisible on white.
- **Colour is managed**: the core emits linear colour, both renderers shade in linear, and
  each encodes to sRGB at the end. Shading in sRGB is what makes a render come out darker
  and more saturated than the palette it was given.
- **Scene style is one definition** - background, lights, material - shared by the
  viewport and Blender, so the preview predicts the render instead of resembling it.
- **WebGL2, not WebGPU.** `navigator.gpu` does not exist in this webview, and WebKit only
  enables it behind a flag this macOS cannot set without private API. It costs nothing:
  sphere impostors need `gl_FragDepth` and instanced draws, and GLSL ES 3.00 has both. The
  renderer sits behind a small backend seam so WebGPU becomes an addition, not a rewrite.
- **The viewport is a small deferred renderer**: geometry into a G-buffer (colour plus
  view-space normals and a real depth texture), then screen-space ambient occlusion, then a
  composite that lays the structure on the background, optionally outlines it, softens the
  impostors' analytic edges, and encodes to sRGB. Occlusion is what reads as depth on a
  flat screen, and it needs the whole scene's depth - which is only available once geometry
  has been captured rather than shown.
- **A colour picker** with the palette ladders, hex and RGB entry, recent colours, an
  eyedropper that samples the viewport, and the system colour panel for anything else. The
  browser `EyeDropper` API does not exist in this webview - measured, not assumed - so
  sampling the viewport is what is available, and it is the case that matters.
- **Lights are nodes**, aimed by azimuth and elevation rather than by three raw components
  which cannot be denormalised by hand, and they drive the viewport and the render
  together.
- **Degrades rather than fails**: a software rasteriser caps the pixel ratio and drops
  large structures to cartoon with an explanation; no WebGL2 at all says so in words.

`geis model.cif` opens straight into it. `GEIS_SCREENSHOT=out.png` with optional
`GEIS_REPRESENTATION` and `GEIS_COLOR` renders a frame and needs nobody at the keyboard -
which is how the renderer is verified here, and how a batch of figures would be made.

Not built yet: a surface representation, a selection language, and arrowheads on beta
strands. The current list of what is missing or only half there is kept in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) under "Known gaps".

## Adding a model

Write a TOML file. There is no Rust to change, and nothing in the core knows what a
protein is.

    schema = 1
    id = "my-model"
    name = "My Model"
    task = "structure_prediction"   # or sequence_design, docking, embedding, msa, custom

    [requirements]
    gpu = "recommended"     # required | recommended | none
    vram_gb = 24
    runtime = "python"      # python | docker | binary | http
    weights = "auto"        # auto | manual | none
    needs_binaries = ["my-cli"]

    [inputs]
    sequence = { type = "fasta", required = true }
    steps = { type = "int", default = 3, min = 1, max = 10 }
    # fasta_header = "A|protein" on a fasta input titles a sequence pasted without one,
    # for an engine that parses the title (Boltz does).

    [outputs]
    structures = { type = "structure", pattern = "**/*.cif", primary = true }

    [[run]]
    command = "my-cli --in {{input.sequence.path}} --out {{job.workdir}} --steps {{input.steps}}"

Drop it in `models/` (bundled) or `~/Library/Application Support/com.sanggeunpark.geis/models`
(yours, and shadows a bundled model of the same id). `GEIS_MODELS_DIR` overrides both.

### The template contract

| placeholder | is |
|---|---|
| `{{job.workdir}}` | the job directory **on the host that runs it** |
| `{{input.x}}` | a scalar, or a staged file's path |
| `{{input.x.path}}` | a staged file's path |
| `{{input.x.raw}}` | the same, unquoted |
| `{{provider.weights_dir}}` | where weights live on that host |
| `{{env.NAME}}` | `"$NAME"`, expanded by the executing shell |

Three rules, each of which exists because the alternative silently produces wrong output:

1. **Values are already shell-quoted. Do not add quotes of your own.** Write
   `echo {{input.label}}`, not `echo "{{input.label}}"` - the second yields
   `"'my label'"` and the inner quotes become literal text.
2. **`{{env.NAME}}` is resolved by the executing shell, not this one.** A cluster job
   reading this machine's environment would substitute an empty string for a database
   path and fail thousands of log lines later.
3. **Declare every input you use, and use every input you declare.** A test enforces the
   second half: an input that no command reads is a control the user changes with no
   effect.

Manifests are staged as files and run with `bash -l <script>`, so the login shell's conda
init, module system, and PATH edits all apply - a command that works in the user's
terminal works here. A step's own `env` table is written into that script as `export`
lines, so it reaches the step on every kind of provider rather than only the local one.
A file input is staged to `inputs/<input name>/<file name>`: two inputs given files of
the same name stay two files.

## Offline rendering

The viewport is for interaction. Blender is for the picture you keep: Cycles gives real
ambient occlusion and soft shadows, which is what makes a contact or a cavity readable and
what a real-time renderer cannot do. "Render this view" takes the camera straight from the
viewport, so the render is the same view rather than a similar one.

- **Blender runs as a subprocess, never `import bpy`** - see Licensing.
- It receives the same geometry the viewport draws, tessellated, with per-vertex colours,
  so one material covers a cartoon, its ligands, and their bonds.
- `GEIS_SCREENSHOT` and `GEIS_BLENDER_RENDER`, with optional `GEIS_REPRESENTATION` and
  `GEIS_COLOR`, produce a viewport frame and a path-traced render with nobody at the
  keyboard - which is how both renderers are verified here and how a batch of figures
  would be made.
- `GEIS_BLENDER` pins a specific build; otherwise PATH and the usual install locations are
  searched.

## Licensing

See [docs/LICENSING.md](docs/LICENSING.md) for the survey behind these choices. The short
version, for a non-commercial project:

- **Our own viewer, not a PyMOL fork.** Open-source PyMOL is BSD-like and legally usable,
  and carries no patent clause - Schrodinger's patents are computational-chemistry
  methods, not graphics. The reason to write our own is architectural: PyMOL's global
  state, file-per-session model, and immediate-mode GL do not fit multi-world state plus a
  job pipeline. Mol* (MIT) is the reference worth studying.
- **Blender runs as a subprocess, never `import bpy`.** Blender treats bpy users as GPL
  derivatives - this is why Molecular Nodes went MIT to GPLv3. Subprocess keeps the app's
  licence free, and offline renders are what Cycles is for anyway.
- **No engine is privileged, and the app never bundles weights.** AlphaFold 3's parameters
  are gated, non-commercial, and non-redistributable, so its manifest declares
  `weights = "manual"` and the UI blocks it until a provider has a weights directory. The
  defaults are the permissive engines: Boltz-2 (MIT), Chai-1 and OpenFold3 (Apache-2.0).
- **AlphaFold Server is not wired in.** It has no public API, and driving its web UI would
  breach its terms. Self-host AF3, or use a service that publishes an API.

What was checked by hand, against what, and what could not be, is in
[docs/TESTING.md](docs/TESTING.md).

## Layout

    models/                 bundled model manifests
    src/                    frontend: game UI, IPC types, console
      ui/nodeEditor.ts      the node plane: pan, zoom, link, and node parameters
      viewer/               WebGL2 renderer: impostors, mesh, camera, payload decoding
    src-tauri/
      assets/probe.sh       host capability probe (runs locally and over ssh)
      src/models/           manifest schema, registry, templating
      src/structure/        PDB and mmCIF readers, bond inference, wire format
      src/geometry/         representations, cartoon splines, colour, tessellation
      src/graph/            the node graph and its evaluator
      src/render/           the Blender bridge
      assets/render_scene.py   lighting, camera and materials for Cycles
      src/compute/          probe, plan, queue, and the four providers
      src/store.rs          one SQLite database
      src/commands.rs       the IPC surface
      tests/                end-to-end job execution, shipped-manifest validation
    docs/
