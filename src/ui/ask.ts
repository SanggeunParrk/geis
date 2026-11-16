// Asking for a line of text.
//
// `window.prompt` is what this was: one call, one answer. In this webview it is neither -
// WKWebView shows a JavaScript prompt only if the host implements the panel for it, and
// this one does not, so `prompt()` returns `null` at once with nothing on screen. Measured
// rather than assumed: zero milliseconds, no dialog. Everything behind one - fetching an
// entry from the PDB, naming a style - looked like a menu item that did nothing.
//
// So the question is asked in the page: a small dialog over a dimmed screen, Enter to
// accept, Escape or a click outside to give up.

import { composing, el } from "./dom";

/**
 * Puts whatever dialogs are already up out of reach while a question is asked over them,
 * and returns the way to give them back.
 *
 * The settings are themselves a modal dialog, and WebKit keeps the keyboard inside the
 * one it knows about: a question raised from them had its Cancel focused and then lost
 * the focus straight back to the settings' close button, so Enter closed the settings
 * instead of answering.
 */
function holdBack(): () => void {
  const under = [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')].filter((dialog) => !dialog.inert);
  for (const dialog of under) dialog.inert = true;
  return () => {
    for (const dialog of under) dialog.inert = false;
  };
}

export interface Question {
  title: string;
  /** What the field starts with, selected so typing replaces it. */
  initial?: string;
  placeholder?: string;
  /** The word on the button that accepts. */
  accept?: string;
  /** Says what is wrong with an answer, or null if it will do. Checked as it is typed. */
  check?: (value: string) => string | null;
  /** A yes-or-no that goes with the answer: a checkbox under the field. */
  toggle?: { label: string; hint?: string; value: boolean; onChange: (value: boolean) => void };
}

/** Resolves with the trimmed answer, or `null` if the question was dismissed or left empty. */
export function askText(question: Question): Promise<string | null> {
  return new Promise((resolve) => {
    const input = el("input", {
      class: "ask-input",
      type: "text",
      value: question.initial ?? "",
      placeholder: question.placeholder ?? "",
      spellcheck: "false",
      autocomplete: "off",
    });
    const problem = el("div", { class: "ask-problem", hidden: "true" });
    const accept = el("button", { class: "btn primary", type: "submit", text: question.accept ?? "OK" });
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });

    // Whatever had the focus gets it back, so the keyboard carries on where it was.
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const release = holdBack();
    let settled = false;
    const finish = (value: string | null) => {
      if (settled) return;
      settled = true;
      window.removeEventListener("keydown", onKey, true);
      backdrop.remove();
      release();
      before?.focus({ preventScroll: true });
      resolve(value);
    };

    const complaint = () => {
      const value = input.value.trim();
      if (!value) return "";
      return question.check?.(value) ?? null;
    };
    const review = () => {
      const said = complaint();
      problem.textContent = said ?? "";
      problem.hidden = !said;
      accept.disabled = said !== null;
    };

    const form = el(
      "form",
      { class: "ask-dialog", role: "dialog", "aria-modal": "true", "aria-label": question.title },
      el("div", { class: "ask-title", text: question.title }),
      input,
      problem,
      question.toggle
        ? el(
            "label",
            { class: "ask-toggle", title: question.toggle.hint ?? "" },
            el("input", {
              type: "checkbox",
              checked: question.toggle.value,
              onchange: (event: Event) => question.toggle?.onChange((event.target as HTMLInputElement).checked),
            }),
            el("span", { text: question.toggle.label }),
          )
        : null,
      el("div", { class: "ask-actions" }, cancel, accept),
    );
    const backdrop = el("div", { class: "ask-backdrop" }, form);

    form.addEventListener("submit", (event) => {
      event.preventDefault();
      if (complaint() === null) finish(input.value.trim());
    });
    cancel.addEventListener("click", () => finish(null));
    input.addEventListener("input", review);
    // A press that starts outside the dialog dismisses it; one that starts inside and is
    // dragged out - selecting the text with the mouse - does not.
    backdrop.addEventListener("pointerdown", (event) => {
      if (event.target === backdrop) finish(null);
    });
    // Captured at the window, so the screen underneath does not also act on the key: its
    // own Escape, Delete and letter shortcuts are not meant for a name being typed.
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        finish(null);
        return;
      }
      if (!form.contains(event.target as Node)) return;
      event.stopPropagation();
      // The Enter that commits an input method's candidate is not the one that accepts.
      if (event.key === "Enter" && composing(event)) event.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);

    document.body.append(backdrop);
    review();
    input.focus();
    input.select();
  });
}

export interface Confirmation {
  title: string;
  /** What will happen, in a sentence, under the title. */
  detail?: string;
  /** The word on the button that goes ahead. */
  accept: string;
}

/**
 * Asks before something that cannot be taken back. Resolves true only if the accepting
 * button was pressed; Escape, Cancel and a click outside are all "no".
 *
 * Cancel has the keyboard to begin with, so an Enter left over from whatever came before
 * does not go ahead with it.
 */
export function askConfirm(question: Confirmation): Promise<boolean> {
  return new Promise((resolve) => {
    const accept = el("button", { class: "btn danger", type: "button", text: question.accept });
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const release = holdBack();
    let settled = false;
    const finish = (value: boolean) => {
      if (settled) return;
      settled = true;
      window.removeEventListener("keydown", onKey, true);
      backdrop.remove();
      release();
      before?.focus({ preventScroll: true });
      resolve(value);
    };
    const dialog = el(
      "div",
      { class: "ask-dialog", role: "alertdialog", "aria-modal": "true", "aria-label": question.title },
      el("div", { class: "ask-title", text: question.title }),
      question.detail ? el("div", { class: "ask-detail", text: question.detail }) : null,
      el("div", { class: "ask-actions" }, cancel, accept),
    );
    const backdrop = el("div", { class: "ask-backdrop" }, dialog);
    accept.addEventListener("click", () => finish(true));
    cancel.addEventListener("click", () => finish(false));
    backdrop.addEventListener("pointerdown", (event) => {
      if (event.target === backdrop) finish(false);
    });
    const onKey = (event: KeyboardEvent) => {
      // The screen underneath hears none of it while the question is up.
      event.stopPropagation();
      if (event.key === "Escape") {
        event.preventDefault();
        finish(false);
      }
    };
    window.addEventListener("keydown", onKey, true);
    document.body.append(backdrop);
    cancel.focus();
  });
}
