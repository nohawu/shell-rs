use super::model::{Resource, ResourceKind};
use crate::i18n::t;

const STATUS: &str = "__SHELLRS_K8S_EXIT__:";
pub(super) const LOG_BYTES: usize = 512 * 1024;

/// Quote every argument, including contexts from an untrusted kubeconfig.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

pub(super) fn kubectl(context: &str) -> String {
    format!("kubectl --context={}", quote(context))
}

/// SSH's shared exec API returns stdout only. Merge stderr and frame the
/// exit status so Forbidden, missing kubectl, and truncation stay visible.
pub(super) fn remote(command: &str) -> String {
    let command = command.replacen("kubectl ", "kubectl --request-timeout=8s ", 1);
    let script = format!(
        "export LC_ALL=C PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"; {{ {command}; }} 2>&1; code=$?; printf '\\n{STATUS}%s\\n' \"$code\""
    );
    // The login shell may be fish or csh. Encode the script so the outer
    // single-quoted argument has neither nested quotes nor history `!`.
    let encoded = script
        .bytes()
        .map(|byte| format!("\\0{byte:03o}"))
        .collect::<String>();
    format!("sh -c 'printf \"%b\" \"{encoded}\" | sh'")
}

pub(super) fn answer(output: &str) -> Result<String, String> {
    let (body, status) = output
        .rsplit_once(&format!("\n{STATUS}"))
        .ok_or_else(|| t!("k8s.incomplete").to_string())?;
    match status.trim() {
        "0" => Ok(body.to_owned()),
        code => Err(format!("{} (exit {code})", body.trim())),
    }
}

pub(super) fn contexts_command() -> String {
    remote(
        "kubectl config current-context && printf '\\n__SHELLRS_CONTEXTS__\\n' && kubectl config get-contexts -o name",
    )
}

pub(super) fn contexts(output: &str) -> Result<(String, Vec<String>), String> {
    let output = answer(output)?;
    let (current, rest) = output
        .split_once("\n__SHELLRS_CONTEXTS__\n")
        .ok_or_else(|| t!("k8s.incomplete").to_string())?;
    let current = current.trim().to_owned();
    let mut names = rest
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if current.is_empty() || !names.contains(&current) {
        return Err(t!("k8s.no_context").to_string());
    }
    Ok((current, names))
}

pub(super) fn namespaces(context: &str) -> String {
    remote(&format!(
        "{} get namespaces -o json --chunk-size=200",
        kubectl(context)
    ))
}

pub(super) fn list(
    context: &str,
    namespace: Option<&str>,
    kind: ResourceKind,
    selector: Option<&str>,
) -> String {
    let scope = namespace
        .map(|ns| format!("--namespace={}", quote(ns)))
        .unwrap_or_else(|| "--all-namespaces".to_owned());
    let selector = selector
        .map(|s| format!(" --selector={}", quote(s)))
        .unwrap_or_default();
    remote(&format!(
        "{} get {} {scope} -o json --chunk-size=200{selector}",
        kubectl(context),
        kind.resource()
    ))
}

pub(super) fn detail(context: &str, resource: &Resource) -> String {
    remote(&generated(context, resource, "get", Some("-o json")))
}

fn generated(context: &str, resource: &Resource, verb: &str, extra: Option<&str>) -> String {
    format!(
        "{} {verb} {} {} --namespace={} {}",
        kubectl(context),
        resource.kind.resource(),
        quote(&resource.name),
        quote(&resource.namespace),
        extra.unwrap_or("")
    )
    .trim_end()
    .to_owned()
}

#[derive(Clone, Debug)]
pub(super) struct LogOptions {
    pub container: String,
    pub tail: u32,
    pub since: String,
    pub previous: bool,
}

impl Default for LogOptions {
    fn default() -> Self {
        Self {
            container: String::new(),
            tail: 1000,
            since: "1h".to_owned(),
            previous: false,
        }
    }
}

pub(super) fn logs(
    context: &str,
    resource: &Resource,
    options: &LogOptions,
    follow: bool,
) -> Option<String> {
    if resource.kind != ResourceKind::Pod || options.container.is_empty() {
        return None;
    }
    let mut command = format!(
        "{} logs {} --namespace={} --container={} --tail={} --timestamps=true",
        kubectl(context),
        quote(&resource.name),
        quote(&resource.namespace),
        quote(&options.container),
        options.tail.clamp(1, 10000)
    );
    if !follow {
        command.push_str(&format!(" --limit-bytes={LOG_BYTES}"));
    }
    if !options.since.is_empty() {
        command.push_str(&format!(" --since={}", quote(&options.since)));
    }
    if options.previous {
        command.push_str(" --previous=true");
    }
    if follow {
        command.push_str(" --follow=true");
    }
    Some(command)
}

pub(super) fn suggestions(
    context: &str,
    resource: &Resource,
    options: &LogOptions,
) -> Vec<(&'static str, String)> {
    let mut commands = vec![
        (
            "k8s.command.describe",
            generated(context, resource, "describe", None),
        ),
        (
            "k8s.command.yaml",
            generated(context, resource, "get", Some("-o yaml")),
        ),
    ];
    if let Some(logs) = logs(context, resource, options, false) {
        commands.push(("k8s.command.logs", logs));
        commands.push((
            "k8s.command.follow",
            logs_command_follow(context, resource, options),
        ));
        commands.push((
            "k8s.command.exec",
            format!(
                "{} exec -it {} --namespace={} --container={} -- /bin/sh",
                kubectl(context),
                quote(&resource.name),
                quote(&resource.namespace),
                quote(&options.container)
            ),
        ));
    }
    commands
}

fn logs_command_follow(context: &str, resource: &Resource, options: &LogOptions) -> String {
    logs(context, resource, options, true).unwrap_or_default()
}
