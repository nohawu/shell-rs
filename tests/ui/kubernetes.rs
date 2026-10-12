//! The production Kubernetes browser, with injected SSH exec replies only.
use gpui_kit::{Focusable as _, SharedString};

use crate::support::*;

const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

const CONTEXTS: &str =
    "prod-cluster\n\n__SHELLRS_CONTEXTS__\nprod-cluster\ntest-cluster\n\n__SHELLRS_K8S_EXIT__:0\n";
const NAMESPACES: &str = "{\"items\":[{\"metadata\":{\"name\":\"prod\"}},{\"metadata\":{\"name\":\"test\"}}]}\n__SHELLRS_K8S_EXIT__:0\n";
const PODS: &str = r#"{"items":[
{"metadata":{"name":"api-0","namespace":"prod","labels":{"app":"api"}},"spec":{"containers":[{"name":"api"},{"name":"sidecar"}],"nodeName":"node-1"},"status":{"phase":"Running","containerStatuses":[{"ready":true},{"ready":true}]}},
{"metadata":{"name":"worker-0","namespace":"test"},"spec":{"containers":[{"name":"worker"}]},"status":{"phase":"Pending"}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const PROD_PODS: &str = r#"{"items":[{"metadata":{"name":"api-0","namespace":"prod"},"spec":{"containers":[{"name":"api"},{"name":"sidecar"}]},"status":{"phase":"Running"}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const DEPLOYMENTS: &str = r#"{"items":[{"metadata":{"name":"api","namespace":"prod"},"spec":{"replicas":3},"status":{"readyReplicas":2}}]}
__SHELLRS_K8S_EXIT__:0
"#;
const POD_DETAIL: &str = r#"{"kind":"Pod","metadata":{"name":"api-0","namespace":"prod"},"spec":{"containers":[{"name":"api","image":"api:v1"},{"name":"sidecar","image":"sidecar:v1"}],"initContainers":[{"name":"init"}],"ephemeralContainers":[{"name":"debug"}]},"status":{"phase":"Running"}}
__SHELLRS_K8S_EXIT__:0
"#;
const DEPLOY_DETAIL: &str = r#"{"kind":"Deployment","metadata":{"name":"api","namespace":"prod"},"spec":{"replicas":3,"selector":{"matchLabels":{"app":"api"}}},"status":{"readyReplicas":2}}
__SHELLRS_K8S_EXIT__:0
"#;
const LOGS: &str = "2026-10-12T01:00:00Z INFO ready\n2026-10-12T01:00:01Z ERROR 请求失败\n2026-10-12T01:00:02Z WARN retry\n\n__SHELLRS_K8S_EXIT__:0\n";
const SIDECAR_LOGS: &str = "sidecar ready\nsidecar healthy\n\n__SHELLRS_K8S_EXIT__:0\n";
const PREVIOUS_LOGS: &str = "previous container exited\n\n__SHELLRS_K8S_EXIT__:0\n";

fn open_kubernetes(
    cx: &mut TestAppContext,
    factory: Arc<FakeTerminalFactory>,
) -> WindowHandle<Root> {
    let (handle, _) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    handle
}

async fn label(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str, expected: &str) {
    let id = SharedString::from(id.to_owned());
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find(id.clone())
            .is_some_and(|element| element.label() == Some(expected))
    })
    .await;
}

async fn container(cx: &mut TestAppContext, handle: WindowHandle<Root>, expected: &str) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window
            .try_find("k8s-container")
            .is_some_and(|element| element.value() == Some(expected))
    })
    .await;
}

fn next_option(cx: &mut TestAppContext, handle: WindowHandle<Root>, id: &str) {
    let id = SharedString::from(id.to_owned());
    in_frame(cx, handle, |window, cx| {
        window.within(id).click("input", cx)
    });
    for key in ["down", "enter"] {
        in_frame(cx, handle, |window, cx| window.press(key, cx));
    }
}

fn script(wire: &str) -> String {
    let encoded = wire
        .strip_prefix("sh -c 'printf \"%b\" \"")
        .unwrap()
        .strip_suffix("\" | sh'")
        .unwrap();
    String::from_utf8(
        encoded
            .as_bytes()
            .as_chunks::<5>()
            .0
            .iter()
            .map(|chunk| u8::from_str_radix(std::str::from_utf8(&chunk[2..]).unwrap(), 8).unwrap())
            .collect(),
    )
    .unwrap()
}

