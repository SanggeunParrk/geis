# The node graph

Every node has a type, a position, and sockets. Links carry typed values from an output
socket to an input socket; a link is `{ from, to, socket }`, where `socket` names the input
it arrives at.

## Node types

- `molecule` - a structure, fetched from the PDB by id or read from a file. It owns the
  **style slots** (see below) and has switches for hydrogens, waters and ligands.
- `chain` - one chain of a molecule, drawn with one of the molecule's slots. Unwire it and
  the chain is not drawn. A chain node's `slot` names which slot it uses; a Style node
  wired to it overrides that.
- `style` - a named style: representation, colour, surface. The same style can be used by
  many chains.
- `colouring`, `solid-colour`, `material` - parts of a style, wired into its sockets.
- `light`, `world` - the lighting rig and the background.
- `viewport` - what the window shows. There is exactly one, and it is the end of the
  pipeline: if a chain is not connected to it, however indirectly, it is not on screen.
- `render` - an offline render with Blender.
- `image`, `text`, `sequence-input`, `predict` - images, notes, sequences, and structure
  prediction jobs.

## Slots

A molecule carries a list of styles, called slots, exactly as a Blender object carries
material slots. A fetched molecule arrives with one slot, `Cartoon`, and every chain names
it. Restyling that one slot restyles every chain that uses it - which is the point. Give a
chain its own look by adding a slot and pointing the chain at it, not by giving every chain
a copy of the same style.

Precedence, most specific first: a Style node wired to the chain, then the chain's slot,
then the molecule's first slot.

## Representations

`cartoon` (the fold as a ribbon), `sticks` (every bond as a rod), `spacefill` (one sphere
per atom at its van der Waals radius), `ball-and-stick`, `backbone` (a plain tube). A style
holds a set of them: a cartoon with its side chains as sticks is one style, not two.

## Colour schemes

`element`, `chain`, `residue-kind`, `secondary`, `confidence` (AlphaFold pLDDT), `b-factor`,
`rainbow` (along the sequence), `chainbow` (one hue per chain). The last two, and
colour-by-chain, are properties of the whole molecule rather than of one chain.
