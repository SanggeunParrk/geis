// Pop-up menus, with submenus.
//
// Its own module because the node editor needs three of them - the toolbar's add menu, the
// plane's right-click menu and a node's - and they should not be three implementations.
//
// Submenus rather than one flat panel: once every step in the app is a node, the catalogue
// is long enough that dumping it into the right-click menu buries the handful of commands
// that are not "add something". So the menu is short, and "Add node" opens into it.

import { el, replace, type Child } from "./dom";

export type MenuEntry =
  | { kind: "item"; label: string; hint?: string; disabled?: boolean; onPick: () => void }
  /**
   * Opens a panel of its own to the side. Built on demand, so it can read live state, and
   * told which level it will live at - a search field inside a submenu has to put its
   * results one level deeper, or it destroys the panel it is being typed into.
   */
  | { kind: "submenu"; label: string; entries: (depth: number) => MenuEntry[] }
  | { kind: "title"; label: string }
  | { kind: "separator" }
  /** An arbitrary control, for the search field. */
  | { kind: "custom"; node: HTMLElement };

export interface MenuHandle {
  /** Opens at a point in client coordinates. */
  open: (client: { x: number; y: number }, entries: MenuEntry[]) => void;
  /**
   * Replaces one level of an already-open menu.
   *
   * For a live search: the field stays in its own panel and keeps the keyboard while the
   * results are rebuilt in the panel beside it on every keystroke.
   */
  openLevel: (depth: number, client: { x: number; y: number }, entries: MenuEntry[]) => void;
  close: () => void;
  readonly isOpen: boolean;
}

/**
 * Creates a menu that lives inside `host`.
 *
 * `host` has to be a positioned element: panels are placed in its coordinates so that
 * panning the plane underneath cannot drag the menu along with it.
 */
export function menu(host: HTMLElement): MenuHandle {
  /** One element per open level. Opening a level closes everything deeper. */
  const panels: HTMLElement[] = [];

  function close(): void {
    while (panels.length > 0) panels.pop()?.remove();
  }

  function closeFrom(depth: number): void {
    while (panels.length > depth) panels.pop()?.remove();
  }

  function render(entry: MenuEntry, depth: number, panel: HTMLElement): Child {
    switch (entry.kind) {
      case "title":
        return el("div", { class: "picker-title", text: entry.label });
      case "separator":
        return el("div", { class: "menu-sep" });
      case "custom":
        return entry.node;
      case "item":
        return el(
          "button",
          {
            class: "menu-entry",
            disabled: entry.disabled === true,
            // Moving onto a plain item closes any submenu that was open beside it, which
            // is what makes a menu feel like one thing rather than a pile of panels.
            onpointerenter: () => closeFrom(depth + 1),
            onclick: () => {
              close();
              entry.onPick();
            },
          },
          el("span", { text: entry.label }),
          entry.hint ? el("span", { class: "menu-hint", text: entry.hint }) : null,
        );
      case "submenu": {
        const button = el(
          "button",
          { class: "menu-entry submenu" },
          el("span", { text: entry.label }),
          el("span", { class: "menu-arrow", text: ">" }),
        );
        const openChild = () => {
          closeFrom(depth + 1);
          const anchor = button.getBoundingClientRect();
          const own = panel.getBoundingClientRect();
          // Opens off the parent's right edge, level with the row that opened it.
          openPanel(depth + 1, { x: own.right - 4, y: anchor.top - 6 }, entry.entries(depth + 1));
          button.classList.add("open");
        };
        button.addEventListener("pointerenter", openChild);
        button.addEventListener("click", openChild);
        return button;
      }
    }
  }

  function openPanel(depth: number, client: { x: number; y: number }, entries: MenuEntry[]): void {
    closeFrom(depth);
    const panel = el("div", { class: "node-menu" });
    host.appendChild(panel);
    panels[depth] = panel;
    replace(panel, ...entries.map((entry) => render(entry, depth, panel)));

    const frame = host.getBoundingClientRect();
    panel.style.left = `${client.x - frame.left}px`;
    panel.style.top = `${client.y - frame.top}px`;
    // Measured after placing, then pulled back inside: a menu that opens off the edge of
    // the window is a menu with unreachable items.
    const own = panel.getBoundingClientRect();
    if (own.right > frame.right - 8) {
      panel.style.left = `${Math.max(8, client.x - frame.left - own.width)}px`;
    }
    if (own.bottom > frame.bottom - 8) {
      panel.style.top = `${Math.max(8, frame.height - own.height - 8)}px`;
    }
  }

  return {
    open(client, entries) {
      close();
      openPanel(0, client, entries);
    },
    openLevel: openPanel,
    close,
    get isOpen() {
      return panels.length > 0;
    },
  };
}
