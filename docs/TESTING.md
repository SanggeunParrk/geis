# What gets tested by hand, and what happened

The unit tests cover what each module computes. This is the other list: the things that
only go wrong in the running app, in this webview, on this platform. Each line says what
was done and what came back, on 2026-11-12, against a copy of real app data in a throwaway
home directory.

Where the list came from: Tauri's own drag-region script (read, not guessed - it is in the
crate), WebKit's bug tracker and forum reports on WebGL in WKWebView, the OpenAI and
Anthropic API references as they stand today, and what the earlier read of the code had
flagged without being able to confirm.

## How it is driven

There is no accessibility permission for scripting the mouse here, so the app is driven
from inside: a development server (not in this repository) serves the same sources with a
small script added that evaluates JavaScript handed to it over HTTP. Pointer and keyboard
events are dispatched in the page, results are read back, and the window is captured with
`screencapture`. It cannot drive a native dialog, the traffic lights, or a real drag of
the window.

## The window

| Check | Result |
|---|---|
| Double-click the title bar: the window zooms, once | **Was broken, fixed.** Tauri's handler for `data-tauri-drag-region` and the app's own `dblclick` handler each toggled the zoom, so the window grew and came straight back. One handler now; `is_maximized` goes false, true, false across two double-clicks. |
| Enter full screen and stay there | Passes. `is_fullscreen` stays true through a screen change; `body.fullscreen` drops the bar's traffic-light gap from 88px to 18px and puts it back on the way out. |
| The window's own title does not sit on the bar | **Was broken, fixed** (`hiddenTitle`). |
| Dragging the window by the bar | Not testable from inside the page. The call is made; whether the window moves needs a hand. |

## Things the webview does not do

| Check | Result |
|---|---|
| `window.prompt()` | **Returns `null` in 0 ms, no dialog.** Three features were built on it. Replaced by `ui/ask.ts`; fetching 1UBQ and duplicating a style both work through it. |
| `window.confirm()` / `alert()` | Not used anywhere. Keep it that way. |
| `[hidden]` on an element whose class sets `display` | **Did not hide.** One rule now makes it. |
| A native `<select>` | Ignores padding and height. Drawn as a field instead. |

## The viewport

| Check | Result |
|---|---|
| Lose the GPU context (`WEBGL_lose_context`) | **Was a blank canvas for good, fixed.** The viewer reports the loss, the screen makes a new one on a new canvas and redraws the last scene with the camera kept. Checked: canvas replaced, context live, scene drawn. The worlds shelf does not recover yet. |
| Shaders compile and link on this machine | Passes: cartoon and ball-and-stick frames captured from 4HHB. Worth repeating on a macOS beta - there is a WebKit report of every program failing to link in one. |
| Click a chain, click the background | Passes: selected, outlined, then cleared. With two molecules the outline lands on the right one. |
| Click a node that is not a molecule | **Was deselected at once, fixed.** A light stays selected and its tab opens. |
| Moving the window between displays of different scale | Not tested: it needs a hand to drag the window. The canvas is sized from `devicePixelRatio` on every draw, and a change of ratio now asks for one; before, nothing did, since a move between displays is not a resize. |

## Worlds and the graph

| Check | Result |
|---|---|
| Make a world from the menu and from the shelf | Passes; an empty world says what to do next instead of reporting an error. |
| Open a world after another | Passes: selection cleared, graph fitted in its pane, previous molecule gone. |
| Delete a molecule | Passes: its chains go with it and the viewport empties. |
| Remove a style slot | Passes: chains on later slots follow them down (2 -> 1), chains on the removed one fall to the first. |
| Undo and redo (Cmd+Z, Shift+Cmd+Z) | **New.** Delete a light, undo, redo, undo: the node list matches at each step and the saved world agrees. |
| Shift+A | Passes (it never fired before: with shift held the key is `A`). |
| Render from the Render tab | Passes: 6.9 s in Cycles on the GPU, an Image node attached and saved, the work directory removed. |

## Compute

| Check | Result |
|---|---|
| Save a Slurm provider with `~/geis` as its directory | Refused, with the reason. |
| Clear the Partition field and save | Stored as nothing, not as an empty string. |
| Edit a provider | Prelude and Environment come back filled in. |
| A settings form taller than the window | **Could not be scrolled, fixed.** |
| Run Boltz-2 on this machine, without Boltz installed | Fails as it should: exit 127, `command not found: boltz` in the console, the job listed as failed. The staged FASTA is titled `>A|protein`. |
| A real prediction, an ssh host, a Slurm cluster | Not tested. No engine is installed here and no host was available. |

