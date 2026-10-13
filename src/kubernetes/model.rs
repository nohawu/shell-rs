use serde_json::Value;

/// Namespaced resource kinds offered by the Kubernetes browser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceKind {
    #[default]
    Pod,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Service,
    Ingress,
    Job,
    CronJob,
    PersistentVolumeClaim,
}

impl ResourceKind {
    pub const ALL: [Self; 10] = [
        Self::Pod,
        Self::Deployment,
        Self::StatefulSet,
        Self::DaemonSet,
        Self::ReplicaSet,
        Self::Service,
        Self::Ingress,
        Self::Job,
        Self::CronJob,
        Self::PersistentVolumeClaim,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pod => "Pod",
            Self::Deployment => "Deployment",
            Self::StatefulSet => "StatefulSet",
            Self::DaemonSet => "DaemonSet",
            Self::ReplicaSet => "ReplicaSet",
            Self::Service => "Service",
            Self::Ingress => "Ingress",
            Self::Job => "Job",
            Self::CronJob => "CronJob",
            Self::PersistentVolumeClaim => "PVC",
        }
    }

    pub(super) fn resource(self) -> &'static str {
        match self {
            Self::Pod => "pods",
            Self::Deployment => "deployments",
            Self::StatefulSet => "statefulsets",
            Self::DaemonSet => "daemonsets",
            Self::ReplicaSet => "replicasets",
            Self::Service => "services",
            Self::Ingress => "ingresses",
            Self::Job => "jobs",
            Self::CronJob => "cronjobs",
            Self::PersistentVolumeClaim => "persistentvolumeclaims",
        }
    }
}

