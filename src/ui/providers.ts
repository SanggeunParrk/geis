// Settings -> Compute. Where "find a GPU" stops being a terminal task.
//
// The four provider kinds are one form with different fields rather than four screens,
// because the mental model is meant to be single: a place work can happen, with a status.

import { askConfirm } from "./ask";
import { api, type HostProbe, type ProviderConfig, type ProviderRecord } from "../ipc";
import { el, formatWhen, replace } from "./dom";

type Draft = {
  id?: string;
  name: string;
  enabled: boolean;
  config: ProviderConfig;
};

const KIND_LABELS: Record<ProviderConfig["kind"], string> = {
  local: "This machine",
  ssh: "Remote host (ssh)",
  slurm: "Slurm cluster",
  http: "HTTP endpoint",
};

const KIND_BLURBS: Record<ProviderConfig["kind"], string> = {
  local: "Runs jobs here, using your login shell so conda and PATH work as they do in your terminal.",
  ssh: "Runs jobs on another machine over ssh. Uses your ~/.ssh/config, so aliases, jump hosts, and agent keys all apply.",
  slurm: "Submits jobs with sbatch through a login node and streams the job's output back while it runs.",
  http: "Sends jobs to a hosted API. The credential is kept in your OS keychain, never in the database.",
};

function blankConfig(kind: ProviderConfig["kind"]): ProviderConfig {
  switch (kind) {
    case "local":
      return { kind: "local", max_parallel: 1 };
    case "ssh":
      return { kind: "ssh", host: "", remote_dir: "", max_parallel: 1 };
    case "slurm":
      return {
        kind: "slurm",
        host: "",
        remote_dir: "",
        partition: "",
        gpus: "",
        cpus_per_task: 8,
        mem: "64G",
        time_limit: "12:00:00",
        max_parallel: 4,
      };
    case "http":
      return { kind: "http", endpoint: "", auth_header: "Authorization", auth_format: "Bearer {}", max_parallel: 2 };
  }
}

function statusDot(record: ProviderRecord): HTMLElement {
  const state = record.status.state;
  const className =
    state === "ready" ? "ok" : state === "degraded" ? "warn" : state === "unreachable" ? "bad" : "";
  return el("span", { class: `dot ${className}`.trim() });
}

function statusText(record: ProviderRecord): string {
  switch (record.status.state) {
    case "ready":
      return record.status.summary;
    case "degraded":
      return record.status.summary;
    case "unreachable":
      return record.status.error;
    default:
      return "not checked yet";
  }
}

