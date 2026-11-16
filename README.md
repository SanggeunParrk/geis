# Geis

*Pronounced like "guys". Named after Irving Geis (1908-1997), the artist who painted the
first protein structure anyone saw - Kendrew's myoglobin, for Scientific American in
1961 - and whose drawings taught a generation what a protein looks like.*

A workbench for protein structures: open or fetch a structure, look at it, style it, find
what holds it together, render a figure of it, and run structure prediction on your own
machines - with an assistant in the side panel that can do all of the same things.

Version 0.0.0. macOS (Apple silicon) is the platform it is built and tested on.

## Download

The macOS app for Apple silicon is on the [Releases](../../releases) page: download the
`.dmg` and drag Geis to Applications. It is not signed by Apple yet, so macOS stops it the
first time - right-click Geis in Applications and choose **Open**, or run
`xattr -dr com.apple.quarantine /Applications/Geis.app` once.

## Build from source

    npm install
    npm run tauri build -- --bundles app

The app is written to `src-tauri/target/release/bundle/macos/Geis.app`; move it to
`/Applications`. Building needs Rust (rustup) and Node 20 or later. Nothing else is
required to look at structures. Optional, and found on their own if present:

- **Blender** (5.2 is the version tested), for ray-traced figures (the Render button).
- **Claude Code** or **Codex**, signed in, for the assistant. Geis runs the tool you
  already have and never sees your account. A local model can be downloaded from
  Settings > Agents instead, and API keys can be added there too.

## The first five minutes

1. Open the app and choose **Worlds**. A world is one session: its molecules, how they
   are drawn, its cameras and its conversation. Double-click one to enter it.
2. **Open a file** (`⌘O`), **fetch from the PDB** (`⇧⌘O`), or drop a `.pdb` / `.cif`
   file on the window.
3. Drag to rotate, scroll to zoom, shift-drag to move. Click to select; `1`-`5` choose
   whether a click means an atom, a residue, a chain, a molecule or a bond.
4. Press **`⌘K`** and type what you want - "sticks", "hydrogen bonds", "colour by chain".
   Every command is there. What it acts on is shown at the top of the viewport: the
   selection if there is one, otherwise everything.
5. Press **`?`** for the full list of keys.

The same commands are on three pies under the pointer - `S` select, `A` act, `V` view -
and the ones you use most can be put on the right-click menu in Settings > Preferences.

## What it does

| | |
|---|---|
| **Read** | PDB and mmCIF, proteins and nucleic acids, ligands, waters, alternate locations, multi-model files (first model). |
| **Draw** | Cartoon, sticks, ball and stick, spacefill, backbone; per chain, and per residue range on top. Colour by chain, element, secondary structure, rainbow, B-factor, or one colour; five palettes. |
| **Find** | Hydrogen bonds, salt bridges, pi stacking, hydrophobic contacts, the hydrophobic core (by solvent accessibility), ligands. |
| **Graph view** | The structure faded, with one sphere per chemical component and the bonds between them - for seeing what is connected to what. |
| **Render** | The same scene through Blender (Cycles or EEVEE), with lights, cameras and materials set in the app. |
| **Predict** | Models described by a TOML manifest run as jobs on this machine, over ssh, through Slurm, or against an HTTP API, with logs streamed back. |
| **Extensions** | Tools such as TM-score install on demand, locally or on a remote machine. |
| **Assistant** | A chat panel per world that can select, style, measure, fetch, render and search the web, asking first if you tell it to. |

Everything in a world is also a node graph (`Tab`), for when the panels are not enough.

## Where things are kept

`~/Library/Application Support/com.sanggeunpark.geis/` holds one SQLite database (worlds, settings,
job history) and the files beside it (job directories, downloaded structures, renders).
Deleting that folder resets the app. API keys, if you add any, are in the system keychain.

## Working on it

    npm test                      # frontend unit tests
    npm run lint                  # eslint, type-aware
    npx tsc --noEmit
    cd src-tauri
    cargo test                    # core tests; no network, GPU or Blender needed
    cargo clippy --all-targets -- -D warnings
    cargo fmt --check
    cd .. && npm run tauri dev

Some core tests read real PDB entries and two start Blender; without the files they pass
by skipping. To run them for real, put `1ubq.cif`, `1ubq.pdb`, `4hhb.cif`, `4hhb.pdb`,
`6vxx.cif` and `6vxx.pdb` from files.rcsb.org in a directory and set
`GEIS_TEST_STRUCTURES` to it.

The one Python file, the Blender render script, is checked with `ruff check`,
`ruff format --check` and `pyright` (settings in `ruff.toml` and `pyrightconfig.json`;
pyright needs the `fake-bpy-module` stubs).

Always launch a development build through `npm run tauri dev`: the debug binary loads
the Vite dev server and shows a blank window without it.

- `docs/ARCHITECTURE.md` - how the parts fit together.
- `docs/NOTES.md` - the long working notes: what each part does and why.
- `docs/TESTING.md` - what is checked by hand in the running app, and how.
- `docs/LICENSING.md` - what the project may and may not borrow from.

## Known limits of this version

- Only macOS on Apple silicon has been run. Nothing is deliberately Mac-only, but other
  platforms are untested.
- The app is not code-signed or notarised.
- Installing the shipped extensions (TM-score, PyRosetta) has been exercised with a test
  extension, not with the real downloads.
- Symmetry mates and biological assemblies are not generated; a file is shown as written.

## License

MIT, copyright Sanggeun Park - see [LICENSE](LICENSE). What the app is built from, and what it is not allowed to
include (model weights among them), is in [docs/LICENSING.md](docs/LICENSING.md).
