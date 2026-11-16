# Licensing

Geis is open source under the **MIT License** (`LICENSE`), copyright Sanggeun Park, who
wrote all of it. Anyone may use, change and redistribute it, commercially or not, keeping
the copyright notice.

Checked before release (2026-11-16):

- **What the app ships.** 340 Rust crates are compiled into the binary; all are MIT,
  Apache-2.0, BSD, ISC, Zlib, Unicode-3.0 or similar permissive licences, except five
  MPL-2.0 crates that Tauri itself depends on (`cssparser`, `cssparser-macros`,
  `dtoa-short`, `selectors`, `option-ext`) and `webpki-roots` (CDLA-Permissive-2.0, the
  Mozilla CA list as data). MPL-2.0 is copyleft per file: those crates' own files stay
  MPL and are used unmodified, which every Tauri app already does; it puts nothing on
  Geis's code. No GPL code is linked. The window's libraries are Tauri's (MIT/Apache),
  xterm.js (MIT) and pdf.js (Apache-2.0).
- **Data.** Bond templates come from the wwPDB Chemical Component Dictionary (CC0);
  radii from published tables; named colours from Catppuccin (MIT).
- **Not shipped.** Model weights (AlphaFold 3's are non-commercial and may not be
  redistributed - the app only ever points at where a user gets them), Blender, llama.cpp
  (fetched on request, MIT), and the checkouts in `vendor/` (StructCooker, BioMol: MIT;
  kept out of the repository by `.gitignore`).
- **Artwork.** The app icon is Sanggeun Park's own. The cell behind the opening screens is
  drawn live from `src/assets/cell/molecules.json`, made from PDB entries and the
  Chemical Component Dictionary's ideal coordinates for its small molecules (both CC0) by
  `src-tauri/examples/cell_assets.rs`.

The earlier posture, recorded below, was academic / non-commercial; the findings still
say what each upstream project allows.

## Viewers

| Project | License | Commercial | Notes |
|---|---|---|---|
| Mol* (Molstar) | MIT | yes | PDBe + RCSB; powers AlphaFold DB. Best large-structure engine to learn from |
| NGL | MIT | yes | Mol*'s predecessor |
| 3Dmol.js | BSD-3 | yes | lightweight |
| Jmol / JSmol | LGPL-2.1 | yes (dynamic link) | Java/JS |
| open-source PyMOL | BSD-like | yes | trademark limits; incentive-build code is NOT in the open repo |
| UCSF ChimeraX | non-commercial only | **no** | separate paid agreement for commercial |
| VMD | custom UIUC | restricted | redistribution limited |
| Molecular Nodes (Blender) | GPL-3.0 | yes, viral | was MIT; relicensed for Blender's extension platform |

### How we are allowed to use PyMOL (policy, 2026-09-09)

**Reference, never transcribe.** PyMOL is the reference for *what a molecular viewer should
do* - which is prior art, not property - and never a source of text to copy. Concretely:

- **Fine**: reading its source or its behaviour to learn what a feature *is* (that a
  cartoon over sticks needs the backbone hidden; that lights belong in camera space; that
  chainbow restarts per chain), then implementing it from the underlying geometry, and
  citing it as prior art in a comment.
- **Not fine**: copying code, copying a table of authored values (its colour cycle, its
  named colours), or adopting its setting values as our specification. Those are its
  expression and its calibration.
- **A rule of thumb that has worked**: if the number can be *derived* from published
  geometry or physics, derive it in code and let the derivation stand as the reason. If it
  cannot be derived, it has to come from a citable publication or from our own
  measurement - not from another program's defaults.
- **Behavioural equivalence is not a goal.** "Match PyMOL's output pair-for-pair" is
  reimplementation by another name. Cross-checking against it as one sanity signal among
  several is fine; making it the acceptance criterion is not.

Audit of what is in the tree (2026-09-09):

| Place | Provenance | Verdict |
|---|---|---|
| `cartoon.rs` helix axis weights | were the two literals from `RepCartoonComputeRoundHelices` | **changed**: `axis_weights()` now solves them from the ideal 100 deg twist - two equations, two unknowns - and the test asserts the derivation rather than the numbers |
| `palette.rs` rainbow ramp | PyMOL's palette *names* six pure hues | ours: a hue sweep 240 to 0 deg; their colour table was deliberately not used |
| chain colour cycle | ours (Okabe-Ito / Catppuccin) | fine; `_color_cycle` was deliberately not used |
| `cartoon_side_chain_helper` behaviour | idea, observed | fine: reimplemented from the geometry (drop polymer backbone, keep the trace atom) |
| camera-relative lights | idea, observed | fine |
| chainbow / cbc / spectrum semantics | documented command behaviour | fine: reimplemented over our own data model |
| element colours | CPK convention, decades old | fine |
| covalent and vdW radii | published tables | fine |
| bond templates | RCSB Chemical Component Dictionary | fine: public data, and cited |

