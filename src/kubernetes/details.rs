use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use unicode_width::UnicodeWidthStr as _;

use super::{
    commands::{self, LogOptions},
    logs::{self, LogLine, Token},
    model::{self, KubernetesCommand, Resource, ResourceKind},
    panel::{defer_dispatch, request},
    presentation::{copy_button, reading, tone_color},
};
use crate::app::{CopyCommand, KubernetesAction};
use crate::i18n::{UiLocale, t};
use crate::shared::soft_tag;
use crate::terminal::ExecTarget;

/// The dialog pins its context, namespace, resource and SSH transport.
/// Closing it drops its reads; changing log options invalidates old replies.
pub(super) struct KubernetesDetails {
    resource: Resource,
    context: String,
    target: ExecTarget,
    dispatch: FocusHandle,
    tab: usize,
    details: Entity<EditorState>,
    overview: Vec<(&'static str, String)>,
    output: String,
    log_lines: Rc<Vec<LogLine>>,
    log_scroll: UniformListScrollHandle,
    log_measure: usize,
    live: bool,
    follow: bool,
    clipped: bool,
    query: Entity<InputState>,
    containers: Vec<String>,
    container_select: Entity<SelectState<Vec<SharedString>>>,
    tail_select: Entity<SelectState<Vec<SharedString>>>,
    since_select: Entity<SelectState<Vec<SharedString>>>,
    options: LogOptions,
    regex: bool,
    logs: Option<Rc<String>>,
    matches: usize,
    total: usize,
    filter_problem: Option<String>,
    problem: Option<String>,
    log_problem: Option<String>,
    pods: Rc<Vec<Resource>>,
    pod_problem: Option<String>,
    pods_loading: bool,
    has_selector: bool,
    pod_scroll: UniformListScrollHandle,
    loading: bool,
    logs_loading: bool,
    generation: u64,
    log_generation: u64,
    task: Option<Task<()>>,
    log_task: Option<Task<()>>,
    pod_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl KubernetesDetails {
    pub fn new(
        resource: Resource,
        context: String,
        target: ExecTarget,
        dispatch: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let details = cx.new(|cx| EditorState::new(window, cx).language("json"));
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(t!("k8s.log_search")));
        let container_select =
            cx.new(|cx| SelectState::new(Vec::<SharedString>::new(), None, window, cx));
        let tail_select = cx.new(|cx| {
            SelectState::new(
                ["100", "500", "1000", "5000", "10000"]
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
                Some(IndexPath::new(2)),
                window,
                cx,
            )
        });
        let since_select = cx.new(|cx| {
            SelectState::new(
                ["5m", "15m", "1h", "6h", "24h"]
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>(),
                Some(IndexPath::new(2)),
                window,
                cx,
            )
        });
        let mut subscriptions = Vec::new();
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &container_select,
            window,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Container(value.to_string())),
                        window,
                        cx,
                    );
                }
            },
        ));
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &tail_select,
            window,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && let Ok(tail) = value.parse()
                {
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Tail(tail)),
                        window,
                        cx,
                    );
                }
            },
        ));
        let handle = dispatch.clone();
        subscriptions.push(cx.subscribe_in(
            &since_select,
            window,
            move |_, _, event: &SelectEvent<Vec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    defer_dispatch(
                        &handle,
                        &KubernetesAction(KubernetesCommand::Since(value.to_string())),
                        window,
                        cx,
                    );
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &query,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_logs(true, window, cx);
                }
            },
        ));
        subscriptions.push(
            cx.observe_global_in::<UiLocale>(window, |this, window, cx| {
                this.query.update(cx, |input, cx| {
                    input.set_placeholder(t!("k8s.log_search"), window, cx)
                });
            }),
        );
        let mut view = Self {
            resource,
            context,
            target,
            dispatch,
            tab: 0,
            details,
            overview: Vec::new(),
            output: String::new(),
            log_lines: Rc::new(Vec::new()),
            log_scroll: UniformListScrollHandle::new(),
            log_measure: 0,
            live: false,
            follow: true,
            clipped: false,
            query,
            containers: Vec::new(),
            container_select,
            tail_select,
            since_select,
            options: LogOptions::default(),
            regex: false,
            logs: None,
            matches: 0,
            total: 0,
            filter_problem: None,
            problem: None,
            log_problem: None,
            pods: Rc::new(Vec::new()),
            pod_problem: None,
            pods_loading: false,
            has_selector: false,
            pod_scroll: UniformListScrollHandle::new(),
            loading: false,
            logs_loading: false,
            generation: 0,
            log_generation: 0,
            task: None,
            log_task: None,
            pod_task: None,
            _subscriptions: subscriptions,
        };
        view.read_details(window, cx);
        view
    }

    pub fn command(
        &mut self,
        command: &KubernetesCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            KubernetesCommand::DetailTab(tab) if *tab <= 2 => {
                self.tab = *tab;
                if self.tab != 1 {
                    self.log_generation += 1;
                    self.log_task = None;
                    self.logs_loading = false;
                }
                if self.tab == 1
                    && self.resource.kind == ResourceKind::Pod
                    && (self.logs.is_none() || self.live)
                    && !self.logs_loading
                {
                    self.read_logs(window, cx);
                }
            }
            KubernetesCommand::Container(name) if self.containers.contains(name) => {
                if self.options.container != *name {
                    self.options.container = name.clone();
                    self.change_logs(window, cx);
                }
            }
            KubernetesCommand::Tail(tail) if [100, 500, 1000, 5000, 10000].contains(tail) => {
                if self.options.tail != *tail {
                    self.options.tail = *tail;
                    self.change_logs(window, cx);
                }
            }
            KubernetesCommand::Since(since)
                if ["5m", "15m", "1h", "6h", "24h"].contains(&since.as_str()) =>
            {
                if self.options.since != *since {
                    self.options.since = since.clone();
                    self.change_logs(window, cx);
                }
            }
            KubernetesCommand::Previous(previous) => {
                if self.options.previous != *previous {
                    self.options.previous = *previous;
                    if *previous {
                        self.live = false;
                    }
                    self.change_logs(window, cx);
                }
            }
            KubernetesCommand::Regex(regex) => {
                self.regex = *regex;
                self.filter_logs(true, window, cx);
            }
            KubernetesCommand::ReadLogs => self.read_logs(window, cx),
            KubernetesCommand::LiveLogs(live) if !self.options.previous => {
                self.live = *live;
                self.log_generation += 1;
                self.log_task = None;
                self.logs_loading = false;
                if self.live && self.tab == 1 {
                    self.read_logs(window, cx);
                }
            }
            KubernetesCommand::FollowLogs(follow) => {
                self.follow = *follow;
                if self.follow {
                    self.log_scroll.scroll_to_bottom();
                }
            }
            KubernetesCommand::RefreshDetails => self.read_details(window, cx),
            KubernetesCommand::RelatedPod(name) => {
                if let Some(pod) = self.pods.iter().find(|pod| &pod.name == name).cloned() {
                    self.resource = pod;
                    self.live = false;
                    self.tab = 0;
                    self.overview.clear();
                    self.details
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    self.containers.clear();
                    self.options.container.clear();
                    self.change_logs(window, cx);
                    self.read_details(window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn change_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.log_generation += 1;
        self.log_task = None;
        self.logs_loading = false;
        self.logs = None;
        self.clipped = false;
        self.log_problem = None;
        self.filter_logs(true, window, cx);
        if self.tab == 1 {
            self.read_logs(window, cx);
        }
    }

    fn read_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self.task = None;
        self.pod_task = None;
        self.loading = true;
        self.problem = None;
        self.pods = Rc::new(Vec::new());
        self.pod_problem = None;
        self.pods_loading = false;
        self.has_selector = false;
        let (target, context, resource, generation) = (
            self.target.clone(),
            self.context.clone(),
            self.resource.clone(),
            self.generation,
        );
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let output = request(&target, commands::detail(&context, &resource), cx).await
                .and_then(|output| commands::answer(&output))
                .and_then(|output| serde_json::from_str::<Value>(&output).map_err(|error| error.to_string()))
                .and_then(|value| {
                    if value["metadata"]["name"].as_str() != Some(&resource.name)
                        || value["metadata"]["namespace"].as_str() != Some(&resource.namespace) {
                        Err(t!("k8s.incomplete").to_string())
                    } else { Ok(value) }
                });
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation { return; }
                this.loading = false;
                match output {
                    Err(why) => this.problem = Some(why),
                    Ok(value) => {
                        this.overview = model::overview(&value, this.resource.kind);
                        this.resource.status = model::status(&value, this.resource.kind);
                        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                        this.details.update(cx, |state, cx| state.set_value(text, window, cx));
                        let names = model::containers(&value);
                        let selected = if names.contains(&this.options.container) { this.options.container.clone() }
                            else { value["metadata"]["annotations"]["kubectl.kubernetes.io/default-container"].as_str()
                                .filter(|name| names.iter().any(|container| container == name))
                                .map(str::to_owned).or_else(|| names.first().cloned()).unwrap_or_default() };
                        let changed = selected != this.options.container;
                        this.containers = names;
                        this.options.container = selected;
                        this.container_select.update(cx, |state, cx| {
                            state.set_items(this.containers.iter().cloned().map(SharedString::from).collect(), window, cx);
                            state.set_selected_index(this.containers.iter().position(|name| name == &this.options.container).map(IndexPath::new), window, cx);
                        });
                        if changed { this.change_logs(window, cx); }
                        if let Some(selector) = model::pod_selector(&value, this.resource.kind) {
                            this.has_selector = true;
                            this.read_pods(selector, window, cx);
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    fn read_pods(&mut self, selector: String, window: &mut Window, cx: &mut Context<Self>) {
        self.pods_loading = true;
        let (target, generation) = (self.target.clone(), self.generation);
        let command = commands::list(
            &self.context,
            Some(&self.resource.namespace),
            ResourceKind::Pod,
            Some(&selector),
        );
        self.pod_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = request(&target, command, cx)
                .await
                .and_then(|output| commands::answer(&output))
                .and_then(|output| model::resources(&output, ResourceKind::Pod));
            let _ = this.update_in(cx, |this, _, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(pods) => this.pods = Rc::new(pods),
                    Err(why) => this.pod_problem = Some(why),
                }
                this.pods_loading = false;
                cx.notify();
            });
        }));
    }

    fn read_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = commands::logs(&self.context, &self.resource, &self.options, false)
        else {
            return;
        };
        self.log_generation += 1;
        self.log_task = None;
        self.logs_loading = true;
        self.log_problem = None;
        let (target, generation) = (self.target.clone(), self.log_generation);
        self.log_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let result = request(&target, commands::remote(&command), cx)
                    .await
                    .and_then(|output| commands::answer(&output));
                let keep_reading = this
                    .update_in(cx, |this, window, cx| {
                        if this.log_generation != generation {
                            return false;
                        }
                        let mut changed = this.logs_loading;
                        this.logs_loading = false;
                        match result {
                            Ok(text) => {
                                let (text, clipped) = logs::bounded(text);
                                changed |= this.clipped != clipped;
                                this.clipped = clipped;
                                if this.logs.as_ref().is_none_or(|old| old.as_str() != text) {
                                    changed = true;
                                    this.logs = Some(Rc::new(text));
                                    this.filter_logs(false, window, cx);
                                }
                            }
                            Err(why) => {
                                changed = true;
                                this.log_problem = Some(why);
                                this.live = false;
                            }
                        }
                        if changed {
                            cx.notify();
                        }
                        this.live && this.tab == 1 && !this.options.previous
                    })
                    .unwrap_or(false);
                if !keep_reading {
                    break;
                }
                // One bounded request at a time. Closing/changing scope drops
                // this task, including its timer and reply receiver.
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        }));
    }

    fn filter_logs(&mut self, reset_scroll: bool, _: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value();
        let logs = self.logs.as_ref().map_or("", |text| text.as_str());
        self.total = logs.lines().count();
        match filter(logs, &query, self.regex) {
            Ok((text, matches)) => {
                self.matches = matches;
                self.filter_problem = None;
                if self.output != text {
                    self.log_lines = Rc::new(text.lines().map(LogLine::new).collect());
                    self.log_measure = self
                        .log_lines
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, line)| line.text.width())
                        .map_or(0, |(ix, _)| ix);
                    self.output = text;
                    if self.live && self.follow {
                        self.log_scroll.scroll_to_bottom();
                    } else if reset_scroll {
                        self.log_scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                }
            }
            Err(why) => {
                self.matches = 0;
                self.filter_problem = Some(why);
                self.output.clear();
                self.log_lines = Rc::new(Vec::new());
            }
        }
        cx.notify();
    }

    fn render_pods(&self, cx: &App) -> AnyElement {
        let pods = self.pods.clone();
        let dispatch = self.dispatch.clone();
        v_flex()
            .gap_2()
            .when(self.resource.kind != ResourceKind::Pod, |body| {
                body.child(div().text_sm().child(t!("k8s.related_pods")))
                    .when(self.pods_loading || self.loading, |body| {
                        body.child(reading("k8s-pods-reading", cx))
                    })
                    .when(
                        !self.loading
                            && !self.pods_loading
                            && pods.is_empty()
                            && self.pod_problem.is_none(),
                        |body| {
                            body.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if self.has_selector {
                                        t!("k8s.no_pods")
                                    } else {
                                        t!("k8s.no_selector")
                                    }),
                            )
                        },
                    )
            })
            .when_some(self.pod_problem.clone(), |body, why| {
                body.child(error("k8s-pods-error", &why, cx))
            })
            .when(!pods.is_empty(), |body| {
                body.child(
                    div()
                        .h(rems(6.))
                        .child(
                            uniform_list("k8s-related-pods", pods.len(), move |range, _, cx| {
                                range
                                    .map(|ix| {
                                        let pod = &pods[ix];
                                        let handle = dispatch.clone();
                                        let name = pod.name.clone();
                                        h_flex()
                                            .px_1()
                                            .gap_2()
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "k8s-related:{}",
                                                    pod.name
                                                )))
                                                .ghost()
                                                .small()
                                                .flex_1()
                                                .min_w_0()
                                                .justify_start()
                                                .label(format!("{} · {}", pod.name, pod.summary))
                                                .on_click(move |_, window, cx| {
                                                    defer_dispatch(
                                                        &handle,
                                                        &KubernetesAction(
                                                            KubernetesCommand::RelatedPod(
                                                                name.clone(),
                                                            ),
                                                        ),
                                                        window,
                                                        cx,
                                                    )
                                                }),
                                            )
                                            .when_some(pod.status.clone(), |row, (label, tone)| {
                                                row.child(soft_tag(label, tone_color(tone, cx)))
                                            })
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .size_full()
                            .track_scroll(&self.pod_scroll),
                        )
                        .vertical_scrollbar(&self.pod_scroll),
                )
            })
            .into_any_element()
    }

    fn render_container(&self, _: &App) -> AnyElement {
        h_flex()
            .gap_2()
            .child(div().text_sm().child(t!("k8s.container")))
            .child(
                Select::new(&self.container_select)
                    .id("k8s-container")
                    .small()
                    .flex_1()
                    .disabled(self.containers.is_empty()),
            )
            .into_any_element()
    }

    fn render_logs(&self, cx: &App) -> AnyElement {
        if self.resource.kind != ResourceKind::Pod {
            return v_flex()
                .gap_3()
                .child(t!("k8s.choose_pod"))
                .child(self.render_pods(cx))
                .into_any_element();
        }
        let (read, previous, regex) = (
            self.dispatch.clone(),
            self.dispatch.clone(),
            self.dispatch.clone(),
        );
        let count = t!("k8s.log_count", count = self.matches, total = self.total);
        let live = self.dispatch.clone();
        let follow = self.dispatch.clone();
        let lines = self.log_lines.clone();
        let dispatch = self.dispatch.clone();
        let measure = self.log_measure;
        v_flex()
            .gap_3()
            .child(self.render_container(cx))
            .child(
                h_flex()
                    .gap_3()
                    .child(div().text_sm().child(t!("k8s.tail")))
                    .child(
                        Select::new(&self.tail_select)
                            .id("k8s-tail")
                            .small()
                            .w(rems(7.)),
                    )
                    .child(div().text_sm().child(t!("k8s.since")))
                    .child(
                        Select::new(&self.since_select)
                            .id("k8s-since")
                            .small()
                            .w(rems(7.)),
                    )
                    .child(
                        Checkbox::new("k8s-previous")
                            .label(t!("k8s.previous"))
                            .checked(self.options.previous)
                            .on_click(move |value, window, cx| {
                                defer_dispatch(
                                    &previous,
                                    &KubernetesAction(KubernetesCommand::Previous(*value)),
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .child(
                        Button::new("k8s-read-logs")
                            .small()
                            .label(t!("k8s.read_logs"))
                            .loading(self.logs_loading)
                            .disabled(self.options.container.is_empty() || self.loading)
                            .on_click(move |_, window, cx| {
                                defer_dispatch(
                                    &read,
                                    &KubernetesAction(KubernetesCommand::ReadLogs),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Input::new(&self.query)
                            .id("k8s-log-search")
                            .small()
                            .flex_1(),
                    )
                    .child(
                        Checkbox::new("k8s-regex")
                            .label(t!("k8s.regex"))
                            .checked(self.regex)
                            .on_click(move |value, window, cx| {
                                defer_dispatch(
                                    &regex,
                                    &KubernetesAction(KubernetesCommand::Regex(*value)),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        Checkbox::new("k8s-live")
                            .label(t!("k8s.live"))
                            .checked(self.live)
                            .disabled(
                                !self.live
                                    && (self.options.previous
                                        || self.loading
                                        || self.options.container.is_empty()),
                            )
                            .on_click(move |value, window, cx| {
                                defer_dispatch(
                                    &live,
                                    &KubernetesAction(KubernetesCommand::LiveLogs(*value)),
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .child(
                        Checkbox::new("k8s-follow")
                            .label(t!("k8s.follow"))
                            .checked(self.follow)
                            .on_click(move |value, window, cx| {
                                defer_dispatch(
                                    &follow,
                                    &KubernetesAction(KubernetesCommand::FollowLogs(*value)),
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .child(soft_tag("INFO", cx.theme().success))
                    .child(soft_tag("WARN", cx.theme().warning))
                    .child(soft_tag("ERROR", cx.theme().danger))
                    .child(
                        copy_button("k8s-copy-logs", self.output.clone(), &self.dispatch)
                            .label(t!("k8s.copy_logs"))
                            .disabled(self.output.is_empty()),
                    ),
            )
            .child(
                div()
                    .id("k8s-log-count")
                    .test_support()
                    .aria_label(count.clone())
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(count),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("k8s.log_limit")),
            )
            .when_some(self.log_problem.clone(), |body, why| {
                body.child(error("k8s-logs-error", &why, cx))
            })
            .when(self.logs_loading, |body| {
                body.child(reading("k8s-logs-reading", cx))
            })
            .when(self.clipped, |body| {
                body.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(t!("k8s.log_clipped")),
                )
            })
            .when(
                self.logs.is_some()
                    && !self.logs_loading
                    && self.filter_problem.is_none()
                    && self.log_problem.is_none()
                    && self.matches == 0,
                |body| {
                    body.child(div().text_sm().child(if self.total == 0 {
                        t!("k8s.no_logs")
                    } else {
                        t!("k8s.no_log_matches")
                    }))
                },
            )
            .when_some(self.filter_problem.clone(), |body, why| {
                body.child(error("k8s-regex-error", &why, cx))
            })
            .child(
                div()
                    .id("k8s-log-output")
                    .test_support()
                    .aria_label(self.output.clone())
                    .h(rems(20.))
                    .min_w_0()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(
                        uniform_list("k8s-log-lines", lines.len(), move |range, _, cx| {
                            range
                                .map(|ix| {
                                    let line = &lines[ix];
                                    let highlights = line.spans.iter().map(|(range, token)| {
                                        (range.clone(), log_style(*token, cx))
                                    });
                                    h_flex()
                                        .gap_2()
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .whitespace_nowrap()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(copy_button(
                                            SharedString::from(format!("k8s-copy-line:{ix}")),
                                            line.raw.clone(),
                                            &dispatch,
                                        ))
                                        .child(
                                            StyledText::new(line.text.clone())
                                                .with_highlights(highlights),
                                        )
                                })
                                .collect::<Vec<_>>()
                        })
                        .size_full()
                        .with_horizontal_sizing_behavior(
                            ListHorizontalSizingBehavior::Unconstrained,
                        )
                        .with_width_from_item(Some(measure))
                        .track_scroll(&self.log_scroll),
                    )
                    .vertical_scrollbar(&self.log_scroll),
            )
            .into_any_element()
    }

    fn render_commands(&self, cx: &App) -> AnyElement {
        let dispatch = self.dispatch.clone();
        v_flex()
            .gap_3()
            .when(self.resource.kind == ResourceKind::Pod, |body| {
                body.child(self.render_container(cx))
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("k8s.command_note")),
            )
            .children(
                commands::suggestions(&self.context, &self.resource, &self.options)
                    .into_iter()
                    .map(|(key, command)| {
                        let handle = dispatch.clone();
                        let copy = command.clone();
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().flex_1().text_sm().child(t!(key)))
                                    .child(
                                        Button::new(SharedString::from(format!("k8s-copy:{key}")))
                                            .ghost()
                                            .small()
                                            .label(t!("history.menu.copy"))
                                            .on_click(move |_, window, cx| {
                                                defer_dispatch(
                                                    &handle,
                                                    &CopyCommand(copy.clone()),
                                                    window,
                                                    cx,
                                                )
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("k8s-command:{key}")))
                                    .test_support()
                                    .aria_label(command.clone())
                                    .p_2()
                                    .bg(cx.theme().muted)
                                    .rounded(cx.theme().radius)
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_xs()
                                    .child(command),
                            )
                    }),
            )
            .into_any_element()
    }
}

impl Render for KubernetesDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.dispatch.clone();
        let refresh = self.dispatch.clone();
        let resource = format!(
            "{} · {}/{} · {}",
            self.resource.kind.label(),
            self.resource.namespace,
            self.resource.name,
            self.context
        );
        v_flex()
            .id("k8s-details")
            .test_support()
            .gap_3()
            .min_w_0()
            .child(
                div()
                    .id("k8s-detail-scope")
                    .test_support()
                    .aria_label(resource.clone())
                    .text_sm()
                    .child(resource),
            )
            .when_some(self.resource.status.clone(), |body, (label, tone)| {
                body.child(h_flex().child(soft_tag(label, tone_color(tone, cx))))
            })
            .child(
                TabBar::new("k8s-detail-tabs")
                    .underline()
                    .selected_index(self.tab)
                    .child(Tab::new().label(t!("k8s.details")))
                    .child(Tab::new().label(t!("k8s.logs")))
                    .child(Tab::new().label(t!("k8s.commands")))
                    .on_click(move |ix, window, cx| {
                        defer_dispatch(
                            &handle,
                            &KubernetesAction(KubernetesCommand::DetailTab(*ix)),
                            window,
                            cx,
                        )
                    }),
            )
            .when_some(self.problem.clone(), |body, why| {
                body.child(error("k8s-detail-error", &why, cx))
            })
            .map(|body| match self.tab {
                1 => body.child(self.render_logs(cx)),
                2 => body.child(self.render_commands(cx)),
                _ => body
                    .children(self.overview.iter().map(|(key, value)| {
                        h_flex()
                            .items_start()
                            .gap_3()
                            .text_sm()
                            .child(
                                div()
                                    .w(rems(8.))
                                    .flex_shrink_0()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!(*key)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("k8s-overview:{key}")))
                                    .test_support()
                                    .aria_label(value.clone())
                                    .flex_1()
                                    .min_w_0()
                                    .child(value.clone()),
                            )
                            .child(copy_button(
                                SharedString::from(format!("k8s-copy-field:{key}")),
                                value.clone(),
                                &self.dispatch,
                            ))
                    }))
                    .child(self.render_pods(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("k8s.json")),
                            )
                            .child(
                                Button::new("k8s-detail-refresh")
                                    .ghost()
                                    .small()
                                    .label(t!("tools.refresh"))
                                    .loading(self.loading)
                                    .on_click(move |_, window, cx| {
                                        defer_dispatch(
                                            &refresh,
                                            &KubernetesAction(KubernetesCommand::RefreshDetails),
                                            window,
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .when(self.loading, |body| {
                        body.child(reading("k8s-details-reading", cx))
                    })
                    .child(
                        div().id("k8s-detail-json").test_support().child(
                            Editor::new(&self.details)
                                .readonly(true)
                                .h(rems(12.))
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_xs(),
                        ),
                    ),
            })
    }
}

fn log_style(token: Token, cx: &App) -> HighlightStyle {
    let syntax = &cx.theme().highlight_theme.style.syntax;
    let color = match token {
        Token::Timestamp | Token::Debug => cx.theme().muted_foreground,
        Token::Info => cx.theme().success,
        Token::Warning => cx.theme().warning,
        Token::Error => cx.theme().danger,
        Token::Key => {
            return syntax.style("property").unwrap_or(HighlightStyle {
                color: Some(cx.theme().primary),
                ..Default::default()
            });
        }
        Token::String => {
            return syntax.style("string").unwrap_or(HighlightStyle {
                color: Some(cx.theme().success),
                ..Default::default()
            });
        }
        Token::Number => {
            return syntax.style("number").unwrap_or(HighlightStyle {
                color: Some(cx.theme().warning),
                ..Default::default()
            });
        }
        Token::Literal => {
            return syntax.style("boolean").unwrap_or(HighlightStyle {
                color: Some(cx.theme().primary),
                ..Default::default()
            });
        }
    };
    HighlightStyle {
        color: Some(color),
        font_weight: matches!(token, Token::Error | Token::Warning).then_some(FontWeight::SEMIBOLD),
        ..Default::default()
    }
}

fn error(id: &'static str, why: &str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .aria_label(why.to_owned())
        .text_sm()
        .text_color(cx.theme().danger)
        .child(why.to_owned())
        .into_any_element()
}

/// Searches only the retrieved log snapshot. Query text never reaches a shell.
pub(super) fn filter(logs: &str, query: &str, regex: bool) -> Result<(String, usize), String> {
    let pattern = if regex && !query.is_empty() {
        Some(
            regex::RegexBuilder::new(query)
                .size_limit(1024 * 1024)
                .build()
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let lines = logs
        .lines()
        .filter(|line| {
            pattern
                .as_ref()
                .map_or_else(|| line.contains(query), |pattern| pattern.is_match(line))
        })
        .collect::<Vec<_>>();
    let count = lines.len();
    Ok((lines.join("\n"), count))
}
