// Settings -> Models. The model list is the registry, rendered.
//
// Every field in this form comes from a manifest's [inputs] table - nothing here knows
// what a sequence or a recycling step is. That is the test of whether the plug-in point
// actually works: a new engine appears in this UI without a line of code changing.

import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { api, events, type ExtensionState, type InputSpec, type ModelCard, type ProviderRecord } from "../ipc";
import { el, plain, replace, webLink } from "./dom";

const TASK_LABELS: Record<string, string> = {
  structure_prediction: "structure prediction",
  sequence_design: "sequence design",
  docking: "docking",
  embedding: "embedding",
  msa: "msa",
  custom: "custom",
};

export function modelsPane(onSubmitted: (jobId: string) => void): {
  element: HTMLElement;
  refresh: () => Promise<void>;
} {
  const list = el("div", { class: "pane-list" });
  const detail = el("div", { class: "pane-detail" });
  const element = el("div", { class: "panes" }, list, detail);

  let models: ModelCard[] = [];
  let providers: ProviderRecord[] = [];
  let selectedId: string | null = null;
  /** Per-model form state, kept across selections so a half-filled form is not lost. */
  const values = new Map<string, Record<string, unknown>>();
  /**
   * The rest of the run form, kept the same way: the card is rebuilt whenever one of the
   * model's jobs changes, and rebuilt from nothing it put "Run on" back to the first
   * provider - so the next Run quietly went somewhere else.
   */
  const chosenProvider = new Map<string, string>();
  const jobNames = new Map<string, string>();
  /** Install or remove actions waiting on their job, by model and provider. */
  const acting = new Set<string>();

  /** Where each extension is installed, by its id. */
  const installs = new Map<string, ExtensionState[]>();
  const isExtension = (model: ModelCard) => model.kind === "extension";

  async function refresh(): Promise<void> {
    [models, providers] = await Promise.all([api.listModels(), api.listProviders()]);
    if (!selectedId && models.length > 0) selectedId = models[0].id;
    await Promise.all(
      models.filter(isExtension).map(async (model) => {
        installs.set(model.id, await api.extensionStates(model.id).catch(() => []));
      }),
    );
    renderList();
    renderDetail();
  }

  // An install is a job, and it ends when the job does: the card follows it rather than
  // waiting to be looked at again.
  void events.onJobChanged((job) => {
    const model = models.find((candidate) => candidate.id === job.model_id);
    if (!model || !isExtension(model) || !element.isConnected) return;
    void api.extensionStates(model.id).then((states) => {
      installs.set(model.id, states);
      renderList();
      if (selectedId === model.id) renderDetail();
    });
  });

  /**
   * An extension's installs: one line per place it could run, with what to do about it.
   *
   * Per provider, because that is what an install is - TM-score on this machine says
   * nothing about the cluster - and each button starts a job, which is where its output
   * and its failure are read.
   */
  function installCard(model: ModelCard): HTMLElement {
    const states = installs.get(model.id) ?? [];
    const install = model.install;
    const act = async (providerId: string, action: "install" | "remove") => {
      // One at a time per place: the row only shows "installing" once the job exists, and
      // a second press before then started a second install over the first.
      const key = `${model.id}\n${providerId}`;
      if (acting.has(key)) return;
      acting.add(key);
      try {
        const job = await api.extensionMaintain(model.id, providerId, action);
        installs.set(model.id, await api.extensionStates(model.id));
        renderDetail();
        renderList();
        if (action === "install") onSubmitted(job.id);
      } catch (error) {
        replace(detail, card(model), el("div", { class: "notice bad", text: String(error) }));
      } finally {
        acting.delete(key);
      }
    };
    const WORDS: Record<ExtensionState["state"], string> = {
      installed: "installed",
      installing: "installing…",
      removing: "removing…",
      failed: "the last install failed",
      absent: "not installed",
    };
    return el(
      "div",
      { class: "card" },
      el("h4", { text: "Install" }),
      install?.terms ? el("div", { class: "notice", text: install.terms }) : null,
      el("div", {
        class: "ext-note",
        text:
          "Installed on the machine it will run on, into a directory of its own there (~/.geis/extensions/" +
          model.id +
          ")." +
          (install?.download_mb ? ` Downloads about ${install.download_mb >= 1000 ? `${(install.download_mb / 1000).toFixed(1)} GB` : `${install.download_mb} MB`}.` : ""),
      }),
      ...states.map((entry) => {
        const provider = providers.find((candidate) => candidate.id === entry.provider_id);
        const busy = entry.state === "installing" || entry.state === "removing";
        return el(
          "div",
          { class: `ext-row ${entry.state}` },
          el("span", { class: `dot ${entry.state === "installed" ? "ok" : entry.state === "failed" ? "bad" : busy ? "warn" : ""}`.trim() }),
          el(
            "div",
            { class: "ext-where" },
            el("strong", { text: provider?.name ?? entry.provider_id }),
            el("span", { text: `${provider?.config.kind ?? ""} · ${WORDS[entry.state]}` }),
          ),
          entry.job_id && entry.state !== "absent"
            ? el("button", { class: "btn", text: "Log", onclick: () => onSubmitted(entry.job_id!) })
            : null,
          entry.state === "installed"
            ? el("button", { class: "btn", text: "Remove", onclick: () => void act(entry.provider_id, "remove") })
            : null,
          el("button", {
            class: `btn${entry.state === "installed" ? "" : " primary"}`,
            text: entry.state === "installed" ? "Reinstall" : entry.state === "failed" ? "Try again" : "Install",
            disabled: busy,
            onclick: () => void act(entry.provider_id, "install"),
          }),
        );
      }),
      states.length === 0 ? el("div", { class: "ext-note", text: "Add a local, ssh or slurm provider in Compute to install it on." }) : null,
    );
  }

  function renderList(): void {
    replace(
      list,
      el(
        "div",
        { class: "list-head" },
        el("span", { text: `Extensions (${models.length})` }),
        el("span", { class: "spacer" }),
        el("button", {
          class: "btn",
          text: "reload",
          onclick: async () => {
            try {
              await api.reloadModels();
              await refresh();
            } catch (error) {
              // A manifest that does not parse is the usual reason; it is said where the
              // model's card goes rather than nowhere.
              replace(detail, el("div", { class: "notice bad", text: String(error) }));
            }
          },
        }),
      ),
      ...models.map((model) =>
        el(
          "button",
          {
            class: "row",
            "aria-selected": String(model.id === selectedId),
            onclick: () => {
              selectedId = model.id;
              renderList();
              renderDetail();
            },
          },
          el(
            "div",
            { class: "row-title" },
            el("strong", { text: model.name }),
            isExtension(model) &&
              el("span", {
                class: `tag ${(installs.get(model.id) ?? []).some((entry) => entry.state === "installed") ? "ok" : ""}`.trim(),
                text: (installs.get(model.id) ?? []).some((entry) => entry.state === "installed") ? "installed" : "extension",
              }),
            model.blocked_reason && el("span", { class: "tag warn", text: "gated" }),
            model.eligible_providers.length === 0 && el("span", { class: "tag bad", text: "no host" }),
          ),
          el("div", {
            class: "row-sub",
            text: `${TASK_LABELS[model.task] ?? model.task} - ${model.license ?? "license unstated"}`,
          }),
        ),
      ),
      models.length === 0 &&
        el("div", { class: "empty" }, "No manifests found. Drop a .toml into the models directory."),
    );
  }

  function renderDetail(): void {
    const model = models.find((candidate) => candidate.id === selectedId);
    if (!model) {
      replace(detail, el("div", { class: "empty" }, "Pick a model."));
      return;
    }
    replace(detail, card(model));
  }

  function card(model: ModelCard): HTMLElement {
    const state = values.get(model.id) ?? defaults(model);
    values.set(model.id, state);

    const eligible = providers.filter((provider) => model.eligible_providers.includes(provider.id));
    const errorSlot = el("div", {});
    const providerSelect = el(
      "select",
      { onchange: (event: Event) => chosenProvider.set(model.id, (event.target as HTMLSelectElement).value) },
      ...eligible.map((provider) =>
        el("option", {
          value: provider.id,
          text: `${provider.name} (${provider.config.kind})`,
          selected: chosenProvider.get(model.id) === provider.id,
        }),
      ),
    );

    const nameInput = el("input", {
      placeholder: model.name,
      value: jobNames.get(model.id) ?? "",
      oninput: (event: Event) => jobNames.set(model.id, (event.target as HTMLInputElement).value),
    });

    const requirements = model.requirements;

    return el(
      "div",
      {},
      el(
        "div",
        { class: "detail-head" },
        el("h3", { text: model.name }),
        el(
          "div",
          { class: "meta" },
          el("span", { text: TASK_LABELS[model.task] ?? model.task }),
          el("span", { text: requirements.runtime }),
          model.license && el("span", { text: model.license }),
          model.version && el("span", { text: `v${model.version}` }),
        ),
      ),
      model.description && el("p", { style: "color:var(--text-dim);margin-top:0", text: model.description }),
      model.blocked_reason && el("div", { class: "notice", text: model.blocked_reason }),
      eligible.length === 0 &&
        el("div", {
          class: "notice bad",
          text:
            requirements.runtime === "http"
              ? "This model needs an HTTP provider. Add one in Compute."
              : "No provider can run this model. Add a local, ssh, or slurm provider in Compute.",
        }),
      requirements.notes && el("div", { class: "notice info", text: requirements.notes }),
      isExtension(model) ? installCard(model) : null,
      el(
        "div",
        { class: "card" },
        el("h4", { text: "Requirements" }),
        el(
          "div",
          { class: "chips" },
          el("span", {
            class: `tag ${requirements.gpu === "required" ? "warn" : ""}`.trim(),
            text: `gpu: ${requirements.gpu}`,
          }),
          requirements.vram_gb != null && el("span", { class: "tag", text: `${requirements.vram_gb} GiB vram` }),
          requirements.disk_gb != null && el("span", { class: "tag", text: `${requirements.disk_gb} GiB disk` }),
          el("span", {
            class: `tag ${requirements.weights === "manual" ? "warn" : ""}`.trim(),
            text: `weights: ${requirements.weights}`,
          }),
          ...requirements.needs_binaries.map((binary) => el("span", { class: "tag", text: binary })),
        ),
      ),
      el(
        "div",
        { class: "card" },
        el("h4", { text: "Run" }),
        el(
          "div",
          { class: "field" },
          el("label", { text: "Job name" }),
          nameInput,
        ),
        el(
          "div",
          { class: "field" },
          el("label", { text: "Run on" }),
          providerSelect,
        ),
        ...Object.entries(model.inputs).map(([name, spec]) => field(name, spec, state)),
        errorSlot,
        el(
          "div",
          { class: "actions" },
          el("button", {
            class: "btn primary",
            text: "Run",
            disabled: eligible.length === 0 || Boolean(model.blocked_reason),
            title: isExtension(model) ? "Runs on a provider it is installed on" : undefined,
            onclick: async (event: MouseEvent) => {
              // One submission at a time per model: a double click was two jobs. Kept on
              // the pane rather than the button, since the card can be rebuilt meanwhile.
              const key = `run\n${model.id}`;
              if (acting.has(key)) return;
              acting.add(key);
              const button = event.currentTarget as HTMLButtonElement;
              button.disabled = true;
              try {
                // Asked for here rather than left to fail on the host with "not found".
                const here = (installs.get(model.id) ?? []).find((entry) => entry.provider_id === providerSelect.value);
                if (isExtension(model) && here?.state !== "installed") {
                  throw new Error(`${model.name} is not installed on that provider yet - install it above first.`);
                }
                const job = await api.submitJob({
                  model_id: model.id,
                  provider_id: providerSelect.value,
                  name: nameInput.value || null,
                  inputs: state,
                });
                onSubmitted(job.id);
              } catch (error) {
                replace(errorSlot, el("div", { class: "notice bad", text: String(error) }));
              } finally {
                acting.delete(key);
                button.disabled = false;
              }
            },
          }),
          webLink(model.homepage) &&
            el("a", {
              class: "btn",
              href: webLink(model.homepage),
              target: "_blank",
              rel: "noreferrer",
              text: "Homepage",
            }),
        ),
      ),
      model.citation &&
        el(
          "div",
          { class: "card" },
          el("h4", { text: "Cite" }),
          el("div", { style: "font-size:12.5px;color:var(--text-dim)", text: model.citation }),
        ),
    );
  }

  function defaults(model: ModelCard): Record<string, unknown> {
    const state: Record<string, unknown> = {};
    for (const [name, spec] of Object.entries(model.inputs)) {
      if (spec.default !== undefined && spec.default !== null) state[name] = spec.default;
      else if (spec.type === "bool") state[name] = false;
      else if (spec.type === "enum") state[name] = spec.choices?.[0] ?? "";
    }
    return state;
  }

  function field(name: string, spec: InputSpec, state: Record<string, unknown>): HTMLElement {
    const label = `${spec.label ?? name}${spec.required ? " *" : ""}`;
    const help = spec.help ? el("div", { class: "help", text: spec.help }) : null;

    switch (spec.type) {
      case "fasta":
      case "text":
        return el(
          "div",
          { class: "field" },
          el("label", { text: label }),
          el("textarea", {
            rows: spec.type === "fasta" ? "5" : "3",
            placeholder: spec.type === "fasta" ? "MKTAYIAKQRQISFVKSHFSRQ...\nor paste FASTA" : "",
            value: plain(state[name]),
            oninput: (event: Event) => (state[name] = (event.target as HTMLTextAreaElement).value),
          }),
          help,
        );

      case "file":
      case "dir": {
        const display = el("input", {
          value: plain(state[name]),
          placeholder: "no file chosen",
          oninput: (event: Event) => (state[name] = (event.target as HTMLInputElement).value),
        });
        return el(
          "div",
          { class: "field" },
          el("label", { text: label }),
          el(
            "div",
            { style: "display:flex;gap:8px" },
            display,
            el("button", {
              class: "btn",
              text: "Browse",
              onclick: async () => {
                const picked = await openFileDialog({
                  directory: spec.type === "dir",
                  multiple: false,
                  filters:
                    spec.extensions && spec.extensions.length > 0
                      ? [{ name: spec.extensions.join("/"), extensions: spec.extensions }]
                      : undefined,
                });
                if (typeof picked === "string") {
                  state[name] = picked;
                  display.value = picked;
                }
              },
            }),
          ),
          help,
        );
      }

      case "bool":
        return el(
          "div",
          { class: "field inline" },
          el("input", {
            type: "checkbox",
            checked: Boolean(state[name]),
            onchange: (event: Event) => (state[name] = (event.target as HTMLInputElement).checked),
          }),
          el("label", { text: spec.label ?? name }),
          help,
        );

      case "enum":
        return el(
          "div",
          { class: "field" },
          el("label", { text: label }),
          el(
            "select",
            {
              onchange: (event: Event) => (state[name] = (event.target as HTMLSelectElement).value),
            },
            ...(spec.choices ?? []).map((choice) =>
              el("option", { value: choice, text: choice, selected: state[name] === choice }),
            ),
          ),
          help,
        );

      case "int":
      case "float":
        return el(
          "div",
          { class: "field" },
          el("label", { text: label }),
          el("input", {
            type: "number",
            step: spec.type === "int" ? "1" : "any",
            min: spec.min ?? undefined,
            max: spec.max ?? undefined,
            value: plain(state[name]),
            oninput: (event: Event) => {
              const raw = (event.target as HTMLInputElement).value;
              state[name] = raw === "" ? null : Number(raw);
            },
          }),
          help,
        );
    }
  }

  return { element, refresh };
}
