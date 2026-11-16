# Selection

A selection is a **view**: a structure, a level, and a list of indices at that level. The
levels are `chain`, `residue` and `atom`, and a view at one level converts to any other
through the structure's index table - the model BioMol uses.

Indices are positions in the structure, not names: residue index 41 is the 42nd residue of
the structure, which is not the same as author residue number 41. Ask for what you mean by
condition and let the core resolve it.

## Selecting by condition

`select` takes a level and a set of conditions, all of which must hold:

- `chains` - `A_1` form or the author's letter; either matches.
- `entity_type` - `polymer`, `non-polymer`, `water`, `branched`.
- `residue_numbers` - inclusive ranges of author numbers, e.g. `[{from: 40, to: 60}]`.
- `residue_names` - three-letter codes, e.g. `["HIS", "CYS"]`.
- `secondary` - `H` (helix), `E` (strand), `C` (other).
- `hetero` - true for ligands and waters, false for the polymer.
- `atom_names` - `CA`, `NZ`.
- `elements` - `FE`, `S`.

"The haem iron" is `{level: "atom", where: {elements: ["FE"]}}`. "Helices of chain B" is
`{level: "residue", where: {chains: ["B_1"], secondary: ["H"]}}`. "The binding site" is not
a condition this understands - ask which residues, or select the ligand and say so.

## What a selection does

It outlines the atoms in the viewport, lights the cells in the BioMol panel, names itself
on the corner card, and points the properties panel at the chain or molecule it belongs to.
It does not change how anything is drawn. Changing the drawing is a style, and a style is
assigned to a slot.