## Agents

Claude and ChatGPT are reached through the user's own subscription and nothing else: the
app runs Claude Code and Codex, which are already signed in, and takes no API key. The
key-based connections an earlier version seeded are removed from a saved list on load.

| Check | Result |
|---|---|
| Find the CLIs | **Was broken, fixed.** Neither `claude` nor `codex` is on the PATH on this machine - both exist only inside their editor extensions - and the connection said "No such file or directory". They are now looked for on the PATH, in the usual install directories, and inside VS Code, Cursor and Windsurf extensions (newest copy). |
| Ask Claude, in the app, to colour a chain | Passes, on the real subscription: the reply asks for `colour_chain`, the tool runs, chain B_1 is on a new orange slot in the saved world, and the second reply says so. |
| The same with ChatGPT | Passes: it read a document first, then coloured C_1 cyan. |
| A CLI that is not signed in | Says so: "`claude` exited with exit status: 1: Not logged in - Please run /login". Before, the reason was lost - Claude Code prints it on stdout and only stderr was read. |
| The CLIs' own tools | Claude Code is run with `--tools ""` and no saved session; Codex in a read-only sandbox, ephemeral. What an agent can do is what this app's tools allow. |
| Where the CLI runs | A temporary directory, not wherever the app was started - from there an agentic CLI picks up that directory's project instructions. |
| A reply in Korean, cut at any byte (local models) | Passes (it panicked before). |
| Which model is answering | Shown, from what the CLI itself reports: `claude-fable-5-2` from Claude Code's stream, `gpt-6.1-sol` from the header Codex prints. Neither name is configured anywhere in this app. |
| Watching it work | Claude Code is run with `--output-format stream-json`: the answer appears as it is written, and a thought as "Thinking" then "Thought for 6s - about 370 tokens". The current Claude models do not return the reasoning itself, only how much of it there was; a local model's reasoning is shown in full. Codex reports whole messages, so its answer arrives at once. |
| A question that needs a lookup first | Passes after a fix: ChatGPT answered "I don't have the chain list" instead of calling `list_molecules` until the tool convention said, in so many words, to call the tool that knows. |
| Whether a subscription is connected | Shown in Agents settings from the tool itself: "Installed - 2.1.322", "Signed in - Claude subscription (team)". Not installed shows the install command; not signed in shows a Sign in button that starts the tool's own browser sign-in. The not-installed and not-signed-in states were read from the tools' real output but the button was not pressed here, since this machine is signed in. |
| The system prompt | One prompt for every model. Claude Code is given it as `--system-prompt`, replacing its coding-agent default; Codex, which has no such argument, reads it at the head of its input. |
| Enter while composing Hangul | The Enter that commits a syllable no longer sends the message (`isComposing`). Not exercised by hand - it needs a real input method. |

The adapters for the two hosted APIs are still in `agents/mod.rs`, exercised by tests
against a stand-in server, but no connection that uses them is seeded or offered. If the
decision to stay with subscriptions is final they can be deleted; nothing else depends on
them. (The local model does depend on the OpenAI *wire format*, which stays.)

## The 0.0.0 pass (2026-11-12)

Driven the same way, against an empty throwaway home and against a copy of real app data.
The screen was locked for most of it, so these were read from the page rather than seen.

