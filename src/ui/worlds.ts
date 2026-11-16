// The shelf of worlds.
//
// The whole screen is one viewport - a made-up one, for showing worlds rather than working
// in them - with every world standing in a row inside it. Pressing an arrow or scrolling
// moves the camera and the lights along that row, carried by a spring so a step is taken
// rather than snatched; nothing is loaded on the way, which is what makes it instant, and
// the neighbours you see at the edges are the worlds themselves rather than pictures of
// them. Only the chosen one turns, and each keeps its own angle, so leaving one does not
// stand it back up.
//
// Each world is built the same size, so the row says nothing about how big the structures
// are - it is for telling worlds apart. At most four molecules of each are built, because a
// row of worlds is a picture to recognise rather than a scene to study.
//
// The one in the middle stands full size and the rest are drawn smaller, which is what a
// character select does: the row is for choosing from, and the choice should be the thing
// you are looking at.
//
// The background is left out, so the structures stand on the page rather than in a box.

import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type SceneRequest, type SceneStyle, type World, type WorldCard } from "../ipc";
import { cellStage, ROW } from "./cellStage";
import { count, el, replace } from "./dom";
import { Viewer } from "../viewer/viewer";
import { screenBar } from "./window";
import { askConfirm } from "./ask";

export interface WorldsHandle {
  element: HTMLElement;
  refresh: () => Promise<void>;
}

export interface WorldsOptions {
  onEnter: (id: string) => void;
  onBack: () => void;
}

/**
 * How big each world is made, how far apart they stand, how far back the camera is, and
 * how small a world one place off the middle is drawn. Shared with the cell the row stands
 * in (`cellStage`), whose camera has to be this one exactly for each world to sit in its
 * vesicle.
 */
const SIZE = ROW.size;
const SPACING = ROW.spacing;
const DISTANCE = ROW.distance;
const SHRINK = ROW.shrink;

/**
 * How the row moves: a critically damped spring, in stiffness and in radians a second.
 *
 * A spring rather than a fraction of the remaining distance a frame, which is what this was
 * before: that kind of easing leaves at once at its fastest and crawls in, so a step feels
 * snatched away rather than carried, and pressing again mid-slide restarts it from a
 * standstill. A spring starts from rest, arrives at rest, and when an arrow is pressed
 * while it is still moving it simply changes where it is heading - the hand already in
 * motion keeps moving.
 */
const STIFFNESS = 200;
const DAMPING = 2 * Math.sqrt(STIFFNESS);

/** How fast the chosen world turns, and how quickly it gets up to that and back down. */
const TURN_RATE = 0.5;
const TURN_EASE = 4;

/** How many molecules of a world are built into the row. */
const MOLECULES_SHOWN = 4;

/** The largest side a world's picture is kept at, in pixels: it stands about 900 points wide. */
const PICTURE_SIDE = 1024;

/**
 * The angle each world's picture was taken at, by world. Kept on this machine beside the
 * pictures' own folder rather than in the world: it is a fact about a picture, not about
 * the world.
 */
const ANGLES_KEY = "geis.worlds.angles";

function savedAngles(): Record<string, number> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(ANGLES_KEY) ?? "{}");
    return parsed && typeof parsed === "object" ? (parsed as Record<string, number>) : {};
  } catch {
    return {};
  }
}

function saveAngle(id: string, angle: number): void {
  try {
    // Kept within a turn, so the number stays small however long a world has turned.
    const angles = savedAngles();
    angles[id] = angle % (2 * Math.PI);
    localStorage.setItem(ANGLES_KEY, JSON.stringify(angles));
  } catch {
    /* the row then comes up at the angle it was left at */
  }
}

