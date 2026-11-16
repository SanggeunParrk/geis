//! The manifests shipped in `models/` are part of the product, so they are checked like
//! code: they must parse, validate, and resolve their templates with nothing missing.

use geis_lib::compute::job::{plan, JobRequest};
use geis_lib::compute::provider::{
    ExecOptions, LocalConfig, ProviderConfig, ProviderRecord, ProviderStatus,
};
use geis_lib::models::manifest::RunStep;
use geis_lib::models::template::{resolve, Bindings};
use geis_lib::models::{Registry, RuntimeKind};
use geis_lib::util;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("models")
}

#[test]
fn every_shipped_manifest_loads() {
    let registry = Registry::load_dirs(&[models_dir()]);
    let problems: Vec<String> = registry
        .problems()
        .iter()
        .map(|problem| format!("{}: {}", problem.path.display(), problem.message))
        .collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(
        registry.len() >= 4,
        "expected the bundled manifests to load"
    );
}

/// The failure this catches: an input declared in the UI that no command ever reads, so a
/// user changes it and nothing happens.
#[test]
fn every_declared_input_is_used_by_some_step() {
    let registry = Registry::load_dirs(&[models_dir()]);
    for manifest in registry.all() {
        let mut text = String::new();
        for step in &manifest.steps {
            text.push_str(&step.command);
        }
        if let Some(http) = &manifest.http {
            text.push_str(&http.submit_url);
            text.push_str(http.submit_body.as_deref().unwrap_or(""));
        }
        for name in manifest.inputs.keys() {
            assert!(
                text.contains(&format!("{{{{input.{name}")),
                "{}: input {name:?} is declared but never used",
                manifest.id
            );
        }
    }
}

/// Resolves every shipped manifest's steps against a realistic job, which is the only way
/// to catch a typo'd placeholder before a user hits it.
#[test]
fn every_template_resolves_with_a_plausible_job() {
    let registry = Registry::load_dirs(&[models_dir()]);
    let root = std::env::temp_dir().join(format!("geis-manifest-{}", util::new_id()));
    std::fs::create_dir_all(&root).unwrap();

    // Stand-ins for the file inputs the manifests ask for.
    let structure = root.join("backbone.pdb");
    std::fs::write(&structure, "ATOM\n").unwrap();
    let json_input = root.join("fold_input.json");
    std::fs::write(&json_input, "{}").unwrap();

    let provider_vars = BTreeMap::from([
        ("weights_dir".to_string(), "/data/weights".to_string()),
        (
            "endpoint".to_string(),
            "https://api.example.com".to_string(),
        ),
        ("scratch".to_string(), "/scratch".to_string()),
        ("host".to_string(), "gpu-box".to_string()),
        ("remote_dir".to_string(), "/scratch/geis".to_string()),
        ("partition".to_string(), "gpu".to_string()),
    ]);

    for manifest in registry.all() {
        let mut inputs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        for (name, spec) in &manifest.inputs {
            use geis_lib::models::InputKind;
            let value = match spec.kind {
                InputKind::Fasta => serde_json::json!("MKTAYIAKQRQISFVK"),
                InputKind::File => {
                    let extensions = spec.extensions.clone().unwrap_or_default();
                    if extensions.iter().any(|e| e == "json") {
                        serde_json::json!(json_input.to_string_lossy())
                    } else {
                        serde_json::json!(structure.to_string_lossy())
                    }
                }
                InputKind::Dir => serde_json::json!(root.to_string_lossy()),
                InputKind::Text => serde_json::json!("text"),
                _ => continue, // numeric and boolean inputs fall back to their defaults
            };
            inputs.insert(name.clone(), value);
        }

        let request = JobRequest {
            model_id: manifest.id.clone(),
            provider_id: "local".into(),
            world_id: None,
            name: Some(format!("{} smoke", manifest.name)),
            inputs,
        };
        let spec = plan(manifest, &request, &root)
            .unwrap_or_else(|error| panic!("{}: {error}", manifest.id));

        if manifest.requirements.runtime == RuntimeKind::Http {
            let bindings = spec.json_bindings(&provider_vars).unwrap();
            let http = manifest.http.as_ref().unwrap();
            let mut bindings_with_id = spec.json_bindings(&provider_vars).unwrap();
            bindings_with_id.set("job.external_id", "abc123");
            resolve(&http.submit_url, &bindings)
                .unwrap_or_else(|error| panic!("{} submit_url: {error}", manifest.id));
            if let Some(body) = &http.submit_body {
                let resolved = resolve(body, &bindings)
                    .unwrap_or_else(|error| panic!("{} submit_body: {error}", manifest.id));
                serde_json::from_str::<serde_json::Value>(&resolved).unwrap_or_else(|error| {
                    panic!(
                        "{} submit_body is not valid JSON: {error}\n{resolved}",
                        manifest.id
                    )
                });
            }
            if let Some(poll) = &http.poll_url {
                resolve(poll, &bindings_with_id)
                    .unwrap_or_else(|error| panic!("{} poll_url: {error}", manifest.id));
            }
            continue;
        }

        let bindings: Bindings = spec.bindings("/scratch/geis/job-1", &provider_vars);
        for (index, step) in spec.steps.iter().enumerate() {
            let step: &RunStep = step;
            resolve(&step.command, &bindings)
                .unwrap_or_else(|error| panic!("{} step {}: {error}", manifest.id, index + 1));
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

/// Gated models must stay gated: if this ever passes without a weights directory, the app
/// would be inviting someone to run AlphaFold 3 with parameters it has no right to.
#[test]
fn manually_gated_models_are_flagged() {
    let registry = Registry::load_dirs(&[models_dir()]);
    let af3 = registry.get("alphafold3").expect("alphafold3 manifest");
    assert!(af3.needs_manual_weights());

    let boltz = registry.get("boltz2").expect("boltz2 manifest");
    assert!(!boltz.needs_manual_weights());
}

/// The local provider exposes exactly the variables the manifests reference.
#[tokio::test]
async fn provider_vars_cover_what_manifests_reference() {
    let record = ProviderRecord {
        id: "local".into(),
        name: "test".into(),
        config: ProviderConfig::Local(LocalConfig {
            exec: ExecOptions {
                weights_dir: Some("/data/weights".into()),
                scratch: Some("/scratch".into()),
                ..Default::default()
            },
        }),
        enabled: true,
        created_at: util::now_iso(),
        probe: None,
        status: ProviderStatus::Unknown,
    };
    let provider = geis_lib::compute::provider::build(&record);
    let vars = provider.vars();
    assert_eq!(
        vars.get("weights_dir").map(String::as_str),
        Some("/data/weights")
    );
    assert_eq!(vars.get("scratch").map(String::as_str), Some("/scratch"));
}