The planned interaction inference follows from this: its criteria come from the literature
(with citations in the code), not from PyMOL's `h_bond_*` settings, and it is validated
against published geometry and our own fixtures.

### PyMOL specifics
- No patent clause in the license at all; no patent grant, no retaliation.
- Schrodinger's patent portfolio that surfaced is computational-chemistry methodology
  (ligand binding-affinity scoring, "values representative of interaction"), **not**
  molecular graphics / rendering / UI. Core graphics techniques (Connolly SES, ribbons,
  ray casting) date to the 1980s-90s and are long expired.
- Trademark IS constrained: derivative software must be "plainly distinguished" from
  Schrodinger's PyMOL products. Only fixed phrases ("Includes PyMOL(TM)", "Based on
  PyMOL(TM) technology", ...) are allowed, each requiring the footnote
  "PyMOL is a trademark of Schrodinger, LLC". Anything else needs written permission.
- Verify with a real FTO search before any commercialization.

### Why we write our own viewer
Not a licensing reason - open-source PyMOL is legally usable. The reason is
architectural: PyMOL's global-state C core, one-file-per-session model, and
immediate-mode GL do not fit multi-world state plus a job pipeline.

## Blender coupling
Blender treats anything using the `bpy` API as a GPL derivative (this is exactly why
Molecular Nodes moved MIT -> GPLv3 - the Blender extensions platform requires it). The
files here that use `bpy` - `src-tauri/assets/render_scene.py`, and the icon's
`scripts/blender/icon.py` - are MIT, which is GPL-compatible: run inside Blender the
combination is GPL, as Blender's own position requires, and nothing in Geis links
Blender. If a stricter reading is ever wanted, those files can be relicensed
GPL-3.0-or-later on their own without touching the rest.

- **Chosen:** run Blender headless as a separate process (`blender -b -P script.py`).
- Rationale is functional, not just legal: Cycles/EEVEE is for offline world thumbnails
  and cinematics. Real-time interaction needs our own WebGPU renderer regardless.

## Prediction / generation engines

| Engine | Code | Weights | Commercial |
|---|---|---|---|
| AlphaFold 3 | Apache-2.0 | gated request, non-commercial, "only if received directly from Google", no redistribution | no |
| AlphaFold Server | - | - | no; **and there is no public API** - automating it risks ToS violation |
| Boltz-2 | MIT | MIT | yes |
| Chai-1 | Apache-2.0 | yes | yes (relicensed from a restrictive license) |
| OpenFold3 | Apache-2.0 | yes | yes |

Consequences baked into the design:
1. **The app never bundles weights.** A manifest declares where weights come from and
   whether the user must accept terms first (`weights = "manual"` gates the UI).
2. **No engine is privileged.** AF3 is one manifest among many; the runner knows nothing
   about protein folding specifically.
3. MSA: the public `api.colabfold.com` server rate-limits and bans IPs under load.
   Local MMseqs2 must be a first-class option, not an afterthought.

## Sources
- https://github.com/schrodinger/pymol-open-source/blob/master/LICENSE
- https://en.wikipedia.org/wiki/PyMOL
- https://github.com/molstar/molstar
- https://github.com/BradyAJohnston/MolecularNodes/blob/main/LICENSE
- https://www.rbvi.ucsf.edu/chimerax/docs/license.html
- https://github.com/google-deepmind/alphafold3
- https://github.com/google-deepmind/alphafold3/blob/main/WEIGHTS_TERMS_OF_USE.md
- https://boltz.bio/boltz2
- https://github.com/sokrypton/ColabFold/issues/721

## Named colours

The named flat colours in `geometry/preset.rs` are [Catppuccin
Frappe](https://github.com/catppuccin/catppuccin) (MIT), with the neutrals from the same
project's Latte surface steps. An earlier version used PyMOL's values on the reasoning that
open-source PyMOL is BSD-like and a table of RGB triples is not much of a work; borrowing
another tool's palette is a needless argument to be in either way, and a published pastel
set is a better fit besides - a figure of a complex needs several colours that sit next to
each other without any one of them shouting.
