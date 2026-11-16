//! Runs models that live behind an HTTP endpoint.
//!
//! The submit/poll/download shape is declared in the manifest's `[http]` section, so a new
//! service is a TOML file rather than a Rust impl. That matters because this is the messy
//! end of the ecosystem: hosted folding services all differ, and some models are only
//! reachable this way.
//!
//! One thing this provider deliberately cannot do is drive a web UI that has no API.
//! AlphaFold Server is the case in point - it has no public API, and automating its web
//! interface would breach its terms. A service either exposes an API we can declare here,
//! or it is not wired in.

use super::exec::{CancelToken, LogSink};
use super::job::JobSpec;
use super::probe::{HostProbe, Tool};
use super::provider::{HttpConfig, Provider, ProviderError, ProviderKind, RunOutcome};
use crate::models::manifest::HttpSpec;
use crate::models::template::resolve;
use crate::util;
use std::collections::BTreeMap;
use std::time::Duration;

/// How many polls in a row may fail before the job is given up on.
const MAX_POLL_FAILURES: u32 = 10;

pub struct HttpProvider {
    id: String,
    config: HttpConfig,
}

impl HttpProvider {
    pub fn new(id: String, config: HttpConfig) -> Self {
        Self { id, config }
    }

    /// Credentials are read from the OS keychain at call time and never cached in the
    /// database, the provider config, or a log line.
    fn credential(&self) -> Option<String> {
        let reference = self.config.secret_ref.as_ref()?;
        crate::legacy::read_secret(crate::SECRET_SERVICE, reference)
    }

    fn client(&self) -> Result<reqwest::Client, ProviderError> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (key, value) in &self.config.headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::try_from(key.as_str()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }
        if let (Some(header), Some(secret)) = (&self.config.auth_header, self.credential()) {
            let rendered = match &self.config.auth_format {
                Some(format) => format.replace("{}", &secret),
                None => secret,
            };
            if let (Ok(name), Ok(mut value)) = (
                reqwest::header::HeaderName::try_from(header.as_str()),
                reqwest::header::HeaderValue::from_str(&rendered),
            ) {
                value.set_sensitive(true);
                headers.insert(name, value);
            }
        }
        reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(300))
            // The credential is a default header, and on a redirect reqwest drops only the
            // standard `Authorization` and cookie headers - an `x-api-key` would follow a
            // redirect to anywhere. So this client follows only redirects that stay on the
            // origin; anything else comes back as the redirect itself.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let stays = attempt
                    .previous()
                    .last()
                    .is_some_and(|from| same_origin_url(from, attempt.url()));
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirects")
                } else if stays {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|e| ProviderError::Setup(e.to_string()))
    }
}

