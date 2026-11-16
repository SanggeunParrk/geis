// Colours by name.
//
// A pie of colours labelled "#e15759" asks the reader to decode hexadecimal. People say
// coral, teal, slate blue; so a colour is shown as the nearest name there is for it, and
// a colour typed as a name is understood.

const NAMES: [string, string][] = [
  ["white", "#ffffff"], ["black", "#000000"], ["grey", "#808080"], ["silver", "#c0c0c0"], ["slate", "#708090"], ["charcoal", "#36454f"],
  ["red", "#e03131"], ["crimson", "#dc143c"], ["coral", "#ff7f50"], ["salmon", "#fa8072"], ["rose", "#d49a9a"], ["brick", "#b5524a"],
  ["orange", "#f28e2b"], ["amber", "#ffbf00"], ["apricot", "#dba77f"], ["peach", "#ffcba4"], ["tan", "#d2b48c"], ["brown", "#9c755f"],
  ["gold", "#edc948"], ["yellow", "#f0e442"], ["sand", "#d3c486"], ["khaki", "#c3b091"], ["olive", "#999933"], ["mustard", "#ccbb44"],
  ["lime", "#a6d854"], ["green", "#2f9e44"], ["forest", "#117733"], ["sage", "#8cc0a4"], ["mint", "#66c2a5"], ["emerald", "#009e73"],
  ["teal", "#44aa99"], ["aqua", "#76b7b2"], ["cyan", "#66ccee"], ["sky blue", "#56b4e9"], ["powder blue", "#8fc3cb"], ["steel blue", "#4e79a7"],
  ["blue", "#0072b2"], ["cornflower", "#86a8cf"], ["navy", "#332288"], ["indigo", "#4b0082"], ["periwinkle", "#8da0cb"], ["lavender", "#b3a8d8"],
  ["purple", "#8f7fd6"], ["violet", "#aa4499"], ["plum", "#882255"], ["orchid", "#c79bbd"], ["mauve", "#b07aa1"], ["magenta", "#cc79a7"],
  ["pink", "#ff9da7"], ["hot pink", "#e78ac3"], ["wine", "#aa3377"], ["vermilion", "#d55e00"], ["maroon", "#800000"], ["beige", "#e5c494"],
];

function channels(hex: string): [number, number, number] {
  // `#rgb` widened first: read as it stands, "#f80" is the number 0xf80, a near-black blue.
  let digits = hex.trim().replace(/^#/, "");
  if (/^[0-9a-fA-F]{3}$/.test(digits)) digits = digits.replace(/./g, "$&$&");
  const value = /^[0-9a-fA-F]{6}$/.test(digits) ? Number.parseInt(digits, 16) : 0;
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

/** The nearest named colour to `#rrggbb` (or `#rgb`). */
export function colourName(hex: string): string {
  const [r, g, b] = channels(hex.toLowerCase());
  let best = NAMES[0][0];
  let least = Infinity;
  for (const [name, value] of NAMES) {
    const [nr, ng, nb] = channels(value);
    // Weighted the way the eye is: more to green, less to blue.
    const distance = 2 * (r - nr) ** 2 + 4 * (g - ng) ** 2 + 3 * (b - nb) ** 2;
    if (distance < least) {
      least = distance;
      best = name;
    }
  }
  return best;
}

/**
 * A colour as somebody typed it - a name from the list, a CSS name, `#rgb` or `#rrggbb` -
 * as `#rrggbb`, or null if it is none of those.
 */
export function parseColour(text: string): string | null {
  const typed = text.trim().toLowerCase();
  if (!typed) return null;
  const named = NAMES.find(([name]) => name === typed || name.replace(" ", "") === typed);
  if (named) return named[1];
  if (/^#?[0-9a-f]{6}$/.test(typed)) return typed.startsWith("#") ? typed : `#${typed}`;
  if (/^#?[0-9a-f]{3}$/.test(typed)) {
    const short = typed.replace("#", "");
    return `#${short[0]}${short[0]}${short[1]}${short[1]}${short[2]}${short[2]}`;
  }
  // Whatever else the browser knows as a colour: "tomato", "rebeccapurple".
  if (typeof document === "undefined" || !/^[a-z]+$/.test(typed)) return null;
  const probe = document.createElement("canvas").getContext("2d");
  if (!probe) return null;
  probe.fillStyle = "#000000";
  probe.fillStyle = typed;
  const first = probe.fillStyle;
  probe.fillStyle = "#ffffff";
  probe.fillStyle = typed;
  // Unchanged by an unknown name: it kept whichever colour was there before.
  return first === probe.fillStyle && /^#[0-9a-f]{6}$/.test(first) ? first : null;
}