#[gpui_kit::test]
async fn kubernetes_discovers_namespaces_and_filters_resource_queries(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        PROD_PODS,
        DEPLOYMENTS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-context").value(), Some("prod-cluster"));
        assert_eq!(window.find("k8s-namespace").value(), Some("全部命名空间"));
        assert!(window.find("k8s-resource:pods:prod/api-0").visible());
        assert!(window.find("k8s-resource:pods:test/worker-0").visible());
        assert!(
            window.find("k8s-context").bounds().bottom()
                <= window.find("k8s-namespace").bounds().top()
        );
    });
    assert!(script(&factory.exec_commands()[2]).contains("--all-namespaces"));
    next_option(cx, handle, "k8s-namespace");
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-namespace").value(), Some("prod"));
        assert!(window.try_find("k8s-resource:pods:test/worker-0").is_none());
    });
    assert!(script(&factory.exec_commands()[3]).contains("--namespace='prod'"));
    next_option(cx, handle, "k8s-kind");
    label(
        cx,
        handle,
        "k8s-resource:deployments:prod/api",
        "prod/api · 2/3",
    )
    .await;
    assert!(script(&factory.exec_commands()[4]).contains("get deployments"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-search", cx);
        window.input("missing", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(
            window.find("k8s-count").label(),
            Some("0 项资源 · 2 个命名空间")
        );
    });
    assert_eq!(factory.exec_count(), 5); // Text filtering runs locally.
}

#[gpui_kit::test]
async fn kubernetes_pod_logs_support_container_previous_and_regex_search(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        POD_DETAIL,
        LOGS,
        SIDECAR_LOGS,
        PREVIOUS_LOGS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    // Open from a real row, wait for the real detail query to discover containers.
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    container(cx, handle, "api").await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
    assert!(script(&factory.exec_commands()[4]).contains("--container='api'"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-log-search", cx);
        window.input("ERROR", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-log-count").label(), Some("匹配 1 / 3 行"));
        assert!(
            window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("请求失败")
        );
        assert!(
            !window
                .find("k8s-log-output")
                .label()
                .unwrap()
                .contains("INFO")
        );
    });
    in_frame(cx, handle, |window, cx| {
        window.press(SELECT_ALL, cx);
        window.input("ERROR|WARN", cx);
        window.click("k8s-regex", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-log-count").label(), Some("匹配 2 / 3 行"))
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-log-search", cx);
        window.press(SELECT_ALL, cx);
        window.input("[", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.find("k8s-regex-error").visible())
    });
    in_frame(cx, handle, |window, cx| {
        window.press(SELECT_ALL, cx);
        window.press("backspace", cx);
    });
    assert_eq!(factory.exec_count(), 5);
    next_option(cx, handle, "k8s-container");
    label(cx, handle, "k8s-log-count", "匹配 2 / 2 行").await;
    assert!(script(&factory.exec_commands()[5]).contains("--container='sidecar'"));
    in_frame(cx, handle, |window, cx| window.click("k8s-previous", cx));
    label(cx, handle, "k8s-log-count", "匹配 1 / 1 行").await;
    let query = script(&factory.exec_commands()[6]);
    assert!(query.contains("--container='sidecar'"));
    assert!(query.contains("--previous=true"));
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(2usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        let command = window
            .find("k8s-command:k8s.command.exec")
            .label()
            .unwrap()
            .to_owned();
        assert!(command.contains("--context='prod-cluster'"));
        assert!(command.contains("--namespace='prod' --container='sidecar' -- /bin/sh"));
        window.click("k8s-copy:k8s.command.exec", cx);
    });
    cx.update(|cx| {
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("--container='sidecar'")
        )
    });
}

#[gpui_kit::test]
async fn kubernetes_workloads_link_to_namespaced_pods_and_logs(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS,
        NAMESPACES,
        PODS,
        DEPLOYMENTS,
        DEPLOY_DETAIL,
        PROD_PODS,
        POD_DETAIL,
        LOGS,
    ]));
    let handle = open_kubernetes(cx, factory.clone());
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-kind");
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:deployments:prod/api:open", cx)
    });
    cx.wait_for(handle.into(), Duration::from_secs(5), |window, _| {
        window.try_find("k8s-related:api-0").is_some()
    })
    .await;
    let query = script(&factory.exec_commands()[5]);
    assert!(query.contains("--namespace='prod'"));
    assert!(query.contains("--selector='app=api'"));
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-related:api-0", cx)
    });
    label(
        cx,
        handle,
        "k8s-detail-scope",
        "Pod · prod/api-0 · prod-cluster",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(1usize, cx)
    });
    container(cx, handle, "api").await;
    label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
}

