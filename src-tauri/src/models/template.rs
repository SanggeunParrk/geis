//! Placeholder substitution for run-step commands.
//!
//! Vocabulary:
//!
//! ```text
//! {{job.id}}              job uuid
//! {{job.workdir}}         absolute workdir on the executing host
//! {{job.name}}            user-facing job name
//! {{input.<name>}}        scalar value as typed
//! {{input.<name>.path}}   staged file path for fasta/file/dir inputs
//! {{input.<name>.raw}}    the same, unquoted
//! {{provider.<key>}}      provider config value (weights_dir, scratch, python, ...)
//! {{env.<NAME>}}          environment variable - see `EnvMode`
//! ```
//!
//! Unknown placeholders are an error rather than an empty string: a silently empty
//! `--weights` flag produces a confusing failure hundreds of log lines later.

use std::collections::BTreeMap;

/// How `{{env.NAME}}` is handled, which depends on *where* the resolved text will run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EnvMode {
    /// Emit `"${NAME}"` and let the executing shell expand it.
    ///
    /// This is the only correct choice for a command that may run on another host. The
    /// variable is set there, by the provider's environment or the site's module system,
    /// and reading this machine's environment instead would substitute an empty string
    /// for a database path and fail thousands of log lines later.
    #[default]
    ShellExpansion,
    /// Substitute this machine's value. For HTTP request bodies, which are built and sent
    /// from here, so there is no other host involved.
    LocalValue,
}

#[derive(Debug, Default)]
pub struct Bindings {
    values: BTreeMap<String, String>,
    env_mode: EnvMode,
}