#[async_trait::async_trait]
impl Provider for HttpProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Http
    }

    fn vars(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("endpoint".into(), self.config.endpoint.clone())])
    }

    /// There is no host to interrogate, so the "probe" is a reachability check dressed up
    /// as a `HostProbe` to keep the settings UI uniform: one list of providers, each with
    /// a status and a summary line.
    async fn probe(&self) -> Result<HostProbe, ProviderError> {
        let client = self.client()?;
        let response = client
            .get(&self.config.endpoint)
            .send()
            .await
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        let host = reqwest::Url::parse(&self.config.endpoint)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| self.config.endpoint.clone());
        Ok(HostProbe {
            os: "remote service".into(),
            arch: "http".into(),
            hostname: host,
            cpu: format!("HTTP {}", response.status().as_u16()),
            tools: vec![Tool {
                name: "endpoint".into(),
                path: self.config.endpoint.clone(),
                version: None,
            }],
            complete: true,
            probed_at: util::now_iso(),
            ..Default::default()
        })
    }

    async fn execute(
        &self,
        spec: &JobSpec,
        sink: &LogSink,
        cancel: &CancelToken,
    ) -> Result<RunOutcome, ProviderError> {
        let http = spec.http.as_ref().ok_or_else(|| {
            ProviderError::Setup(
                "this model has no [http] section, so it cannot run on an HTTP provider".into(),
            )
        })?;
        let client = self.client()?;
        let bindings = spec
            .json_bindings(&self.vars())
            .map_err(ProviderError::Io)?;

        let submit_url = resolve(&http.submit_url, &bindings)?;
        sink.note(&format!("{} {}", http.submit_method, redact(&submit_url)));

        let mut request = client.request(
            http.submit_method.parse().unwrap_or(reqwest::Method::POST),
            &submit_url,
        );
        if let Some(template) = &http.submit_body {
            let body = resolve(template, &bindings)?;
            // Fail here rather than sending malformed JSON and reading a 400 back: the
            // manifest author needs to know it was their template.
            serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
                ProviderError::Setup(format!("submit_body did not resolve to valid JSON: {e}"))
            })?;
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ProviderError::Remote(format!(
                "submit returned {}: {}",
                status.as_u16(),
                truncate(&text, 400)
            )));
        }
        let submitted: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);

        // A service that answers synchronously has no id and no poll url; treat the
        // submit response as the final one.
        let external_id = http
            .id_pointer
            .as_ref()
            .and_then(|pointer| submitted.pointer(pointer))
            .map(stringify);

        // A manifest that polls needs the id to poll with. Carrying on without one would
        // take the submit acknowledgement for the result and report success with nothing
        // in hand.
        if http.poll_url.is_some() && external_id.is_none() {
            return Err(ProviderError::Remote(format!(
                "the service accepted the job but its response has no id at {}: {}",
                http.id_pointer
                    .as_deref()
                    .unwrap_or("(no id_pointer declared)"),
                truncate(&text, 400)
            )));
        }

        let mut latest = submitted;
        if let (Some(poll_template), Some(id)) = (&http.poll_url, external_id.clone()) {
            let mut poll_bindings = spec
                .json_bindings(&self.vars())
                .map_err(ProviderError::Io)?;
            poll_bindings.set("job.external_id", id.clone());
            let poll_url = resolve(poll_template, &poll_bindings)?;
            sink.note(&format!(
                "submitted as {id}; polling every {}s",
                http.poll_interval_secs
            ));

            let mut last_status = String::new();
            // A poll that fails is not a job that failed: the service is still working
            // through a dropped connection or a 502 from its proxy. Only a run of them is
            // taken as an answer, and the run is counted in polls so the patience scales
            // with the interval the manifest chose.
            let mut failures = 0u32;
            loop {
                if cancel.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                tokio::time::sleep(Duration::from_secs(http.poll_interval_secs.max(2))).await;

                let polled = match client.get(&poll_url).send().await {
                    Ok(response) => {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        if status.is_success() {
                            serde_json::from_str::<serde_json::Value>(&body)
                                .map_err(|e| format!("poll response is not JSON: {e}"))
                        } else if status.is_client_error()
                            && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                            && status != reqwest::StatusCode::REQUEST_TIMEOUT
                        {
                            // 401, 403, 404: asking again will not change the answer.
                            return Err(ProviderError::Remote(format!(
                                "poll returned {}: {}",
                                status.as_u16(),
                                truncate(&body, 400)
                            )));
                        } else {
                            Err(format!("poll returned {}", status.as_u16()))
                        }
                    }
                    Err(error) => Err(error.to_string()),
                };
                latest = match polled {
                    Ok(value) => {
                        failures = 0;
                        value
                    }
                    Err(reason) => {
                        failures += 1;
                        if failures >= MAX_POLL_FAILURES {
                            return Err(ProviderError::Unreachable(format!(
                                "gave up after {failures} failed polls: {reason}"
                            )));
                        }
                        sink.error(&format!("{reason} - retrying"));
                        continue;
                    }
                };

                let state = http
                    .status_pointer
                    .as_ref()
                    .and_then(|pointer| latest.pointer(pointer))
                    .map(stringify)
                    .unwrap_or_default();
                if state != last_status && !state.is_empty() {
                    sink.note(&format!("status: {state}"));
                    last_status = state.clone();
                }
                if http
                    .failed_values
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(&state))
                {
                    return Err(ProviderError::Remote(format!("service reported {state}")));
                }
                if http
                    .done_values
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(&state))
                {
                    break;
                }
            }
        }

        let downloaded =
            download_results(&client, &self.config.endpoint, http, &latest, spec, sink).await?;
        sink.note(&format!("{downloaded} result file(s) downloaded"));

        Ok(RunOutcome {
            exit_code: 0,
            external_id,
        })
    }
}

