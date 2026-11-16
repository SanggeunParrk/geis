// The colour picker.
//
// Modelled on what everyone already knows from a slide editor: a grid of swatches for the
// common case, an eyedropper, and numeric entry for when a specific value is required.
//
// The browser's `EyeDropper` API would be the obvious way to sample a colour, and it does
// not exist in this webview - measured, not assumed. So the eyedropper samples the
// viewport instead: the caller supplies a function that reads a pixel, which covers the
// case that actually matters here, taking a colour off the structure on screen. The system
// colour panel is offered alongside it, and that one does have a screen-wide magnifier.

import { api, type SwatchGroup } from "../ipc";
import { el, replace } from "./dom";

export interface ColorPickerOptions {
  /** Current value as #rrggbb. */
  value: string;
  onChange: (hex: string) => void;
  /** Reads a pixel from the viewport, for the eyedropper. Null while nothing is drawn. */
  sampleViewport?: (client: { x: number; y: number }) => Promise<string | null>;
  /** Turns the crosshair cursor on while the eyedropper is armed. */
  setPickingCursor?: (picking: boolean) => void;
}

const RECENT_KEY = "geis.recentColors";
const RECENT_LIMIT = 10;

let cachedSwatches: SwatchGroup[] | null = null;

function readRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    return raw ? (JSON.parse(raw) as string[]).slice(0, RECENT_LIMIT) : [];
  } catch {
    // A private window or blocked site data; recents are a convenience, not state.
    return [];
  }
}

function rememberRecent(hex: string): void {
  try {
    const next = [hex, ...readRecent().filter((entry) => entry !== hex)].slice(0, RECENT_LIMIT);
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* ignore */
  }
}

