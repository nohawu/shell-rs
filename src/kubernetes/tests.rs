use serde_json::json;

use super::{
    commands::{self, LogOptions},
    details::filter,
    model::{self, Resource, ResourceKind},
};

fn pod() -> Resource {
    Resource {
        kind: ResourceKind::Pod,
        name: "web-0".into(),
        namespace: "production".into(),
        summary: String::new(),
        labels: String::new(),
    }
}

#[test]
fn framed_replies_distinguish_rbac_errors_empty_output_and_truncation() {
    assert_eq!(
        commands::answer("\n__SHELLRS_K8S_EXIT__:0\n"),
        Ok(String::new())
    );
    assert_eq!(
        commands::answer("Error from server (Forbidden)\n__SHELLRS_K8S_EXIT__:1\n"),
        Err("Error from server (Forbidden) (exit 1)".into())
    );
    assert!(commands::answer("{\"items\":[]}").is_err());
    // Marker-like content inside a log line is not the final status.
    assert_eq!(
        commands::answer("__SHELLRS_K8S_EXIT__:1\ntext\n__SHELLRS_K8S_EXIT__:0\n"),
        Ok("__SHELLRS_K8S_EXIT__:1\ntext".into())
    );
}

#[test]
fn discovery_preserves_context_names_and_all_namespaces() {
    let (current, contexts) = commands::contexts(
        "prod\n\n__SHELLRS_CONTEXTS__\nqa\nprod\nprod\n\n__SHELLRS_K8S_EXIT__:0\n",
    )
    .unwrap();
    assert_eq!(current, "prod");
    assert_eq!(contexts, ["prod", "qa"]);
    assert_eq!(
        model::names(r#"{"items":[{"metadata":{"name":"z"}},{"metadata":{"name":"a"}}]}"#).unwrap(),
        ["a", "z"]
    );
    assert!(model::names(r#"{"items":[{}]}"#).is_err());
}

#[test]
fn queries_are_explicitly_scoped_and_shell_quoted() {
    let context = "prod';$(touch /tmp/oops)";
    let wire = commands::list(context, Some("team-a"), ResourceKind::Deployment, None);
    assert!(wire.starts_with("sh -c '"));
    let command = script(&wire);
    assert!(command.contains("deployments"));
    assert!(command.contains("--namespace="));
    assert!(!command.contains("--all-namespaces"));
    assert!(
        script(&commands::list("prod", None, ResourceKind::Service, None))
            .contains("--all-namespaces")
    );
    let generated = commands::suggestions(
        context,
        &pod(),
        &LogOptions {
            container: "api".into(),
            ..Default::default()
        },
    );
    assert!(
        generated
            .iter()
            .all(|(_, command)| command.contains("--context='prod'\\'';$(touch /tmp/oops)'"))
    );
    assert!(
        generated
            .iter()
            .all(|(_, command)| command.contains("--namespace='production'"))
    );
    assert!(
        generated
            .iter()
            .any(|(_, command)| command.contains("--container='api' -- /bin/sh"))
    );
}

#[test]
fn log_queries_bound_work_and_bind_container_previous_and_time() {
    let options = LogOptions {
        container: "init".into(),
        tail: 500,
        since: "15m".into(),
        previous: true,
    };
    let command = commands::logs("cluster", &pod(), &options, false).unwrap();
    for option in [
        "--container='init'",
        "--tail=500",
        "--limit-bytes=524288",
        "--since='15m'",
        "--previous=true",
        "--timestamps=true",
    ] {
        assert!(command.contains(option), "{option}: {command}");
    }
    assert!(!command.contains("--follow"));
    assert!(
        commands::logs("cluster", &pod(), &options, true)
            .unwrap()
            .contains("--follow=true")
    );
    let follow = commands::logs("cluster", &pod(), &options, true).unwrap();
    assert!(!follow.contains("--limit-bytes"));
    assert!(!follow.contains("--request-timeout"));
    assert!(commands::logs("cluster", &pod(), &LogOptions::default(), false).is_none());
}

#[test]
fn pods_report_readiness_restarts_waiting_reason_and_node() {
    let rows = model::resources(
        &json!({"items":[{"metadata":{"name":"web","namespace":"prod","labels":{"app":"web"}},
        "spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"nodeName":"node-1"},
        "status":{"phase":"Running","containerStatuses":[{"ready":true,"restartCount":2},
            {"ready":false,"restartCount":3,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}]})
        .to_string(),
        ResourceKind::Pod,
    )
    .unwrap();
    assert_eq!(rows[0].summary, "CrashLoopBackOff · 1/2 · ↻ 5 · node-1");
    assert!(rows[0].matches("app=web"));
    assert!(rows[0].matches("CRASHLOOP"));
    assert!(!rows[0].matches("staging"));
}

#[test]
fn basic_resource_kinds_parse_without_hiding_empty_or_invalid_lists() {
    for kind in ResourceKind::ALL {
        assert!(
            model::resources(r#"{"items":[]}"#, kind)
                .unwrap()
                .is_empty()
        );
        let rows = model::resources(r#"{"items":[{"metadata":{"name":"app","namespace":"ns"},"spec":{"replicas":3},"status":{"readyReplicas":2}}]}"#, kind).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].namespace, "ns");
        assert!(model::resources(r#"{"items":[{"metadata":{"name":"app"}}]}"#, kind).is_err());
    }
    let rows = model::resources(r#"{"items":[{"metadata":{"name":"web","namespace":"ns"},"spec":{"type":"ClusterIP","clusterIP":"10.0.0.1","ports":[{"port":80,"protocol":"TCP"}]}}]}"#, ResourceKind::Service).unwrap();
    assert_eq!(rows[0].summary, "ClusterIP · 10.0.0.1 · 80/TCP");
}

#[test]
fn container_discovery_includes_init_and_ephemeral_and_selectors_are_complete() {
    assert_eq!(
        model::containers(
            &json!({"spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"initContainers":[{"name":"init"}],"ephemeralContainers":[{"name":"debug"}]}})
        ),
        ["api", "sidecar", "init", "debug"]
    );
    let selector = model::pod_selector(&json!({"spec":{"selector":{"matchLabels":{"app":"web"},"matchExpressions":[{"key":"tier","operator":"In","values":["api","worker"]},{"key":"disabled","operator":"DoesNotExist"}]}}}), ResourceKind::Deployment).unwrap();
    assert_eq!(selector, "app=web,tier in (api,worker),!disabled");
    assert_eq!(
        model::pod_selector(
            &json!({"spec":{"selector":{"app":"web"}}}),
            ResourceKind::Service
        ),
        Some("app=web".into())
    );
    assert_eq!(
        model::pod_selector(&json!({"spec":{}}), ResourceKind::Service),
        None
    );
}

#[test]
fn log_search_is_literal_or_regex_and_never_silently_accepts_invalid_patterns() {
    let logs = "INFO ready\nERROR 请求失败\nWARN retry\nerror lower case\n";
    assert_eq!(
        filter(logs, "ERROR", false).unwrap(),
        ("ERROR 请求失败".into(), 1)
    );
    assert_eq!(filter(logs, "ERROR|WARN", true).unwrap().1, 2);
    assert_eq!(filter(logs, "ERROR|WARN", false).unwrap().1, 0);
    assert_eq!(filter(logs, "请求", false).unwrap().1, 1);
    assert!(filter(logs, "[", true).is_err());
    assert_eq!(filter(logs, "", false).unwrap().1, 4);
    assert_eq!(filter("", "", false).unwrap(), (String::new(), 0));
}

fn script(wire: &str) -> String {
    let encoded = wire
        .strip_prefix("sh -c 'printf \"%b\" \"")
        .unwrap()
        .strip_suffix("\" | sh'")
        .unwrap();
    let bytes = encoded
        .as_bytes()
        .as_chunks::<5>()
        .0
        .iter()
        .map(|chunk| {
            assert_eq!(&chunk[..2], b"\\0");
            u8::from_str_radix(std::str::from_utf8(&chunk[2..]).unwrap(), 8).unwrap()
        })
        .collect::<Vec<_>>();
    String::from_utf8(bytes).unwrap()
}

#[cfg(unix)]
#[test]
fn remote_scripts_run_in_sh_and_capture_stderr_and_exit_status() {
    let wire = commands::remote("printf 'hello\\n'; printf 'Forbidden\\n' >&2; (exit 7)");
    let output = crate::testing::sh(&wire, None);
    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        commands::answer(&output),
        Err("hello\nForbidden (exit 7)".into())
    );
    let context = "production'\"!$(printf INJECTED)";
    let wire = commands::remote(&format!(
        "kubectl() {{ printf '%s\\n' \"$@\"; }}; {} get pods",
        commands::kubectl(context)
    ));
    let output = crate::testing::sh(&wire, None);
    let output = commands::answer(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert!(
        output
            .lines()
            .any(|line| line == format!("--context={context}"))
    );
}