/// Saves whatever the result pointer names into the job workdir.
///
/// The pointer may name a single URL, a list of URLs, or inline content - all three occur
/// in practice, so all three are handled rather than requiring the service to be tidy.
async fn download_results(
    client: &reqwest::Client,
    spec_endpoint: &str,
    http: &HttpSpec,
    payload: &serde_json::Value,
    spec: &JobSpec,
    sink: &LogSink,
) -> Result<usize, ProviderError> {
    let output_dir = spec.local_dir.join("outputs");
    std::fs::create_dir_all(&output_dir)?;

    // Always keep the final response: it is the confidence scores and metadata, and
    // without it a failed download is unexplainable after the fact.
    std::fs::write(
        output_dir.join("response.json"),
        serde_json::to_vec_pretty(payload).unwrap_or_default(),
    )?;

    let Some(pointer) = &http.result_pointer else {
        return Ok(1);
    };
    // The manifest said where the result is and it is not there. The saved response is
    // enough to match a loose output pattern, so letting this through reports a success
    // that holds no structure.
    let Some(target) = payload.pointer(pointer) else {
        return Err(ProviderError::Remote(format!(
            "result_pointer {pointer} is not present in the response (kept as outputs/response.json)"
        )));
    };

    let mut urls = Vec::new();
    match target {
        serde_json::Value::String(value) => urls.push(value.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                match item {
                    serde_json::Value::String(value) => urls.push(value.clone()),
                    serde_json::Value::Object(map) => {
                        if let Some(serde_json::Value::String(value)) = map.get("url") {
                            urls.push(value.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        other => {
            std::fs::write(output_dir.join("result.json"), stringify(other))?;
            return Ok(2);
        }
    }

    let anonymous = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| ProviderError::Setup(e.to_string()))?;
    let mut count = 1;
    for (index, url) in urls.iter().enumerate() {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            // Not a URL - the service inlined the structure itself, which several do.
            std::fs::write(output_dir.join(format!("result-{index}.cif")), url)?;
            count += 1;
            continue;
        }
        let filename = reqwest::Url::parse(url)
            .ok()
            .and_then(|parsed| {
                parsed
                    .path_segments()
                    .and_then(|mut segments| segments.next_back().map(str::to_string))
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("result-{index}"));
        // Two results may share a last path segment (`/download?id=1`, `/download?id=2`),
        // and one may be called response.json; neither is allowed to replace a file
        // already written.
        let filename = if output_dir.join(&filename).exists() {
            format!("{index}-{filename}")
        } else {
            filename
        };
        // The credential belongs to the endpoint. A result is often a presigned link to
        // someone else's storage, which has no business seeing it - and which rejects a
        // request that arrives carrying two kinds of authorisation.
        let downloader = if same_origin(url, spec_endpoint) {
            client
        } else {
            &anonymous
        };
        let mut response = downloader
            .get(url)
            .send()
            .await
            .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
        // The endpoint handing the file off to storage elsewhere - a presigned link - is
        // the redirect the credentialled client will not follow. It is followed here,
        // without the credential.
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|location| response.url().join(location).ok());
            if let Some(next) = next {
                response = anonymous
                    .get(next)
                    .send()
                    .await
                    .map_err(|e| ProviderError::Unreachable(e.to_string()))?;
            }
        }
        if !response.status().is_success() {
            return Err(ProviderError::Remote(format!(
                "downloading {filename} returned {}",
                response.status().as_u16()
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ProviderError::Remote(e.to_string()))?;
        std::fs::write(output_dir.join(&filename), &bytes)?;
        sink.note(&format!("downloaded {filename} ({} bytes)", bytes.len()));
        count += 1;
    }
    Ok(count)
}

/// Whether two URLs name the same scheme, host and port.
fn same_origin(url: &str, endpoint: &str) -> bool {
    match (reqwest::Url::parse(url), reqwest::Url::parse(endpoint)) {
        (Ok(a), Ok(b)) => same_origin_url(&a, &b),
        _ => false,
    }
}

fn same_origin_url(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn stringify(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        // Back off to a character boundary: a body in any script but ASCII would
        // otherwise panic here, inside the job's task, and leave it "running" for good.
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &text[..end])
    }
}

/// Strips query strings before logging a URL: API keys are passed that way often enough
/// that logging one verbatim would eventually leak a credential into a shareable log.
fn redact(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?..."),
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncating_never_splits_a_character() {
        // Byte 400 falls inside a three-byte character.
        let text = "가".repeat(200);
        let out = truncate(&text, 400);
        assert!(out.ends_with("..."));
        assert_eq!(out.trim_end_matches("...").chars().count(), 133);
        assert_eq!(truncate("short", 400), "short");
    }

    #[test]
    fn the_credential_only_goes_back_to_the_endpoint() {
        let endpoint = "https://api.example/v1";
        assert!(same_origin("https://api.example/files/a.cif", endpoint));
        assert!(same_origin("https://api.example:443/files/a.cif", endpoint));
        assert!(!same_origin(
            "https://bucket.s3.amazonaws.com/a.cif?X-Amz=1",
            endpoint
        ));
        assert!(!same_origin("http://api.example/files/a.cif", endpoint));
        assert!(!same_origin("not a url", endpoint));
    }

    #[test]
    fn a_redirect_keeps_the_credential_only_on_its_origin() {
        let url = |text: &str| reqwest::Url::parse(text).unwrap();
        let api = url("https://api.example/v1/jobs/1");
        assert!(same_origin_url(
            &api,
            &url("https://api.example/v1/jobs/1/result")
        ));
        assert!(!same_origin_url(
            &api,
            &url("https://bucket.s3.example/x?sig=1")
        ));
        assert!(
            !same_origin_url(&api, &url("http://api.example/v1/jobs/1")),
            "a downgrade is another origin"
        );
        assert!(!same_origin_url(&api, &url("https://api.example:8443/v1")));
        assert!(!same_origin_url(&api, &url("https://api.example.evil/v1")));
    }

    #[test]
    fn redacts_query_strings() {
        assert_eq!(
            redact("https://api.example/v1/fold?key=abc123"),
            "https://api.example/v1/fold?..."
        );
        assert_eq!(
            redact("https://api.example/v1/fold"),
            "https://api.example/v1/fold"
        );
    }
}