| Check | Result |
|---|---|
| First launch, first world | Opens with the viewport alone and the empty-world card: Open a file, Fetch from the PDB, and the drop hint |
| A structure file dropped on the window (the webview's drag events, emitted by hand) | The veil shows on enter, 1UBQ and 6VXX load on drop; a non-structure file is refused with a line saying what is read |
| `⌘K` palette | Opens, filters ("hyd" finds the hydrogen-bond and hydrophobic commands), Enter runs, Escape closes |
| `?` sheet, and the `menu` event for it | Both open the shortcut sheet |
| Seventeen commands in a row through `geis:command` (find, show, colour, undo, redo, zoom, view mode, solvent) | No error, the target chip follows |
| Selecting in one world, then entering a new one | The new world acts on "everything"; nothing carried over |
| Deleting an empty world / one with molecules | The empty one goes at once; the other asks, and Cancel keeps it |
| Sending to the assistant with nothing set up | "No assistant is set up yet" with a button that opens Settings > Agents |
| The copy of real data under the new build | The world, its molecule, its object and its chat all load |
| The release build under the content security policy | Boots, loads a structure, draws it (one-shot capture) |
| Blender render script after the typing pass | Cycles on Metal and EEVEE both write an image; a truncated mesh file is refused by name |

A second round, the same night, for what the first had left unseen:

| Check | Result |
|---|---|
| Window frame | A saved frame is restored at launch (four of five launches; the one miss was not reproduced). Un-maximising writes the frame; the maximised size is not written |
| Undo and Redo from the menu (`menu` event) | With nothing focused they step the scene (sticks added, undone, redone). With the agent's box focused they undo and redo its text and leave the scene alone |
| Digit keys with the shortcut sheet up | Ignored while it is up, and work again once it is closed |
| A number field emptied and left | Keeps its value (Samples stayed 64); a typed 128 is taken |
| Save image with the viewport pane closed | The menu opens on the desk with the viewport entries greyed out; Escape closes it; Layout > Viewport only brings the viewport back |
| Clear in one world, then another world | Undo clear is offered only in the world that was cleared |
| `/compact` with one message | Says there is not enough to summarise |
| Check connection | The button reads "checking..." while it runs and comes back; the host's summary is filled in |
| The production content security policy, served to the development page | A remote image is blocked; an image through the asset protocol and a data image load; the core answers; a world with a molecule, contacts, graph view, the palette and two settings sections raise no violation |
| Opening a file through LaunchServices (`open -a`, on a copy of the app with its own identifier) | Into the running app: opens. Launching the app with the file: **did not open** - the event arrived before there was state to keep it in. Fixed (kept until setup, and asked for again once the window is listening); then three cold launches in a row opened it |

A third round, for the review fixes that had only been compiled, and for how the new
pieces sit on screen (measured in the page: inside the window, in front of what is behind
them, no text cut off - the screen was still locked):

| Check | Result |
|---|---|
| Two molecules with the same residue features; Hide sticks on a selection in one | Only that molecule loses them; undo brings them back |
| Render through Blender (4 samples, 320 x 240), Render pressed twice | The button reads "Rendering..." and is off, so one render runs. Save image then offers "Save the render..."; after the view is rotated it says the render is older than the scene |
| Save on a new provider pressed twice | One provider |
| Typing in an agent setting while the settings are told to redraw | The field keeps the keyboard and the text; the redraw happens once the field is left |
| A question asked from inside the settings overlay | **Was drawn behind the overlay**, with the keyboard on the overlay's close button, and Escape closed both. Fixed: the question is in front, Cancel has the keyboard, the settings are out of reach while it is up, and Escape closes only the question |
| The Layout menu | **Reached under the agent panel**, where its right half could not be seen or pressed. Fixed: it stays within the desk and every entry is reachable; both arrangements apply |
| Palette, shortcut sheet, Save image menu, toast, target chip | Inside the window, on top, nothing cut off. The sheet fits without scrolling at the default window size |
| A file that is not a structure dropped on the window | Refused with a line saying which files are read |

A fourth round, with a stand-in for a local model (a small server answering the chat
completions API slowly, and asking for one tool call on request), so the assistant's
paths could be run for real:

| Check | Result |
|---|---|
| Stop on an answer that is streaming | The send button is a stop button while it runs; stopping keeps the words so far, says so, and nothing more arrives |
| A tool call under "Change freely" | Runs: the viewport goes to graph view, and the answer follows |
| A tool call under "Ask before changing" | Run and Refuse are offered; Run runs it |
| Leaving for another world with Run / Refuse still up | The other world's panel is free at once and answers; the waiting call is answered as refused |
| A slow answer, another world, and back before it ends | The answer finishes in the world it was asked in and is saved with that world's conversation |
| The menu bar of the running app (`GEIS_PRINT_MENU`) | Every item is there with the id the window listens for: Settings, Open, Fetch, Save Image, Undo, Redo, Find a Command, Zoom All, Structure or Graph View, Keyboard Shortcuts |
| A node pinned in the properties panel, then deleted | The panel lets go of it and follows the next node picked; undo brings the node back |
| A shipped style's Surface fields | Out of reach of the pointer and the keyboard (`inert`); an edit forced through a synthetic event is not kept |
| Run pressed twice on a model's form | One job |
| The Jobs filter while the list refreshes | The same control stays, with its choice |
| A model run on a machine where it is not installed | Was "exited with code 127"; now says the command was not found there and the model is not installed |

Still not exercised, because nothing here can press a real key, click the menu bar or
drag a real file: the menu bar's items being clicked and their key equivalents being
pressed, and a drop from Finder. Each was driven through the event the operating system
would send, not by the operating system sending it.