/// Commands routed through the workspace's single Kubernetes handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KubernetesCommand {
    Refresh,
    Context(String),
    Namespace(Option<String>),
    Kind(ResourceKind),
    Details { namespace: String, name: String },
    CloseDetails,
    DetailTab(usize),
    Container(String),
    Tail(u32),
    Since(String),
    Previous(bool),
    Regex(bool),
    ReadLogs,
    LiveLogs(bool),
    FollowLogs(bool),
    CopyValue(String),
    RefreshDetails,
    RelatedPod(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Resource {
    pub kind: ResourceKind,
    pub name: String,
    pub namespace: String,
    pub summary: String,
    pub labels: String,
    pub status: Option<(String, Tone)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    Good,
    Warning,
    Danger,
    Quiet,
}

pub(super) fn status(value: &Value, kind: ResourceKind) -> Option<(String, Tone)> {
    let status = &value["status"];
    let spec = &value["spec"];
    let count = |v: &Value| v.as_u64().unwrap_or(0);
    let readiness = |ready: u64, desired: u64| {
        (
            format!("{ready}/{desired}"),
            if ready >= desired {
                Tone::Good
            } else {
                Tone::Warning
            },
        )
    };
    Some(match kind {
        ResourceKind::Pod => {
            let states = status["containerStatuses"].as_array();
            let reason = ["initContainerStatuses", "containerStatuses"]
                .into_iter()
                .flat_map(|field| status[field].as_array().into_iter().flatten())
                .find_map(|s| s["state"]["waiting"]["reason"].as_str());
            let phase = if value["metadata"]["deletionTimestamp"].is_string() {
                "Terminating"
            } else {
                reason.unwrap_or(status["phase"].as_str().unwrap_or("Unknown"))
            };
            let ready = states.is_some_and(|items| {
                !items.is_empty()
                    && spec["containers"]
                        .as_array()
                        .is_none_or(|containers| containers.len() == items.len())
                    && items.iter().all(|s| s["ready"] == true)
            });
            let tone = match phase {
                "Running" if ready => Tone::Good,
                "Succeeded" => Tone::Good,
                "Failed"
                | "CrashLoopBackOff"
                | "ImagePullBackOff"
                | "ErrImagePull"
                | "CreateContainerConfigError" => Tone::Danger,
                "Pending" | "Running" | "ContainerCreating" | "PodInitializing" | "Terminating" => {
                    Tone::Warning
                }
                _ => Tone::Quiet,
            };
            (phase.to_owned(), tone)
        }
        ResourceKind::Deployment | ResourceKind::StatefulSet | ResourceKind::ReplicaSet => {
            readiness(
                count(&status["readyReplicas"]),
                spec["replicas"].as_u64().unwrap_or(1),
            )
        }
        ResourceKind::DaemonSet => readiness(
            count(&status["numberReady"]),
            count(&status["desiredNumberScheduled"]),
        ),
        ResourceKind::PersistentVolumeClaim => {
            let phase = status["phase"].as_str().unwrap_or("Unknown");
            (
                phase.to_owned(),
                match phase {
                    "Bound" => Tone::Good,
                    "Lost" => Tone::Danger,
                    "Pending" => Tone::Warning,
                    _ => Tone::Quiet,
                },
            )
        }
        ResourceKind::Job => {
            let failed = status["conditions"].as_array().is_some_and(|conditions| {
                conditions
                    .iter()
                    .any(|c| c["type"] == "Failed" && c["status"] == "True")
            });
            if failed {
                ("Failed".into(), Tone::Danger)
            } else {
                readiness(
                    count(&status["succeeded"]),
                    spec["completions"].as_u64().unwrap_or(1),
                )
            }
        }
        _ => return None,
    })
}

impl Resource {
    pub fn key(&self) -> String {
        format!(
            "k8s-resource:{}:{}/{}",
            self.kind.resource(),
            self.namespace,
            self.name
        )
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        [&self.name, &self.namespace, &self.summary, &self.labels]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
    }
}

pub(super) fn names(output: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(output).map_err(|error| error.to_string())?;
    let items = value["items"]
        .as_array()
        .ok_or("Missing Kubernetes items")?;
    let mut names = items
        .iter()
        .map(|item| {
            item["metadata"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "Missing resource name".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

pub(super) fn resources(output: &str, kind: ResourceKind) -> Result<Vec<Resource>, String> {
    let value: Value = serde_json::from_str(output).map_err(|error| error.to_string())?;
    let items = value["items"]
        .as_array()
        .ok_or("Missing Kubernetes items")?;
    let mut rows = items
        .iter()
        .map(|item| {
            let name = item["metadata"]["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or("Missing resource name")?
                .to_owned();
            let namespace = item["metadata"]["namespace"]
                .as_str()
                .filter(|ns| !ns.is_empty())
                .ok_or("Missing resource namespace")?
                .to_owned();
            let labels = item["metadata"]["labels"]
                .as_object()
                .map(|labels| {
                    labels
                        .iter()
                        .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            Ok(Resource {
                kind,
                name,
                namespace,
                summary: summary(item, kind),
                labels,
                status: status(item, kind),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    rows.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
    Ok(rows)
}

fn summary(item: &Value, kind: ResourceKind) -> String {
    let status = &item["status"];
    let spec = &item["spec"];
    let text = |value: &Value| value.as_str().unwrap_or("—").to_owned();
    let count = |value: &Value| value.as_u64().unwrap_or(0);
    match kind {
        ResourceKind::Pod => {
            let statuses = status["containerStatuses"].as_array();
            let ready = statuses.map_or(0, |items| {
                items.iter().filter(|s| s["ready"] == true).count()
            });
            let total = spec["containers"].as_array().map_or(0, Vec::len);
            let restarts: u64 = statuses.map_or(0, |items| {
                items.iter().map(|s| count(&s["restartCount"])).sum()
            });
            let phase =
                self::status(item, kind).map_or_else(|| "Unknown".to_owned(), |(phase, _)| phase);
            format!(
                "{phase} · {ready}/{total} · ↻ {restarts} · {}",
                text(&spec["nodeName"])
            )
        }
        ResourceKind::Deployment | ResourceKind::StatefulSet | ResourceKind::ReplicaSet => format!(
            "{}/{}",
            count(&status["readyReplicas"]),
            spec["replicas"].as_u64().unwrap_or(1)
        ),
        ResourceKind::DaemonSet => format!(
            "{}/{}",
            count(&status["numberReady"]),
            count(&status["desiredNumberScheduled"])
        ),
        ResourceKind::Service => {
            let ports = spec["ports"]
                .as_array()
                .map(|ports| {
                    ports
                        .iter()
                        .map(|p| {
                            format!(
                                "{}/{}",
                                count(&p["port"]),
                                p["protocol"].as_str().unwrap_or("TCP")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!(
                "{} · {} · {ports}",
                text(&spec["type"]),
                text(&spec["clusterIP"])
            )
        }
        ResourceKind::Ingress => spec["rules"]
            .as_array()
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|rule| rule["host"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default(),
        ResourceKind::Job => format!(
            "{}/{} · active {} · failed {}",
            count(&status["succeeded"]),
            spec["completions"].as_u64().unwrap_or(1),
            count(&status["active"]),
            count(&status["failed"])
        ),
        ResourceKind::CronJob => format!(
            "{} · suspend {}",
            text(&spec["schedule"]),
            spec["suspend"].as_bool().unwrap_or(false)
        ),
        ResourceKind::PersistentVolumeClaim => format!(
            "{} · {} · {}",
            text(&status["phase"]),
            text(&status["capacity"]["storage"]),
            text(&spec["storageClassName"])
        ),
    }
}

pub(super) fn containers(value: &Value) -> Vec<String> {
    ["containers", "initContainers", "ephemeralContainers"]
        .into_iter()
        .flat_map(|field| value["spec"][field].as_array().into_iter().flatten())
        .filter_map(|container| container["name"].as_str().map(str::to_owned))
        .collect()
}

/// A selector for workloads and services, including set-based expressions.
pub(super) fn pod_selector(value: &Value, kind: ResourceKind) -> Option<String> {
    let selector = &value["spec"]["selector"];
    let labels = if kind == ResourceKind::Service {
        selector
    } else {
        &selector["matchLabels"]
    };
    let mut terms = labels
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, val)| val.as_str().map(|val| format!("{key}={val}")))
        .collect::<Vec<_>>();
    if let Some(expressions) = selector["matchExpressions"].as_array() {
        for expr in expressions {
            let key = expr["key"].as_str()?;
            let values = expr["values"]
                .as_array()
                .map(|vals| {
                    vals.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            terms.push(match expr["operator"].as_str()? {
                "In" => format!("{key} in ({values})"),
                "NotIn" => format!("{key} notin ({values})"),
                "Exists" => key.to_owned(),
                "DoesNotExist" => format!("!{key}"),
                _ => return None,
            });
        }
    }
    (!terms.is_empty()).then(|| terms.join(","))
}

pub(super) fn overview(value: &Value, kind: ResourceKind) -> Vec<(&'static str, String)> {
    let labels = value["metadata"]["labels"]
        .as_object()
        .map(|labels| {
            labels
                .iter()
                .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let spec = if kind == ResourceKind::Pod {
        &value["spec"]
    } else if kind == ResourceKind::CronJob {
        &value["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    } else {
        &value["spec"]["template"]["spec"]
    };
    let images = ["containers", "initContainers", "ephemeralContainers"]
        .into_iter()
        .flat_map(|field| spec[field].as_array().into_iter().flatten())
        .map(|container| {
            format!(
                "{}: {}",
                container["name"].as_str().unwrap_or(""),
                container["image"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut rows = vec![("k8s.summary", summary(value, kind))];
    for (key, field) in [
        ("k8s.name", &value["metadata"]["name"]),
        ("k8s.namespace", &value["metadata"]["namespace"]),
        ("k8s.pod_ip", &value["status"]["podIP"]),
        ("k8s.host_ip", &value["status"]["hostIP"]),
        ("k8s.cluster_ip", &value["spec"]["clusterIP"]),
        ("k8s.node", &value["spec"]["nodeName"]),
    ] {
        if let Some(text) = field.as_str().filter(|text| !text.is_empty()) {
            rows.push((key, text.to_owned()));
        }
    }
    if let Some(addresses) = value["status"]["loadBalancer"]["ingress"].as_array() {
        let addresses = addresses
            .iter()
            .filter_map(|a| a["ip"].as_str().or_else(|| a["hostname"].as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        if !addresses.is_empty() {
            rows.push(("k8s.external_ip", addresses));
        }
    }
    if let Some(created) = value["metadata"]["creationTimestamp"].as_str() {
        rows.push(("k8s.created", created.to_owned()));
    }
    if !labels.is_empty() {
        rows.push(("k8s.labels", labels));
    }
    if !images.is_empty() {
        rows.push(("k8s.images", images));
    }
    rows
}
