// The things the viewport can be told to do, by name.
//
// One list, so the same command can sit in the right-click menu, be chosen in the
// preferences, and later be bound to a key - a shortcut is a name pointing at one of
// these, and which names are on the menu is the user's to say. What each does is the
// viewer screen's (`runCommand`); this is only what they are called.

export interface CommandInfo {
  id: string;
  label: string;
  group: "Show" | "Colour" | "Find" | "View" | "File" | "Select";
}

export const COMMANDS: CommandInfo[] = [
  // Show adds a representation to what is already drawn, and Hide takes one away - as
  // PyMOL's `show` and `hide` do. "Show as" is the one that replaces.
  { id: "show.sticks", label: "Show sticks", group: "Show" },
  { id: "hide.sticks", label: "Hide sticks", group: "Show" },
  { id: "show.cartoon", label: "Show cartoon", group: "Show" },
  { id: "hide.cartoon", label: "Hide cartoon", group: "Show" },
  { id: "show.ball-and-stick", label: "Show ball and stick", group: "Show" },
  { id: "hide.ball-and-stick", label: "Hide ball and stick", group: "Show" },
  { id: "show.spacefill", label: "Show spacefill", group: "Show" },
  { id: "hide.spacefill", label: "Hide spacefill", group: "Show" },
  { id: "as.cartoon", label: "Show as cartoon only", group: "Show" },
  { id: "as.sticks", label: "Show as sticks only", group: "Show" },
  { id: "hide", label: "Hide everything", group: "Show" },
  { id: "colour.chain", label: "Colour by chain", group: "Colour" },
  { id: "colour.element", label: "Colour by element", group: "Colour" },
  { id: "colour.secondary", label: "Colour by secondary structure", group: "Colour" },
  { id: "colour.rainbow", label: "Colour rainbow", group: "Colour" },
  // The contacts are found when a structure is opened; these show them, and hide them
  // again when they are already shown.
  { id: "find.polar", label: "Find hydrogen bonds", group: "Find" },
  { id: "find.salt-bridges", label: "Find salt bridges", group: "Find" },
  { id: "find.pi", label: "Find pi stacking", group: "Find" },
  { id: "find.hydrophobic", label: "Find hydrophobic contacts", group: "Find" },
  { id: "find.core", label: "Select the hydrophobic core", group: "Find" },
  { id: "find.site", label: "Select the binding site", group: "Find" },
  { id: "find.interface", label: "Select the interface between chains", group: "Find" },
  { id: "find.clear", label: "Hide contacts", group: "Find" },
  { id: "find.ligands", label: "Select ligands", group: "Find" },
  { id: "zoom.all", label: "Zoom all", group: "View" },
  { id: "zoom.selection", label: "Zoom to selection", group: "View" },
  { id: "mode.toggle", label: "Switch structure / graph view", group: "View" },
  { id: "solvent.toggle", label: "Show or remove solvent", group: "View" },
  { id: "ligands.toggle", label: "Show or remove ligands", group: "View" },
  { id: "hydrogens.toggle", label: "Show or remove hydrogens", group: "View" },
  { id: "fetch", label: "Fetch from the PDB...", group: "File" },
  { id: "open", label: "Open a file...", group: "File" },
  { id: "save.image", label: "Save image...", group: "File" },
  // A 3D model of the scene for slides: PowerPoint, Word and Keynote take a .glb.
  { id: "export.glb", label: "Export a 3D model for slides (.glb)...", group: "File" },
  { id: "object.make", label: "Make an object of the selection...", group: "Select" },
  // Blender's Delete: what is selected goes. A whole molecule is taken out of the world;
  // part of one is hidden, since it cannot be taken out of its file.
  { id: "delete", label: "Delete the selection", group: "Select" },
  // Empties the scene: every molecule. ⌘Z puts it back.
  { id: "clear.all", label: "Clear all", group: "Select" },
  { id: "select.clear", label: "Clear selection", group: "Select" },
  { id: "styles.clear", label: "Clear residue styles", group: "Select" },
];

/** What the right-click menu holds until someone says otherwise. */
export const DEFAULT_MENU = ["show.sticks", "hide.sticks", "zoom.all", "clear.all", "fetch", "save.image"];

/** The default menus of earlier versions: a saved menu that is one of these was never chosen. */
export const EARLIER_MENUS = [["show.sticks", "hide.sticks", "zoom.all", "fetch", "save.image"]];
