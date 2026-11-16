# Architecture

Working title: `Geis` (rename freely - it is the crate name, package name, and window
title, nothing else depends on it).

## Shape

    Tauri 2 (Rust core)                        WebView (TypeScript)
    +---------------------------+              +--------------------------+
    | probe    hardware/runtime |   IPC        | game UI: main menu,      |
    | registry model manifests  | <----------> | world select, settings   |
    | queue    job scheduler    |   events     | compute panel + terminal |
    | provider local/ssh/slurm/ |              | WebGPU viewport (later)  |
    |          http adapters    |              +--------------------------+
    | store    SQLite           |
    +---------------------------+
              |                    \
      Blender headless              remote hosts / HTTP endpoints
      (subprocess, offline renders)

## The one idea the compute layer is built around

The core knows nothing about proteins. It knows about **manifests** (what a model is,
declaratively) and **providers** (where a command can run). Adding AlphaFold 3, Boltz-2,
ProteinMPNN, RFdiffusion, or an unreleased in-house model is writing a TOML file - never
Rust.

    Manifest (models/*.toml)      Provider (settings)          Job
    what to run, what it needs -> where to run it, how to  ->  one execution,
    inputs/outputs schema         reach it, what GPU it has     logs + artifacts

A `JobSpec` is the fully-resolved product of (manifest + user inputs + provider).
Providers receive a `JobSpec` and never parse manifests themselves.

## Providers

| id | transport | GPU discovery | artifacts |
|---|---|---|---|
| `local` | pty spawn | nvidia-smi / Metal / rocm-smi | already on disk |
| `ssh` | system `ssh` in a pty | remote nvidia-smi over ssh | `rsync` pull |
| `slurm` | `sbatch`/`sacct` over ssh | `sinfo` gres parsing | `rsync` pull |
| `http` | REST, declared in TOML | reported by endpoint | download |

`ssh` shells out to the system `ssh` binary rather than linking a Rust SSH client. This
inherits `~/.ssh/config`, jump hosts, agent forwarding, hardware keys, and kerberos for
free - all things clusters actually use, and all things an embedded client fights.

## Hiding the GPU console
The pty is always real and always captured; the UI decides whether to show it. Settings
-> Compute reveals the same stream a terminal would, so a failing job is debuggable
without leaving the app, and a working job never mentions CUDA to the user.

## Worlds
One app, one SQLite database, N worlds - explicitly not PyMOL's file-per-session model.
A world owns structures, selections, representations, camera state, and the job history
that produced it. Thumbnails are Blender headless renders cached per world.

## Status
- [x] docs, licensing survey
- [x] compute layer: probe, registry, plan, queue, four providers, pty capture
- [x] settings UI (compute / models / jobs) + console panel
- [x] main menu shell
- [x] structure readers: PDB, mmCIF, bond inference, binary wire format
- [x] WebGL2 viewer: sphere/cylinder impostors, cartoon ribbons, five colour schemes
- [x] geometry in Rust, shared by the viewport and the renderer
- [x] Blender bridge: subprocess, tessellated mesh, path-traced figures
- [x] viewport deferred pipeline: G-buffer, SSAO, outline, sRGB composite
- [x] palettes, managed colour, and one scene style shared with the renderer
- [x] colour picker
- [x] node graph: typed sockets, pure evaluator, prediction nodes through the job queue
- [x] world system: worlds, the shelf, a graph and a camera per world
- [x] viewer: picking, selection as a view, the BioMol map
- [x] agents: connections, streaming, tools, a local engine
- [ ] `Component` node holding a `Spec`, and per-atom style resolution  <- next
- [ ] viewer: surfaces, a selection language, beta-strand arrowheads
- [ ] Blender render pipeline for cinematics

### Why the graph is the interface
A pipeline is a graph, and the panels this replaced could only show one path through it.
Putting prediction, drawing and lighting on the same plane also removes the seam between
"run a model" and "look at the result", which were separate screens with nothing
connecting them.

The evaluator is pure and the effects live in the command layer. That split is what makes
the graph testable: a cycle, a wrongly typed link, a prediction that has not finished, and
a graph with two outputs are all unit tests with no IO.

### Why geometry lives in Rust
The viewport and Blender both need to know what a cartoon looks like. Building it in the
webview leaves the render script to reimplement it, and two definitions of a ribbon drift -
at which point a render of a world no longer matches the viewer that made it. So the core
owns `structure -> geometry`, the viewport receives it over the binary IPC, and Blender
receives the same thing tessellated. Colour is in the same place for the same reason: the
AlphaFold confidence bands have to mean one thing in both.

### Why WebGL2 rather than WebGPU
`navigator.gpu` is absent in this WKWebView (macOS 14.5), and WebKit gates WebGPU behind
an experimental flag that can only be set through private API - which would not reproduce
on another machine. WebGL2 loses nothing that matters: `gl_FragDepth` and instanced draws
are what impostor ray-casting needs, and both are core in GLSL ES 3.00. The renderer is
split so that `gl.ts` is the only file that knows which API is in use.

### Why the window drag is JavaScript
The title bar is an overlay, so there is no frame to grab and moving the window is the
app's job. The obvious way - `-webkit-app-region: drag` - is a Chromium property; WKWebView
ignores it *silently*, which cost two rounds of "the window still does not move". The real
mechanism is Tauri's `startDragging`, called from `pointerdown`, and it needs
`core:window:allow-start-dragging` in `capabilities/default.json`: it is not in
`core:window:default`, so without it the correct call is denied too. Both failures are
quiet, which is why `ui/window.ts` reports them, and why every screen bar is built by one
`screenBar` helper - the main menu having no drag region at all was how this started.

### Why copying an image goes through the core
The webview's clipboard API only permits a write inside the user gesture that started it,
and reading the image's bytes first loses that gesture - after which WebKit *resolves the
promise anyway*, so the first version reported "image copied" and copied nothing. Handing
`ClipboardItem` a promise is the documented fix for that, but a programmatic click is not a
user gesture either, so the path could not be tested. The core writes to the system
pasteboard through `osascript` instead: no gesture rule, and verifiable from a shell
(`osascript -e 'clipboard info'` shows `«class PNGf»` afterwards). macOS only.

### Why the scripted view runs after evaluation, not during it
`GEIS_REPRESENTATION` and friends change the graph and ask for a re-evaluation. Called from
inside `drawScene` - which runs inside an evaluation - that request hit the "one evaluation
at a time" guard, was deferred, and the screenshot was then taken of the frame already on
screen. Every scripted capture was of the scene *before* the change it had just made, which
is exactly the wrong way for a test harness to fail: silently, with a plausible image. It
now runs once, after the guard is released.

### What actually made the helices ugly, part two
The frame was ninety degrees off. PyMOL's orientation vector is the ribbon's *normal*, and
using it as the *width* direction lays the band radially - a screw thread rather than a
ribbon. Four rounds of this went unnoticed because a helix in a whole-protein view is forty
pixels across; rendering one ideal twenty-four-residue helix on its own showed it
immediately. The lesson is the fixture, not the fix: `ideal_helix_structure` in
`geometry/cartoon.rs`, and a test that measures the mesh's radial spread, which is about
0.5 A for a correct ribbon and 2.8 A for an auger.

### What actually made the helices ugly
Three attempts, and none of the first two was the spline. The cause was a leftover rule
from the loop code: each frame was flipped to agree with the previous one, which is right
for a carbonyl-derived face and catastrophic for a helix. `cross(axis, tangent)` turns with
the tangent, so consecutive faces along a helix are a hundred degrees apart with a
*negative* dot product - the rule inverted every other frame and the ribbon shredded while
the geometry underneath was smooth. Mol\* does the same match once per segment, not per
pair, which is the distinction. Pinned by a test that asserts the hundred degrees and the
negative dot, so the rule cannot come back by accident.

### Why the cartoon reads PyMOL's source rather than its docs
The wiki says `cartoon_round_helices` "makes helices round by forcing orientation vectors to
point along the helix axes" and that turning it off "allows helices to pass more closely
through C-alpha positions" - which reads as though it moves the guide points, and a first
attempt here did exactly that. It does not:
`RepCartoonComputeRoundHelices` takes the positions as `const` and writes only `voptr`, the
orientation vectors. Smoothing positions onto the axis collapses the helix's 2.3 A radius,
and a broad spiralling ribbon becomes a tight corkscrew. The weights it uses (0.2130,
0.2870) are not arbitrary either: they cancel the radial component exactly at 100 degrees
per residue, which is worth a test rather than a comment.

### Where the helix meets the loop
Inside a helix the path is generated as a helix about its fitted axis; elsewhere it is a
spline through the trace. Both pass through the alpha carbon where they meet, but in
different directions, and switching from one to the other bent the ribbon by up to
eighty degrees at a single alpha carbon - the kinks at every helix end, and either side of
a kink in a helix's axis. The helix is now eased in from the spline and back out to it by
a smoothstep, whose slope is zero at the join, so the path leaves the join along the
spline's own direction. A step that does not look like a helix's (outside 50-135 degrees
round the axis, or nearer it than 1.6 A - a last residue pulled off towards the loop) is
drawn as spline, and the per-residue axes are smoothed (1, 2, 1) along the run, since a
four-residue window carries each irregularity of the helix as a wobble. Measured with
the ignored test `geometry::cartoon::tests::kinks` (sharpest bends between samples over
real files): haemoglobin's bends over 60 degrees went from 20 to none.

The breadth of the ribbon is blended over four residues by the cubic B-spline's weights
(`blended_profile`), not over the one stretch between two. Over one, a helix went from its
full 1.15 A half-width to a 0.3 A thread in 3.8 A - after the path had already left the
helix - and every helix ended in a broad flat flag standing out from the loop. Spread
this way it starts narrowing at the helix's last residue, is a thread a residue later,
and its edge has no corner at either end.

### The screen layout
Blender's, and it took three tries to get right, so it is written down. The right side is
one strip the full height of the window: outliner on top, properties underneath, and the
style panel at the bottom of the properties. The viewport and the node graph are stacked in
the column to its *left* - not above a right panel that stops where the graph starts, which
is what the first two attempts did. The viewport's own overlay carries only what is
selected and any problem; the legend and the geometry counts are gone.

### The properties editor, and where it came from
Blender 5.2 is installed on this machine and its properties editor is bundled Python, so
the layout was read rather than guessed:
`/Applications/Blender.app/Contents/Resources/5.2/scripts/startup/bl_ui/space_properties.py`
and `properties_material.py`. Blender also screenshots its own UI
(`bpy.ops.screen.screenshot` from an `app.timers` callback), which is how the rendered
panel was checked - macOS `screencapture` was no use, since Blender's window was on
another Space and System Events has no accessibility permission here.

What it is made of, and what Geis copies:

- a header with a search field that filters properties by name;
- a **navigation bar**: icon-only tabs in a region of their own down the editor's left
  edge, drawn by `layout.prop_tabs_enum(view, "context", icon_only=True)` at `scale 1.4`.
  Twenty are possible and the active object decides which exist;
- a **breadcrumb**, `Cube > Cube > Material`, with a pin that stops the editor following
  the selection;
- the **datablock block** - a header-less panel (`bl_options = {'HIDE_HEADER'}`) holding
  `template_list` over the object's material slots with a `+ - v ^ v` button column, then
  `template_ID(ob, "active_material", new="material.new")`: browse the file's materials,
  rename, duplicate, unlink;
- panels, `DEFAULT_CLOSED` for the optional ones, `bl_parent_id` for sub-panels, `bl_order`
  to pin some to the bottom, and `use_property_split = True` for the right-aligned label
  column.

The important one is `EEVEE_MATERIAL_PT_surface`, which does not lay out fields at all:

    node = ntree.get_output_node('EEVEE')
    layout.template_node_view(ntree, node, find_node_input(node, "Surface"))

The panel *is* the node graph, drawn in a panel. Geis's equivalent is `styleGroups`, which
both the Style node's face and the Style tab's panels are built from - so a style has one
definition and the panel cannot drift from the node.

### The screen as a desk of panes
The fixed grid - viewport over graph, a strip on the right - was the first thing to go once
a PAE plot or a log had to be shown beside a structure. The first replacement was Blender's
answer, a binary tree of areas cut and joined with the mouse, and it was the wrong shape
for this: what was wanted was a browser's windows - drag a title bar to move, drag an edge
to resize, let them overlap. `panes.ts` is that desk and nothing else: a list of panes with
fractional rectangles and a stacking order, and `movePane`, `resizePane`, `bringToFront`,
`addPane`, `removePane`, `setPaneEditor`, `snapAt`/`snapPane`/`restorePane`/
`toggleMaximise`, `parseScreen`, all pure, all tested. Fractions rather than pixels so a
resized window keeps the arrangement; a minimum size so a pane can never be dragged down to
nothing; a grip so a pane may hang off the desk but never out of reach. Snapping is the
desktop's rule - top edge fills, sides halve, corners quarter - and a snapped pane keeps
its previous rectangle in `before` so the maximise button, a double-click on the title, or
dragging it away all put it back; moving or resizing by hand forgets it. The desk may be
emptied entirely; the empty desk's right-click menu makes a new pane where you clicked.
Away from the edges `snapAt` takes the desk's tiling into account: given the snap tiles the
other panes occupy (`occupiedSnaps`), it offers the largest free tile under the pointer, so
a desk with its top two quarters taken hands a third pane the bottom half.

A pane is a strip of tabs (`Tab { id, editor }`, one `active`), as a browser window is:
`addTab`, `closeTab` (the last tab closes the pane), `activateTab`, `setTabEditor`,
`moveTab` between panes, `detachTab` into a new pane. Image and text hosts are keyed by tab
id, so two Image tabs are two images; the singletons - viewport, BioMol panel, graph,
Agents panel - are pulled out of wherever they were when asked for elsewhere, the tab they
left closing with them (`dropSingleton`), and its pane too if it held nothing else. Asking
for one that is already in this pane only activates it. The first version left an Image tab
behind instead, which read as having pressed the wrong menu item. The screen keeps one element per pane and
re-renders the strip only when its tabs or active tab change; the active tab's editor is
reparented only when the active tab changes. `parseScreen` reads the pre-tab shape - a pane
with one `editor` - as a pane with one tab.

The screen keeps one element per pane and restyles it in place, so a drag is a
`left`/`top`/`width`/`height` change and nothing is rebuilt. Singleton editors - the
viewport's canvas with its WebGL context, the graph editor with its state - are reparented
only when the pane showing them changes, and told to re-measure afterwards. There is one
of each; asking for one in a second pane moves it and leaves an image pane behind, because
"put the graph here" is what was meant. Images and text are cheap and can be shown anywhere.

The outliner and properties are not panes, and that is deliberate: they are the one place
that always answers "what is selected", and an arrangement that can bury them is one you
can get lost in. So the desk covers only the left of the screen, and the right column is
fixed - resizable by its divider, saved with the desk, never an editor you can swap away.

### Worlds
A world is a session: `worlds.rs` holds the model - id, name, when it was made and last
opened, the graph, and the camera - and the shelf is one JSON list in the settings table,
because a world is kilobytes and keeping them together makes renaming or reordering a single
write. The one thing kept outside is the card's picture, a PNG per world under `worlds/`,
since a base64 image in every read of the list is waste. `graph_load`, `graph_save` and
`graph_reset` act on the world that is open rather than on a settings row, and `all_worlds`
migrates the pre-worlds `graph` row into the first world the first time it is asked.

The preview's transparency is one uniform: the composite pass writes `uBackgroundAlpha`
into the background branch instead of 1, and `Viewer::setTransparent` is what asks for it.
The same switch is flicked for a moment when a world's card picture is taken, so the still
neighbours on the shelf sit on the page like the turning one between them.

`ui/worlds.ts` is the shelf, and the shelf is one made-up viewport - a view for showing
worlds, not for working in them. Every world is built into a single scene at its own place
along a line (`ScenePart.place`/`size`, applied by `Scene::place` in `geometry/mesh.rs`),
sized alike, at most four molecules each; one `graph_evaluate` and one `build_scene` per
world, once. An arrow, a scroll, a dot or an arrow key moves the *camera* along that row -
`Viewer::lookAt(centre, distance, radius)`, eased per frame - so moving costs nothing and
loads nothing.

The slide is a critically damped spring (`travelBy`, stiffness 200, substepped at 240 Hz so
a late frame cannot make it ring), not a fraction of the remaining distance per frame: that
older easing left at once at its fastest and crawled in over most of a second, and an arrow
pressed mid-slide restarted it from a standstill. A spring leaves from rest, arrives at
rest, and an arrow pressed while it is moving only changes where it is going - half the
journey in 100 ms, settled by 450, no overshoot. The size each world is drawn at eases with
`smoothstep` for the same reason: a straight ramp has a corner at each end of the journey,
and the eye reads that corner as the slide catching.

Every world keeps its own angle - `Row.turns`, one per place in the row, uploaded as a
uniform array (`ROW_LIMIT` of 32; a longer shelf still stands and is still sized right, its
far worlds simply do not turn). One shared angle and a band that had it could not describe
two worlds: on every press the world arriving jumped to however far the last one had got
and the world leaving snapped upright. Each world's turning also eases in and out rather
than starting and stopping dead.

Three rules make it a shelf rather than a viewport. The mouse does not drive it:
`Viewer::setInteractive(false)` refuses `pointerdown`, `dblclick` and `wheel`, and the
wheel the shelf listens for is its own, for moving along the row. Clicking does not enter;
the `Enter` button and the Enter key do, because a shelf you fall through is a shelf you
cannot look at. And the *world* turns, not the camera: `Viewer::setRow(pitch, focus, shrink, turns)` sets a
pose that the three vertex shaders apply themselves through `ROW_GLSL` in
`viewer/shading.ts`, each point posed by the world it stands in. So the one you chose is
alive and its neighbours stand still,
which is what a character select looks like, and it costs no rebuild - the geometry is
uploaded once and the turn is a uniform. The lamps are forced to follow the view here, so
the light travels with the camera rather than leaving the far worlds lit from behind.

Leaving the screen stops its frame loop, so coming back re-reads the worlds and starts it
again - but it only builds the row anew when the row would differ. Each card carries a
`stamp`, a hash of its graph, and the row remembers what it was built from; a visit that
changed nothing costs two commands instead of an evaluation and a geometry build per world,
which is what made leaving stutter for half a second. The list is ordered by when each
world was made rather than by when it was last used, so the shelf does not rearrange itself
behind your back - and so leaving a world is not itself a change to the row.

The row is built cheap: `SceneRequest.coarse` picks `cartoon::Detail::coarse` - six
subdivisions and a twelve-point profile instead of eight and twenty - and `build_scene` is
an async command, so the work runs off the main thread. Together those turned the first
press of Worlds from a 17.8 MB payload and a 507 ms frozen window into 2.9 MB and 91 ms,
with no stall anywhere in the journey. A render always asks for full detail; so does the
viewer you work in.

That level started at three and eight, which was cheaper again and wrong: a ribbon that
coarse is flat-sided, and a flat-sided ribbon turning slowly changes face in steps the eye
reads as a judder. Animation now runs inside the viewer's display callback, immediately
before drawing. The shelf previously requested a second callback to draw, which presented
the previous pose. A smoothed frame duration did not fix presentation and distorted elapsed
time after a late frame, so rotation now integrates its easing using the animation
timestamp. Setters during animation coalesce into the current draw. Detaching the shelf
stops the loop; returning starts it with a fresh timestamp. Default-framebuffer MSAA is
disabled because only a fullscreen composite is drawn there; geometry antialiasing remains
in the offscreen post pass. Commands are flushed after the composite without waiting for
the GPU.

Nothing takes a picture of a world any more. The card used to hold a PNG, captured on the
way out; the row draws the worlds themselves now, so it was a redraw, a read-back and a PNG
encode of a retina canvas paid on every Back press for something nobody read.

The app holds two WebGL contexts, the viewer's and this one.

### Selection as a view
`structure/index.rs` is BioMol's data model in Rust: `IndexTable` (forward `atom_to_res`,
`res_to_chain`; CSR reverse `res_atom_indptr/indices`, `chain_res_indptr/indices`), built
from a `Structure` on demand; `Level`; `View { level, indices }` with `at/to_atoms/
to_residues/to_chains` (unique, first-occurrence order) and `bounded`; `Spec { level, where }`
whose `Where` is the ANDed conditions of BioMol's `select(**kwargs)` written down, and
`evaluate`, one pass over the atoms lifting the survivors to the level asked for; `masks`,
the viewport's atom and residue bytes (none of the latter for an atom view, since a ribbon
is a residue's). Commands: `select_view`, `view_mask`, `describe_view` (counts, a label, the
chain ids touched, and for atom views their residues), `chain_residues`, `residue_atoms`.

On the screen, `selection.ts` is the pure half - `combine` (replace/add/toggle across one
structure and level, else start over), `rangeBetween`, `layMask` into the scene's id space -
and `viewerScreen`'s `applySelection` is the one place a view becomes an outline, a card, a
lit tree, a painted strip and a selected graph node (the chain's when one chain, else the
molecule's; guarded so the outliner's callback does not come back round as a new chain
selection). `biomol.ts` is the panel: the layered map. `molmap/layout.ts` is its geometry, pure -
`layoutMap` lays the non-water chains across the molecule's width, each residue one unit or
room for its atoms when open (an `Openings` map of residue → 0..1 lets an animation hand in
how far), lifts a one-residue ligand to a floor, spans each parent over its children, and
places arcs for the residue links and for an open residue's bonds; `hitTest` reads a point
from the deepest layer up; `fitCamera`/`mapOrigins` place stacked maps. `molmap/tween.ts` is
the animation: a `Tween` heads for a target from wherever it is, a `Pulse` fires once. The
panel itself is a canvas with a frame loop that runs only while something is moving, a
camera of three tweens, a per-residue opening tween, and gestures - pan on empty space,
click a cell, drag along the residue layer for a range, double-click a residue to open its
atoms and a chain to glide into it, wheel to zoom about the pointer. `structure_map` hands
it a structure's chains as parallel arrays and its non-backbone residue links in one call;
`residue_atoms` hands it an opened residue's atoms and their bonds.

The next step is the one this was for: a `Component` node holding a `Spec`, evaluated by
the core into a view, that a style slot is assigned to - and per-atom style resolution,
most specific level first, later wins at the same level.

### Agents
An agent is a model with tools, and the split is the interesting part. The core
(`agents/mod.rs`) owns exactly two things: the credential, read from the keychain at call
time so it never crosses into the webview, and the adapters that turn one neutral
`ChatRequest`/`ChatReply` into each provider's wire format and back. Local models need no
adapter of their own - they speak OpenAI's Chat Completions - which is why three kinds of
connection cost two adapters. `agents/docs.rs` embeds the documents with `include_str!`.

A fourth kind, `cli`, is how a *subscription* is used: it spawns a first-party agent CLI the
user has installed and signed in to - Claude Code, Codex - with the prompt on stdin. It is
the only honest route, since a subscription is not an API key and neither the chat websites
nor another program's login may be driven. Such a CLI has its own agent loop and no tool
protocol to speak, so `cli_prompt` flattens the conversation and asks for a fenced JSON
block when it wants a tool, and `cli_reply` reads the last one; that convention is looser
than the two real tool APIs, which is why a key is preferred when there is one.

`agents/weights.rs` is the local-model catalogue and the download. The catalogue is a
literal list with real content lengths, checked against the servers, so the panel can say
what a download costs before it starts; the 7B entry points at a single-file quantisation
rather than the official two-part split, since only llama.cpp can reassemble shards and a
downloaded file should work wherever it is pointed. `fetch` is the download itself, taken
out of the command so it can be tested without an app: it resumes with a Range header,
starts the part again when the server answers 200 instead of 206, renames only when
complete, and reports often enough to feel live without flooding the webview. The command
around it owns the cancel flag and turns reports into `local-model:progress` events.

`agents/engine.rs` is llama.cpp: a pinned build, one asset per platform with the digest
GitHub publishes for it, and the small pieces around running it - finding `llama-server`
wherever the archive put it (looked for rather than assumed, because the layout has moved
before), the arguments (`--jinja` above all, without which there are no tool calls), a free
port, and waiting for `/health` while several gigabytes load. Unpacking shells out to
`tar`, which reads both .tar.gz and .zip on every platform we target and is one fewer crate
to keep current. `engine_start` holds the child in `AppState` with `kill_on_drop`, a
window-destroyed handler on the *main* window kills it as well, and its id is written to
`engine.pid` so `kill_stale` can end one that survived a force-quit - checked against `ps`
first, since process ids are reused.

`agents/stream.rs` is the streaming half: a `Reader` per wire format that takes raw bytes,
splits them into server-sent event lines, and returns `Delta`s while accumulating the reply.
It carries two pieces of state that only a stream needs - bytes that are not yet a whole
line, and characters held back because they might be the start of a `<think>` tag - and
`finish` flushes both. `complete_streaming` drives it; `agent_stream` is the command, taking
a `tauri::ipc::Channel` so the pieces go to the one caller. The loop's `complete` hook takes
an `onDelta` callback, and `ui/agents.ts` keeps a `LiveTurn` of the elements being filled in.

One Tauri behaviour shapes the settings window: a command invoked from a second window
loses its reply if anything else talks to the core while it runs. The window therefore
treats every long command as fire-and-forget and polls for the state it wanted (`until` in
`agentSettings.ts`), and `announce_agents` defers its broadcast past the end of the command
that triggered it.

Everything else is in the UI, because that is where the graph, the viewport and the
selection are, and a loop that ran in the core would reach back across the boundary for
every step. `agent/loop.ts` is the turn - model, tools, model, until it stops asking -
pure enough to test, and tested for the things that only show up late: a result attached to
the wrong call, a refusal that leaves the model waiting, a loop that never ends.
`agent/tools.ts` is the list of what an agent may do, with the implementations handed in by
the screen so the file reads as a policy document; `ui/agents.ts` is the panel, where the
conversation, the call cards and the approval buttons live; The panel is the screen's right column - the outliner and
properties having moved to the left - rather than a pane on the desk, with its own divider
and a fold whose state rides along in the saved layout; `readEditor` returns null for an editor this version has no
panel for, and `readDesk` drops that tab rather than rejecting the whole layout.
`ui/agentSettings.ts` is the
connections, the model catalogue and the engine, rendered into a second window (`main.ts`
routes on `?panel=`, and the window is opened from the panel's Settings button).

The tool list carries one deliberately high-level entry, `colour_chain`. Everything else is
a primitive - slots, assignments, styles - and a model that has to compose them for "make
chain A magenta" often will not; the one that answered by trying to fetch an entry called
`A_1` is the reason both that tool and the redirecting error message exist.

The screen's `agentHost` is deliberately thin and deliberately shared: `setStyle` resolves
the molecule, the slot and the fields, then goes through the same `setGraph`, `save`,
`evaluate` path a click goes through. One way for the scene to change means one way for it
to be undone, and it means an agent cannot reach a state a user could not.

### Chain identity
A chain has three names, and the code keeps all of them. `Chain::id` is `asym_id_oper_id`
(`A_1`), the form BioMol and StructCooker use: the mmCIF `label_asym_id`, which is the
archive's own partition of the structure, plus the assembly operator that produced this copy
- `1` for the asymmetric unit and for the first copy in an assembly, `2` and up for the
copies an assembly adds (see "Fetching the biological assembly"). `auth_id` is the author's letter, which a PDB file has and a paper
cites. `label_asym_id` is kept on its own too, because `_struct_conn` and the secondary
structure categories name chains by whichever of the two suits them, and
`residue_indices_in_range` accepts any of the three.

`entity_type` comes from `_struct_asym` joined to `_entity`, and is *inferred* for files
without an entity table - all water is `water`, no polymer residue is `non-polymer`,
otherwise `polymer` - so the two readers describe a chain in the same words. That is also
why the two-readers test no longer asserts equal chain counts: a PDB file's 18 author chains
of 6VXX are 51 asyms in its mmCIF, and what the two must agree on is the polymer chains,
by author id.

### The stick-joint investigation, and how it was settled
"Sticks show balls" had one real cause and one false lead. The real cause was a single
cylinder radius per scene, taken from the unassigned style: sticks at 0.22 A under spheres
at 0.30 A. Fixed by carrying the radius per bond.

The false lead: after the fix, close-ups still showed caps that *read* wider than their
sticks. Three measurements settled it rather than a fourth look. The live scene's radius
histogram: one value, 0.300, for every sphere and every cylinder. Blender, tessellating the
same payload from the same camera: flush caps. And a synthetic payload fed straight to the
viewer - one sphere and one cylinder of equal radius, camera square on - measured 240 and
236 pixels across. Equal within antialiasing, so the impostor and the prism agree; what
remained in the close-ups was perspective on the nearer atoms. The lesson, again: a shader
question is answered by a measurement, not by an impression of a crop.

### How picking works, and why it is not a fourth channel
A click has to answer "what is under this pixel". The G-buffer is full - colour, packed
normal, and the selection mask in the normal target's alpha - and adding a fourth target
to every frame to serve a gesture that happens a few times a minute is the wrong trade. So
picking is a *second draw*, on the click only:

1. bind the G-buffer, scissor to the one pixel under the cursor, clear it to zero;
2. draw the scene with `uPicking = 1`, which switches each geometry shader's fragment
   output from shading to `encodePick(vId)` - the same programs, not copies of them, so
   the sphere impostor cannot drift between the two paths;
3. `readPixels` that pixel and decode it.

The encoding is two bits of tag and thirty of index, because the geometry knows different
things in different passes: a sphere and a stick end know their *atom*, a ribbon vertex
knows only its *residue*. Without the tag, 237 could be either. Zero decodes to nothing,
which is what makes a click on the background a clear rather than a hit on atom 0.

Resolving an id to a name is the core's job (`pick_target`): the scene's id space is
per-part, so the frontend finds the part from the header's offsets and the core turns
(structure, atom) into chain, residue and atom. It answers all three levels at once
because residue and atom picking are next, and that should be a matter of which field the
caller reads.

Two details that cost a round each:

- **A click is a press that did not travel.** Selecting on any pointerup means an orbit
  ends by selecting whatever the cursor happens to be over. Three pixels of slack.
- **`drawProperties` was refusing to redraw.** Its guard - do not rebuild while the panel
  has focus - was written for a drag on a slider and applied to *any* focus, including a
  button. After clicking a tab, clearing the selection from the viewport left the old
  selection on screen: the state was right and the panel was stale. Now only text entry
  blocks it.

### How the selection outline works
The requirement is small and the plumbing is not: clicking a chain in the outliner has to
outline that chain in the viewport. Three ways to do it, and the reasons for the one chosen:

- *Re-draw the selected subset with a fatter radius, behind the real one.* Needs the subset
  to be separable in the geometry, which it is not - the scene is one buffer per primitive
  type.
- *A per-primitive selected attribute.* Correct, but the selection changes on every click
  and the attribute lives beside the geometry, so each click re-uploads tens of megabytes.
- *A mask texture keyed by id.* What this does. The scene already carries an atom id per
  sphere and per stick end, and a residue id per ribbon vertex; the mask is an R8 texture
  the size of the id space, uploaded on selection change - a few kilobytes - and read in
  the vertex shader with `texelFetch`.

The mask then has to reach the composite, and it rides in the **alpha of the normal
target**, which packs its normal into RGB and had a spare channel. A third render target
would have been cleaner to read and one more thing to be unsupported.

Three things went wrong on the way, all of them invisible rather than loud, which is why
each is written down:

1. **`Scene::append` did not offset ids.** A known gap, harmless until something was keyed
   by an id. Fixed, with the per-part offsets in the header so the frontend can lay a
   structure's mask into the scene's index space.
2. **An R8 texture is normalised.** The core reports the mask as 0 or 1, so the shader read
   1/255 and every threshold said "not selected". The upload now expands to 0 or 255. The
   symptom was an outline four thousandths of a shade deep - a bug that looks exactly like
   nothing happening.
3. **The normal target's alpha was cleared to 1**, so the background read as *selected*:
   a molecule with every chain selected had no boundary anywhere and got no outline at all.
   Cleared to zero now, with the other channels left as they were, since the occlusion and
   outline passes read them.

Diagnosing it took three rounds of rendering the mask straight to the screen and reading
the pixel histogram - "the level is 128, so the uniform arrived and the lookup returned
zero", then "the level is 50, so the bound texture is 801 wide, which is the right one" -
which is the only way to see inside a shader from outside it.

### How bonds are worked out
The first version inferred them from distance, which was wrong, and the correction came
from StructCooker: bonds are *data*. A file names its chemical components; the Chemical
Component Dictionary states which atoms of each are bonded and with what order. So the
bonds of a residue are a lookup, and `transforms/ccd.py` uses distance only to validate.

`structure/bonds.rs` builds the graph in this order:

1. **the file's own `_chem_comp_bond`** - and the assumption that a coordinate-only mmCIF
   does not carry one turned out to be false: the RCSB entry files do, for every component
   in the entry, ligands included, with `value_order` and `pdbx_aromatic_flag`;
2. **the built-in table** (`components.rs`, generated from the CCD: 30 components, 704
   bonds) for the standard residues, which is what a PDB file gets, since it carries no
   dictionary at all;
3. **polymer links** between consecutive residues of a chain, C to N or O3' to P depending
   on `_chem_comp.type`. Distance appears here, and only as a sanity check: a file with a
   missing loop numbers its residues consecutively across the gap, and a stick drawn across
   it is a lie about the model;
4. **`_struct_conn`** - disulf, covale, metalc. Author ids, and records for a symmetry mate
   are dropped, since the atom they name is not in this model;
5. **distance**, for a component nothing above knew - a novel ligand in a PDB file. Marked
   `BondKind::Inferred`.

Measured on 4HHB: 4704 bonds, 4000+ from templates, 570 peptide links, four haem
coordinations from `_struct_conn`, and zero inferred. On 6VXX the mmCIF now finds 84 bonds
the PDB file cannot - its glycan links and disulfides - which is why the two-readers test
asserts that the CIF knows *at least* as much rather than exactly the same.

Two consequences worth stating. A bond carries `BondOrder` (single, double, triple,
aromatic) and `BondKind` (where it came from), because a carbonyl is a double bond and a
guess should be able to admit it. And metal coordination is not drawn as a stick - it is a
contact, which is what PyMOL does too - so `Bond::is_covalent` gates the geometry while the
edge stays in the graph for an interaction view to draw.

`bonds::implausible` is StructCooker's validation, the same 0.5 to 1.3 times the covalent
radii sum. It reports; it does not delete. A bond the file states is a fact about the
deposition, and a viewer that quietly drops the ones it dislikes hides the very thing
someone would want to see.

### Why sticks looked broken under a cartoon
None of the bonding above was the reason a cartoon-and-sticks picture of 4HHB looked full
of broken bonds. The bonds were all there; the backbone ones were *inside the ribbon*, which is drawn
through the same space, leaving the outward halves of N, C and O sticking out of it as
stubs. PyMOL hit this long ago and its answer is `cartoon_side_chain_helper`, which its own
presets switch on whenever they show both - so `buried_by_cartoon` drops polymer backbone
atoms from Sticks and Ball-and-stick when the same style draws a Cartoon, keeping the
residue's trace atom so the side chain has an attachment point. Rendering the same view
before and after: 3.74M triangles to 2.43M.

### What actually clips a sphere impostor
An impostor sphere is a quad plus a ray-sphere intersection in the fragment shader, and the
only thing the quad has to do is contain the sphere's silhouette. The silhouette is the
circle where the tangent cone from the eye touches the sphere: it lies at distance
`|C| - r^2/|C|` from the eye with radius `r*sqrt(|C|^2 - r^2)/|C|`, in the plane
perpendicular to the eye-to-centre axis - *not* in the image plane, and not centred on the
projection of the centre.

The old quad lived in the image plane at the centre's depth with half-size
`r*|C|/sqrt(|C|^2-r^2)`, which is the right size for a sphere on the view axis and too
small for one off it. Worked through with numbers, for `r = 1` at view-space `(10, 0, -10)`:

    true silhouette, projected:   x/|z| in [0.8676, 1.1520]
    the old quad, projected:      x/|z| in [0.8998, 1.1002]

so the sphere was cut on both sides. The error grows with the off-axis angle, which is why
it showed up on the atoms around the rim of a molecule that fills the frame and not on the
ones in the middle. Building the quad in the silhouette's own plane covers it exactly,
because a square around a circle in a plane projects to a region containing the circle's
projection.

Measured on the same spacefill view of 4HHB, old shader against new: 3159 pixels change at
the fit view and 276 four zoom steps out, with channel deltas up to 242 - thin slivers, one
to two pixels along each affected silhouette. Two other suspects were ruled out by the same
method: ambient occlusion (turned off, the silhouettes are unchanged) and depth precision
(`near` is `distance - 1.5R`, so the far/near ratio stays near 1.5 however far out you go).

A methodological note, because it wasted two rounds: comparing before-and-after captures
means killing the app between them. Editing a shader while `tauri dev` is running triggers
an HMR reload, the startup probe runs again, and the "before" screenshots are quietly
overwritten by "after" ones - which showed up as two identical images and one impossible
measurement.

### Why the Style tab does not paint
A material in Blender is assigned, so editing it repaints immediately, and Geis's Style tab
worked that way at first: it edited the molecule's active slot. The instruction was that
this is wrong - a style is composed first and applied second, and a panel that repaints on
every drag has nowhere to compose in.

So the tab edits the *library*, and assignment is a button. The mechanics: unsaved edits
live in `styleEdits` (by entry name) so a drag reads back the value it is being dragged to,
`persist` writes the entry through `style_save` when the widget lets go, and `assignStyleTo`
is the only path from the tab into the graph - it writes the style into a slot of the same
name and points the chosen chains at it. The field helpers take a `persist` callback for
exactly this reason: `checkbox`, `select` and `dragField` used to commit the graph
themselves, which is what made the panel live whether it wanted to be or not.

Two things still edit live, on purpose: a Style node wired into the graph (a node is part of
the pipeline, and the tab shows it instead of the library when one is selected), and the
Structure tab's group Colour, which is an apply action rather than an authoring one.

### Where the colouring schemes come from
PyMOL is installed here too, so the schemes were read rather than reconstructed:
`/Applications/PyMOL.app/Contents/lib/python3.10/site-packages/pymol/util.py` and
`viewing.py`.

    def chainbow(selection='(all)', palette="rainbow", ...):
        for model in _self.get_object_list('(' + selection + ')'):
            for a in _self.get_chains('model %s & (%s)' % (model, selection)) or ['']:
                _self.spectrum('count', palette, "(chain '%s' & model %s & (%s))" % ...,
                               byres=1)

    def cbc(selection='(all)', first_color=7, ...):
        for c, a in enumerate(_self.get_chains(selection)):
            _self.color(_color_cycle[c % _color_cycle_len], "(chain %s and (%s))" % (a, selection))

    palette_colors_dict = { 'rainbow': 'blue cyan green yellow orange red', ... }

Three things follow. First, `chainbow` is a loop over chains, and `spectrum count` is an
ordinal over whatever the selection holds - so the difference between rainbow and chainbow
is only whether the sweep restarts, which is one bool in `sequence_positions`. Second,
`byres=1` means a residue is one colour, so the ramp is sampled per residue and not per
atom. Third, `spectrum` normalises over min..max of the expression and interpolates in
RGB, so a B-factor picture is a ramp over the range the structure has rather than over a
fixed 0..100.

The colour values are not PyMOL's. The earlier instruction was that lifting PyMOL's
palette is a licensing risk, and `_color_cycle` is exactly the kind of table that means -
so the chain colours stay ours (the categorical palette) and the rainbow is a hue sweep
from 240 to 0 degrees, which passes through the same six colours PyMOL names because those
are pure hues. Waters are left on the solvent colour rather than being given a place in the
sweep, and anything non-polymer takes the end of its chain's ramp so a ligand is coloured
like the chain it sits in.

### Why styles live in the molecule's slots
A chain used to carry its own style, which made a four-chain molecule four copies of one
decision: restyling a tetramer was four identical edits, and "a bundle of styles" had
nowhere to be. Blender's answer is material slots - the object owns the materials and the
geometry carries an index - so that is the model: `NodeKind::Molecule` holds
`slots: Vec<StyleSlot>` and `NodeKind::Chain` holds `slot: Option<usize>`.

`Graph::chain_style` resolves in one place, in this order: a wired Style node, the named
slot, the chain's own style, the default. The third exists only for graphs saved before
slots; the second falls back to the first slot rather than to the default when the index is
past the end, because a slot deleted from under a chain should leave it looking like the
molecule it belongs to. Moving a slot repoints the chains that named it, or the molecule
silently restyles itself.

### Why arrange measures node widths
Layered layout ranks nodes by longest path to a sink, so the dependency order was right
from the start; what kept going wrong was the geometry. Each column was placed one node
width apart, which is fine until a column holds something that is not one node wide - a
folded molecule group carries its own inner two-column layout and is roughly twice as
wide as a plain node. Columns now accumulate from the widest node in each, and
`layoutWithGroups` measures a group's inner layout to report its width, so the chains
inside a frame and the viewport after it no longer land on top of each other. The symptom
read as "arrange ignores dependencies" because overlapping columns look exactly like
equal ranks.

### Why the colour popover lives on the body
Both the things a colour field can appear inside - a 208px node and a 272px side panel -
clip their overflow, and the popover is 268px wide. Positioned inside either one it was
drawn and then cut away, so the picker existed and could not be clicked. It is now
appended to `document.body` with `position: fixed`, placed from the swatch's client rect
and clamped to the window, and the outside-click handler excludes it explicitly since it
is no longer a descendant of the field.

### A note on a self-inflicted wound
About a dozen graph tests were deleted in one careless edit while chains were being turned
into nodes: a slice replacement whose start was a helper defined above the tests and whose
end marker was below them took everything in between. It showed up as the count dropping
from 166 to 153, and there is no git history here to recover from, so they were rewritten
from scratch. Slice-by-marker edits over a test module are not worth the keystrokes they
save.

### The audit of 2026-11-12
A read of the whole tree for what the tests did not catch. Everything was green going in -
which is the finding: the suite covered what each module had been built to do, and the
defects sat at the seams, in inputs nobody had typed yet, and in paths only a failure
takes. What follows is the part worth keeping, not the list.

**The fixture tests were not running.** `real_structures` and `blender_render` skip when
`GEIS_TEST_STRUCTURES` is unset, and a skipped test prints `ok`. Nine tests reported
passing without having read a file. Run for real, they did pass - but nobody could have
known that from the output, and the README now says how to run them.

**Blender reports a crashed script as success.** Measured on 5.2: an uncaught Python
exception in `-P script` exits 0. The bridge trusted the exit code and then checked that
the output path existed - which a previous render had left there. So a failed render
returned the old picture. It now passes `--python-exit-code 1`, renders to a file of its
own and moves it into place, and works in a directory per render rather than one shared by
every render, which two Render nodes started together overwrote under each other.

**Cycles was rendering on the CPU and logging that it was not.** Also measured: after a
factory reset the device list is empty until `get_devices()` is called, and naming a
backend with no hardware behind it raises nothing. The script named Metal, set
`device = "GPU"`, and logged "cycles device: METAL" with zero devices enabled. It asks
each backend what it found now, and uses one only if it found something.

**A cartoon of DNA drew nothing.** The break between two residues was one distance, 5.5 A,
which is right for alpha carbons and less than the gap between any two phosphates. Every
step of a strand was a break, every nucleotide a segment of one point, and a segment of one
point is skipped - while the cartoon went on hiding the atoms the ribbon was supposed to
replace. Nucleotides have their own limit; and the atoms a cartoon hides are now the ones
a ribbon was actually built over (`cartoon::covered`), so a residue the ribbon cannot
reach is drawn rather than deleted.

**The helix, part three.** Two things the ideal-helix fixture did not show because it
starts at residue zero and was only measured in its middle. The first residue and the last
two of every helix had no axis: their axis point is borrowed from a neighbour, and the
direction was then taken from "the next fitted point", which was that same point. And the
fitted point for a residue sits midway between it and the next one along the axis, so the
helical path ran three quarters of an angstrom ahead of the alpha carbons and jumped at
both ends, where the spline takes over. Each centre is now slid along its axis to its own
residue's height, and a helix is matched to the ribbon before it once, where it starts.
Both are tests: every residue has an axis level with its alpha carbon, and the path at
`t = 0` and `t = 1` is on the alpha carbons to within 0.02 A.

**A file with `_chem_comp` and no `_chem_comp_bond` had no bonds.** The file's dictionary
was looked up before the built-in one, and an entry with a name and an empty bond list is
still an entry. That shape is what prediction tools write. Related, in the same function:
a modified residue in a PDB file - a phosphoserine, with no dictionary to describe it -
was classed as non-polymer and cut out of its chain on both sides; it is now recognised
by its backbone atoms.

**mmCIF writes a one-row table without `loop_`.** As `_category.item value` lines. Only
`_struct.title` was read that way, so an entry with exactly one disulfide, one helix or
one chain lost its `_struct_conn`, `_struct_conf` or `_struct_asym`. The pairs of each
category are gathered into a one-row table and filed like a loop.

**Streaming a reply in Korean panicked on the first delta.** The hold-back for a `<think>`
tag split across chunks sliced the last few *bytes* off each delta to compare them, and in
any script but ASCII those bytes are the inside of a character. Separately, each network
chunk was decoded on its own, so a character cut by a chunk boundary became two U+FFFD.
The reader takes bytes and decodes whole lines now. An `error` event mid-stream is
returned as an error rather than as an empty answer, and a tool call cut off inside its
arguments is refused rather than run with none.

**One unreadable entry emptied a whole list.** Worlds, user styles and agent settings were
each read as `from_str(..).ok().unwrap_or_default()` and written back by the next save -
so a single world this build could not parse meant every world was replaced by one new
empty one. Lists are read an entry at a time (`Store::list_setting`), and whenever
anything is left out the text as it stood is kept under `<key>.unreadable`.

**The structure cache held six and was keyed by request.** Every evaluation parsed every
file again under a new id, pushing older ids out. A viewport with seven molecules, or a
shelf whose worlds held seven between them, evaluated fine and then failed to build with
"no longer open". The cache is keyed by path and modification time, so a file open is a
file found; ids are therefore stable across evaluations, which nothing depended on being
otherwise.

**Clicking a Light deselected it.** Selecting a node that is not part of a molecule
cleared the structure selection - and, through the same call, the node selection that had
just been made. No properties tab, no delete key, no alt-drag aim, for every node that is
not a molecule or a chain.

**Quoting for ssh is quoting twice.** `ssh host env K='v' bash -l '/path'` has its quotes
removed by the local shell; ssh joins what is left with spaces and the remote shell parses
it again, bare. A provider environment value with a space in it broke the step, and one
with a `;` ran a second command. The remote command line is quoted as a single word.

**rsync brought back a stale log over the live one.** The job directory is uploaded with
`job.log` in it - open, a few lines long - and downloaded again at the end, when the local
log is longer and newer. rsync replaced it by rename, and the sink went on writing to a
file with no name. Excluded in both directions.

### What clicking through it found
The audit above was a reading; this was the app driven by hand, each screen looked at.
Four things only showed up that way.

**`window.prompt` does nothing here.** WKWebView shows a JavaScript prompt only if its
host implements the panel, and this one does not: `prompt()` returns `null` in zero
milliseconds with nothing on screen. Three menu items were built on it - fetch an entry
from the PDB, duplicate a style, register a slot's style in the library - and each was a
click that did nothing. `ui/ask.ts` asks in the page instead, and can say what is wrong
with an answer before it is accepted.

**`hidden` did not hide.** A class that sets `display: flex` outranks the browser's own
`[hidden] { display: none }`. The viewport's selection card is hidden by that attribute,
so it sat in the corner as an empty pill when nothing was selected and went on showing
the last selection after it was cleared. One rule at the top of the stylesheet makes the
attribute mean what it says everywhere; the same trap had already been patched once, for
the download bar alone.

**A settings form longer than the window could not be scrolled.** The two-pane screens
were as tall as their content, so neither half had a height to scroll within and the
Slurm form's lower fields and its Save button were below the bottom of the window.

**The window's own title was drawn over the screen's.** With an overlay title bar the
native title is still painted, centred, on top of the bar the app draws there.
`hiddenTitle` in `tauri.conf.json`.

Smaller, from the same pass: the outliner is as tall as what is in it until its divider
is dragged, rather than a fixed 190px that cut a short list off; a deposition's title in
the bar is one line with an ellipsis; a select is drawn like the fields beside it instead
of as a macOS pop-up button; opening a world clears the selection and fits its graph in
the pane, since node ids and the pane's pan are both shared between worlds; and a world
with no molecule says what to do next rather than "no chain connected" in red.

### The molecule as sequence
The BioMol panel was a map - chains as bars along one endless strip, residues as slivers
without letters, links as arcs - and it was replaced, because one chain of four fitted on
screen and nothing on it could be read. `ui/sequence.ts` is what every structure tool
settles on: each chain its sequence in one-letter codes, wrapped to the pane, tinted by
secondary structure and numbered every ten; what is not polymer as chips named for what
they are (`HEM`, not `E_1`); the non-backbone links as a list that says what they join.
Click, drag, shift-click and cmd-click select, through the same `View` everything else
uses, and a selection made in the viewport is lit and scrolled to here. Plain elements,
not a canvas. `ui/biomol.ts` and `ui/molmap/` - the map - are still in the tree with their
tests, unwired; delete them once the sequence view has been lived with.

### Who answers, and on whose account
Claude and ChatGPT are reached by running the provider's own command-line tool, which the
person has installed and signed in to. "Connected" is therefore two facts about that tool
- it is here, and it is signed in - and the Agents settings show both, read from the tool
(`claude auth status`, `codex login status`), with the install command or a button that
starts the tool's own browser sign-in when one is missing. This app has no login of its
own and stores no credential for either.

**This needs a decision before the app is given to anyone else.** Anthropic's Agent SDK
documentation says: "Unless previously approved, Anthropic does not allow third party
developers to offer claude.ai login or rate limits for their products ... Use the API key
authentication methods". An app that ships with "use your Claude subscription" as its way
of reaching Claude is at least close to what that sentence describes, whoever's computer
the tool runs on. For a release that leaves either asking Anthropic, or offering an API
key as the supported route - the adapters for which are still in `agents/mod.rs`, tested,
and seeded nowhere. OpenAI's position on Codex was not looked up.

### Known gaps in what is built
- `ssh` and `slurm` providers are written against the documented behaviour of `ssh`,
  `rsync`, `sbatch`, `squeue`, and `sacct` but have not been run against a real cluster.
  The local provider is covered end to end by tests; the remote ones are not.
- Job state lives only in SQLite, so a submitted Slurm job is orphaned if the app exits
  mid-run - `reap_interrupted_jobs` marks it failed rather than reattaching. Reattaching
  by scheduler id is worth doing once the world system needs long runs.
- A prediction started from a graph is filed under the world that was open; a job started
  from Settings -> Models still carries no `world_id`.
- The viewer has no selection language and no surface representation. Beta strands are
  drawn as flat ribbons without arrowheads.
- Cancelling a local job sends SIGHUP and nothing stronger - the pty layer's killer does
  no more - so a child that ignores it keeps its pty open and the job never settles.
- The Slurm log tail counts its offset in decoded characters, so an invalid UTF-8 byte in
  `slurm.out` makes it skip the two bytes after it.
- The element table holds thirty elements. Anything else a file names - mercury, cadmium,
  platinum - is drawn as an unknown atom: default radius, no colour of its own. It used to
  be guessed from the atom name's first letter, which was worse (see the audit below).
- Whether a B-factor column holds confidence is decided by what the file says about how
  it was made, then by range. A prediction that states an experimental method, or an
  experiment that states none, is still read the wrong way round.
- A lost WebGL context is recovered in the viewer screen, by making a new viewer on a new
  canvas. The worlds shelf does not do the same: its row stays blank until the screen is
  left and entered again.
- Claude and ChatGPT are offered through the subscription CLIs only. The key-based
  adapters remain in `agents/mod.rs` with their tests but nothing seeds a connection that
  uses them; `docs/TESTING.md` says what was checked on each route.
- The subscription route has no real tool protocol: the CLI is handed the whole
  conversation as text and asked for a fenced JSON block when it wants a tool, one call per
  reply, so a turn that needs three tools is three runs of the CLI. What it does while it
  runs is read from its own narration (`agents/cli.rs`): Claude Code streams, Codex does not.
- A local model gets a system prompt of its own (`localSystemPrompt` in `agent/tools.ts`):
  rules, a request-to-tool table and worked examples, where the hosted models get prose.
  It has not been run against a local model yet - none is installed on this machine.
- A ribbon's end caps reuse the last ring's vertices, so they are shaded with normals that
  lie in the cap's own plane.
- A Blender render blocks its IPC call rather than going through the job queue, so the UI
  shows a spinner instead of a progress log and cannot cancel it. Renders are seconds to
  minutes; a long one belongs in the queue.
- The tessellator drops to draft quality past four million triangles rather than streaming
  geometry, so a spacefill render of a very large assembly is coarser than a figure wants.
- The viewport has ambient occlusion but no shadows: a shadow map per light is the next
  step, and it is what still separates the preview from the render.
- Point and area lights are placed and scaled correctly for Blender, but the viewport
  shades every light as a sun. On a rig that uses one, the preview is brighter and flatter
  than the render.
- A render cannot differ from the viewport. It takes only the view, which is the point -
  but it does mean a figure lit differently from the preview is not expressible, and with
  one Viewport allowed per graph there is no second view to point at.
- One material for the whole scene: both renderers shade with a single set of surface
  uniforms, so a chain whose style asks for a different surface is drawn with the
  unassigned style's. The Molecule group reports this as a warning rather than applying
  half of what the node says.
- A style is copied into a slot or a node, not linked to a library entry, so editing it
  does not update the library or anything else using the same style. Blender's shared
  datablocks are the model; a slot is shared *within* a molecule, which is as far as this
  goes - two molecules cannot share one slot.
- Style slots belong to a molecule, so a chain cannot name a slot on a different molecule,
  and there is no scene-wide slot list.
- The properties panel has no search field, and the scene-wide tabs (Render, Output, World)
  hold one panel each rather than Blender's split of Sampling, Format and Output. The
  tabs, the breadcrumb, the pin, the slot list and the split field layout are there; the
  sub-panel hierarchy inside them is not.
- A library entry is read-only in the panel: it is changed by applying it to a node,
  editing there and saving back.
- A style network cannot mix or ramp colours - a `Colouring` or a `Colour` feeds a style
  directly. There is no Mix node, and no selection language to colour part of a chain.
- The node editor has no copy and paste, no multi-select, and no reroute nodes. Undo is
  the screen's, a stack of whole graphs - see `remember` in `ui/viewerScreen.ts`.
  Duplicate on the node's right-click menu covers the common case; links are still removed
  by pulling from the input they land on.
- Arrange is a one-shot layout, not a constraint: it reads the current rows to seed the
  ordering, then overwrites every position. There is no way to lay out a selection only.
- Only one Viewport output is allowed, so there is no way to compare two representations
  side by side in the preview. Render nodes have no such limit.
- A Render node's camera comes from the viewport, so a render whose geometry differs from
  what the viewport is showing can be framed for the wrong molecule. A camera node, or a
  frame-to-fit option on the Render node, is the fix.
- Lights are hidden from camera rays (`visible_camera = False`), because Cycles otherwise
  renders an area light as a visible rectangle and a point light as a sphere. This has not
  been observed in a render from this app - at four times the subject radius the lamps fall
  outside a tightly framed shot - but a lamp is not part of the subject and a closer light
  or a wider field of view would put it in frame.
- A reported "rectangular mesh" in renders was never reproduced. Ruled out by measurement:
  area and point lamps in frame (rendered with `visible_camera` on, no rectangle), the
  tessellator (ball-and-stick renders clean, and cylinders are eight-sided, not four), and
  flat shading (Blender 5.2 defaults `use_smooth` to true, `mesh.update()` creates the
  edges, and `normals_split_custom_set_from_vertices` applies with zero error - checked in
  a headless Blender). The remaining candidate is that the Image node drew at 55% opacity,
  showing the editor's grid through the picture, which is fixed.
- Aiming a light is the only object manipulation in the viewport. There is no transform on
  a node, so nothing else can be moved, and there is no gizmo - the feedback is the shading
  itself.
- A render blocks its IPC call rather than going through the job queue, so a long Cycles
  render cannot be watched or cancelled the way a prediction can.
- Secondary structure is read from the file. There is no DSSP-style assignment, so a model
  written without annotations renders entirely as coil - AlphaFold and the PDB both
  annotate, but a raw prediction from some tools will not.

## What belongs to a world

A world owns everything that is a session: its graph, its camera, its undo history, its
selection, the renders in flight, the jobs submitted from it (`jobs.world_id`), and the
agent conversation (`world_chat` / `world_chat_save`, stored under `world.chat.<id>` and
removed with the world). Opening a world swaps all of them; a turn still running when the
world is left finishes into the world it was asked in, draws nothing in the new one, and
has its tool calls refused. The viewer's bar carries the world's name. Only the pane
layout and the settings - agents, compute, models - are the app's rather than a world's.

Settings is one screen with a rail: Agents (the roster, each with whether it could answer
now, sign-in, and which one is in use), Compute, Models, Jobs (all worlds, filterable).

### Subscription first, key as an option

Claude and ChatGPT are seeded as subscription connections: Geis runs the provider's own
command-line tool, unmodified, as the user installed it, and sign-in completes in that
tool's own flow - Geis never holds a password or a token, and usage is billed to the user.
That is the shape Anthropic's "Can customers offer Claude Code in their products?" terms
describe, and the one other tools that survived the 2026 enforcement (Cline, Roo, Zed) use;
implementing claude.ai OAuth in the app is what was blocked. Key-based connections
(`claude-api`, `openai-api`) are not seeded but can be added and removed under Settings ->
Agents, for people without a subscription or if `claude -p` use is metered separately.

## Graph view (demo)

A mode of the viewport, not a pane: `Graph` in the bar under it. The structure is drawn as
usual and composited faded towards the background (`uFade` in the composite), then the
graph is drawn in a second trip through the same G-buffer and blended over it, so it is
seen through the ghost. One sphere per chemical component (`MapChain.centers`/`kinds`: CA
for an amino acid, C4' for a nucleotide, the centroid otherwise), one stick per bond
between components (`StructureMap.backbone` and `links`). Sphere ids are residue ids in
the scene's id space, so hover, click and the selection outline use the residue mask and
`pick_target` unchanged. Layout is `src/viewer/graphView.ts`. Not yet: only-wired chains,
labels on the spheres, edges that can be picked.

The graph's spheres carry a label - a residue's one-letter code, a ligand's component id -
drawn in the sphere shader from a glyph atlas (`packLabel`, `aLabel`, unit 5), so letters
are occluded like the spheres they are on. Modes follow Blender's 3D view, read from its
source: a row of icon-only mode buttons at the right end of the viewport's header (where the
3D view has `row.prop(shading, "type", text="", expand=True)`), and Z - its shading pie -
for the mode pie at the pointer: hold Z, move towards an item, let go; or tap Z and click
(first item west, second east). Ctrl+Tab opens the same pie and Alt+Z flips. A
right click that does not travel opens the viewport's menu (mode, remove/show solvent,
ligands, hydrogens - written to every molecule node's options - clear selection, frame,
save image); one that travels is still the pan. The graph leaves out the waters and
ligands the molecule's options leave out.

What a click selects is three icon buttons at the left end of the viewport's header and
1, 2, 3 on the keyboard - atom, residue, chain, finest first - after Blender's edit mode
(`mesh.select_mode` on the number row). The pane layout is a world's too
(`world_layout` / `world_layout_save`, key `layout:<id>`): a world made now is saved as
`default` and opens on the viewport alone; one from before gets the arrangement the app
had, once, and keeps its own from then on. With no node graph on a new world's screen,
fetching and opening a file are also in the viewport's right-click menu.

A bond of the graph can be pointed at and picked: the graph's sticks carry their own index
as a pick id (tag 3, `Picked.kind === "edge"`), and `GraphView.edgeInfo` says what each is
- peptide or phosphodiester bond, disulfide bridge, metal coordination, covalent link -
which two components it joins, and how far apart their centres are. Hover reads it out;
a click recolours the bond, selects its two components, and puts a card on the viewport.

### The web, and pictures from it

The subscription CLIs keep one thing of their own: the web. Claude Code is run with
`--tools WebSearch,WebFetch --allowedTools WebSearch,WebFetch` (still no shell and no
files) and Codex with `--search`; a connection saved with the earlier, offline arguments
is brought up to these unless it was edited. The local model has no web and the prompt
tells it to say so. `show_image {url}` is this app's half: `fetch_image` downloads an
https image into `cache/images`, it becomes an Image node in the world, and a pane opens
on it in the right half of the desk.

### Nucleic acids

A strand's cartoon is a round backbone tube (0.5 A) through the phosphates, and for each
nucleotide a rod from the backbone to where the base attaches (N9 of a purine, N1 of a
pyrimidine) and the base as a flat slab over its ring atoms - a hexagon, plus a pentagon
for a purine (`cartoon::add_base`). Under the secondary-structure scheme a nucleotide is
coloured by its base (A red, T blue, U cyan, G green, C yellow), since it has no helix or
sheet to be coloured by. Two things in `Structure::build` keep a strand whole: a ligand
with a sugar whose phosphate or O3' is bonded to a neighbouring nucleotide is adopted as a
nucleotide (the modified bases of a tRNA), and a nucleotide with no phosphate - the 5' end
- is traced through its O5'.

### More for the agent, and more pies

A subscription connection carries a chosen model and effort (`Connection.choice`,
`.effort`), passed to the CLI as `--model`/`--effort` (Claude) or `--model`/
`-c model_reasoning_effort` (Codex) and picked in the composer. The agent's tools now
cover what the screen can do: `list_links`, `list_ligands`, `measure_distance`,
`set_view_mode`, `set_select_level`, `focus`, `set_world`, `set_light`,
`remove_molecule`, `open_file`, `show_pane`, `render`, `save_viewport_image`, `undo`,
`redo`, beside `show_image`. Pies are one to a key, as Blender has them, not nested: Z
mode, S show as, C colour by - S and C restyle every slot of every molecule.

### Extensions

A manifest with `kind = "extension"` and an `[install]` section is a tool the user adds
when they need it, installed by this app on whichever provider it is to run on. Installing
and removing are jobs (`Manifest::maintenance`): the same manifest with its install
commands where its run commands were, submitted through the queue, so they are logged,
cancellable and work over ssh and slurm exactly as a run does. Everything goes under
`{{ext.dir}}` - `$HOME/.geis/extensions/<id>` on the executing host. Whether it is
installed on a provider is read off the job history (`extension_states`): the last install
or removal there and how it ended - this app's record of what it did, not an inspection of
the host. Settings -> Extensions lists models and extensions together; an extension's card
has a row per provider with Install / Reinstall / Remove / Log. Shipped: `tmscore`
(US-align, compiled on install) and `pyrosetta` (venv + pyrosetta-installer). Not yet:
nodes or agent tools provided by an extension, a registry to fetch manifests from, a
check that inspects the host.

### Action after selection

What an action changes is the selection, chosen first: `actionTarget` is the selection
as the chains it lies in, shown at all times in the viewport's header ("Acts on chain
A_1", or "everything"), and in the title of the S and C pies. A targeted chain gets a
style slot of its own if it shared one. A style is still a chain's, so a selection of
residues acts on their chains and the readout says so. Select level 4 is Link: it opens
the graph view and makes a click land on bonds only (`Viewer::setGraphPick`).

### Preferences

Settings -> Preferences holds defaults for what is made next, never state of what exists:
the categorical palette (Okabe-Ito, Tol Bright, Tol Muted, Tableau 10, ColorBrewer Set2,
Vivid - `geometry/palette.rs`, mirrored for swatches in `src/prefs.ts`), a new world's
background and layout, a new molecule's representation, scheme and which atoms it draws,
the select level a world opens at, and which method each analysis uses (align: Kabsch or
TM-align; over which atoms; where secondary structure comes from). Stored as one JSON
object under the `preferences` setting; `mergePreferences` lays it over the defaults so an
older save reads a newer field as its default. The method choices are recorded ahead of
the features that will read them.

### BioMol features (stage 1 of BioMol-as-source)

The decision: a molecule's appearance belongs to the BioMol - features on chains and
residues - with the viewport's own data (camera, background, lights, mode) kept apart, and
the node graph one editor of it. What exists so far is the residue level: `Feature
{chain, from, to, show, hide, colour}` on the molecule (`NodeKind::Molecule.features`),
carried through `PlanPart`/`ScenePart` to `build_scene_with`, which lays features over the
chain styles - extra representations drawn whole, a colour written into every colouring,
hidden atoms skipped. Made by the S and C pies on a residue selection, by `style_residues`,
cleared from the viewport menu. Not yet: chain styles are still the graph's slots rather
than chain-level features, the feature list still rides inside the graph's molecule node
rather than in a `biomols` block of the world, and the viewport's data is still the World
node and `World.camera`. Moving those is the rest of the inversion.

### Three chains: S, A, V

The viewport's pies are three hierarchical chains, one key each: **S**election (a
molecule, then a chain of it or the whole; or everything; or what a click selects),
**A**ction on what is selected (Show, Colour, Find, Hide, Focus - Show and Colour act on
the target's chains, or as residue features when the selection is finer), and **V**iew
(Mode, Frame, Contents, Panes). An item with more under it carries `next` and opens the
next pie where the first was; past eight items the last place is "More". Z, C and
Ctrl+Tab no longer open pies.

Colour preferences are per way of colouring: the categorical palette (chains), the element
table (Geis / CPK / PyMOL) and the secondary-structure set (soft / classic / cool) - these
two are app-wide (`palette::set_theme`, applied when preferences are read or saved) - and
the gradient new styles use. On the shelf, a world's molecules are placed together by the
box around all of them (`Scene::place_within`), not each centred and scaled on its own.

The outliner's Scene is the molecules; lights, outputs and the world sit in one folded
branch below it ("Viewport and render"). An action on chains keeps chains that shared a
slot sharing one, and `tidySlots` then merges slots that draw alike, drops slots no chain
uses and names made slots for what they look like. What is sent to a model is shortened by
`agent/compact.ts` to the connection's room (45k characters for the local model's 32,768
tokens, 400k otherwise): old tool results emptied first, then whole early exchanges
dropped; what the world keeps is untouched.

### The local model's window, and what it is sent

The engine's context is chosen when it starts (`engine::choose_context`): the widest of
32,768 / 24,576 / 16,384 / 12,288 / 8,192 tokens at which the weights plus the key-value
cache stay within 55% of the machine's memory and within what the model was trained on.
The cache's cost per token is read from the GGUF header (`layers x embedding/heads x
kv_heads x 2 x 2 bytes`): 135 kB for Qwen3.8 9B, so 4.4 GB at 32,768 - which with 5.8 GB
of weights does not fit a 16 GB machine, and is why that machine gets 24,576. A local
connection is given `LOCAL_TOOLS` (14 of the tools) and a prompt naming only those, about
5,300 tokens against the full set's 9,000; the conversation's room is the window less that
and a reply.

### The left column, and cameras

The outliner is the scene - its molecules and their chains, under the panel's one
"Scene" title - and nothing else. The panel below keeps its tabs (Render, Output, World,
Structure, Chain, Style, Light); what the outliner's branches used to reach is in them:
the lights under World, the cameras under Render. The main camera is the view itself and
follows it; a made camera stays put, and is left the moment the view is moved. A chain's panel shows its style as
fields (`chainStylePanels`); editing one gives the chain its own slot first if it shared
one. Cameras are kept per world (`world_view` / `world_view_save`, key `view:<id>`): made
from the current view, looked through from the panel, the View chain or `0`, moved to the
view, renamed, removed.

A conversation nearing its room is summarised before the turn (`splitForSummary`,
`SUMMARY_SYSTEM`): the early exchanges are put to the model as a transcript, and its
summary replaces them in the kept conversation too, shown folded in the panel. The
dropping in `compact` stays as the fallback. Select levels are five, finest first - atom,
residue, chain, molecule, bond (keys 1-5) - each with a monochrome mark from
`ui/icons.ts`, which the scene list uses too. "Bond" is a bond between two components:
backbone, disulfide, metal contact or covalent link; not a bond between atoms of one.

### Commands, shortcuts, contacts, skills

`src/commands.ts` names what the viewport can be told to do; `RUN` in the viewer screen is
what each name does. The right-click menu is the list of names in
`preferences.contextMenu` (default: show sticks, zoom all, fetch, save image), chosen in
Settings -> Preferences. Polar contacts and salt bridges are found from geometry
(`structure/contacts.rs`: N/O pairs within 3.5 A, carboxylate-to-charged-nitrogen within
4.0 A, never within a residue or between sequence neighbours, waters left out) around the
selection or through everything, and drawn as dashed lines by a cylinder pass of their own
(`Viewer::setContacts`). Choosing a select level never changes the mode; the level in use
is named under its buttons; the target readout is top centre.

The agent's Clear moves the conversation into a bin (`tmp/chats`, one file a clear, the
oldest emptied past 20 MB) and Undo clear takes the newest back. `/` in the agent's box
lists skills: `/clear` and `/compact` act; the rest are instructions sent as the message -
four built in, and the user's own as `.md` files in the app's `skills` folder
(`agent_skills`).

### Show, hide, and the settings over a world

`show.*` commands add a representation to what is drawn and `hide.*` take one away
(PyMOL's `show` and `hide`); `as.*` and the Action chain's "Show as" replace. The
right-click menu always ends in Preferences, which opens the settings as a window over the
world (`openOverlay` in `main.ts`, on the `geis:settings` event): fixed in the middle, the
world dimmed behind it, closed with Esc or a press outside. The viewer's bar and the agent
panel's Settings open the same window.

Contacts follow published criteria (`structure/contacts.rs`): hydrogen bonds by HBPLUS's
heavy-atom rule (donor-acceptor within 3.5 A, both antecedent angles at least 90 degrees,
donor and acceptor roles by residue and atom name), salt bridges within 4.0 A, pi stacking
and hydrophobic contacts by PLIP's thresholds, and the hydrophobic core as hydrophobic
residues with 20 or more beta carbons within 10 A.

The world background is sRGB (`hexToSrgb`), not linear: it used to be run through
`hexToUnit` and so converted twice, arriving darker than the swatch picked. Defaults are
quieter - secondary-structure colours about half way to grey, a "Soft" categorical
palette, a background of #d5d8dd - and a saved preference or a world still holding an old
default follows the new one.

### Objects

A selection can be kept under a name (`NodeKind::Molecule.objects`: a name and runs of
residues by chain and author number) - PyMOL's named selections. Made from the Action
chain or the `object.make` command, listed under its molecule in the scene, selected again
with a click and then acted on like any selection, removed with its cross. The core only
carries them. `selectedRuns` reads the residues of a selection at whatever level it was
made; it used to read them only from an atom view's summary, so an action on picked
residues - the hydrophobic core, coloured - found none and did nothing.

Colours are shown and taken by name (`ui/colourNames.ts`): a pie item is the nearest name
to its colour, not its code, and Custom - at the bottom of both colour pies - reads a name
or a hex code. A kept selection has its own mark (`object` in `ui/icons.ts`).

The hydrophobic core is by relative solvent accessibility (`contacts::accessible_surface`,
Shrake-Rupley with a 1.4 A probe; hydrophobic residues showing a fifth or less of their
Tien et al. maximum), not a neighbour count - a fixed count missed ubiquitin's Leu43 by
one. The properties panel goes to the selection's own tab only when the selection changes;
asked on every redraw it undid every click on Render or World while a chain was selected.
"Save image" asks which picture: the render (none yet, current, or older than the scene -
told by `sceneStamp`), a fresh render, or the viewport as it is.

## The release pass (0.0.0)

What was added so that the app behaves like a finished one rather than a working one.

- **Commands by name.** `src/commands.ts` is the one list of what the viewport can be told
  to do; `viewerScreen`'s `RUN` table is what each does. The right-click menu, the pies,
  the command palette (`src/ui/palette.ts`, `⌘K` or F3) and the application menu all run
  entries of that table, so none of them can offer something the others cannot. The
  palette's search is a pure function (`rankItems`) with its own tests.
- **The application menu** (`src-tauri/src/menu.rs`). Items that are the app's own are sent
  to the window as a `menu` event carrying an id; `main.ts` routes Settings itself and
  hands the rest to the open world as `geis:command`. Undo and Redo are the app's items,
  not the system's: with a text field focused they are passed to it, otherwise they step
  the scene's history. A key can arrive both as a page key event and as a menu item, so
  the screen's `fire` drops a repeat of the same id within 150 ms.
- **The window's frame** (`src-tauri/src/window_frame.rs`) is written as it changes and
  restored at launch only if its title bar would land on an attached screen.
- **Opening files.** `⌘O`, the empty world's buttons, the menu, and a file dropped on the
  window all end in `openPath`. Drops come from the webview's own drag-and-drop event,
  because a page `drop` event never fires in this webview and a `File` has no path.
- **Asking first.** Deleting a world that holds molecules, removing a provider, a local
  model or a library style goes through `askConfirm` (`src/ui/ask.ts`), with Cancel
  focused. Things that can be undone are not asked about.
- **Failures are said.** After boot, an uncaught error or rejected promise shows a toast
  (`src/ui/toast.ts`) instead of going only to the log.
- **Content security policy.** The release build serves the page under a policy that
  allows scripts only from the bundle and network access only to the core's IPC; the
  page itself fetches nothing. Development keeps no policy (`devCsp`), since Vite's
  reload needs inline scripts and a websocket.
- **Input methods.** Enter that commits a Hangul or Kana candidate is not Enter:
  `composing()` in `src/ui/dom.ts` checks both `isComposing` and key code 229, which is
  how WebKit marks the commit.
- **Reading saved data that this build does not understand.** A graph node of an unknown
  type is kept aside (`Graph.unread`) and written back, rather than failing the whole
  world; preferences drop unknown entries one at a time rather than resetting.
- **Checks.** `cargo fmt`, `cargo clippy -D warnings`, `eslint` with type-aware rules
  (no floating promises), `ruff` and `pyright` for the Blender script. See the README.

## The view in the undo history

Undo covers moves of the view as well as edits, in the order they were made. The view is
moved from many places - a drag, the wheel, Zoom, a camera looked through, the agent - so
none of them reports it: `viewerScreen` looks at the camera five times a second, and when
it has moved and then been still for two looks, where it rested before becomes a step
(`{ view }` in the same stacks as the graph snapshots). A whole drag is one step. A world
being opened is framed and then put where it was left; neither is the user's move, so
nothing counts until the view has been still for a second after either.

A pie opened by S, A or V stays up when the key is tapped, whatever the pointer did in
that moment; only a key held past `PIE_TAP_MS` with the pointer aimed at an item picks on
release. The same key pressed again puts the pie away.

## The graph view's colours and found contacts

A component's sphere is the colour its residue is drawn in on screen, read off the scene
itself (`residueColours` in `src/viewer/graphView.ts`): the ribbon's colour where there is
one, otherwise the colour most of the residue's atoms share. `StructureMap` carries each
residue's atom range for that. A component hidden in the structure view keeps the graph's
own chain colour.

What Find finds is kept two ways (`drawContacts` in `viewerScreen`): dashed lines between
atoms for the structure view, and edges between components for the graph - one edge per
pair of components, with the number of contacts and the closest of them on its card. They
are edges like any other: with the level on Bond they can be picked.

## Preset analyses

What is done with a structure once it is open is much the same every time, so it is done
without being asked. As a structure first comes on screen `viewerScreen.analyse` starts
eight analyses of it, each its own `structure_preset` call (`commands.rs`), each off the
main thread: four kinds of contact (hydrogen bonds, salt bridges, pi stacking, hydrophobic
contacts) and four sets of residues (hydrophobic core, binding site, interface between
chains, ligands). They appear under the molecule in the scene list as a `Groups` branch:
a set of residues is selected by a click, a set of contacts is shown or hidden by one,
and several kinds can be shown at once, each in its colour. The Find commands no longer
compute anything; they show, hide or select these groups.

Cost, release build, this machine: all eight take 5 ms for 660 atoms, about 80 ms for
24,000, about 190 ms for 58,000 (1UF2). The buried-surface search was nine tenths of it
on one core and is shared out across cores (`accessible_surface`).

Progress is said in two places: the `Groups` row reads "analysing n/8" with a spinner, and
the viewport carries a line per structure still being analysed. The same line covers the
steps before it - "Reading file", "Fetching from the PDB", "Drawing" - the first of which
starts at once and the others only when an evaluation has run for a quarter of a second.
Commands that read a file or build a structure's map are `async` and do their work with
`spawn_blocking`: a command that is not `async` runs on the main thread, and the window
cannot draw its own progress while one does.

## The biological assembly

An entry fetched by id is shown as its first biological assembly - the molecule as it
works - unless the Fetch dialog's "Load the biological assembly" box is unticked. The
choice is kept on the molecule node (`assembly`) and remembered for the next fetch.

The assembly is built from the entry's own file, StructCooker's way
(`vendor/StructCooker`, `instructions/transforms/cif.py`: `parse_expression`,
`parse_assembly_dict`, `get_struct_oper`, `build_assembly_dict`), ported to the Rust
reader as `cif::operators_of` and `cif::build_assembly`:

- `_pdbx_struct_assembly_gen` says which chains the assembly holds and by which operators;
  an operator expression is a list, a range, or both; a product of two groups,
  `(X0)(1-60)`, is skipped, as StructCooker skips it.
- `_pdbx_struct_oper_list` gives each operator as a rotation and a translation; a chain is
  copied once per operator and moved by it.
- A copy is named `asym_id_oper_id` - `A_1`, `A_2`. Chains the assembly does not name are
  not in it. `_struct_conn` records are applied in every copy, between ends made by the
  same operator. A file that states no assembly is its deposited chains under operator 1.

Checked against the archive's ready-made assembly files: for 1STP, 4HHB and 1UBQ every
atom lands in the same place. Nothing extra is downloaded. StructCooker and DataCooker
themselves are Python (ray, lmdb, numpy 2, Python 3.10 or later) and are not run by the
app; they are the reference this follows, kept in `vendor/` for reading.

A capsid is the hard case. 1UF2's assembly is sixty copies: 3,487,800 atoms, 446,040
residues, 960 chains. It is built in about five seconds and takes about a gigabyte. So
that it can be drawn, the ribbon is thinned as the residue count rises
(`cartoon::Detail::for_residue_count`): past 120,000 residues it is two subdivisions and a
six-point profile, which for 1UF2 is 10.7 million triangles instead of over a hundred
million. A file opened from disk is shown as written unless its node says otherwise.

## Large assemblies: what went wrong, and the limits now

A world holding 1UF2's capsid twice killed the window. Opening the shelf of worlds built
a preview of it, and the preview's ribbon detail was a fixed setting that stood in for the
size-based one instead of below it: 64 million triangles and two gigabytes per capsid,
four for the world, against 326 MB for the same capsid in the viewport. The core reached
9.5 GB building it and the web content process 7.9 GB receiving it, and macOS ended the
web content process. Now `cartoon::Detail::for_scene` never gives a preview more detail
than the full view, and past 30,000 residues a preview is a square tube with one segment
a residue: 108 MB for the capsid.

The preset analyses were the second problem. A capsid's hydrogen bonds and hydrophobic
contacts are about 900,000 records, which as JSON with atom names was 160 MB to parse in
the window. They are now packed (`PresetResult::encode`, 36 bytes a contact, decoded by
`viewer/presets.ts` into flat arrays) and run one at a time across all structures, quick
ones first. And they were matched to the structures on screen by object identity, which
every evaluation renews, so after a redraw a structure looked closed: its remaining
analyses were skipped and the progress line stopped. They are matched by id.

Measured with the user's own world (1UBQ, two 1UF2 capsids, 1M4X), shelf then world: the
core peaks at 2.6 GB, the web content process at 3.0 GB and settles at 2.7; all analyses
finish in about 40 seconds.

## Deleting, and the scene list's menu

Delete follows Blender's `object.delete`: the Delete key deletes the selection at once,
X asks first with one menu item under the pointer, and both are undone by ⌘Z. A molecule
is what a world holds, so a selection that is a whole molecule takes it out of the world
(`removeMolecules`, shared with the agent's `remove_molecule`); part of one cannot be taken
out of its file and is hidden instead, which the line under the viewport says. Clear all
empties the scene - every molecule, what was shown of them, the selection - as one step.

The scene list answers a right-click the way Blender's Outliner does: a molecule, chain,
object or group row opens a menu of what can be done to that thing - select, zoom to,
show, hide, show as, colour by, its groups, make an object of it, delete. Each entry
selects the thing first and then runs what the viewport runs on a selection, so the two
cannot mean different things.

Navigation differs from Blender's on one point: left drag orbits, where Blender needs the
middle button (or its opt-in Alt emulation); shift-drag and
middle or right drag move, the wheel zooms, and Control-drag zooms too (Blender's
Ctrl with the middle button), for a mouse without a wheel.

## Round helices in a capsid, and the analyses in parallel

A helix turns once in 3.6 residues. The thinning that kept a capsid inside memory sampled
every residue twice with a six-sided cross-section, so its helices were polygons of seven
points a turn. A helix now has its own sampling (`Detail::helix_subdivisions`: five a
residue past 120,000 residues, six below), and the profile there is eight-sided. The detail
is chosen for the whole scene's residues (`cartoon::build_among`, from `build_scene`), so
two capsids are drawn at what two can afford rather than each at what one can. To pay for
it the mesh goes to the window with a byte a channel for normals and colours (wire version
2, `push_snorm8` / `push_unorm8`, read by WebGL as normalised bytes): 24 bytes a vertex
instead of 40. 1UF2's capsid is 23.5 million triangles and 538 MB, against 10.7 million
and 326 MB with polygonal helices; one capsid on screen holds the web content process at
about 1.6 GB.

The real fix is to draw an assembly as its asymmetric unit sixty times over, with a
transform each, as Mol* does: full detail at a sixtieth of the memory. That changes what a
copy can be (one style and one colouring for all copies of a chain), and is not done yet.

The analyses: the pair search behind every kind of contact is shared across cores; pi
stacking compares a ring with the rings in neighbouring grid cells instead of with every
ring; the presets skip formatting the two ends' names, which the window never reads; the
bond list is built flat instead of as a list per atom; and the core measures only the
hydrophobic residues' atoms. On 1UF2's capsid all eight take 4.4 s instead of 15.4, with
the same counts.

## Saving pictures, and the shelf's pictures

A picture the size of the screen is one sample a pixel, and a ribbon thinner than a pixel
comes and goes along its length: a capsid saved that way is speckle at any size. So the
size of the file and how clean it is are separate choices. `Viewer.capturePicture` draws
the view four times larger each way (lowered as far as the graphics card needs), with the
pixel-sized effects - outline, occlusion blur, the antialiasing nudge - scaled to match
(`pixelScale`, applied to the composite's `uTexel`), and halves it down to the size asked
for. Save image offers sizes (as on screen, twice, four times, the Render node's width);
each is supersampled.

Saving, and rendering, show a gauge (`ui/progress.ts`): the steps of a viewport save, and
for a Blender render the sample count Blender reports - the render script hangs a
`render_stats` handler that prints `[progress]` lines, `render::render_reporting` reads
Blender's output as it arrives, and `render:progress` events carry the fraction and stage
to the window.

A Render node renders into one file of its own: rendering again replaces it, as Blender's
Render Result is replaced, instead of adding untitled-1, untitled-2 and so on. A file the
user named is never written over unasked. An image shown in a pane carries the time it was
written in its address, so a picture rendered again to the same file is not shown from
the cache.

Each world has a picture (`thumbnails/<id>.png`): taken as the world is left, and taken on
the shelf for a world that has none, once the 3D row is up and resting on it. The shelf
shows the pictures at once - the chosen world in the middle, its neighbours either side -
and fades to the 3D row when that has been built. Nothing on the shelf says "loading".

The progress of a save or render is not a dialog: it is a task on the status bar at the
foot of the window (`showProgress`), as Blender shows a render, and a render's task has a
cross that stops Blender (`render_cancel`, which wakes `render::CANCEL`). The left of the
same bar says what happened - a file saved, a structure being opened or analysed
(`setStatusNotes`); nothing is written over the viewport for it.

The save panel sets the size by dragging, with the longest side read out in pixels and in
angstroms per pixel, and "Match the zoom" picks the size at which the view as zoomed now is
drawn at a fixed number of pixels per angstrom. "Export a 3D model for slides (.glb)"
writes the scene's triangles as glTF 2.0 binary with vertex colours (`geometry::gltf`):
PowerPoint's 3D Models insert reads `.glb`, and keeps per-vertex colour, which an `.obj`
or `.ply` would lose.

## Analyses on the graphics card

The analyses' cost is one question asked of every atom: what is near it. `gpu.rs` asks it
on the graphics card with wgpu (Metal): the atoms are sorted into a grid on the processor,
and a WGSL compute shader runs a thread per atom over the 27 cells around it - for the
buried surface, each atom's 96 probe points against its neighbours; for contacts, every
pair within the cutoff, appended to a buffer through an atomic counter (and run again with
a buffer the counted size if the first guess was short). What makes a pair a hydrogen bond
or a salt bridge - its roles, its angles - is still decided on the processor from that
list. With no adapter, or anything failing, the same work runs on the processor, and
`GEIS_NO_GPU` forces that; `tests/gpu_parity.rs` runs every preset both ways on 1UBQ, 4HHB
and the 1UF2 capsid and the two agree exactly. On the capsid the neighbour searches take
less than half the time.

A structure's eight analyses now run at once rather than in turn; structures still take
turns, so two capsids are not in memory twice at the same moment. On the capsid all eight
are in about a second and a half after it is drawn.

## Colouring many chains

A scene used to compute a colour array the size of the whole structure for each distinct
colouring, and build the ribbon once per colouring - each pass walking every chain and
re-orienting every triangle so far. Colour a capsid's 960 chains each its own colour and
that was 960 of each, one after another. Now each scheme is computed once, solid colours
need no array, and the atoms' colours are merged into one array the ribbon is built from
in a single pass. That pass is shared between the processor's cores by residues, each core
building its own piece of mesh with room reserved up front, joined in order - the same
triangles in the same order. The capsid coloured chain by chain builds in 0.3 s at full
detail (`tests/scene_timing.rs`).

## The file viewer

What was the Image pane is the file viewer: a render, or any file opened into it or dropped
on the window that is not a structure. Pictures are shown as they are; PDFs are drawn page
by page by pdf.js (the webview shows a blank frame for a PDF in a frame), loaded the first
time one is opened; Word, RTF and OpenDocument text are turned into a page by macOS's
`textutil` and shown in a frame that runs nothing; PowerPoint, Excel, Keynote and the like
get Quick Look's picture of their first page (`qlmanage -t`), kept per file version; text
is shown as text, the first megabyte of it. Anything else, and any of these, can be handed
to the application it belongs to.

## The top bar

Laid out as VS Code's title bar: back and the world's name on the left, the command search
in the middle (⌘K), and icons for Save image and Layout on the right. Settings are the gear
at the foot of the activity bar. The open structure's title, which ran across the middle of
the bar, is the world name's tooltip. The select level's name appears under its buttons only
while the pointer is on one, or for a moment after a key changes it.

## Assemblies, models and alternate locations

An entry is read as one version of itself (`structure::Reading`): which assembly, which
model, which alternate locations. They are three different things and are kept apart.

- **Assembly**: which molecules, how many copies. Every assembly in
  `_pdbx_struct_assembly` can be chosen, not only the first, and a product expression such
  as `(X0)(1-10,26-30)` is built - every operator of the second group followed by every one
  of the first, the last applied first, as the PDBx dictionary defines it. 1UF2's six
  assemblies come out with exactly the chain counts the file declares (960, 16, 80, 96, 16,
  240), and no two atoms of different chains closer than 1.5 Å in any of them
  (`tests/assembly_clash.rs`).
- **Model**: one whole state of the molecule - an NMR ensemble's members. One is read; the
  others are listed and can be chosen.
- **Alternate location**: a local doubt, a side chain seen in two places. Chosen residue by
  residue: the label asked for where the residue has it, otherwise its most occupied
  conformer (as ChimeraX does) - not the first label seen, and not one label for the whole
  entry.

StructCooker files each (assembly, model, altloc) as a whole structure of its own, which
suits a training set; a molecule being looked at is one molecule with versions. The
molecule node carries the choice as `version` (`assembly_id`, `model`, `alternate`), and
the scene list shows it in a row under the molecule that opens a menu of what the file
offers. Switching keeps the chain nodes of chains that are still there, with their styles,
removes the rest, adds the new ones, and frames the camera afresh.

## Work that was done in turn

Found by timing each stage of opening the 1UF2 capsid, and fixed:

- Secondary structure: each helix and strand range compared chain names for every residue
  of the structure; matched once per chain now. With the bond hash set replaced by a sort
  at the end, the bond templates shared between cores, and no strings allocated per atom
  while residues are grouped, reading the assembly went from 5.0 s on one core to 1.6 s.
- A scene's molecules are built at once, off the async runtime; a world's molecules, and
  the shelf's worlds, are opened at once rather than one after another.

## The cell, and the worlds' place

The home screen and the shelf are drawn live behind the page (`ui/cellStage.ts`,
`viewer/cellRenderer.ts`), in two places.

- **The cell, behind the home screen.** Real molecules - ubiquitin, lysozyme, GFP,
  haemoglobin, an antibody, growth hormone with its receptor, GroEL-GroES, the
  SARS-CoV-2 spike, a tRNA - each read as its biological assembly by the app's own reader
  and reduced to one bead per residue (`src-tauri/examples/cell_assets.rs` writes
  `src/assets/cell/molecules.json`; the coordinates are CC0), ~1,500 copies drifting and
  turning, and actin filaments built from the monomer on the filament's helix (27.5 A rise,
  -166.7 degree twist). Nucleic acids too: nucleosomes (1KX5) and 70S ribosomes (4YBB)
  from the same reader, and DNA and messenger RNA laid along smooth random curves - one
  turn of B-DNA (ten pairs, two backbones 144 degrees apart) or ten nucleotides of RNA a
  piece, each turned from the last by the bend alone (parallel transport), so a helix runs
  on unbroken. Chromatin is beads on a string, a nucleosome every eighth turn; polysomes
  are ribosomes strung along a message. DNA is amber and RNA rose, the warm few among cool
  proteins. Every bead of every copy is one instanced draw of sphere impostors;
  what moves each frame is each copy's place and rotation, eight floats in a float texture.
  The look is the old Blender render's: lit mostly from behind by a blue glow deep in
  the cell (the worlds' place shares it, so the tone holds on the way through), colours mostly teal and blue with
  a few warm molecules, beads swollen into soft lumps, haze, depth of field, a vignette and
  a little grain. The camera wanders on a path kept clear of molecules, with the space
  ahead of it clear for the title.
- **Small molecules and ions.** ATP, ADP, NAD+, haem, glucose and S-adenosylmethionine,
  one bead per heavy atom from the Chemical Component Dictionary's ideal coordinates
  (`cell_assets.rs` reads `<ID>_ideal.sdf` beside the entries), grouped and coloured by
  element; and potassium, sodium, chloride, magnesium and calcium ions, single beads the
  glass swells into droplets. Three hundred-odd molecules and four hundred-odd ions,
  tumbling faster than the proteins. Their beads are marked (a negative radius) so the
  glass view keeps them near the camera, where a protein's give way to its ribbon.
- **Glass.** The cell's molecules are always drawn as glass (the opaque Goodsell look is
  still in the renderer, but nothing asks for it; the button that switched is gone): each
  a clear shell with its secondary structure inside. The glass is the viewport's own
  (`GLASS_GLSL` in `viewer/glass.ts`): what is inside bent along the shell's slope and
  tinted by the molecule's colour, its surroundings reflected by Schlick's Fresnel, two
  highlights and a bright edge. Only the surroundings differ - the cell reflects its own
  glow, never darker than what is behind (a dark floor reflected in each turned-away
  lump was a dark patch on it) - and, because the shell is swollen beads, lumpy under its
  smoothing, it is bent half as far and its highlights are a third as strong, or every
  lump shows as a bubble with a glint. What is seen unbent is gathered only where it is
  needed (off the glass, at its edge, where a chain is in front): doing both gathers on
  every pixel of glass cost six frames a second. Inside, the copies within 50 units of the camera are
  drawn as ribbons (`viewer/cellRibbon.ts`): from each molecule's trace atoms and
  secondary structure, which `cell_assets.rs` writes beside its beads, by the viewport
  cartoon's rules cut down. A helix is a helix about its own axis (the mean of a turn's
  worth of alpha carbons, smoothed along the run), eased in from the spline and out to
  it at its ends; a spline through alpha carbons 100 degrees apart is a rounded triangle,
  and under glass every helix looked faceted. The face points out from a helix's axis
  and across a strand's pleat, each made to agree with the one before (a helix matched
  once, where it starts), so a section never turns a half turn between residues; the
  breadth is blended over four residues by the B-spline, as the cartoon's is. Built once
  when glass is first shown, at two details - ten points round and three to five
  sections a residue within 28 units, six and two to three beyond (the fine ones
  everywhere cost a quarter of the frame rate) - and drawn instanced, template by
  template, for the near copies only. Further copies keep small vivid beads, where the
  ribbons would be a blur anyway. The shell is the beads swollen to 2.8 times into a
  half-size target that keeps only the nearest surface's normal, tint and depth. The
  compose pass smooths that normal over a disc sized to the distance - wide enough that
  the lumps where beads meet read as one shell rather than a bunch of grapes - and lays
  the shell over the chain: clear where it faces the eye, bright towards its edge, with a
  highlight. Its edge is how much of a ring of taps round the pixel is shell, and its
  tint and normal are read from a tap that is: read at the pixel alone, the half-size
  target's edge was a staircase two pixels a step, darkened where the pixel itself had
  missed. Half size is what keeps it at frame rate - a full-size shell was 29 fps against
  44 - since the swollen beads overdraw heavily, and moving them to their front point so
  the depth test could skip them made every molecule look scaled like a fish.
- **The worlds' place, behind the shelf.** Somewhere else: dark, quiet, a hundred-odd
  molecules drifting and turning far behind the row and above and below it (never
  between it and the camera), out of focus, under a thinner haze than the cell's; and the
  worlds hanging in a row. It held grey dust before, which read as nothing in particular. Choosing Worlds sends the camera rushing into the cell's
  light; at the fastest moment the window flashes and the camera comes out in the worlds'
  place. Back reverses it. The camera there is the shelf viewer's exactly (`ROW` is
  shared), so the worlds the shelf draws stand in that space. The flash is dark, not
  light. Entering a world from the shelf is a plain cut: the rush belongs to the one
  passage between the two places, and repeated on every world it was a wait.

### The shelf's pictures

Until the row is built, each world's picture stands in for it, and has to stand exactly
where the world will. Both are one square (`standing` in `worlds.ts`): centred where the
world's middle is drawn - which is above the screen's middle by the row's lift - as wide
as the world can reach turning, shrunk as the row shrinks it. The picture is cut from that
square and laid back over it. It was cut from the canvas's middle and laid in boxes set
by percentages, so it was the wrong size and too low; and the square reaches past the
canvas's top, where WebKit clips a source rectangle and stretches the rest, so it is cut
by drawing the whole frame shifted instead. A picture is retaken once a visit, so an old
one does not outlive a change.

A world in the row turns, and its picture does not. The angle each picture was taken at
is kept beside it (local storage, `geis.worlds.angles`), and a row about to be built
stands each world at its picture's angle - unseen, since the row is hidden while it
builds - so the row comes up exactly as the pictures show it, and the row appears under
them at once while they fade off it. Before, the pictures stood at whatever angle they
were taken at and the row came up upright: every launch, the worlds jumped.

The picture is copied from the frame in the task that drew it, straight into a square
of at most 1024 pixels, and encoded off the main thread (`toBlob`). It was encoded whole
as a PNG, decoded, cut and encoded again: two retina-sized encodes, which stalled the row
for 160 ms twice, a second after it came up. The pictures are decoded before they are
shown (`img.decode()`), not when first painted as the shelf fades in.

### Coming through to the shelf

Three things made the way into the shelf stutter, measured as gaps between frames:

- The shelf asked the cell to fly through only after the list of worlds had come back,
  so for a few frames its caption, arrows and the row from the last visit showed over the
  cell before it hid them. It asks first now, before anything is awaited, while the
  screen is being put on the page.
- The row was handed to the graphics card - a new viewer, its shaders, the geometry -
  as soon as it was built, which was in the middle of the flight: a 95 ms hitch with the
  camera at its fastest. The core still builds it at once, off the main thread, but the
  hand-over waits for the camera to stop (`cellStage.settled()`), when the pictures are
  standing in for the row and nothing moving is seen to stop.
- The picture taken a second after arriving, above.

### Keeping a capsid responsive

A 1UF2 assembly is 3.5 million atoms, and anything per atom done on a hover or a click
shows as a stall. Three were: selection masks crossed IPC as JSON arrays of numbers (seven
megabytes to write and parse per hover - now raw bytes, `SelectionMask::encode`); the
commands that build them ran on the main thread, which holds up the window's drawing
(they and the other per-structure commands are `#[tauri::command(async)]`); and every
mask upload allocated two buffers of that size and a new texture (`MaskTexture` reuses
its storage and skips a kind whose inputs are unchanged). Picks read back through a
fence a frame later (`pickLater`) instead of waiting for the GPU, and hover asks only
once the pointer rests.

## The glass representation

Glass is a representation, beside cartoon and sticks (`Representation::Glass`): shown,
hidden and shown-as the same way, on a molecule, a chain or a few residues (a feature).
It was a third viewport mode at first, over every molecule at once in a fixed ramp of
blues; as a representation it goes where it is asked for and is coloured as the atoms
under it are - each vertex the colour of the atom nearest it, so under the default
scheme the glass and the ribbon inside it are one chain's colour - unless the style gives
it one of its own (`Style::glass`, "Glass colour of its own" in the style's panel,
`glass_colour` to the agent). It draws no atoms, so it never decides the scene's primary
representation while there is another.

The core builds it with the rest of the scene (`Scene::glass`, wire version 3): a
Gaussian surface over the glass atoms (`geometry/envelope.rs`) - Blinn blobs, one per atom
or one per residue past 120,000 atoms, summed on a grid of at most six million points
across the cores - blurred over about 2 A and meshed by surface nets, each vertex's normal
the field's slope blended from its cell's eight corners. Unblurred, with each normal taken
at the nearest corner, it was a bag of marbles under glass: every atom-sized lump two
highlights and a bright crease where it met the next. Haemoglobin's takes 0.07 s; the
1UF2 capsid's assembly, 1.6 million triangles, 0.87 s.

The viewport draws it after the composite (`viewer/glass.ts`, posed with the shelf's row
like the other geometry). The picture on the screen is copied aside, the surface's back
faces are drawn into a depth texture of their own, and its front faces - depth first,
then colour where nearest, so a fold shows its nearest layer - as glass (`GLASS_GLSL`,
shared with the home screen's cell):

- refracting: the picture behind is read displaced along the surface's slope, more
  through thicker glass and at a steeper face, a little split by colour at the rim;
- absorbing: tinted by the thickness the light came through (front to back face), so a
  face is clear and a rim takes the atoms' colour;
- reflecting its surroundings by Schlick's Fresnel - in the viewport a soft studio, a
  bright sky and a dim floor, with two lamps - and a thin bright line where it turns away.

The ribbons in a scene with glass are lit differently (`uGlow` in `SHADING_GLSL`): from
within, in their own colour a little richer, brightest where they turn away, with the
occlusion turned down and no outline - a glowing core, so they read through the glass
and its reflections instead of greying under them.

## Floating panels, and a world coming together

On trial: the viewport is the whole window and the panels stand over it as a game's HUD
does (`floating` on the viewer screen, remembered; the Layout menu puts them back
beside it). The bar runs along the top and the status line along the foot, both
translucent; the tools and the conversation are panels of smoked glass down either side;
the viewport's pane loses its title strip and its own controls stand clear of the bar
and the panels. The columns' dividers have nothing to divide and go.

Entering a world holds the screen dark until the world is on screen (at most a second
and a half), then puts it together (`assemble`): the viewport out of a bright blur, then
the bar, the tools, the panel either side and the status line, one after another, each a
bright line drawn across and widening into the panel, with a flicker as it settles.
Assembled round an empty viewport, the molecule popped in after the panels.

## Colour features at scale

Colouring a selection writes features: residue ranges with a colour. A selection
scattered over a capsid - its hydrophobic core - is a hundred thousand ranges, and
colouring repeatedly added more over the old ones. One world grew to 24 MB, read on every
listing, opening and save, which was the stall on the way to the shelf; and the scene
builder found each range's residues by walking the whole structure, so the world never
finished drawing. Now: a feature can carry many ranges (`ranges: "53,57-58,61"`); colouring
writes one feature per chain; stored features are compacted to what each residue ends up as
(`compact_features`) when worlds are read or written; and the scene builder sorts each chain
name's residues once and finds every range by bisection. That world went from 21.6 MB to
0.69 MB, and from never drawing to drawing in four seconds.

## The name, and the data from before it

The app was called mini until its first public release, and is now Geis, after Irving
Geis. The data folder follows the app's identifier, which changed with the name
(`dev.mini.app` to `com.sanggeunpark.geis`), so `legacy.rs` carries a user's data over once,
before anything is opened: the old folder is renamed into the new place (not copied -
downloaded weights can be gigabytes), `mini.sqlite` with its write-ahead log becomes
`geis.sqlite`, and every stored text value holding a path into the old folder - a render's
picture in a world's graph, a job's directory - is repointed. A new folder that already
exists is never overwritten. Credentials are read from the new keychain service and then the
old one; `MINI_BLENDER` and `MINI_MODELS_DIR` are still read beside their `GEIS_` names; and
on a compute machine an extension already installed under `~/.mini/extensions` is used from
there, while new ones go to `~/.geis/extensions`.
