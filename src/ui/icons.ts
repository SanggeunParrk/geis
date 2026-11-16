// The four things a structure is made of, and the bond between two of them, as marks.
//
// One colour each - they take the text's - and sixteen units square, so the same mark
// reads in a toolbar button, a row of the scene's list and a pie. Small things first: an
// atom is a dot; a residue is a few atoms joined; a chain is the line residues make; a
// molecule is the whole folded thing; a bond is what joins two of any of them.

export type StructureIcon = "atom" | "residue" | "chain" | "molecule" | "bond" | "object";

const PATHS: Record<StructureIcon, string> = {
  atom: '<circle cx="8" cy="8" r="3.4"/>',
  residue:
    '<path d="M4 11.2 L8 4.8 L12 11.2" fill="none" stroke="currentColor" stroke-width="1.5"/>' +
    '<circle cx="4" cy="11.2" r="2.2"/><circle cx="8" cy="4.8" r="2.2"/><circle cx="12" cy="11.2" r="2.2"/>',
  chain: '<path d="M1.8 11 C3.8 2.5, 6.8 2.5, 8 8 S12.2 13.5, 14.2 5" fill="none" stroke="currentColor" stroke-width="2.3" stroke-linecap="round"/>',
  // A globule: what a folded chain, or several, looks like from far enough away.
  molecule:
    '<circle cx="6" cy="6.2" r="3.9"/><circle cx="10.8" cy="7.2" r="3.2"/><circle cx="7.6" cy="11" r="3.5"/>',
  // A selection that was kept: a few things picked out, with a line drawn round them.
  object:
    '<path d="M8 1.6 a6.4 6.4 0 1 1 -0.01 0" fill="none" stroke="currentColor" stroke-width="1.3" stroke-dasharray="2.4 2"/>' +
    '<circle cx="5.6" cy="9.4" r="1.9"/><circle cx="10.2" cy="9.9" r="1.9"/><circle cx="8.2" cy="5.6" r="1.9"/>',
  bond:
    '<path d="M4.6 11.4 L11.4 4.6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"/>' +
    '<circle cx="3.8" cy="12.2" r="2.3"/><circle cx="12.2" cy="3.8" r="2.3"/>',
};

/** The mark as SVG text, for where markup is what is wanted. */
export function iconMarkup(name: StructureIcon, size = 16): string {
  return `<svg viewBox="0 0 16 16" width="${size}" height="${size}" fill="currentColor" aria-hidden="true">${PATHS[name]}</svg>`;
}

/** The mark as an element. */
export function icon(name: StructureIcon, size = 14): HTMLElement {
  const holder = document.createElement("span");
  holder.className = `structure-icon ${name}`;
  holder.innerHTML = iconMarkup(name, size);
  return holder;
}
