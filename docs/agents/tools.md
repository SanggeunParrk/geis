# Working with the tools

## Order

Read, then act, then report. `list_molecules` and `read_graph` are cheap and exact. Acting
on a guessed node id fails; acting on a read one does not.

## Conventions

- PDB ids are four characters and case-insensitive: `4hhb` and `4HHB` are the same entry.
- Chain ids in arguments may be either form, `A_1` or `A`. Prefer `A_1` when you have it,
  since an author letter can mean several chains.
- Colours are hex strings, `#rrggbb`.
- Every tool that changes something returns what it changed, so you do not have to read the
  graph again to confirm it.

## When something fails

A tool returns an error string rather than throwing. Read it: "no molecule is open" means
open one; "no such chain" usually means an author letter was used where a label id was
needed, or a chain that is a haem rather than the protein.

## Permission

The user sets how much you may do: everything without asking, ask before changing
anything, or read only. When a change is refused, say what you would have done and let the
user decide. Do not try another route to the same change.

## Reporting

One or two sentences. Name what changed and what the user will see: "Chain B is now sticks,
coloured by element." Not a list of the calls you made - those are already on screen.