#[gpui_kit::test]
async fn kubernetes_details_close_and_reopen_from_every_tab(cx: &mut TestAppContext) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, POD_DETAIL, POD_DETAIL, POD_DETAIL, LOGS, POD_DETAIL, LOGS,
        POD_DETAIL, POD_DETAIL,
    ]));
    let handle = open_kubernetes(cx, factory);
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    for tab in 0usize..3 {
        for pointer in [true, false] {
            in_frame(cx, handle, |window, cx| {
                window.click("k8s-resource:pods:prod/api-0:open", cx)
            });
            label(
                cx,
                handle,
                "k8s-overview:k8s.images",
                "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
            )
            .await;
            in_frame(cx, handle, |window, cx| {
                window.within("k8s-detail-tabs").click(tab, cx)
            });
            if tab == 1 {
                label(cx, handle, "k8s-log-count", "匹配 3 / 3 行").await;
                in_frame(cx, handle, |window, cx| {
                    window.click("k8s-log-search", cx);
                    window.input("ERROR", cx);
                });
            } else if tab == 2 {
                in_frame(cx, handle, |window, cx| {
                    window.click("k8s-copy:k8s.command.exec", cx)
                });
            }
            in_frame(cx, handle, |window, cx| {
                if pointer {
                    window.click("k8s-detail-close", cx);
                } else {
                    window.press("escape", cx);
                }
            });
            in_frame(cx, handle, |window, _| {
                assert!(
                    window.try_find("k8s-details").is_none(),
                    "tab {tab} must close via {}",
                    if pointer { "close button" } else { "Escape" }
                );
                assert!(window.find("k8s-resource:pods:prod/api-0:open").visible());
                assert_eq!(window.find("kubernetes").focused(), Some(true));
            });
        }
    }
}

#[gpui_kit::test]
async fn kubernetes_details_close_button_works_if_focus_returns_to_terminal(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(FakeTerminalFactory::answering(&[
        CONTEXTS, NAMESPACES, PODS, POD_DETAIL,
    ]));
    let (handle, workspace) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory);
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    label(
        cx,
        handle,
        "k8s-overview:k8s.images",
        "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.within("k8s-detail-tabs").click(2usize, cx)
    });
    in_frame(cx, handle, |window, cx| {
        let focus = workspace
            .read(cx)
            .remote_terminal(shellrs::terminal::RemoteTerminalId(INITIAL_WEB_TERMINAL))
            .unwrap()
            .read(cx)
            .focus_handle(cx);
        window.focus(&focus, cx);
        window.click("k8s-detail-close", cx);
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
        assert_eq!(window.find("kubernetes").focused(), Some(true));
    });
}

#[gpui_kit::test]
async fn kubernetes_rbac_failures_are_visible_without_partial_or_automatic_queries(
    cx: &mut TestAppContext,
) {
    let denied = "Error from server (Forbidden): namespaces is forbidden\n__SHELLRS_K8S_EXIT__:1\n";
    let factory = Arc::new(FakeTerminalFactory::answering(&[CONTEXTS, denied]));
    let handle = open_kubernetes(cx, factory.clone());
    label(
        cx,
        handle,
        "k8s-error",
        "Error from server (Forbidden): namespaces is forbidden (exit 1)",
    )
    .await;
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(factory.exec_count(), 2);
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-resource:pods:prod/api-0").is_none())
    });
}

#[derive(Default)]
struct PendingFactory {
    pending: Arc<Mutex<std::collections::VecDeque<shellrs::terminal::ExecRequest>>>,
    commands: Arc<Mutex<Vec<String>>>,
}

struct PendingTransport {
    pending: Arc<Mutex<std::collections::VecDeque<shellrs::terminal::ExecRequest>>>,
    commands: Arc<Mutex<Vec<String>>>,
}

impl TerminalTransportFactory for PendingFactory {
    fn create(&self) -> Box<dyn TerminalTransport> {
        Box::new(PendingTransport {
            pending: self.pending.clone(),
            commands: self.commands.clone(),
        })
    }
}

