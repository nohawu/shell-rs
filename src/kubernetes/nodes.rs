//! Node capacity and system information are independent of optional metrics.
use crate::i18n::t;
use serde_json::Value;

pub(super) fn summary(value: &Value) -> String {
    let capacity = &value["status"]["capacity"];
    let ip = value["status"]["addresses"]
        .as_array()
        .and_then(|items| items.iter().find(|a| a["type"] == "InternalIP"))
        .and_then(|a| a["address"].as_str())
        .unwrap_or("—");
    format!(
        "CPU {} · {} · {ip}",
        capacity["cpu"].as_str().unwrap_or("—"),
        capacity["memory"].as_str().unwrap_or("—")
    )
}

pub(super) fn overview(value: &Value) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    for (key, field) in [
        ("k8s.name", &value["metadata"]["name"]),
        ("k8s.created", &value["metadata"]["creationTimestamp"]),
        ("k8s.cpu_capacity", &value["status"]["capacity"]["cpu"]),
        (
            "k8s.cpu_allocatable",
            &value["status"]["allocatable"]["cpu"],
        ),
        (
            "k8s.memory_capacity",
            &value["status"]["capacity"]["memory"],
        ),
        (
            "k8s.memory_allocatable",
            &value["status"]["allocatable"]["memory"],
        ),
        ("k8s.pod_capacity", &value["status"]["allocatable"]["pods"]),
        ("k8s.node_os", &value["status"]["nodeInfo"]["osImage"]),
        (
            "k8s.node_arch",
            &value["status"]["nodeInfo"]["architecture"],
        ),
        (
            "k8s.node_kernel",
            &value["status"]["nodeInfo"]["kernelVersion"],
        ),
        (
            "k8s.node_kubelet",
            &value["status"]["nodeInfo"]["kubeletVersion"],
        ),
        (
            "k8s.node_runtime",
            &value["status"]["nodeInfo"]["containerRuntimeVersion"],
        ),
    ] {
        if let Some(text) = field.as_str().filter(|s| !s.is_empty()) {
            rows.push((key, text.to_owned()));
        }
    }
    if let Some(addresses) = value["status"]["addresses"].as_array() {
        rows.push((
            "k8s.node_addresses",
            addresses
                .iter()
                .filter_map(|a| {
                    Some(format!(
                        "{}: {}",
                        a["type"].as_str()?,
                        a["address"].as_str()?
                    ))
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    if let Some(labels) = value["metadata"]["labels"].as_object() {
        rows.push((
            "k8s.labels",
            labels
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if let Some(conditions) = value["status"]["conditions"].as_array() {
        rows.push((
            "k8s.node_conditions",
            conditions
                .iter()
                .map(|c| {
                    format!(
                        "{}={} · {}",
                        c["type"].as_str().unwrap_or(""),
                        c["status"].as_str().unwrap_or(""),
                        c["message"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    if let Some(taints) = value["spec"]["taints"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        rows.push((
            "k8s.node_taints",
            taints
                .iter()
                .map(|c| {
                    format!(
                        "{}={}:{}",
                        c["key"].as_str().unwrap_or(""),
                        c["value"].as_str().unwrap_or(""),
                        c["effect"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    rows
}

/// kubectl owns Metrics API version discovery and unit/percentage formatting.
/// Require the named node and all four columns; never synthesize zero usage.
pub(super) fn metrics(output: &str, name: &str) -> Result<Vec<(&'static str, String)>, String> {
    let columns = output
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .find(|c| c.len() == 5 && c[0] == name)
        .ok_or_else(|| t!("k8s.metrics_incomplete").to_string())?;
    for field in [&columns[2], &columns[4]] {
        if !field
            .strip_suffix('%')
            .is_some_and(|v| v.parse::<u64>().is_ok())
        {
            return Err(t!("k8s.metrics_incomplete").to_string());
        }
    }
    if columns[1] == "<unknown>" || columns[3] == "<unknown>" {
        return Err(t!("k8s.metrics_incomplete").to_string());
    }
    Ok(vec![
        ("k8s.cpu_usage", format!("{} · {}", columns[1], columns[2])),
        (
            "k8s.memory_usage",
            format!("{} · {}", columns[3], columns[4]),
        ),
    ])
}
