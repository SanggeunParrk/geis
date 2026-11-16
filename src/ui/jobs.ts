// Settings -> Jobs. What ran, where, and why it failed.

import { api, events, type JobDetail, type JobRecord, type JobState } from "../ipc";
import { JobConsole } from "./console";
import { append, el, formatBytes, formatDuration, formatWhen, replace } from "./dom";

const LIVE: JobState[] = ["queued", "staging", "pending", "running", "fetching"];

function stateClass(state: JobState): string {
  if (state === "succeeded") return "ok";
  if (state === "failed") return "bad";
  if (state === "cancelled") return "";
  return "busy";
}

export function jobsPane(onOpenStructure?: (path: string) => void): {
  element: HTMLElement;
  refresh: () => Promise<void>;
  show: (jobId: string) => Promise<void>;
  dispose: () => void;
} {
  const list = el("div", { class: "pane-list" });
  const detail = el("div", { class: "pane-detail" });
  const element = el("div", { class: "panes" }, list, detail);
  const console_ = new JobConsole();

  let jobs: JobRecord[] = [];
  let selectedId: string | null = null;
  /** World names by id, so a job can say which world it was run from. */
  let worldNames = new Map<string, string>();
  /** Which world's jobs are listed; null is all of them. */
  let worldFilter: string | null = null;
  let unlisten: (() => void) | null = null;
  let subscribed = false;
  let disposed = false;
  /** Ticks the elapsed-time display while anything is in flight. */
  let ticker: number | null = null;

  async function load(): Promise<void> {
    const cards = await api.worldsList().catch(() => []);
    worldNames = new Map(cards.map((card) => [card.id, card.name]));
    // A world deleted since it was picked is not a filter anyone can clear from the list.
    if (worldFilter && !worldNames.has(worldFilter)) worldFilter = null;
    jobs = await api.listJobs(worldFilter ?? undefined);
  }

  function worldOf(job: JobRecord): string {
    if (!job.world_id) return "no world";
    return worldNames.get(job.world_id) ?? "deleted world";
  }

  async function refresh(): Promise<void> {
    await load();
    if (!jobs.some((job) => job.id === selectedId)) selectedId = jobs[0]?.id ?? null;
    renderList();
    await renderDetail();
    ensureSubscription();
    ensureTicker();
  }

  function ensureSubscription(): void {
    // Marked before the listener exists: `refresh` and `show` can both get here while it is
    // still being opened, and each would otherwise open one - every job event then handled
    // twice, and one listener that `dispose` never reaches.
    if (subscribed) return;
    subscribed = true;
    void events
      .onJobChanged(async (job) => {
        const index = jobs.findIndex((candidate) => candidate.id === job.id);
        if (index >= 0) jobs[index] = job;
        else if (!worldFilter || job.world_id === worldFilter) jobs.unshift(job);
        renderList();
        if (job.id === selectedId) await renderDetail();
        ensureTicker();
      })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      });
  }

  function ensureTicker(): void {
    const live = jobs.some((job) => LIVE.includes(job.state));
    if (live && ticker === null) {
      ticker = window.setInterval(() => renderList(), 1000);
    } else if (!live && ticker !== null) {
      window.clearInterval(ticker);
      ticker = null;
    }
  }

  async function show(jobId: string): Promise<void> {
    selectedId = jobId;
    // The job being shown may belong to a world the list is not filtered to.
    worldFilter = null;
    await load();
    renderList();
    await renderDetail();
    ensureSubscription();
    ensureTicker();
  }

  /**
   * The list's head is built once and kept. The list is redrawn every second while a job
   * runs, and the world filter used to be rebuilt with it - which shut its menu while it
   * was being chosen from. Its options change only when the worlds or the filter do.
   */
  const countLabel = el("span");
  const filter = el("select", {
    class: "list-filter",
    title: "Which world's jobs to list",
    onchange: (event: Event) => {
      worldFilter = (event.target as HTMLSelectElement).value || null;
      void refresh();
    },
  });
  const head = el("div", { class: "list-head" }, countLabel, filter);
  let filterDrawn = "";

  function renderList(): void {
    countLabel.textContent = `Jobs (${jobs.length})`;
    const wanted = JSON.stringify([worldFilter, [...worldNames]]);
    if (wanted !== filterDrawn) {
      filterDrawn = wanted;
      replace(
        filter,
        el("option", { value: "", text: "All worlds" }),
        ...[...worldNames].map(([id, name]) =>
          el("option", { value: id, text: name, selected: id === worldFilter ? "" : undefined }),
        ),
      );
    }
    // Everything after the head, so the head itself never leaves the page.
    if (head.parentNode !== list) list.prepend(head);
    while (list.lastChild && list.lastChild !== head) list.removeChild(list.lastChild);
    append(list, [
      ...jobs.map((job) =>
        el(
          "button",
          {
            class: "row",
            "aria-selected": String(job.id === selectedId),
            onclick: async () => {
              selectedId = job.id;
              renderList();
              await renderDetail();
            },
          },
          el(
            "div",
            { class: "row-title" },
            el("span", { class: `dot ${stateClass(job.state)}`.trim() }),
            el("strong", { text: job.name }),
            el("span", { class: "tag", text: job.state }),
          ),
          el("div", {
            class: "row-sub",
            text: `${worldOf(job)} · ${job.model_name} on ${job.provider_id} - ${
              LIVE.includes(job.state)
                ? formatDuration(job.started_at ?? job.created_at, null)
                : formatWhen(job.finished_at ?? job.created_at)
            }`,
          }),
        ),
      ),
      jobs.length === 0 && el("div", { class: "empty" }, "Nothing has been run yet."),
    ]);
  }

  /** An action on the shown job that failed, said at the top of its detail. */
  function failed(error: unknown): void {
    detail.prepend(el("div", { class: "notice bad", text: String(error) }));
  }

  async function renderDetail(): Promise<void> {
    if (!selectedId) {
      replace(detail, el("div", { class: "empty" }, "Pick a job to see its output."));
      return;
    }
    const wanted = selectedId;
    let info: JobDetail;
    try {
      info = await api.jobDetail(wanted);
    } catch (error) {
      if (selectedId === wanted) replace(detail, el("div", { class: "notice bad", text: String(error) }));
      return;
    }
    // A second job clicked while the first was loading: the answer that comes back last
    // is not necessarily the one that was asked for last.
    if (selectedId !== wanted) return;
    const job = info.job;

    replace(
      detail,
      el(
        "div",
        { class: "detail-head" },
        el("h3", { text: job.name }),
        el(
          "div",
          { class: "meta" },
          el("span", { class: `tag ${stateClass(job.state)}`.trim(), text: job.state }),
          el("span", { text: job.model_name }),
          el("span", { text: `on ${job.provider_id}` }),
          el("span", { text: formatDuration(job.started_at ?? job.created_at, job.finished_at) }),
          job.external_id && el("span", { text: job.external_id }),
        ),
      ),
      job.error && el("div", { class: "notice bad", text: job.error }),
      console_.element,
      el(
        "div",
        { class: "card" },
        el("h4", { text: `Results (${info.artifacts.length})` }),
        ...(info.artifacts.length === 0
          ? [el("div", { style: "color:var(--text-faint);font-size:12.5px", text: "none yet" })]
          : info.artifacts.map((artifact) =>
              el(
                "div",
                { class: "file-row" },
                el("span", { class: `tag ${artifact.primary ? "accent" : ""}`.trim(), text: artifact.kind }),
                el("span", { class: "path", text: artifact.path }),
                el("span", { style: "color:var(--text-faint)", text: formatBytes(artifact.bytes) }),
                // A predicted structure is worth looking at, which is the whole point of
                // having run the job.
                artifact.kind === "structure" && onOpenStructure
                  ? el("button", {
                      class: "btn",
                      text: "View",
                      onclick: () => onOpenStructure(artifact.path),
                    })
                  : null,
              ),
            )),
      ),
      el(
        "div",
        { class: "actions" },
        info.is_running &&
          el("button", {
            class: "btn danger",
            text: "Cancel job",
            onclick: async (event: MouseEvent) => {
              const button = event.currentTarget as HTMLButtonElement;
              button.disabled = true;
              try {
                await api.cancelJob(job.id);
              } catch (error) {
                // The job's own event redraws this on success; a refusal has to be said.
                button.disabled = false;
                failed(error);
              }
            },
          }),
        el("button", {
          class: "btn",
          text: "Open job folder",
          onclick: () => void api.revealPath(job.local_dir).catch(failed),
        }),
      ),
    );

    await console_.attach(job.id, `${job.name} - ${job.model_name}`);
  }

  function dispose(): void {
    disposed = true;
    unlisten?.();
    unlisten = null;
    if (ticker !== null) window.clearInterval(ticker);
    console_.dispose();
  }

  return { element, refresh, show, dispose };
}