export function providersPane(): { element: HTMLElement; refresh: () => Promise<void> } {
  const list = el("div", { class: "pane-list" });
  const detail = el("div", { class: "pane-detail" });
  const element = el("div", { class: "panes" }, list, detail);

  let records: ProviderRecord[] = [];
  let selectedId: string | null = null;
  let draft: Draft | null = null;
  /** The provider whose connection check is running, if one is. */
  let busy: string | null = null;

  async function refresh(): Promise<void> {
    records = await api.listProviders();
    if (!selectedId && records.length > 0) selectedId = records[0].id;
    renderList();
    // An open edit form is the user's, half typed: a check finishing elsewhere must not
    // rebuild it, which took the focus and the pasted key with it.
    if (!draft) renderDetail();
  }

  /** Says why an action on a provider failed, over its card - if that card is still showing. */
  function failed(record: ProviderRecord, error: unknown): void {
    renderList();
    if (draft || selectedId !== record.id) return;
    replace(detail, el("div", { class: "notice bad", text: String(error) }), view(record));
  }

  function select(id: string | null): void {
    selectedId = id;
    draft = null;
    renderList();
    renderDetail();
  }

  function startNew(kind: ProviderConfig["kind"]): void {
    selectedId = null;
    draft = { name: "", enabled: true, config: blankConfig(kind) };
    renderList();
    renderDetail();
  }

  function renderList(): void {
    replace(
      list,
      el(
        "div",
        { class: "list-head" },
        el("span", { text: "Providers" }),
        el("span", { class: "spacer" }),
      ),
      ...records.map((record) =>
        el(
          "button",
          {
            class: "row",
            "aria-selected": String(record.id === selectedId && !draft),
            onclick: () => select(record.id),
          },
          el(
            "div",
            { class: "row-title" },
            statusDot(record),
            el("strong", { text: record.name }),
            el("span", { class: "tag", text: record.config.kind }),
            !record.enabled && el("span", { class: "tag", text: "off" }),
          ),
          el("div", { class: "row-sub", text: statusText(record) }),
        ),
      ),
      el(
        "div",
        { class: "list-head", style: "padding-top:16px" },
        el("span", { text: "Add" }),
      ),
      ...(Object.keys(KIND_LABELS) as ProviderConfig["kind"][]).map((kind) =>
        el(
          "button",
          { class: "row", onclick: () => startNew(kind) },
          el("div", { class: "row-title" }, el("span", { text: `+ ${KIND_LABELS[kind]}` })),
        ),
      ),
    );
  }

  function renderDetail(): void {
    if (draft) {
      replace(detail, editor(draft));
      return;
    }
    const record = records.find((candidate) => candidate.id === selectedId);
    if (!record) {
      replace(
        detail,
        el("div", { class: "empty" }, "Pick a provider, or add one to run jobs somewhere new."),
      );
      return;
    }
    replace(detail, view(record));
  }

  function view(record: ProviderRecord): HTMLElement {
    const checking = busy === record.id;
    const probeButton = el("button", {
      class: "btn primary",
      text: checking ? "checking..." : "Check connection",
      disabled: checking,
      onclick: async () => {
        busy = record.id;
        renderDetail();
        try {
          await api.probeProvider(record.id);
          // Cleared before the redraw, or the redrawn button is drawn still checking.
          busy = null;
          await refresh();
        } catch (error) {
          busy = null;
          failed(record, error);
        }
      },
    });

    return el(
      "div",
      {},
      el(
        "div",
        { class: "detail-head" },
        el("h3", { text: record.name }),
        el(
          "div",
          { class: "meta" },
          el("span", { text: KIND_LABELS[record.config.kind] }),
          el("span", { text: `id: ${record.id}` }),
          record.probe && el("span", { text: `checked ${formatWhen(record.probe.probed_at)}` }),
        ),
      ),
      record.status.state === "unreachable" &&
        el("div", { class: "notice bad", text: record.status.error }),
      record.status.state === "degraded" &&
        el(
          "div",
          { class: "notice" },
          ...record.status.warnings.map((warning) => el("div", { text: warning })),
        ),
      el("div", { class: "card" }, el("h4", { text: "Connection" }), configSummary(record.config)),
      record.probe ? probeCard(record.probe) : el("div", { class: "notice info", text: "Not checked yet - run a connection check to see what this host has." }),
      el(
        "div",
        { class: "actions" },
        probeButton,
        el("button", {
          class: "btn",
          text: "Edit",
          onclick: () => {
            draft = {
              id: record.id,
              name: record.name,
              enabled: record.enabled,
              config: structuredClone(record.config),
            };
            renderList();
            renderDetail();
          },
        }),
        el("button", {
          class: "btn",
          text: record.enabled ? "Disable" : "Enable",
          onclick: async () => {
            try {
              await api.saveProvider({
                id: record.id,
                name: record.name,
                enabled: !record.enabled,
                config: record.config,
              });
              await refresh();
            } catch (error) {
              failed(record, error);
            }
          },
        }),
        record.id !== "local" &&
          el("button", {
            class: "btn danger",
            text: "Remove",
            onclick: async () => {
              const sure = await askConfirm({
                title: `Remove "${record.name}"?`,
                detail: "Its address and settings are forgotten. Jobs it has already run stay in the list.",
                accept: "Remove",
              });
              if (!sure) return;
              try {
                await api.deleteProvider(record.id);
                selectedId = null;
                await refresh();
              } catch (error) {
                failed(record, error);
              }
            },
          }),
      ),
    );
  }

  function configSummary(config: ProviderConfig): HTMLElement {
    const rows: [string, string][] = [];
    if (config.kind === "ssh" || config.kind === "slurm") {
      rows.push(["ssh destination", config.host || "-"]);
      rows.push(["remote job directory", config.remote_dir || "-"]);
    }
    if (config.kind === "slurm") {
      rows.push(["partition", config.partition || "(default)"]);
      rows.push(["gres", config.gpus ? `gpu:${config.gpus}` : "(none requested)"]);
      rows.push(["resources", `${config.cpus_per_task ?? "-"} cpus, ${config.mem ?? "-"}, ${config.time_limit ?? "-"}`]);
    }
    if (config.kind === "http") {
      rows.push(["endpoint", config.endpoint]);
      rows.push(["credential", config.secret_ref ? `keychain: ${config.secret_ref}` : "none"]);
    }
    if (config.kind !== "http") {
      rows.push(["weights directory", config.weights_dir || "(not set)"]);
      rows.push(["scratch", config.scratch || "(default)"]);
      if (config.prelude?.length) rows.push(["prelude", config.prelude.join(" ; ")]);
      const env = Object.entries(config.env ?? {});
      if (env.length) rows.push(["environment", env.map(([k, v]) => `${k}=${v}`).join(" ")]);
    }
    rows.push(["parallel jobs", String(config.max_parallel ?? 1)]);

    const definitions = el("dl", { class: "kv" });
    for (const [key, value] of rows) {
      definitions.append(el("dt", { text: key }), el("dd", { text: value }));
    }
    return definitions;
  }

  function probeCard(probe: HostProbe): HTMLElement {
    return el(
      "div",
      { class: "card" },
      el("h4", { text: "What this host has" }),
      el(
        "dl",
        { class: "kv" },
        el("dt", { text: "host" }),
        el("dd", { text: `${probe.hostname} (${probe.os} ${probe.arch})` }),
        el("dt", { text: "cpu" }),
        el("dd", { text: `${probe.cpu} - ${probe.cores} cores, ${probe.memory_gb} GiB RAM` }),
        el("dt", { text: "gpu" }),
        el(
          "dd",
          {},
          probe.gpus.length === 0
            ? "none detected"
            : probe.gpus
                .map((gpu) => `${gpu.name}${gpu.memory_gb ? ` - ${gpu.memory_gb} GiB` : ""} [${gpu.backend}]`)
                .join("\n"),
        ),
        el("dt", { text: "scratch" }),
        el("dd", {
          text: probe.scratch
            ? `${probe.scratch}${probe.scratch_free_gb ? ` (${probe.scratch_free_gb} GiB free)` : ""}`
            : "-",
        }),
      ),
      el(
        "div",
        { style: "margin-top:12px" },
        el("div", { class: "field" }, el("label", { text: "tooling found" })),
        el(
          "div",
          { class: "chips" },
          ...(probe.tools.length === 0
            ? [el("span", { class: "tag bad", text: "nothing recognised" })]
            : probe.tools.map((tool) =>
                el("span", {
                  class: "tag",
                  text: tool.version ? `${tool.name} ${shortVersion(tool.version)}` : tool.name,
                  title: `${tool.path}\n${tool.version ?? ""}`,
                }),
              )),
        ),
      ),
    );
  }

  function editor(current: Draft): HTMLElement {
    const kind = current.config.kind;
    const form = el("div", {});
    const errorSlot = el("div", {});

    const text = (
      label: string,
      value: string,
      onInput: (value: string) => void,
      help?: string,
      placeholder?: string,
    ) =>
      el(
        "div",
        { class: "field" },
        el("label", { text: label }),
        el("input", {
          value,
          placeholder: placeholder ?? "",
          oninput: (event: Event) => onInput((event.target as HTMLInputElement).value),
        }),
        help && el("div", { class: "help", text: help }),
      );

    const number = (label: string, value: number | null | undefined, onInput: (value: number) => void) =>
      el(
        "div",
        { class: "field" },
        el("label", { text: label }),
        el("input", {
          type: "number",
          value: String(value ?? 1),
          oninput: (event: Event) => onInput(Number((event.target as HTMLInputElement).value)),
        }),
      );

    form.append(
      el(
        "div",
        { class: "detail-head" },
        el("h3", { text: current.id ? `Edit ${current.name || "provider"}` : `New ${KIND_LABELS[kind].toLowerCase()}` }),
        el("div", { class: "meta" }, el("span", { text: KIND_BLURBS[kind] })),
      ),
      errorSlot,
    );

    const card = el("div", { class: "card" }, el("h4", { text: "Connection" }));
    card.append(
      text("Name", current.name, (value) => (current.name = value), undefined, "Lab workstation"),
    );

    const config = current.config;
    if (config.kind === "ssh" || config.kind === "slurm") {
      card.append(
        text(
          "ssh destination",
          config.host,
          (value) => (config.host = value),
          "A ~/.ssh/config alias is best - jump hosts and keys then work exactly as they do in your terminal.",
          "gpu-box or user@10.0.0.4",
        ),
        text(
          "Remote job directory",
          config.remote_dir,
          (value) => (config.remote_dir = value),
          "Absolute path on that host. Each job gets its own subdirectory here.",
          "/scratch/me/geis",
        ),
      );
    }
    if (config.kind === "slurm") {
      card.append(
        el(
          "div",
          { class: "grid-2" },
          text("Partition", config.partition ?? "", (value) => (config.partition = value), undefined, "gpu"),
          text("Account", config.account ?? "", (value) => (config.account = value)),
          text(
            "GPUs (gres)",
            config.gpus ?? "",
            (value) => (config.gpus = value),
            "Becomes --gres=gpu:<value>",
            "a100:1",
          ),
          text("Memory", config.mem ?? "", (value) => (config.mem = value), undefined, "64G"),
          text("Time limit", config.time_limit ?? "", (value) => (config.time_limit = value), undefined, "12:00:00"),
          number("CPUs per task", config.cpus_per_task, (value) => (config.cpus_per_task = value)),
        ),
      );
    }
    if (config.kind === "http") {
      const secretInput = el("input", {
        type: "password",
        placeholder: "paste to set or replace",
      });
      card.append(
        text(
          "Endpoint",
          config.endpoint,
          (value) => (config.endpoint = value),
          "Base URL. Manifests reference it as {{provider.endpoint}}.",
          "https://api.example.com/v1",
        ),
        el(
          "div",
          { class: "grid-2" },
          text("Auth header", config.auth_header ?? "", (value) => (config.auth_header = value), undefined, "Authorization"),
          text("Auth format", config.auth_format ?? "", (value) => (config.auth_format = value), "{} is replaced by the key", "Bearer {}"),
        ),
        el(
          "div",
          { class: "field" },
          el("label", { text: "API key" }),
          secretInput,
          el("div", {
            class: "help",
            text: "Stored in your OS keychain under this provider's id, not in the database or any log.",
          }),
        ),
      );
      current.config = config;
      (current as Draft & { secretInput?: HTMLInputElement }).secretInput = secretInput;
    }
    form.append(card);

    if (config.kind !== "http") {
      const execCard = el("div", { class: "card" }, el("h4", { text: "Execution environment" }));
      execCard.append(
        text(
          "Weights directory",
          config.weights_dir ?? "",
          (value) => (config.weights_dir = value),
          "Where model weights live on that host. Manifests use {{provider.weights_dir}} instead of hardcoding paths - and gated models stay blocked until this is set.",
          "/data/weights",
        ),
        text("Scratch directory", config.scratch ?? "", (value) => (config.scratch = value)),
        el(
          "div",
          { class: "field" },
          el("label", { text: "Prelude (one command per line)" }),
          el("textarea", {
            rows: "3",
            placeholder: "module load cuda/12.4\nconda activate folding",
            value: (config.prelude ?? []).join("\n"),
            oninput: (event: Event) => {
              config.prelude = (event.target as HTMLTextAreaElement).value
                .split("\n")
                .map((line) => line.trim())
                .filter(Boolean);
            },
          }),
          el("div", { class: "help", text: "Runs before every step. Failures here stop the job immediately rather than surfacing later as a confusing CUDA error." }),
        ),
        el(
          "div",
          { class: "field" },
          el("label", { text: "Environment (KEY=value per line)" }),
          el("textarea", {
            rows: "3",
            placeholder: "HF_HOME=/data/hf\nCUDA_VISIBLE_DEVICES=0",
            value: Object.entries(config.env ?? {})
              .map(([key, value]) => `${key}=${value}`)
              .join("\n"),
            oninput: (event: Event) => {
              const env: Record<string, string> = {};
              for (const line of (event.target as HTMLTextAreaElement).value.split("\n")) {
                const index = line.indexOf("=");
                if (index > 0) env[line.slice(0, index).trim()] = line.slice(index + 1).trim();
              }
              config.env = env;
            },
          }),
        ),
      );
      form.append(execCard);
    }

    form.append(
      el(
        "div",
        { class: "card" },
        el("h4", { text: "Scheduling" }),
        number("Jobs at once", config.max_parallel, (value) => (config.max_parallel = Math.max(1, value))),
        el("div", {
          class: "help",
          text: "Two large predictions sharing one GPU do not run twice as fast; they run out of memory.",
        }),
      ),
      el(
        "div",
        { class: "actions" },
        el("button", {
          class: "btn primary",
          text: "Save",
          onclick: async (event: MouseEvent) => {
            // One save at a time: a new provider has no id until the first one answers, so
            // a second press meanwhile made a second provider.
            const button = event.currentTarget as HTMLButtonElement;
            if (button.disabled) return;
            button.disabled = true;
            try {
              const saved = await api.saveProvider({
                id: current.id,
                name: current.name,
                enabled: current.enabled,
                config: current.config,
              });
              // From here it exists: a retry after the key fails to save updates this one.
              current.id = saved.id;
              const secretInput = (current as Draft & { secretInput?: HTMLInputElement }).secretInput;
              if (secretInput?.value) {
                await api.setSecret(saved.id, secretInput.value);
                if (saved.config.kind === "http") {
                  await api.saveProvider({
                    id: saved.id,
                    name: saved.name,
                    enabled: saved.enabled,
                    config: { ...saved.config, secret_ref: saved.id },
                  });
                }
              }
              draft = null;
              selectedId = saved.id;
              await refresh();
            } catch (error) {
              replace(errorSlot, el("div", { class: "notice bad", text: String(error) }));
            } finally {
              button.disabled = false;
            }
          },
        }),
        el("button", {
          class: "btn",
          text: "Cancel",
          onclick: () => {
            draft = null;
            renderList();
            renderDetail();
          },
        }),
      ),
    );

    return form;
  }

  return { element, refresh };
}

/** Tool versions arrive as whole banner lines; keep the part that identifies a version. */
function shortVersion(version: string): string {
  const match = version.match(/\d+(\.\d+)+/);
  return match ? match[0] : version.slice(0, 12);
}
