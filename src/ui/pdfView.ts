// A PDF, drawn page by page into the file viewer.
//
// The webview will not draw a PDF inside a frame - it showed a white box - so the pages
// are drawn by pdf.js, Mozilla's reader, which is what Firefox shows PDFs with. It is
// loaded the first time a PDF is opened, not with the window: most sessions never open one.

import { api } from "../ipc";
import { el } from "./dom";

/** Draws every page of the PDF at `path` into a scrolling column, as the pane's width allows. */
export function pdfView(path: string): HTMLElement {
  const column = el("div", { class: "pdf-view" }, el("div", { class: "properties-empty", text: "Opening..." }));
  void (async () => {
    try {
      const pdfjs = await import("pdfjs-dist");
      const worker = await import("pdfjs-dist/build/pdf.worker.min.mjs?url");
      pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
      const bytes = new Uint8Array(await api.fileBytes(path));
      const document = await pdfjs.getDocument({ data: bytes, isEvalSupported: false }).promise;
      column.replaceChildren();
      // Drawn sharp for the screen it is on, and as wide as the pane was when it opened.
      const scale = window.devicePixelRatio || 1;
      // Less its padding and room for a scroll bar, so a page never needs scrolling sideways.
      const width = Math.max(column.clientWidth - 40, 240);
      for (let number = 1; number <= document.numPages; number += 1) {
        const page = await document.getPage(number);
        const natural = page.getViewport({ scale: 1 });
        const viewport = page.getViewport({ scale: (width / natural.width) * scale });
        const canvas = el("canvas", { class: "pdf-page" });
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        canvas.style.width = `${Math.floor(viewport.width / scale)}px`;
        column.append(canvas);
        const context = canvas.getContext("2d");
        if (context) await page.render({ canvasContext: context, viewport }).promise;
      }
    } catch (error) {
      column.replaceChildren(el("div", { class: "properties-empty", text: `Could not read this PDF: ${String(error)}` }));
    }
  })();
  return column;
}