impl Bindings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_env_mode(mode: EnvMode) -> Self {
        Self {
            values: BTreeMap::new(),
            env_mode: mode,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.values.insert(key.into(), value.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("unclosed {{{{ in template")]
    Unclosed,
    #[error("unknown placeholder {{{{{0}}}}}")]
    Unknown(String),
    #[error("{0:?} is not a usable environment variable name")]
    BadEnvName(String),
}

/// Guards the one place template text becomes shell syntax rather than a quoted value.
fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

pub fn resolve(template: &str, bindings: &Bindings) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or(TemplateError::Unclosed)?;
        let key = after[..end].trim();
        let value = match bindings.get(key) {
            Some(value) => value.to_string(),
            None => match key.strip_prefix("env.") {
                Some(var) if !is_env_name(var) => {
                    return Err(TemplateError::BadEnvName(var.to_string()))
                }
                Some(var) => match bindings.env_mode {
                    EnvMode::ShellExpansion => format!("\"${{{var}}}\""),
                    EnvMode::LocalValue => std::env::var(var).unwrap_or_default(),
                },
                None => return Err(TemplateError::Unknown(key.to_string())),
            },
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Quote a path for a POSIX shell. Run steps are shell strings by design (clusters need
/// pipes, module loads, and activation scripts), so every interpolated path gets quoted.
pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/=:@,+".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_known_keys() {
        let mut b = Bindings::new();
        b.set("job.workdir", "/tmp/j1").set("input.recycles", "3");
        let out = resolve("run --out {{job.workdir}} -r {{input.recycles}}", &b).unwrap();
        assert_eq!(out, "run --out /tmp/j1 -r 3");
    }

    #[test]
    fn rejects_unknown_keys() {
        let b = Bindings::new();
        assert!(matches!(
            resolve("run {{input.nope}}", &b),
            Err(TemplateError::Unknown(_))
        ));
    }

    #[test]
    fn env_vars_are_expanded_by_the_executing_shell_not_this_one() {
        std::env::set_var("GEIS_TEST_LOCAL_ONLY", "local-value");

        // A remote command must carry the variable through, not this machine's value.
        let shell = Bindings::with_env_mode(EnvMode::ShellExpansion);
        assert_eq!(
            resolve("--db {{env.GEIS_TEST_LOCAL_ONLY}}", &shell).unwrap(),
            "--db \"${GEIS_TEST_LOCAL_ONLY}\""
        );

        // An HTTP body is built here, so here is where it should be read.
        let local = Bindings::with_env_mode(EnvMode::LocalValue);
        assert_eq!(
            resolve("{{env.GEIS_TEST_LOCAL_ONLY}}", &local).unwrap(),
            "local-value"
        );
    }

    #[test]
    fn an_unset_variable_still_expands_on_the_far_side() {
        let shell = Bindings::with_env_mode(EnvMode::ShellExpansion);
        assert_eq!(
            resolve("{{env.DEFINITELY_NOT_SET_HERE}}", &shell).unwrap(),
            "\"${DEFINITELY_NOT_SET_HERE}\""
        );
    }

    #[test]
    fn env_names_that_would_inject_shell_syntax_are_refused() {
        let shell = Bindings::with_env_mode(EnvMode::ShellExpansion);
        for bad in ["PATH; rm -rf /", "a b", "", "1PATH"] {
            assert!(
                matches!(
                    resolve(&format!("{{{{env.{bad}}}}}"), &shell),
                    Err(TemplateError::BadEnvName(_))
                ),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn a_quoted_value_dropped_into_manifest_quotes_would_break() {
        // The reason `bindings()` documents "do not add your own quotes": this is what a
        // manifest author gets if they do.
        let mut b = Bindings::new();
        b.set("input.label", shell_quote("from test"));
        assert_eq!(
            resolve("echo \"MARKER {{input.label}}\"", &b).unwrap(),
            "echo \"MARKER 'from test'\""
        );
        assert_eq!(
            resolve("echo MARKER {{input.label}}", &b).unwrap(),
            "echo MARKER 'from test'"
        );
    }

    #[test]
    fn a_value_that_looks_like_a_placeholder_is_not_expanded_again() {
        // A sequence name or job name may contain `{{`; it is text, and a second pass over
        // it would either fail the job or leak another binding into the command.
        let mut b = Bindings::new();
        b.set("input.name", "{{job.secret}}")
            .set("job.secret", "leaked");
        assert_eq!(
            resolve("x {{input.name}} y", &b).unwrap(),
            "x {{job.secret}} y"
        );
    }

    #[test]
    fn malformed_templates_are_errors() {
        let mut b = Bindings::new();
        b.set("a", "1");
        assert!(matches!(
            resolve("run {{a", &b),
            Err(TemplateError::Unclosed)
        ));
        assert!(matches!(
            resolve("run {{", &b),
            Err(TemplateError::Unclosed)
        ));
        // Nested braces read as a key with braces in it, which is never bound.
        assert!(matches!(
            resolve("{{ {{a}} }}", &b),
            Err(TemplateError::Unknown(_))
        ));
        assert!(matches!(
            resolve("{{}}", &b),
            Err(TemplateError::Unknown(_))
        ));
        // Lone closing braces and single braces are plain text.
        assert_eq!(resolve("}} { } {{ a }}", &b).unwrap(), "}} { } 1");
        // Multi-byte text around a placeholder survives slicing.
        assert_eq!(resolve("단백질 {{a}} 구조", &b).unwrap(), "단백질 1 구조");
    }

    /// What `sh` would make of a single word: just enough of POSIX quoting to check that
    /// `shell_quote` round-trips.
    fn unquote(word: &str) -> Option<String> {
        let mut out = String::new();
        let mut quoted = false;
        let mut chars = word.chars();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('\'', _) => quoted = !quoted,
                ('\\', false) => out.push(chars.next()?),
                // Anything the shell would act on outside quotes means the word was unsafe.
                (c, false) if " \t\n;&|$`\"<>()*?[]#~{}!".contains(c) => return None,
                (c, _) => out.push(c),
            }
        }
        (!quoted).then_some(out)
    }

    #[test]
    fn hostile_values_stay_one_literal_word() {
        for value in [
            "",
            "plain",
            "with space",
            "semi;colon",
            "$(reboot)",
            "`id`",
            "new\nline",
            "it's",
            "''",
            "a\\b",
            "\"dq\"",
            "*.cif",
            "~/x",
            "-rf",
            "단백질",
        ] {
            let quoted = shell_quote(value);
            assert_eq!(
                unquote(&quoted).as_deref(),
                Some(value),
                "{value:?} quoted as {quoted}"
            );
        }
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(shell_quote("/tmp/a_b-1.cif"), "/tmp/a_b-1.cif");
        assert_eq!(shell_quote("/tmp/my runs"), "'/tmp/my runs'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