export function worldsScreen(options: WorldsOptions): WorldsHandle {
  const canvas = el("canvas", { class: "worlds-canvas" });
  const left = el("button", { class: "worlds-arrow left", title: "Previous world (←)" }, arrow("‹"));
  const right = el("button", { class: "worlds-arrow right", title: "Next world (→)" }, arrow("›"));
  // Each world's picture, from when it was last looked at, is what the shelf shows first -
  // at once, with nothing said about loading. The row in 3D is built behind the pictures
  // and takes their place once it is up; a world holding a capsid takes seconds to build,
  // and a shelf that waited for it looked broken.
  const thumbSlots = [-1, 0, 1].map((offset) =>
    el("img", { class: `worlds-thumb ${offset === 0 ? "here" : offset < 0 ? "before" : "after"}`, alt: "", hidden: "" }),
  );
  const thumbLayer = el("div", { class: "worlds-thumbs" }, ...thumbSlots);
  // Each world lives in a vesicle in the cell behind the page (`cellStage`); this canvas
  // draws only the worlds, transparent around them, over the cell's.
  const space = el("div", { class: "worlds-space" }, canvas, thumbLayer, left, right);
  const title = el("input", { class: "worlds-name", spellcheck: "false" });
  const detail = el("div", { class: "worlds-detail" });
  const dots = el("div", { class: "worlds-dots" });
  const element = el(
    "div",
    { class: "screen worlds" },
    screenBar(
      el("button", { class: "btn", text: "< Back", onclick: () => options.onBack() }),
      el("h2", { text: "Worlds" }),
      el("span", { class: "spacer" }),
    ),
    el(
      "div",
      { class: "worlds-body" },
      space,
      el(
        "div",
        { class: "worlds-caption" },
        dots,
        title,
        detail,
        el(
          "div",
          { class: "worlds-actions" },
          // Here as well as in the bar: this row is where the eye already is, and a shelf
          // whose only way to grow is a button in the far corner reads as a fixed set.
          el("button", { class: "btn", text: "New world", onclick: () => void create() }),
          el("button", { class: "btn primary", text: "Enter", onclick: enter }),
          el("button", { class: "btn", text: "Delete", onclick: () => void remove() }),
        ),
      ),
    ),
  );

  function arrow(glyph: string): HTMLElement {
    return el("span", { class: "worlds-arrow-glyph", text: glyph });
  }

  let cards: WorldCard[] = [];
  let selected = 0;
  let viewer: Viewer | null = null;
  /** The lighting of the row, whose lamps travel with the camera. */
  let style: SceneStyle | null = null;
  /** Which build is current, so a slow one that has been replaced stops. */
  let token = 0;
  /** What the row standing in the viewport was built from, so it is not built twice. */
  let built = "";
  /** Where the camera is heading along the row, where it is now, and how fast. */
  let aim = 0;
  let along = 0;
  let speed = 0;
  /** How far each world has turned, by its place in the row. */
  let turns: number[] = [];
  /** How much of its turning speed each world is currently using, 0 to 1. */
  let turning: number[] = [];

  // --- moving along the row --------------------------------------------------------------

  /** What the row would have to be built from, so an unchanged shelf is not built again. */
  function signature(): string {
    return cards.map((card) => `${card.id}:${card.stamp}`).join(",");
  }

  async function refresh(): Promise<void> {
    // Coming through from the cell: the worlds are kept back until the camera is here.
    // Asked before anything is awaited - this runs as the screen is put on the page, so the
    // shelf is hidden before its first paint. Asked after the list came back, the caption,
    // the arrows and the row left from the last visit showed over the cell for a few frames.
    if (cellStage.shelf(selected)) {
      element.classList.add("arriving");
      setTimeout(() => element.classList.remove("arriving"), 1250);
    }
    const before = new Map(cards.map((card, index) => [card.id, index]));
    cards = await api.worldsList();
    const active = await api.worldActive().catch(() => "");
    const at = cards.findIndex((card) => card.id === active);
    selected = at >= 0 ? at : 0;
    aim = selected;
    along = selected;
    speed = 0;
    cellStage.follow(along);
    // Each world keeps its angle by id, so a world made or deleted does not hand its angle
    // to its neighbour; one not seen yet this launch stands at the angle its picture was
    // taken at, so the row comes up exactly as the picture shows it.
    const angles = savedAngles();
    turns = cards.map((card) => {
      const was = before.get(card.id);
      return was !== undefined ? (turns[was] ?? 0) : (angles[card.id] ?? 0);
    });
    turning = cards.map((card) => {
      const was = before.get(card.id);
      return was !== undefined ? (turning[was] ?? 0) : 0;
    });
    drawCaption();
    placeThumbs();
    // The arrow keys and Enter are listened for here, so here is where the focus goes:
    // arriving from the menu left it on the button that was pressed, and the shelf heard
    // nothing until it was clicked.
    if (document.activeElement !== title) element.focus({ preventScroll: true });
    // Coming back from a world is the common case and mostly nothing has changed, so the
    // row is kept: rebuilding it is a graph evaluation and a geometry build per world, and
    // paying that for a screen you are only passing through is what made leaving stutter.
    if (viewer && built !== "" && built === signature()) {
      turn();
      return;
    }
    await build();
  }

  function select(index: number): void {
    if (index < 0 || index >= cards.length || index === selected) return;
    selected = index;
    aim = index;
    drawCaption();
  }

  /** Into the world in front: straight in, with nothing in the way. */
  function enter(): void {
    const card = cards[selected];
    if (card) options.onEnter(card.id);
  }

  // A double click on the world in front walks into it, the way it opens anything else.
  // On the stage only: the arrows sit over it and a fast pair of clicks on one is two steps.
  canvas.addEventListener("dblclick", (event) => {
    event.preventDefault();
    enter();
  });
  left.addEventListener("click", () => select(selected - 1));
  right.addEventListener("click", () => select(selected + 1));

  /** Whether the 3D row is up; until it is, the pictures stand in for it. */
  let live = false;
  /** The pictures fetched so far, by world: an address, or null for none taken yet. */
  const thumbs = new Map<string, string | null>();

  async function thumbOf(card: WorldCard): Promise<string | null> {
    if (!thumbs.has(card.id)) {
      const found = await api.worldThumbnail(card.id).catch(() => null);
      thumbs.set(card.id, found ? `${convertFileSrc(found.path)}?v=${found.written}` : null);
    }
    return thumbs.get(card.id) ?? null;
  }

  /** The chosen world's picture in the middle and its neighbours' either side, until the row is up. */
  async function drawThumbs(): Promise<void> {
    thumbLayer.classList.toggle("gone", live);
    const at = selected;
    const urls = await Promise.all(
      [-1, 0, 1].map((offset) => {
        const card = cards[at + offset];
        return card && card.molecules > 0 ? thumbOf(card) : Promise.resolve(null);
      }),
    );
    if (at !== selected) return;
    // Decoded before they are shown, off the main thread: a picture this size decoded as
    // it is first painted held the window for 80 ms as the shelf faded in.
    await Promise.all(
      thumbSlots.map(async (slot, index) => {
        const url = urls[index];
        if (!url || slot.getAttribute("src") === url) return;
        slot.src = url;
        await slot.decode().catch(() => {});
      }),
    );
    if (at !== selected) return;
    thumbSlots.forEach((slot, index) => {
      slot.hidden = !urls[index];
    });
  }

  /**
   * The square a world in the row stands in, in the space's pixels: centred where its
   * middle is drawn, as wide as it can reach turning (its longest half-side is `SIZE`, so a
   * corner swings out to `SIZE` times the square root of three), shrunk as the row shrinks
   * it. The picture is cut from this square and laid back over the same square, so the
   * picture and the world it stands in for line up.
   */
  function standing(index: number): { x: number; y: number; side: number } {
    const width = space.clientWidth;
    const height = space.clientHeight;
    const unit = height / 2 / (DISTANCE * Math.tan(ROW.fovY / 2));
    const away = Math.min(Math.abs(index - along), 1);
    const smooth = away * away * (3 - 2 * away);
    const scale = 1 + (SHRINK - 1) * smooth;
    return {
      x: width / 2 + (index - along) * SPACING * unit,
      // The camera looks at `ROW.lift` below the row, so the row stands that far above the middle.
      y: height / 2 + ROW.lift * unit,
      side: 2 * SIZE * 1.75 * unit * scale,
    };
  }

  /**
   * Takes a picture of the chosen world from the row once a visit, so the next visit shows
   * it at once and as it looks now. Cut from the square it stands in (`standing`), and the
   * angle the world stood at is kept with it, so the row comes up at that angle.
   *
   * The frame is copied straight into the small square, in the task that drew it. It was
   * encoded whole as a PNG, decoded, cut and encoded again - two retina-sized encodes on
   * the main thread, which stalled the row twice for 160 ms a second after it came up.
   */
  let picturing: number | null = null;
  const pictured = new Set<string>();
  function pictureIfMissing(): void {
    if (picturing !== null) window.clearTimeout(picturing);
    // After the row has come to rest on it, not while it is still sliding past.
    picturing = window.setTimeout(() => {
      picturing = null;
      const card = cards[selected];
      if (!live || !viewer || !card || card.molecules === 0 || pictured.has(card.id)) return;
      if (Math.abs(along - selected) > 0.01) {
        pictureIfMissing();
        return;
      }
      pictured.add(card.id);
      const box = standing(selected);
      const angle = turns[selected] ?? 0;
      try {
        const frame = viewer.drawnCanvas();
        const ratio = frame.width / Math.max(space.clientWidth, 1);
        const square = box.side * ratio;
        const side = Math.max(1, Math.round(Math.min(square, PICTURE_SIDE)));
        const k = side / square;
        const crop = document.createElement("canvas");
        crop.width = side;
        crop.height = side;
        // The whole frame drawn shifted, not a source rectangle cut from it: the square can
        // reach past the canvas's top, and WebKit clips a source rectangle that does to
        // the image and stretches what is left - the picture came out shifted.
        crop
          .getContext("2d")
          ?.drawImage(frame, (square / 2 - box.x * ratio) * k, (square / 2 - box.y * ratio) * k, frame.width * k, frame.height * k);
        crop.toBlob((blob) => {
          if (!blob) {
            pictured.delete(card.id);
            return;
          }
          const reader = new FileReader();
          reader.onload = () => {
            if (typeof reader.result !== "string") return;
            void api
              .worldThumbnailSave(card.id, reader.result)
              .then(() => {
                thumbs.delete(card.id);
                saveAngle(card.id, angle);
              })
              .catch(() => pictured.delete(card.id));
          };
          reader.readAsDataURL(blob);
        }, "image/png");
      } catch {
        /* no picture this time; the next visit tries again */
        pictured.delete(card.id);
      }
    }, 1200);
  }

  function drawCaption(): void {
    void drawThumbs();
    pictureIfMissing();
    const card = cards[selected];
    left.disabled = selected <= 0;
    right.disabled = selected >= cards.length - 1;
    // With one world there is nowhere to go, and two dead arrows only say so twice.
    left.hidden = right.hidden = cards.length <= 1;
    replace(
      dots,
      ...cards.map((entry, index) =>
        el("span", {
          class: `worlds-dot${index === selected ? " here" : ""}`,
          title: entry.name,
          onclick: () => select(index),
        }),
      ),
    );
    if (!card) {
      title.value = "";
      detail.textContent = "No worlds yet.";
      return;
    }
    title.value = card.name;
    detail.textContent = [
      card.molecules === 0
        ? "empty - enter it and open a structure"
        : card.molecules === 1
          ? "1 molecule"
          : `${card.molecules} molecules`,
      card.chains > 0 ? count(card.chains, "chain") : "",
      `opened ${when(card.opened)}`,
    ]
      .filter(Boolean)
      .join(" · ");
  }

  function when(at: number): string {
    const days = Math.floor((Date.now() - at) / 86_400_000);
    if (days <= 0) return "today";
    if (days === 1) return "yesterday";
    if (days < 30) return `${days} days ago`;
    return new Date(at).toLocaleDateString();
  }

  // --- the row itself --------------------------------------------------------------------

  /**
   * Builds every world into the one viewport, standing in a row.
   *
   * The same path the real viewer takes - evaluate the graph, build the geometry - so the
   * shelf cannot show something a world does not contain.
   */
  async function build(): Promise<void> {
    const mine = ++token;
    canvas.classList.add("loading");
    live = false;
    // A picture taken since the last visit is the one to show, and the row is built
    // because something changed, so each world is pictured again once it is up.
    thumbs.clear();
    pictured.clear();
    // The pictures stand in until the row is up, so the row has to come up as they show
    // it: each world at the angle its picture was taken at. Unseen - the row is hidden
    // while it builds - so nothing is seen to snap.
    const angles = savedAngles();
    turns = cards.map((card, index) => angles[card.id] ?? turns[index] ?? 0);
    void drawThumbs();
    try {
      await buildRow(mine);
    } finally {
      if (mine === token) {
        live = true;
        void drawThumbs();
        pictureIfMissing();
      }
    }
  }

  /** The row's viewer, made the first time the row is drawn. */
  function ensureViewer(): Viewer | null {
    if (!viewer) {
      try {
        viewer = new Viewer(canvas);
        viewer.setTransparent(true);
        // A shelf is looked at, not flown through.
        viewer.setInteractive(false);
      } catch {
        return null;
      }
    }
    return viewer;
  }

  async function buildRow(mine: number): Promise<void> {
    built = signature();
    // The row's lighting is the first drawable world's as it is *now*. Kept from the
    // first build ever made, the shelf went on being lit by a world that had since been
    // relit, or deleted.
    let lighting: SceneStyle | null = null;
    const parts: SceneRequest["parts"] = [];
    // Every world read and evaluated at once: one after another, a shelf with a capsid on
    // it waited the capsid's reading before it began on the next world.
    const evaluated = await Promise.all(
      cards.map(async (card) => {
        try {
          const world: World = await api.worldRead(card.id);
          const result = await api.graphEvaluate(world.graph, false);
          return { plan: result.evaluation.plan, structures: result.structures };
        } catch {
          return null;
        }
      }),
    );
    if (mine !== token) return;
    for (const [index, entry] of evaluated.entries()) {
      if (!entry) continue;
      const { plan, structures } = entry;
      if (!plan || structures.length === 0) continue;
      lighting ??= lit(plan.style);
      for (const [at, part] of plan.parts.slice(0, MOLECULES_SHOWN).entries()) {
        const structure = structures[at];
        if (!structure) continue;
        parts.push({
          structure_id: structure.id,
          options: part.options,
          assignments: part.assignments,
          features: part.features ?? [],
          place: [index * SPACING, 0, 0],
          size: SIZE,
        });
      }
    }
    if (parts.length === 0 || !lighting) {
      // Nothing was built, so nothing is standing there to keep - and what stood there
      // before has to go: delete the only world with a molecule in it and that molecule
      // went on turning on the shelf, over a caption that said "empty".
      built = "";
      viewer?.clearScene();
      canvas.classList.remove("loading");
      turn();
      return;
    }
    style = lighting;
    // Coarse: four ribbons an inch across and turning. The full detail was seventeen
    // megabytes to build and hand over - half a second of frozen window on the way in -
    // and at that size it is the same picture.
    let payload: Awaited<ReturnType<typeof api.buildScene>> | null = null;
    try {
      payload = await api.buildScene({ parts, style, coarse: true });
    } catch {
      // A world whose file has moved still has a name and a place in the row; entering it
      // is where that is worth saying.
      built = "";
    }
    // The building above is the core's, off this thread; what follows - making the
    // viewer, compiling its shaders, handing the row to the graphics card - holds the
    // window for a tenth of a second or more. Done while the camera was still flying in
    // from the cell, it was a hitch in the middle of the passage; once the camera is still,
    // the pictures are standing in for the row and nothing is seen to stop.
    await cellStage.settled();
    if (mine !== token) return;
    const shown = ensureViewer();
    if (!shown) {
      canvas.classList.remove("loading");
      return;
    }
    if (payload) {
      try {
        shown.setStyle(style);
        // Not reframed: the camera is aimed along the row, not at whatever was loaded.
        await shown.setScene(payload, false);
      } catch {
        built = "";
        if (mine === token) shown.clearScene();
      }
    } else {
      shown.clearScene();
    }
    if (mine !== token) return;
    // Laid out and posed before it is shown: the first frame it is seen in is already
    // where the row is.
    shown.lookAt([along * SPACING, ROW.lift, 0], DISTANCE, SIZE * 2.4);
    shown.setRow(SPACING, along * SPACING, SHRINK, turns);
    canvas.classList.toggle("instant", !thumbSlots[1].hidden);
    canvas.classList.remove("loading");
    turn();
  }

  /**
   * The lighting the row is shown under.
   *
   * Every lamp is made to follow the view, so travelling along the row carries the light
   * with the camera: a world lit from behind as you arrive at it would be a world you
   * cannot see. The world's own lighting is for the world, not for the shelf.
   */
  function lit(base: SceneStyle): SceneStyle {
    return {
      ...base,
      lights: base.lights.map((light) => ({ ...light, follow_view: true })),
      ambient_occlusion: base.ambient_occlusion,
    };
  }

  /**
   * One loop: carries the camera along the row, and turns the world in front of it.
   *
   * It stops on its own when the window is hidden, because the browser stops handing out
   * frames, and picks up where it left off when the window comes back.
   */
  function turn(): void {
    let last: number | null = null;
    viewer?.animate((now) => {
      if (!element.isConnected || !viewer) return false;
      const seconds = last === null ? 0 : Math.min((now - last) / 1000, 0.1);
      last = now;
      travelBy(seconds);
      // Only the chosen one turns, and it keeps its own angle: the world you leave stops
      // where it was rather than snapping upright behind you. It comes up to speed and
      // back down rather than starting and stopping dead, which is the difference between
      // a thing that is alive and a thing being switched on.
      for (let index = 0; index < cards.length; index++) {
        const want = index === selected ? 1 : 0;
        const rate = turning[index] ?? 0;
        const decay = Math.exp(-seconds * TURN_EASE);
        const eased = want + (rate - want) * decay;
        turning[index] = eased;
        // Integrate the easing over this interval, rather than sampling its endpoint.
        // The angle then depends on elapsed time, not the number of frames delivered.
        turns[index] = (turns[index] ?? 0) + TURN_RATE *
          (want * seconds + (rate - want) * (1 - decay) / TURN_EASE);
      }
      viewer.lookAt([along * SPACING, ROW.lift, 0], DISTANCE, SIZE * 2.4);
      cellStage.follow(along);
      placeThumbs();
      // The one in the middle is full size and its neighbours are smaller, as a character
      // select keeps the chosen one in front. Measured from where the camera actually is,
      // so a world grows on the way in rather than popping when it arrives.
      viewer.setRow(SPACING, along * SPACING, SHRINK, turns);
      return true;
    });
  }

  /**
   * Stands the pictures shown until the row is up where their worlds will be drawn: a
   * world `dx` along the row is `dx` times the pixels a unit spans at the camera's
   * distance from the middle of the screen.
   */
  function placeThumbs(): void {
    if (space.clientWidth === 0 || space.clientHeight === 0) return;
    thumbSlots.forEach((slot, offset) => {
      const box = standing(selected - 1 + offset);
      slot.style.left = `${box.x.toFixed(1)}px`;
      slot.style.top = `${box.y.toFixed(1)}px`;
      slot.style.width = `${box.side.toFixed(1)}px`;
      slot.style.height = `${box.side.toFixed(1)}px`;
    });
  }
  new ResizeObserver(() => placeThumbs()).observe(space);

  /**
   * Moves the camera towards the world that was chosen.
   *
   * Taken in small fixed steps rather than one step of however long the frame was, because
   * a spring integrated in one big jump overshoots and rings - the slide would wobble on a
   * frame the browser was late with, which is exactly when it must not.
   */
  function travelBy(seconds: number): void {
    let left = seconds;
    while (left > 0) {
      const step = Math.min(left, 1 / 240);
      left -= step;
      speed += ((aim - along) * STIFFNESS - speed * DAMPING) * step;
      along += speed * step;
    }
    // Settled: within a thousandth of a world and barely moving. Snapping here rather than
    // easing forever keeps the shrink and the camera exactly where they belong at rest.
    if (Math.abs(aim - along) < 0.001 && Math.abs(speed) < 0.01) {
      along = aim;
      speed = 0;
    }
  }

  // --- making and unmaking ----------------------------------------------------------------

  async function create(): Promise<void> {
    let made;
    try {
      made = await api.worldCreate("New world");
      cards = await api.worldsList();
    } catch (error) {
      detail.textContent = String(error);
      return;
    }
    selected = Math.max(0, cards.findIndex((entry) => entry.id === made.id));
    aim = selected;
    drawCaption();
    await build();
    title.focus();
    title.select();
  }

  async function remove(): Promise<void> {
    const card = cards[selected];
    if (!card) return;
    // A world is its molecules, their styling, its cameras and its conversation, and
    // deleting it cannot be undone. An empty one has nothing to lose and goes at once.
    if (card.molecules > 0) {
      const sure = await askConfirm({
        title: `Delete "${card.name}"?`,
        detail: `${count(card.molecules, "molecule")}, how they are drawn, the cameras and the conversation with the agent go with it. This cannot be undone.`,
        accept: "Delete world",
      });
      if (!sure) return;
    }
    try {
      await api.worldDelete(card.id);
    } catch (error) {
      detail.textContent = String(error);
      return;
    }
    selected = Math.max(0, selected - 1);
    await refresh();
  }

  // Renaming is in place: the name under the row is the field that changes it.
  const rename = () => {
    const card = cards[selected];
    if (!card || title.value.trim() === card.name) return;
    void api
      .worldRename(card.id, title.value)
      .then(async () => {
        cards = await api.worldsList();
        drawCaption();
      })
      .catch((error) => {
        // A world that has gone - deleted from another window - cannot be renamed, and
        // that is worth one line under the row, not the whole screen.
        title.value = card.name;
        detail.textContent = String(error);
      });
  };
  title.addEventListener("change", rename);
  title.addEventListener("blur", rename);
  title.addEventListener("keydown", (event) => {
    if (event.key === "Enter") title.blur();
  });

  /**
   * A scroll travels along the row.
   *
   * Accumulated rather than a world per event, because a trackpad sends dozens of small
   * deltas for one flick and a world each would fly past the end of the shelf. Sideways and
   * up-down both count: a mouse has one wheel and a trackpad has two axes.
   */
  let travel = 0;
  element.addEventListener(
    "wheel",
    (event) => {
      if (document.activeElement === title) return;
      event.preventDefault();
      travel += Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
      const step = 80;
      while (Math.abs(travel) >= step) {
        const direction = travel > 0 ? 1 : -1;
        travel -= direction * step;
        select(selected + direction);
      }
    },
    { passive: false },
  );

  element.addEventListener("keydown", (event) => {
    if (document.activeElement === title) return;
    // A key pressed on a button is that button's: Enter on Delete deletes, and must not
    // also walk into the world being deleted.
    if ((event.target as HTMLElement | null)?.closest("button, input, select, textarea")) return;
    if (event.key === "ArrowRight") select(selected + 1);
    else if (event.key === "ArrowLeft") select(selected - 1);
    else if (event.key === "Enter") enter();
  });
  element.tabIndex = 0;

  void refresh();
  return { element, refresh };
}