export function normalizeHex(input: string): string | null {
  let value = input.trim().replace(/^#/, "");
  if (/^[0-9a-fA-F]{3}$/.test(value)) {
    value = value
      .split("")
      .map((character) => character + character)
      .join("");
  }
  return /^[0-9a-fA-F]{6}$/.test(value) ? `#${value.toLowerCase()}` : null;
}

export function hexToRgb(hex: string): [number, number, number] {
  const value = normalizeHex(hex) ?? "#000000";
  return [
    parseInt(value.slice(1, 3), 16),
    parseInt(value.slice(3, 5), 16),
    parseInt(value.slice(5, 7), 16),
  ];
}

export function rgbToHex(r: number, g: number, b: number): string {
  // NaN, from a colour the core could not compute, would otherwise print as "#NaNNaNNaN":
  // not a colour, and one the swatch and every later parse silently mistake for black.
  const clamp = (value: number) => (Number.isFinite(value) ? Math.max(0, Math.min(255, Math.round(value))) : value > 0 ? 255 : 0);
  return `#${[r, g, b].map((channel) => clamp(channel).toString(16).padStart(2, "0")).join("")}`;
}

/**
 * Hex to the 0..1 floats the core and the renderer use - which are **linear**, not sRGB.
 *
 * The transfer function is the part that matters. A hex string is sRGB, the same numbers a
 * screen and a colour picker deal in; every float triple in the graph (a style's flat
 * colour, a light, the world background) is linear, because that is the space light is
 * added in. Dividing by 255 and stopping there stores sRGB in a linear field, and mid grey
 * comes out at 0.5 where it should be 0.21 - so a picked colour rendered pale and washed
 * out, while the same colour chosen from the named list (which the core converts with
 * `color::to_linear`) rendered correctly. Same function, both sides, so the two agree.
 */
export function hexToUnit(hex: string): [number, number, number] {
  const [r, g, b] = hexToRgb(hex);
  const linear = (value: number) => {
    const unit = value / 255;
    return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
  };
  return [linear(r), linear(g), linear(b)];
}

/**
 * Hex to 0..1 as it stands, with no curve: for the one colour that is kept in sRGB.
 *
 * The world's background is that one. The core, the viewport and the Blender script all
 * read it as the sRGB value (each converts for itself), so a background run through
 * `hexToUnit` first was converted twice and came out darker than the swatch that was
 * picked - a mid grey chosen in the picker arrived as a dark one.
 */
export function hexToSrgb(hex: string): [number, number, number] {
  const [r, g, b] = hexToRgb(hex);
  return [r / 255, g / 255, b / 255];
}

export function srgbToHex(color: [number, number, number]): string {
  const channel = (value: number) => Math.round(Math.min(1, Math.max(0, value)) * 255);
  return rgbToHex(channel(color[0]), channel(color[1]), channel(color[2]));
}

export function unitToHex(color: [number, number, number]): string {
  const srgb = (value: number) => {
    const clamped = Math.max(0, Math.min(1, value));
    return 255 * (clamped <= 0.0031308 ? clamped * 12.92 : 1.055 * clamped ** (1 / 2.4) - 0.055);
  };
  return rgbToHex(srgb(color[0]), srgb(color[1]), srgb(color[2]));
}

/**
 * A swatch button that opens the picker.
 *
 * Returned as an element the caller drops into a form row, because that is how every use
 * of it looks: a label, a swatch, and the picker appearing under it.
 */
export function colorField(label: string, options: ColorPickerOptions): HTMLElement {
  const swatch = el("button", {
    class: "color-swatch",
    style: `background: ${options.value}`,
    title: options.value,
  });
  // On the body, fixed, rather than inside the field. The field lives in a 208px node and
  // in a 272px panel, both of which clip their overflow - so a 268px popover positioned
  // inside one was simply cut away, which is what "the colour picker cannot pick a
  // colour" was.
  const popover = el("div", { class: "color-popover", hidden: "true" });
  const wrapper = el("div", { class: "color-field" }, swatch);

  let open = false;
  // Clicking anywhere else closes it, which is what a popover is expected to do. The
  // popover is not inside the field any more, so it has to be excluded explicitly.
  // Listening only while open: a field is rebuilt every time its panel is redrawn, and a
  // listener added per field and never removed is one more on every click in the
  // document for every colour field there has ever been.
  const outside = (event: MouseEvent) => {
    const target = event.target as Node;
    if (open && !wrapper.contains(target) && !popover.contains(target)) close();
  };
  const close = () => {
    open = false;
    popover.hidden = true;
    popover.remove();
    document.removeEventListener("click", outside);
    options.setPickingCursor?.(false);
  };

  /** Under the swatch, pulled back inside the window if it would spill. */
  const place = () => {
    const anchor = swatch.getBoundingClientRect();
    popover.style.visibility = "hidden";
    popover.hidden = false;
    document.body.appendChild(popover);
    const own = popover.getBoundingClientRect();
    const left = Math.min(
      Math.max(8, anchor.right - own.width),
      window.innerWidth - own.width - 8,
    );
    const below = anchor.bottom + 6;
    const top =
      below + own.height > window.innerHeight - 8
        ? Math.max(8, anchor.top - own.height - 6)
        : below;
    popover.style.left = `${left}px`;
    popover.style.top = `${top}px`;
    popover.style.visibility = "visible";
  };

  const apply = (hex: string) => {
    const normalized = normalizeHex(hex);
    if (!normalized) return;
    swatch.style.background = normalized;
    swatch.title = normalized;
    rememberRecent(normalized);
    options.onChange(normalized);
  };

  swatch.addEventListener("click", (event) => {
    event.stopPropagation();
    if (open) {
      close();
      return;
    }
    open = true;
    document.addEventListener("click", outside);
    // Placed again once the swatches have loaded - unless it was closed while they did,
    // in which case placing it would put a closed popover back on the page.
    void renderPopover(popover, options, apply, close).then(() => {
      if (open) place();
    });
    place();
  });

  return el(
    "div",
    { class: "field" },
    label ? el("label", { text: label }) : null,
    wrapper,
  );
}

async function renderPopover(
  popover: HTMLElement,
  options: ColorPickerOptions,
  apply: (hex: string) => void,
  close: () => void,
): Promise<void> {
  if (!cachedSwatches) {
    cachedSwatches = await api.colorSwatches();
  }
  const swatches = cachedSwatches;

  const hexInput = el("input", { class: "hex-input", value: options.value });
  const channels: HTMLInputElement[] = [];

  const setFromHex = (hex: string, updateInputs = true) => {
    const normalized = normalizeHex(hex);
    if (!normalized) return;
    apply(normalized);
    if (updateInputs) {
      hexInput.value = normalized;
      const rgb = hexToRgb(normalized);
      channels.forEach((input, index) => (input.value = String(rgb[index])));
    }
  };

  const grid = (colors: string[]) =>
    el(
      "div",
      { class: "swatch-row" },
      ...colors.map((color) =>
        el("button", {
          class: "swatch",
          style: `background: ${color}`,
          title: color,
          onclick: () => setFromHex(color),
        }),
      ),
    );

  const recent = readRecent();
  const categorical = swatches.filter((group) =>
    ["Colour-vision safe", "Vivid"].includes(group.name),
  );
  const ladders = swatches.filter(
    (group) => !["Colour-vision safe", "Vivid"].includes(group.name),
  );

  const rgb = hexToRgb(options.value);
  const rgbRow = el("div", { class: "rgb-row" });
  ["R", "G", "B"].forEach((name, index) => {
    const input = el("input", {
      type: "number",
      min: "0",
      max: "255",
      value: String(rgb[index]),
      oninput: () => {
        const values = channels.map((entry) => Number(entry.value));
        const hex = rgbToHex(values[0], values[1], values[2]);
        hexInput.value = hex;
        apply(hex);
      },
    });
    channels.push(input);
    rgbRow.append(el("label", { class: "channel", text: name }), input);
  });

  const nativeInput = el("input", {
    type: "color",
    class: "native-color",
    value: options.value,
    oninput: (event: Event) => setFromHex((event.target as HTMLInputElement).value),
  });

  const eyedropper = el("button", {
    class: "btn",
    text: "Eyedropper",
    title: "Click a pixel in the viewport to take its colour",
    onclick: () => {
      if (!options.sampleViewport) return;
      options.setPickingCursor?.(true);
      // The press and release that make up the sampling click are kept from the page as
      // well as the click itself. The viewport acts on the release - it selects whatever
      // is under the pointer - so sampling a colour off a chain also selected the chain.
      const swallow = (event: Event) => {
        if (!popover.contains(event.target as Node)) event.stopPropagation();
      };
      const disarm = () => {
        window.removeEventListener("click", onPick, true);
        window.removeEventListener("pointerdown", swallow, true);
        window.removeEventListener("pointerup", swallow, true);
        options.setPickingCursor?.(false);
      };
      const onPick = async (event: MouseEvent) => {
        disarm();
        event.stopPropagation();
        // A click back inside the picker - on this button again, say - is a change of
        // mind, not a sample. Passed on to the button it would arm a second time behind a
        // popover that had just closed.
        if (popover.contains(event.target as Node)) {
          event.preventDefault();
          return;
        }
        const picked = await options.sampleViewport?.({ x: event.clientX, y: event.clientY });
        if (picked) setFromHex(picked);
        close();
      };
      // Capture phase, and on the next tick so this very click does not trigger it.
      setTimeout(() => {
        window.addEventListener("click", onPick, true);
        window.addEventListener("pointerdown", swallow, true);
        window.addEventListener("pointerup", swallow, true);
      }, 0);
    },
  });

  replace(
    popover,
    el(
      "div",
      { class: "picker-section" },
      el("div", { class: "picker-title", text: "Palette" }),
      ...categorical.map((group) => grid(group.colors)),
    ),
    el(
      "div",
      { class: "picker-section" },
      el("div", { class: "picker-title", text: "Standard" }),
      el(
        "div",
        { class: "swatch-grid" },
        ...ladders.map((group) => grid(group.colors)),
      ),
    ),
    recent.length > 0
      ? el(
          "div",
          { class: "picker-section" },
          el("div", { class: "picker-title", text: "Recent" }),
          grid(recent),
        )
      : null,
    el(
      "div",
      { class: "picker-section" },
      el(
        "div",
        { class: "picker-row" },
        el("label", { class: "channel", text: "Hex" }),
        hexInput,
      ),
      rgbRow,
      el(
        "div",
        { class: "actions" },
        options.sampleViewport ? eyedropper : null,
        el(
          "label",
          { class: "btn", title: "The system colour panel, which has a screen-wide magnifier" },
          "More colours",
          nativeInput,
        ),
      ),
    ),
  );

  hexInput.addEventListener("change", () => setFromHex(hexInput.value));
}