impl TerminalTransport for PendingTransport {
    fn run(
        self: Box<Self>,
        _: TerminalSize,
        commands: mpsc::Receiver<TerminalTransportCommand>,
        events: async_channel::Sender<TerminalTransportEvent>,
    ) -> anyhow::Result<()> {
        events.send_blocking(TerminalTransportEvent::Started)?;
        while let Ok(command) = commands.recv() {
            match command {
                TerminalTransportCommand::Exec(request) => {
                    self.commands.lock().unwrap().push(request.command.clone());
                    self.pending.lock().unwrap().push_back(request);
                }
                TerminalTransportCommand::Shutdown => break,
                _ => {}
            }
        }
        Ok(())
    }
}

async fn reply(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    factory: &PendingFactory,
    output: &str,
) {
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        !factory.pending.lock().unwrap().is_empty()
    })
    .await;
    let pending = factory.pending.lock().unwrap().pop_front().unwrap();
    // An obsolete query intentionally has no receiver left.
    let _ = pending.reply.send(Ok(output.to_owned()));
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn kubernetes_details_close_while_loading_and_on_backdrop(cx: &mut TestAppContext) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    in_frame(cx, handle, |window, cx| {
        assert!(window.find("k8s-details").visible());
        window.click("k8s-detail-close", cx);
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
        assert!(window.find("k8s-resource:pods:prod/api-0:open").visible());
    });
    in_frame(cx, handle, |window, cx| {
        window.click("k8s-resource:pods:prod/api-0:open", cx)
    });
    reply(cx, handle, &factory, POD_DETAIL).await;
    label(
        cx,
        handle,
        "k8s-overview:k8s.images",
        "api: api:v1\nsidecar: sidecar:v1\ninit: \ndebug: ",
    )
    .await;
    in_frame(cx, handle, |window, cx| {
        window.click_at(
            "dialog",
            gpui_kit::point(gpui_kit::px(8.), gpui_kit::px(100.)),
            cx,
        );
    });
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-details").is_none());
    });
}

#[gpui_kit::test]
async fn kubernetes_finishes_namespace_discovery_when_resource_type_changes(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    next_option(cx, handle, "k8s-kind");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    reply(cx, handle, &factory, NAMESPACES).await; // Cancelled discovery.
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-namespace");
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-kind").value(), Some("Deployment"));
        assert_eq!(window.find("k8s-namespace").value(), Some("prod"));
    });
    let queries = factory.commands.lock().unwrap();
    let query = script(queries.last().unwrap());
    assert!(query.contains("get deployments"));
    assert!(query.contains("--namespace='prod'"));
}

#[gpui_kit::test]
async fn kubernetes_rejects_old_scope_replies_and_explicitly_switches_context(
    cx: &mut TestAppContext,
) {
    let factory = Arc::new(PendingFactory::default());
    let (handle, _) = open_workspace_with_remote_factory(cx, HostStore::seed(), factory.clone());
    in_frame(cx, handle, |window, cx| {
        window.click(("terminal-tab", INITIAL_WEB_TERMINAL), cx)
    });
    in_frame(cx, handle, |window, cx| window.click("tool-kubernetes", cx));
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, PODS).await;
    label(cx, handle, "k8s-count", "2 项资源 · 2 个命名空间").await;
    next_option(cx, handle, "k8s-namespace");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 1
    })
    .await;
    next_option(cx, handle, "k8s-kind");
    cx.wait_for(handle.into(), Duration::from_secs(5), |_, _| {
        factory.pending.lock().unwrap().len() == 2
    })
    .await;
    reply(cx, handle, &factory, PROD_PODS).await; // Previous Pod scope.
    in_frame(cx, handle, |window, _| {
        assert!(window.try_find("k8s-resource:pods:prod/api-0").is_none())
    });
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(
        cx,
        handle,
        "k8s-resource:deployments:prod/api",
        "prod/api · 2/3",
    )
    .await;
    next_option(cx, handle, "k8s-context");
    reply(cx, handle, &factory, CONTEXTS).await;
    reply(cx, handle, &factory, NAMESPACES).await;
    reply(cx, handle, &factory, DEPLOYMENTS).await;
    label(cx, handle, "k8s-count", "1 项资源 · 2 个命名空间").await;
    in_frame(cx, handle, |window, _| {
        assert_eq!(window.find("k8s-context").value(), Some("test-cluster"));
        assert_eq!(window.find("k8s-namespace").value(), Some("全部命名空间"));
    });
    let queries = factory.commands.lock().unwrap();
    let query = script(queries.last().unwrap());
    assert!(query.contains("--context='test-cluster'"));
    assert!(query.contains("--all-namespaces"));
}
