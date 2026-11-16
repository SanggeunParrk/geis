// The hidden GPU console, made visible on request.
//
// The premise of the compute layer is that a working job never mentions CUDA to the user.
// The corollary is that a broken one has to: this panel shows the same pty stream a
// terminal would, so a failing run is debuggable without leaving the app.

import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { api, events, type LogChunk } from "../ipc";
import { el } from "./dom";

export class JobConsole {
  readonly element: HTMLElement;
  private readonly host: HTMLElement;
  private readonly label: HTMLElement;
  private terminal: Terminal | null = null;
  private fit: FitAddon | null = null;
  private unlisten: (() => void) | null = null;
  private jobId: string | null = null;
  private resizeObserver: ResizeObserver | null = null;

  constructor() {
    this.host = el("div", { class: "console-host" });
    this.label = el("span", { text: "no job selected" });
    this.element = el(
      "div",
      { class: "console-wrap" },
      el(
        "div",
        { class: "console-bar" },
        el("span", { text: "COMPUTE CONSOLE" }),
        this.label,
        el("span", { class: "spacer" }),
        el("button", {
          class: "btn",
          text: "clear",
          onclick: () => this.terminal?.clear(),
        }),
      ),
      this.host,
    );
  }

  /** Created lazily: most sessions never open the console, and a Terminal is not free. */
  private ensureTerminal(): Terminal {
    if (this.terminal) return this.terminal;
    const terminal = new Terminal({
      fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
      fontSize: 12,
      lineHeight: 1.35,
      convertEol: true,
      scrollback: 20000,
      theme: {
        background: "#07090d",
        foreground: "#c8d0e0",
        cursor: "#6cc2ff",
        selectionBackground: "#2b6d99",
      },
    });
    this.fit = new FitAddon();
    terminal.loadAddon(this.fit);
    terminal.open(this.host);
    this.fit.fit();
    this.resizeObserver = new ResizeObserver(() => this.fit?.fit());
    this.resizeObserver.observe(this.host);
    this.terminal = terminal;
    return terminal;
  }

  /**
   * Show a job's output: the log so far, then everything that arrives after.
   *
   * Reading the file first and only then subscribing would drop whatever was written in
   * between, so the subscription is opened first and its chunks are buffered until the
   * backlog has been written.
   */
  async attach(jobId: string, jobName: string): Promise<void> {
    if (this.jobId === jobId) return;
    this.detach();
    this.jobId = jobId;
    this.label.textContent = jobName;

    const terminal = this.ensureTerminal();
    terminal.clear();

    const pending: string[] = [];
    let draining = true;
    // Each listener is for the job it was opened for, not for "whichever job is showing":
    // filtered on the console's current job, one that outlived its attach prints the next
    // job's lines a second time.
    const stop = await events.onJobLog((chunk: LogChunk) => {
      if (chunk.job_id !== jobId) return;
      if (draining) pending.push(chunk.text);
      else terminal.write(chunk.text);
    });
    // Another job was chosen while the subscription was opening. This one is no longer
    // wanted, and keeping it would overwrite - and so leak - the one that is.
    if (this.jobId !== jobId) {
      stop();
      return;
    }
    this.unlisten = stop;

    let backlog: string | null = null;
    try {
      backlog = (await api.readJobLog(jobId)).text;
    } catch (error) {
      if (this.jobId === jobId) {
        terminal.writeln(`\x1b[31m# could not read the job log: ${String(error)}\x1b[0m`);
      }
    }
    // Chosen away from while the log was being read: its backlog belongs to no terminal
    // now, least of all the one showing the job that replaced it.
    if (this.jobId !== jobId) return;
    if (backlog) terminal.write(backlog);

    draining = false;
    for (const text of pending) terminal.write(text);
  }

  detach(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.jobId = null;
    this.label.textContent = "no job selected";
  }

  dispose(): void {
    this.detach();
    this.resizeObserver?.disconnect();
    this.terminal?.dispose();
    this.terminal = null;
  }
}
